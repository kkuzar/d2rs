//! Safe-write layer: commit protocol (plan 4.2) and rename transaction (4.3).
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const MAX_SAVE: usize = 8192;
const KEEP: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub mtime: Option<SystemTime>,
    pub hash: u64,
}

pub fn fingerprint(path: &Path) -> io::Result<Fingerprint> {
    let b = d2host::read(path)?;
    let mut h = DefaultHasher::new();
    h.write(&b);
    Ok(Fingerprint { len: b.len() as u64, mtime: d2host::modified(path).ok(), hash: h.finish() })
}

/// Name of a running game process (native, Wine or macOS), if any.
pub fn game_running() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    return None;
    #[cfg(not(target_arch = "wasm32"))]
    game_running_native()
}

#[cfg(not(target_arch = "wasm32"))]
fn game_running_native() -> Option<String> {
    use sysinfo::{ProcessesToUpdate, System};
    let Ok(mut s) = System::new() else { return Some("process list unavailable".into()) };
    s.refresh_processes(ProcessesToUpdate::All, true);
    s.processes().values().find_map(|p| {
        let name = p.name().to_string_lossy().to_lowercase();
        let cmd = p.cmd().iter().map(|a| a.to_string_lossy().to_lowercase()).collect::<Vec<_>>().join(" ");
        let hit = name == "game.exe" || name == "diablo ii.exe" || name == "diablo ii" || name.contains("game.exe") || cmd.contains("game.exe"); // real process name
        hit.then_some(name)
    })
}

pub fn default_backup_root() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "d2rs").map(|d| d.data_dir().join("backups"))
}

/// Backup root for one save folder (plan 4.2 `<save-dir-id>`), so same-named characters in different
/// folders never share backups. The id is a stable FNV-1a hash of the canonical folder path.
pub fn backup_root_for(save_dir: &Path) -> Option<PathBuf> {
    let p = d2host::canonicalize(save_dir);
    let id = p.to_string_lossy().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    default_backup_root().map(|r| r.join(format!("{id:016x}")))
}

#[derive(Debug)]
pub enum CommitError {
    GameRunning(String),
    Changed,
    Verify(String),
    TooBig(usize),
    Mismatch,
    Io(io::Error),
}
impl From<io::Error> for CommitError {
    fn from(e: io::Error) -> Self {
        CommitError::Io(e)
    }
}
impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CommitError {}

#[derive(Debug)]
pub struct CommitReport {
    pub backup: PathBuf,
    pub fingerprint: Fingerprint,
}

/// Writer configuration; `game_check` is injectable for tests.
pub struct Writer {
    pub game_check: fn() -> Option<String>,
    pub backup_root: PathBuf,
}

#[doc(hidden)]
pub fn tmp_path(target: &Path) -> PathBuf {
    let n = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{n}.d2rs-tmp"))
}

/// `d`, or "." when it is empty (`Path::parent` of a bare file name is "").
fn here(d: &Path) -> &Path {
    if d.as_os_str().is_empty() { Path::new(".") } else { d }
}

/// temp file, write_all, sync_all, rename over target, fsync parent dir (Unix).
#[doc(hidden)]
pub fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(target);
    let _ = d2host::remove_file(&tmp);
    let r = d2host::write(&tmp, bytes).and_then(|()| d2host::rename(&tmp, target));
    if r.is_err() {
        let _ = d2host::remove_file(&tmp);
        return r;
    }
    d2host::sync_dir(here(target.parent().unwrap_or(Path::new(""))))
}

/// Files of a character in `dir`: `<stem>.d2s|key|map|d2x|ma*` (`with_shared` adds the PlugY shared stashes).
fn char_files(dir: &Path, stem: &str, with_shared: bool) -> io::Result<Vec<PathBuf>> {
    let mut v = vec![];
    for p in d2host::read_dir(here(dir))? {
        let n = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let own = n.strip_prefix(stem).and_then(|r| r.strip_prefix('.')).is_some_and(|x| matches!(x, "d2s" | "key" | "map" | "d2x") || x.starts_with("ma"));
        if d2host::is_file(&p) && (own || (with_shared && n.ends_with("SharedStashSave.sss"))) {
            v.push(p);
        }
    }
    Ok(v)
}

/// Copies the bytes only (not a read-only attribute, which would block the sync and later pruning).
fn copy_set(files: &[PathBuf], to: &Path) -> io::Result<()> {
    d2host::create_dir_all(to)?;
    for f in files {
        d2host::write(&to.join(f.file_name().unwrap_or_default()), &d2host::read(f)?)?;
    }
    Ok(())
}

fn stamp(n: &str) -> Option<u64> {
    n.parse().ok()
}

/// Timestamped backup dirs of `stem`, newest first.
pub fn backups(backup_root: &Path, stem: &str) -> Vec<PathBuf> {
    let mut v: Vec<(u64, PathBuf)> = d2host::read_dir(&backup_root.join(stem))
        .into_iter()
        .flatten()
        .filter_map(|p| stamp(&p.file_name()?.to_string_lossy()).map(|t| (t, p)))
        .collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.0));
    v.into_iter().map(|x| x.1).collect()
}

impl Writer {
    fn guard(&self) -> Result<(), CommitError> {
        match (self.game_check)() {
            Some(g) => Err(CommitError::GameRunning(g)),
            None => Ok(()),
        }
    }

