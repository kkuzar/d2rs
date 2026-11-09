//! d2rs: `d2rs` opens the GUI, `d2rs cli <cmd>` runs the command line tools.
// Release builds on Windows open no console window; `d2rs cli` re-attaches to the parent console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_arch = "wasm32")]
fn main() {
    d2rs_app::platform::start()
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().is_some_and(|c| c == "cli") {
        #[cfg(windows)]
        // SAFETY: AttachConsole has no preconditions; failure (no parent console) is harmless.
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
        }
        // `d2rs cli info f | head -3`: a closed pipe ends the printing quietly (every write command commits before it prints nothing)
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |i| {
            if !i.to_string().contains("failed printing to stdout: Broken pipe") {
                hook(i);
            }
        }));
        std::process::exit(d2rs_app::cli::run(&a[1..]));
    }
    d2rs_app::gui::run()
}
