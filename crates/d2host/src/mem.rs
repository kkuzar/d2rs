//! In-memory file table (thread-local). Keys are normalised absolute `/` paths; folders are implied by keys plus markers.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

enum Data {
    Bytes(Vec<u8>),
    #[cfg(target_arch = "wasm32")]
    Js(js_sys::Uint8Array),
}

struct Entry {
    data: Data,
    ver: u64,
}

#[derive(Default)]
struct Table {
    files: BTreeMap<String, Entry>,
    dirs: BTreeSet<String>,
    clock: u64,
}

thread_local! {
    static T: RefCell<Table> = RefCell::default();
    static ON: Cell<bool> = const { Cell::new(cfg!(target_arch = "wasm32")) };
}

pub fn on() -> bool {
    ON.get()
}

pub fn use_memory() {
    ON.set(true);
}

fn err<T>(k: ErrorKind) -> io::Result<T> {
    Err(k.into())
}

fn key(p: &Path) -> String {
    let mut v: Vec<String> = vec![];
    for c in p.components() {
        match c {
            Component::Normal(s) => v.push(s.to_string_lossy().into_owned()),
            Component::ParentDir => drop(v.pop()),
            _ => {}
        }
    }
    format!("/{}", v.join("/"))
}

fn under(k: &str, dir: &str) -> bool {
    dir == "/" || k.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

fn with<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    T.with(|t| f(&mut t.borrow_mut()))
}

fn put(t: &mut Table, k: String, data: Data) {
    t.clock += 1;
    let ver = t.clock;
    t.files.insert(k, Entry { data, ver });
}

fn bytes(e: &Entry) -> Vec<u8> {
    match &e.data {
        Data::Bytes(b) => b.clone(),
        #[cfg(target_arch = "wasm32")]
        Data::Js(a) => a.to_vec(),
    }
}

pub fn read(p: &Path) -> io::Result<Vec<u8>> {
    with(|t| t.files.get(&key(p)).map(bytes).ok_or_else(|| ErrorKind::NotFound.into()))
}

pub fn read_at(p: &Path, off: u64, buf: &mut [u8]) -> io::Result<()> {
    with(|t| {
        let Some(e) = t.files.get(&key(p)) else { return err(ErrorKind::NotFound) };
        let end = off.checked_add(buf.len() as u64).ok_or(ErrorKind::UnexpectedEof)?;
        match &e.data {
            Data::Bytes(b) if end <= b.len() as u64 => buf.copy_from_slice(&b[off as usize..end as usize]),
            #[cfg(target_arch = "wasm32")]
            Data::Js(a) if end <= u64::from(a.length()) => a.subarray(off as u32, end as u32).copy_to(buf),
            _ => return err(ErrorKind::UnexpectedEof),
        }
        Ok(())
    })
}

pub fn len(p: &Path) -> io::Result<u64> {
    with(|t| match t.files.get(&key(p)) {
        Some(Entry { data: Data::Bytes(b), .. }) => Ok(b.len() as u64),
        #[cfg(target_arch = "wasm32")]
        Some(Entry { data: Data::Js(a), .. }) => Ok(u64::from(a.length())),
        None => err(ErrorKind::NotFound),
    })
}

pub fn modified(p: &Path) -> io::Result<SystemTime> {
    with(|t| t.files.get(&key(p)).map(|e| UNIX_EPOCH + Duration::from_secs(e.ver)).ok_or_else(|| ErrorKind::NotFound.into()))
}

pub fn write(p: &Path, data: &[u8]) -> io::Result<()> {
    mount(p, data.to_vec());
    Ok(())
}

pub fn mount(p: &Path, data: Vec<u8>) {
    with(|t| put(t, key(p), Data::Bytes(data)));
}

#[cfg(target_arch = "wasm32")]
pub fn mount_js(p: &Path, data: js_sys::Uint8Array) {
    with(|t| put(t, key(p), Data::Js(data)));
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    with(|t| {
        let Some(e) = t.files.remove(&key(from)) else { return err(ErrorKind::NotFound) };
        put(t, key(to), e.data);
        Ok(())
    })
}

pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    let b = read(from)?;
    write(to, &b)
}

pub fn remove_file(p: &Path) -> io::Result<()> {
    with(|t| t.files.remove(&key(p)).map(drop).ok_or_else(|| ErrorKind::NotFound.into()))
}

pub fn create_dir_all(p: &Path) -> io::Result<()> {
    with(|t| t.dirs.insert(key(p)));
    Ok(())
}

pub fn remove_dir_all(p: &Path) -> io::Result<()> {
    let k = key(p);
    with(|t| {
        let n = t.files.len() + t.dirs.len();
        t.files.retain(|f, _| !under(f, &k));
        t.dirs.retain(|d| *d != k && !under(d, &k));
        if t.files.len() + t.dirs.len() == n { err(ErrorKind::NotFound) } else { Ok(()) }
    })
}

pub fn is_file(p: &Path) -> bool {
    with(|t| t.files.contains_key(&key(p)))
}

pub fn is_dir(p: &Path) -> bool {
    let k = key(p);
    let below = format!("{}/", k.trim_end_matches('/'));
    with(|t| k == "/" || t.dirs.contains(&k) || t.dirs.range(below.clone()..).next().is_some_and(|d| d.starts_with(&below)) || t.files.range(below.clone()..).next().is_some_and(|(f, _)| f.starts_with(&below)))
}

pub fn read_dir(p: &Path) -> io::Result<Vec<PathBuf>> {
    let k = key(p);
    if !is_dir(p) {
        return err(ErrorKind::NotFound);
    }
    let below = format!("{}/", k.trim_end_matches('/'));
    let kids: BTreeSet<String> = with(|t| {
        let all = t.files.range(below.clone()..).map(|(f, _)| f).take_while(|f| f.starts_with(&below)).chain(t.dirs.range(below.clone()..).take_while(|d| d.starts_with(&below)));
        all.filter_map(|f| f[below.len()..].split('/').next().filter(|n| !n.is_empty()).map(|n| format!("{below}{n}"))).collect()
    });
    Ok(kids.into_iter().map(PathBuf::from).collect())
}

/// Drops every file and folder marker under `prefix`.
pub fn unmount(prefix: &Path) {
    let k = key(prefix);
    with(|t| {
        t.files.retain(|f, _| !under(f, &k));
        t.dirs.retain(|d| !under(d, &k));
    });
}