    /// Steps 1-3: game check, fingerprint check, verify and size limit.
    pub fn preflight(&self, target: &Path, opened: &Fingerprint, new: &[u8], verify: &dyn Fn(&[u8]) -> Result<(), String>) -> Result<(), CommitError> {
        self.guard()?;
        if &fingerprint(target)? != opened {
            return Err(CommitError::Changed);
        }
        verify(new).map_err(CommitError::Verify)?;
        // The 8 KiB read limit is the game's .d2s loader; PlugY .d2x/.sss stashes are larger by design.
        if new.len() > MAX_SAVE && target.extension().is_some_and(|e| e.eq_ignore_ascii_case("d2s")) {
            return Err(CommitError::TooBig(new.len()));
        }
        Ok(())
    }

    /// Step 4: back up the whole set plus `extra` (the files about to be replaced), keep `first/` once,
    /// prune to the newest 50. Returns the backup dir.
    pub fn backup(&self, dir: &Path, stem: &str, extra: &[PathBuf]) -> io::Result<PathBuf> {
        let mut files = char_files(dir, stem, true)?;
        for e in extra {
            if d2host::is_file(e) && !files.iter().any(|f| f.file_name() == e.file_name()) {
                files.push(e.clone());
            }
        }
        let base = self.backup_root.join(stem);
        let now = d2host::now_nanos() / 1_000_000_000;
        // Newer than every existing backup even if the clock went back, so pruning never drops this one.
        let newest = backups(&self.backup_root, stem).first().and_then(|p| stamp(&p.file_name()?.to_string_lossy()));
        let b = base.join(newest.map_or(now, |n| now.max(n + 1)).to_string());
        copy_set(&files, &b)?;
        if !d2host::exists(&base.join("first")) {
            copy_set(&files, &base.join("first"))?;
        }
        for old in backups(&self.backup_root, stem).into_iter().skip(KEEP) {
            d2host::remove_dir_all(&old)?;
        }
        Ok(b)
    }

    pub fn commit(&self, target: &Path, opened: &Fingerprint, new: &[u8], verify: impl Fn(&[u8]) -> Result<(), String>) -> Result<CommitReport, CommitError> {
        self.preflight(target, opened, new, &verify)?;
        let (dir, stem) = split(target);
        let backup = self.backup(dir, &stem, &[target.to_path_buf()])?;
        atomic_write(target, new)?;
        if d2host::read(target)? != new {
            atomic_write(target, &d2host::read(&backup.join(target.file_name().unwrap_or_default()))?)?;
            return Err(CommitError::Mismatch);
        }
        Ok(CommitReport { backup, fingerprint: fingerprint(target)? })
    }

    /// Character rename: new set under `new_stem`, verified, then the old set is removed.
    pub fn rename_character(&self, dir: &Path, old_stem: &str, new_stem: &str, new_d2s: &[u8], opened: &Fingerprint, verify: impl Fn(&[u8]) -> Result<(), String>) -> Result<CommitReport, CommitError> {
        let old_d2s = dir.join(format!("{old_stem}.d2s"));
        let new_d2s_path = dir.join(format!("{new_stem}.d2s"));
        if d2host::exists(&new_d2s_path) || !char_files(dir, new_stem, false)?.is_empty() {
            return Err(CommitError::Io(io::Error::new(io::ErrorKind::AlreadyExists, "new name exists")));
        }
        self.preflight(&old_d2s, opened, new_d2s, &verify)?;
        let backup = self.backup(dir, old_stem, &[])?;
        let old = char_files(dir, old_stem, false)?;
        let mut made = vec![];
        let r = (|| -> Result<(), CommitError> {
            for f in &old {
                let ext = f.file_name().unwrap_or_default().to_string_lossy()[old_stem.len()..].to_string();
                let dest = dir.join(format!("{new_stem}{ext}"));
                made.push(dest.clone());
                let bytes = if ext == ".d2s" { new_d2s.to_vec() } else { d2host::read(f)? };
                atomic_write(&dest, &bytes)?;
                if d2host::read(&dest)? != bytes {
                    return Err(CommitError::Mismatch);
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            made.iter().for_each(|p| drop(d2host::remove_file(p)));
            return Err(e);
        }
        // The old .d2s goes first; if it stays, the new set goes, so the character is never duplicated. After
        // that the rename has happened: a companion that cannot be removed (read-only, locked) must not fail it,
        // or the caller's rollback would bring the old character back next to the new one.
        if let Err(e) = d2host::remove_file(&old_d2s) {
            made.iter().for_each(|p| drop(d2host::remove_file(p)));
            return Err(e.into());
        }
        old.iter().filter(|f| f.extension().is_none_or(|e| e != "d2s")).for_each(|f| drop(d2host::remove_file(f)));
        Ok(CommitReport { backup, fingerprint: fingerprint(&new_d2s_path)? })
    }

    /// Restore a backup dir into `target_dir` (current set is backed up first, writes use temp+rename).
    pub fn restore(&self, backup_dir: &Path, target_dir: &Path) -> Result<(), CommitError> {
        self.guard()?;
        let files: Vec<PathBuf> = d2host::read_dir(backup_dir)?.into_iter().filter(|p| d2host::is_file(p)).collect();
        // Back up every file about to be replaced, also for a backup without a .d2s (a shared stash).
        if let Some(main) = files.iter().find(|p| p.extension().is_some_and(|e| e == "d2s")).or(files.first()) {
            let cur: Vec<PathBuf> = files.iter().map(|f| target_dir.join(f.file_name().unwrap_or_default())).collect();
            self.backup(target_dir, &split(main).1, &cur)?;
        }
        for f in files {
            atomic_write(&target_dir.join(f.file_name().unwrap_or_default()), &d2host::read(&f)?)?;
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn split(target: &Path) -> (&Path, String) {
    (here(target.parent().unwrap_or(Path::new(""))), target.file_stem().unwrap_or_default().to_string_lossy().into_owned())
}
