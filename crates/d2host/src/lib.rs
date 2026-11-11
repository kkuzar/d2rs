//! Every file, clock and background-job call of d2rs goes through here: std on desktop, an in-memory file table in the browser.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
mod mem;

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use mem::{mount, unmount, use_memory};
#[cfg(target_arch = "wasm32")]
pub use mem::mount_js;

pub fn read(p: &Path) -> io::Result<Vec<u8>> {
    if mem::on() { mem::read(p) } else { std::fs::read(p) }
}

pub fn read_to_string(p: &Path) -> io::Result<String> {
    String::from_utf8(read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn read_at(p: &Path, off: u64, buf: &mut [u8]) -> io::Result<()> {
    if mem::on() {
        return mem::read_at(p, off, buf);
    }
    let mut f = std::fs::File::open(p)?;
    f.seek(SeekFrom::Start(off))?;
    f.read_exact(buf)
}

pub fn len(p: &Path) -> io::Result<u64> {
    if mem::on() { mem::len(p) } else { Ok(std::fs::metadata(p)?.len()) }
}

pub fn modified(p: &Path) -> io::Result<SystemTime> {
    if mem::on() { mem::modified(p) } else { std::fs::metadata(p)?.modified() }
}

pub fn write(p: &Path, data: &[u8]) -> io::Result<()> {
    if mem::on() {
        return mem::write(p, data);
    }
    let mut f = std::fs::File::create(p)?;
    f.write_all(data)?;
    f.sync_all()
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if mem::on() { mem::rename(from, to) } else { std::fs::rename(from, to) }
}

pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    if mem::on() { mem::copy(from, to) } else { std::fs::copy(from, to).map(drop) }
}

pub fn remove_file(p: &Path) -> io::Result<()> {
    if mem::on() { mem::remove_file(p) } else { std::fs::remove_file(p) }
}

pub fn create_dir_all(p: &Path) -> io::Result<()> {
    if mem::on() { mem::create_dir_all(p) } else { std::fs::create_dir_all(p) }
}

pub fn remove_dir_all(p: &Path) -> io::Result<()> {
    if mem::on() { mem::remove_dir_all(p) } else { std::fs::remove_dir_all(p) }
}

/// Sorted full paths of the direct children.
pub fn read_dir(p: &Path) -> io::Result<Vec<PathBuf>> {
    if mem::on() {
        return mem::read_dir(p);
    }
    let mut v = std::fs::read_dir(p)?.map(|e| e.map(|e| e.path())).collect::<io::Result<Vec<_>>>()?;
    v.sort();
    Ok(v)
}

pub fn exists(p: &Path) -> bool {
    is_file(p) || is_dir(p)
}

pub fn is_dir(p: &Path) -> bool {
    if mem::on() { mem::is_dir(p) } else { p.is_dir() }
}

pub fn is_file(p: &Path) -> bool {
    if mem::on() { mem::is_file(p) } else { p.is_file() }
}

/// The canonical path, or `p` itself when it cannot be resolved.
pub fn canonicalize(p: &Path) -> PathBuf {
    if mem::on() { p.to_path_buf() } else { std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()) }
}

/// Fsync a folder (unix); a no-op elsewhere and in memory.
pub fn sync_dir(p: &Path) -> io::Result<()> {
    if cfg!(unix) && !mem::on() { std::fs::File::open(p)?.sync_all() } else { Ok(()) }
}

pub fn now_nanos() -> u64 {
    #[cfg(target_arch = "wasm32")]
    return (js_sys::Date::now() * 1e6) as u64;
    #[cfg(not(target_arch = "wasm32"))]
    SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// Runs `job` in the background: a thread (inline on a memory thread) or `setTimeout(0)` in the browser.
pub fn spawn(job: impl FnOnce() + Send + 'static) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let f = wasm_bindgen::closure::Closure::once_into_js(job);
        if let Some(w) = web_sys::window() {
            drop(w.set_timeout_with_callback_and_timeout_and_arguments_0(f.unchecked_ref(), 0));
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if mem::on() {
        job();
    } else {
        std::thread::spawn(job);
    }
}
