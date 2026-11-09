// Embeds the emblem as the Windows exe icon. Only when building on Windows for Windows (winresource is a Windows-host
// build-dependency); a no-op for macOS, Linux and wasm32.
fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new().set_icon("icons/icon.ico").compile().expect("embed icon.ico");
    }
}
