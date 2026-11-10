//! Where the picked files come from: a native dialog, or on the web an `<input type=file>` mounted into `d2host`'s memory files.
use eframe::egui;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick { SaveFolder, GameFolder, SaveFile, Png, ExportFolder }

pub enum Keep { Js, Bytes, Skip }

/// What the web build keeps of a picked file: the big tables stay in the browser's memory (`Js`), small files are copied (`Bytes`).
pub fn keep(name: &str, size: u64) -> Keep {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        "patch_d2.mpq" | "d2exp.mpq" | "d2data.mpq" => Keep::Js,
        _ if n.ends_with(".mpq") || size > 16 << 20 => Keep::Skip,
        _ => Keep::Bytes,
    }
}

static DONE: Mutex<Vec<(Pick, PathBuf)>> = Mutex::new(Vec::new());
static ERR: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn deliver(kind: Pick, root: PathBuf) {
    if let Ok(mut d) = DONE.lock() {
        d.retain(|(k, _)| *k != kind);
        d.push((kind, root));
    }
}

pub fn picked(kind: Pick) -> Option<PathBuf> {
    let mut d = DONE.lock().ok()?;
    d.iter().position(|(k, _)| *k == kind).map(|i| d.remove(i).1)
}

/// The import and download errors since the last call (web), as one line.
pub fn take_error() -> Option<String> {
    let mut e = ERR.lock().ok()?;
    let first = e.first()?.clone();
    let n = e.len();
    e.clear();
    Some(if n > 1 { format!("{first} (and {} more)", n - 1) } else { first })
}

