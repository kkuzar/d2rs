//! Per-OS process backend. All `unsafe` lives here.
//! macOS needs the debugger entitlement or root plus admin approval for `task_for_pid` (best effort),
//! and assumes the Wine image base 0x400000.
#[cfg(any(unix, windows))]
use crate::Memory;
use std::{io, path::PathBuf};

pub struct Process {
    /// Module base of Game.exe.
    pub base: u32,
    exe: Option<PathBuf>,
    #[cfg(target_os = "linux")]
    pid: i32,
    #[cfg(target_os = "macos")]
    task: u32,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl Process {
    /// Path of the running Game.exe on this machine, when it could be determined.
    pub fn exe_path(&self) -> Option<PathBuf> {
        self.exe.clone()
    }
}

#[cfg(unix)]
pub fn find_game() -> io::Result<Process> {
    let out = std::process::Command::new("ps").args(["-axo", "pid=,command="]).output()?;
    let me = std::process::id();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.to_lowercase().contains("game.exe"))
        .filter_map(|l| l.split_whitespace().next()?.parse::<u32>().ok())
        .find(|&p| p != me)
        .map_or_else(|| Err(io::Error::new(io::ErrorKind::NotFound, "Game.exe not running")), Process::attach)
}

#[cfg(target_os = "linux")]
impl Process {
    pub fn attach(pid: u32) -> io::Result<Process> {
        let maps = std::fs::read_to_string(format!("/proc/{pid}/maps"))?;
        let line = maps.lines().find(|l| l.to_lowercase().ends_with("game.exe")).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "game.exe mapping not found"))?;
        let base = u32::from_str_radix(line.split('-').next().unwrap_or(""), 16).map_err(|_| io::ErrorKind::InvalidData)?;
        let exe = line.find('/').map(|i| PathBuf::from(&line[i..]));
        Ok(Process { base, exe, pid: pid as i32 })
    }
}
#[cfg(target_os = "linux")]
impl Memory for Process {
    fn read(&self, addr: u32, buf: &mut [u8]) -> io::Result<()> {
        let l = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
        let r = libc::iovec { iov_base: addr as usize as *mut _, iov_len: buf.len() };
        // SAFETY: both iovecs describe valid ranges (local buffer, remote address checked by the kernel).
        let n = unsafe { libc::process_vm_readv(self.pid, &l, 1, &r, 1, 0) };
        if n == buf.len() as isize { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
    fn write(&self, addr: u32, buf: &[u8]) -> io::Result<()> {
        let l = libc::iovec { iov_base: buf.as_ptr() as *mut _, iov_len: buf.len() };
        let r = libc::iovec { iov_base: addr as usize as *mut _, iov_len: buf.len() };
        // SAFETY: the local iovec is only read; the remote range is validated by the kernel.
        let n = unsafe { libc::process_vm_writev(self.pid, &l, 1, &r, 1, 0) };
        if n == buf.len() as isize { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}

#[cfg(target_os = "macos")]
impl Process {
    pub fn attach(pid: u32) -> io::Result<Process> {
        use mach2::{kern_return::KERN_SUCCESS, traps::{mach_task_self, task_for_pid}};
        let mut task = 0;
        // SAFETY: `task` is a valid out pointer for the duration of the call.
        let kr = unsafe { task_for_pid(mach_task_self(), pid as i32, &mut task) };
        if kr != KERN_SUCCESS {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("task_for_pid failed ({kr}); needs debugger entitlement / root")));
        }
        // Best effort: the command line may contain spaces, so take everything up to the token ending in game.exe.
        let out = std::process::Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).output()?;
        let cmd = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let end = cmd.to_lowercase().find("game.exe").map(|i| i + 8);
        let exe = end.map(|e| cmd[..e].trim_start_matches("Z:").replace('\\', "/")).map(PathBuf::from);
        Ok(Process { base: 0x400000, exe, task })
    }
}
#[cfg(target_os = "macos")]
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: `task` is the send right obtained from task_for_pid in `attach` and is released exactly once.
        unsafe { mach2::mach_port::mach_port_deallocate(mach2::traps::mach_task_self(), self.task) };
    }
}
#[cfg(target_os = "macos")]
impl Memory for Process {
    fn read(&self, addr: u32, buf: &mut [u8]) -> io::Result<()> {
        let mut out = 0;
        // SAFETY: `buf` is a valid writable buffer of the given length; `out` is a valid out pointer.
        let kr = unsafe { mach2::vm::mach_vm_read_overwrite(self.task, addr as u64, buf.len() as u64, buf.as_mut_ptr() as u64, &mut out) };
        if kr == 0 && out == buf.len() as u64 { Ok(()) } else { Err(io::Error::other(format!("mach_vm_read_overwrite {kr}"))) }
    }
    fn write(&self, addr: u32, buf: &[u8]) -> io::Result<()> {
        // SAFETY: `buf` is a valid readable buffer of the given length.
        let kr = unsafe { mach2::vm::mach_vm_write(self.task, addr as u64, buf.as_ptr() as usize, buf.len() as u32) };
        if kr == 0 { Ok(()) } else { Err(io::Error::other(format!("mach_vm_write {kr}"))) }
    }
}

#[cfg(windows)]
mod win {
    pub use windows_sys::Win32::{Foundation::*, System::{Diagnostics::{Debug::*, ToolHelp::*}, Threading::*}};
}
#[cfg(windows)]
pub fn find_game() -> io::Result<Process> {
    use win::*;
    // SAFETY: the snapshot handle is closed; PROCESSENTRY32W is zeroed with dwSize set as required.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut pid = None;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = String::from_utf16_lossy(&e.szExeFile).trim_end_matches('\0').to_lowercase();
            if n == "game.exe" { pid = Some(e.th32ProcessID); break; }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
        pid.map_or_else(|| Err(io::Error::new(io::ErrorKind::NotFound, "Game.exe not running")), Process::attach)
    }
}
#[cfg(windows)]
impl Process {
    pub fn attach(pid: u32) -> io::Result<Process> {
        use win::*;
        // SAFETY: handles are checked before use; MODULEENTRY32W is zeroed with dwSize set; the first module is the exe.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
            let mut m: MODULEENTRY32W = std::mem::zeroed();
            m.dwSize = size_of::<MODULEENTRY32W>() as u32;
            let ok = Module32FirstW(snap, &mut m);
            CloseHandle(snap);
            if ok == 0 { return Err(io::Error::last_os_error()); }
            let handle = OpenProcess(PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION, 0, pid);
            if handle.is_null() { return Err(io::Error::last_os_error()); }
            let mut buf = [0u16; 1024];
            let mut n = buf.len() as u32;
            let exe = (QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut n) != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])));
            Ok(Process { base: m.modBaseAddr as usize as u32, exe, handle })
        }
    }
}
#[cfg(windows)]
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `attach` and is closed exactly once.
        unsafe { win::CloseHandle(self.handle) };
    }
}
#[cfg(windows)]
impl Memory for Process {
    fn read(&self, addr: u32, buf: &mut [u8]) -> io::Result<()> {
        let mut n = 0;
        // SAFETY: `buf` is a valid writable buffer of the given length.
        let ok = unsafe { win::ReadProcessMemory(self.handle, addr as usize as _, buf.as_mut_ptr().cast(), buf.len(), &mut n) };
        if ok != 0 && n == buf.len() { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
    fn write(&self, addr: u32, buf: &[u8]) -> io::Result<()> {
        let mut n = 0;
        // SAFETY: `buf` is a valid readable buffer of the given length.
        let ok = unsafe { win::WriteProcessMemory(self.handle, addr as usize as _, buf.as_ptr().cast(), buf.len(), &mut n) };
        if ok != 0 && n == buf.len() { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}