/// What the web build does with a file of a picked folder (`rel` is its path from the picked folder's parent).
pub fn plan(kind: Pick, rel: &str, size: u64) -> Keep {
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts.last().copied().unwrap_or_default();
    let (lower, ext) = (name.to_ascii_lowercase(), name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase()));
    let saves = matches!(ext.as_str(), "d2s" | "d2x" | "sss" | "key" | "map") || (ext.len() == 3 && ext.starts_with("ma")) || lower == "plugy.ini";
    match kind {
        Pick::SaveFile | Pick::Png => Keep::Bytes,
        Pick::SaveFolder if saves && size <= 16 << 20 => Keep::Bytes,
        Pick::GameFolder if parts.iter().any(|d| ["sfx", "music", "speech", "video"].iter().any(|m| d.eq_ignore_ascii_case(m))) => Keep::Skip,
        Pick::GameFolder if parts.len() == 2 || parts.get(1).is_some_and(|d| d.eq_ignore_ascii_case("data")) => match keep(name, size) {
            Keep::Js if parts.len() != 2 => Keep::Skip,
            k => k,
        },
        _ => Keep::Skip,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pick(kind: Pick, _: &egui::Context) {
    let d = rfd::FileDialog::new();
    let r = match kind {
        Pick::SaveFile => d.add_filter("Character save", &["d2s"]).pick_file(),
        Pick::Png => d.add_filter("PNG", &["png"]).pick_file(),
        _ => d.pick_folder(),
    };
    if let Some(p) = r {
        deliver(kind, p);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn offer(_: &[PathBuf]) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn guard(_: bool) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn release(_: Option<&std::path::Path>) {}

#[cfg(target_arch = "wasm32")]
pub use web::{guard, offer, pick, release, start};

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::path::Path;
    use wasm_bindgen::prelude::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Blob, BlobPropertyBag, File, HtmlAnchorElement, HtmlCanvasElement, HtmlInputElement, Url};

    const CAP: u64 = 1 << 30;
    static GEN: [AtomicU32; 5] = [const { AtomicU32::new(0) }; 5];
    static COUNT: AtomicU32 = AtomicU32::new(0);
    static LAST: Mutex<Vec<(Pick, PathBuf)>> = Mutex::new(Vec::new());

    fn fail(e: String) {
        if let Ok(mut m) = ERR.lock() {
            m.push(e);
        }
    }

    static UNSAVED: AtomicBool = AtomicBool::new(false);

    /// Asks the browser to confirm before the tab closes or reloads while there are unsaved edits.
    pub fn guard(unsaved: bool) {
        UNSAVED.store(unsaved, SeqCst);
    }

    pub fn start() {
        d2host::use_memory();
        if let Some(w) = web_sys::window() {
            let f = Closure::<dyn Fn(web_sys::Event)>::new(|e: web_sys::Event| if UNSAVED.load(SeqCst) { e.prevent_default() });
            w.set_onbeforeunload(Some(f.as_ref().unchecked_ref()));
            f.forget();
        }
        wasm_bindgen_futures::spawn_local(async {
            let canvas = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("d2rs")).and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok());
            let Some(canvas) = canvas else { return web_sys::console::error_1(&"no canvas #d2rs".into()) };
            let r = eframe::WebRunner::new().start(canvas, eframe::WebOptions::default(), Box::new(|_| Ok(Box::new(crate::gui::App::new())))).await;
            let note = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("loading"));
            match (r, note) {
                (Ok(()), Some(n)) => n.remove(),
                (Err(e), n) => {
                    web_sys::console::error_1(&e);
                    n.iter().for_each(|n| n.set_text_content(Some("d2rs could not start in this browser (WebGL is needed). See the browser console.")));
                }
                _ => {}
            }
        });
    }

    pub fn pick(kind: Pick, ctx: &egui::Context) {
        if kind == Pick::ExportFolder {
            return deliver(kind, "/export".into());
        }
        if let Err(e) = open(kind, ctx.clone()) {
            fail(format!("Could not open the file picker: {e:?}"));
        }
    }

    fn open(kind: Pick, ctx: egui::Context) -> Result<(), JsValue> {
        let doc = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let input: HtmlInputElement = doc.create_element("input")?.dyn_into()?;
        input.set_type("file");
        match kind {
            Pick::SaveFile => input.set_accept(".d2s"),
            Pick::Png => input.set_accept(".png"),
            _ => input.set_webkitdirectory(true),
        }
        let i = input.clone();
        let cb = Closure::once(move || {
            let files: Vec<File> = i.files().map(|l| (0..l.length()).filter_map(|n| l.get(n)).collect()).unwrap_or_default();
            wasm_bindgen_futures::spawn_local(async move {
                import(kind, files).await;
                ctx.request_repaint();
            });
        });
        input.set_onchange(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
        input.click();
        Ok(())
    }

    async fn import(kind: Pick, files: Vec<File>) {
        if files.is_empty() {
            return;
        }
        let gen_ = GEN[kind as usize].fetch_add(1, SeqCst) + 1;
        // Every pick gets its own folder, so a pick never removes the files of the open document.
        let base = match kind {
            Pick::SaveFolder => "/saves",
            Pick::GameFolder => "/game",
            Pick::SaveFile => "/open",
            _ => "/png",
        };
        let prefix = Path::new(base).join(COUNT.fetch_add(1, SeqCst).to_string());
        let mut plan_: Vec<(File, String, Keep)> = vec![];
        for f in files {
            let rel = js_sys::Reflect::get(&f, &"webkitRelativePath".into()).ok().and_then(|v| v.as_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| f.name());
            let k = plan(kind, &rel, f.size() as u64);
            if !matches!(k, Keep::Skip) {
                plan_.push((f, rel, k));
            }
        }
        if plan_.iter().map(|(f, ..)| f.size() as u64).sum::<u64>() > CAP {
            return fail("Folder too large (over 1 GiB to read): pick the game folder or the save folder itself.".into());
        }
        let mut root = prefix.clone();
        for (f, rel, keep) in plan_ {
            if GEN[kind as usize].load(SeqCst) != gen_ {
                return;
            }
            let path = prefix.join(&rel);
            root = match rel.split_once('/') {
                Some((first, _)) => prefix.join(first),
                None => path.clone(),
            };
            match JsFuture::from(f.array_buffer()).await {
                Ok(buf) => {
                    let a = js_sys::Uint8Array::new(&buf);
                    match keep {
                        Keep::Js => d2host::mount_js(&path, a),
                        _ => d2host::mount(&path, a.to_vec()),
                    }
                }
                Err(e) => fail(format!("Could not read {}: {e:?}", f.name())),
            }
        }
        if GEN[kind as usize].load(SeqCst) != gen_ {
            return;
        }
        // A new game folder or PNG replaces the old one; saves stay (the open document may live there).
        if let Ok(mut l) = LAST.lock() {
            if matches!(kind, Pick::GameFolder | Pick::Png) {
                l.iter().filter(|(k, _)| *k == kind).for_each(|(_, p)| d2host::unmount(p));
            }
            if !matches!(kind, Pick::SaveFolder | Pick::SaveFile) {
                l.retain(|(k, _)| *k != kind);
            }
            l.push((kind, prefix));
        }
        deliver(kind, root);
    }

    /// Drops the older save mounts, except the newest pick (the one listed) and the one the open document (`keep`) lives in.
    pub fn release(keep: Option<&Path>) {
        if let Ok(mut l) = LAST.lock() {
            let newest = l.iter().rposition(|(k, _)| matches!(k, Pick::SaveFolder | Pick::SaveFile));
            let mut i = 0;
            l.retain(|(k, p)| {
                i += 1;
                let old = matches!(k, Pick::SaveFolder | Pick::SaveFile) && Some(i - 1) != newest && !keep.is_some_and(|c| c.starts_with(p));
                if old {
                    d2host::unmount(p);
                }
                !old
            });
        }
    }

    /// Downloads each file through a temporary link.
    pub fn offer(paths: &[PathBuf]) {
        for p in paths {
            match d2host::read(p) {
                Ok(b) => {
                    if let Err(e) = download(p, &b) {
                        fail(format!("Could not download {}: {e:?}", p.display()));
                    }
                }
                Err(e) => fail(format!("Could not read {}: {e}", p.display())),
            }
        }
    }

    fn download(p: &Path, bytes: &[u8]) -> Result<(), JsValue> {
        let opts = BlobPropertyBag::new();
        opts.set_type("application/octet-stream");
        let blob = Blob::new_with_u8_array_sequence_and_options(&js_sys::Array::of1(&js_sys::Uint8Array::from(bytes)), &opts)?;
        let url = Url::create_object_url_with_blob(&blob)?;
        let a: HtmlAnchorElement = web_sys::window().and_then(|w| w.document()).ok_or("no document")?.create_element("a")?.dyn_into()?;
        a.set_href(&url);
        a.set_download(&p.file_name().unwrap_or_default().to_string_lossy());
        a.click();
        let later = Closure::once_into_js(move || drop(Url::revoke_object_url(&url)));
        web_sys::window().ok_or("no window")?.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), 60_000)?;
        Ok(())
    }
}
