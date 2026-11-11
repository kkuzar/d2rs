//! eframe GUI shell: file list, Character / Stats / Items / Backups tabs, status bar, undo/redo.
use crate::items::ItemsUi;
#[cfg(not(target_arch = "wasm32"))]
use crate::live::Live;
use crate::platform::{self, Pick};
use crate::cli::{has_error, stem, write_save};
use d2s::game;
use d2s::validate::{Finding, Level, validate};
use d2s::{Save, Schema};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use std::path::{Path, PathBuf};

const TABS: [&str; 9] = ["Character", "Stats", "Skills", "Quests", "Waypoints", "Items", "Live", "Backups", "Map"];
const DIFFS: [&str; 3] = ["Normal", "Nightmare", "Hell"];

#[cfg(not(target_arch = "wasm32"))]
pub fn run() -> eframe::Result {
    let mut options = eframe::NativeOptions::default();
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../icons/icon.png")) {
        options.viewport = options.viewport.with_icon(icon);
    }
    eframe::run_native("d2rs", options, Box::new(|_| Ok(Box::new(App::new()))))
}

const SAVES: &str = "Saved Games/Diablo II"; // real folder name
const MAC_SAVES: &str = "Library/Application Support/Blizzard/Diablo II"; // real folder name
const MAC_APP: &str = "/Applications/Diablo II/Save"; // real folder name

fn default_folders() -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    [
        home.join(SAVES),
        MAC_APP.into(),
        home.join(MAC_SAVES),
        home.join(format!(".wine/drive_c/users/{user}")).join(SAVES),
    ]
    .into_iter()
    .filter(|p| d2host::is_dir(p))
    .collect()
}

#[doc(hidden)]
pub struct Open {
    path: PathBuf,
    #[doc(hidden)] pub save: Save,
    fp: d2io::Fingerprint,
    #[doc(hidden)] pub findings: Vec<Finding>,
    size: usize,
    name_buf: String,
}

/// An undo step: the .d2s and, for an Items tab edit, the stash documents (a move can change both).
type Snap = (Save, Option<Vec<(d2s::plugy::Stash, bool)>>);

#[doc(hidden)]
pub struct App {
    folders: Vec<PathBuf>,
    files: Vec<PathBuf>,
    #[doc(hidden)] pub cur: Option<Open>,
    #[doc(hidden)] pub undo: Vec<Snap>,
    redo: Vec<Snap>,
    #[doc(hidden)] pub tab: usize,
    diff: usize,
    advanced: bool,
    gesture: bool,
    msg: String,
    note: String,
    writer: d2io::Writer,
    #[doc(hidden)] pub items: ItemsUi,
    map: crate::map::MapUi,
    #[cfg(not(target_arch = "wasm32"))]
    live: Live,
    dirty: bool,
    close_ok: bool,
    setup_open: bool,
}

impl App {
    #[doc(hidden)] pub fn new() -> Self {
        let mut a = App {
            folders: default_folders(),
            files: vec![],
            cur: None,
            undo: vec![],
            redo: vec![],
            tab: 0,
            diff: 0,
            advanced: false,
            gesture: false,
            msg: String::new(),
            note: String::new(),
            items: ItemsUi::new(),
            map: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            live: Live::default(),
            dirty: false,
            close_ok: false,
            setup_open: false,
            writer: d2io::Writer { game_check: d2io::game_running, backup_root: "backups".into() },
        };
        crate::setup_ui::adapt(a.items.game_dir(), None, None);
        a.scan();
        a
    }

    fn scan(&mut self) {
        let extra: Vec<PathBuf> = self.files.iter().filter(|f| !self.folders.iter().any(|d| f.parent() == Some(d))).cloned().collect();
        self.files = self.folders.iter().flat_map(|d| d2host::read_dir(d).into_iter().flatten()).filter(|p| p.extension().is_some_and(|e| e == "d2s")).chain(extra).collect();
        self.files.sort();
        self.files.dedup();
    }

    fn unsaved(&self) -> bool {
        self.cur.is_some() && (self.dirty || self.items.dirty_docs())
    }

    fn confirm_discard(&self) -> bool {
        !self.unsaved() || rfd::MessageDialog::new().set_title("Unsaved changes").set_description("Discard the unsaved changes?").set_buttons(rfd::MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes
    }

    #[doc(hidden)] pub fn open(&mut self, path: PathBuf) {
        if !self.confirm_discard() {
            return;
        }
        let r = d2host::read(&path).map_err(|e| e.to_string()).and_then(|b| {
            crate::setup_ui::adapt(self.items.game_dir(), path.parent(), Some(&b));
            let sch = Schema::classic();
            Save::verify(&b, sch).map_err(|e| format!("self-check failed: {e}"))?;
            let save = Save::parse(&b, sch).map_err(|e| e.to_string())?;
            crate::setup_ui::grids(&save);
            Ok((save, d2io::fingerprint(&path).map_err(|e| e.to_string())?))
        });
        match r {
            Ok((save, fp)) => {
                self.dirty = false;
                if let Some(r) = path.parent().and_then(d2io::backup_root_for) {
                    self.writer.backup_root = r;
                }
                if !self.files.contains(&path) {
                    self.files.push(path.clone());
                }
                self.cur = Some(Open { name_buf: save.name(), path, save, fp, findings: vec![], size: 0 });
                (self.undo, self.redo, self.msg) = (vec![], vec![], String::new());
                if let Some(c) = &self.cur {
                    self.items.load(&c.path, &c.save);
                    self.msg = self.items.msg.clone();
                }
                self.refresh();
                platform::release(self.cur.as_ref().map(|c| c.path.as_path()));
            }
            Err(e) => self.alert("Cannot open", &format!("{}: {e}", path.display())),
        }
    }

    fn refresh(&mut self) {
        if let Some(c) = &mut self.cur {
            c.findings = validate(&c.save, Schema::classic(), &c.save.name());
            c.size = c.save.to_bytes(Schema::classic()).map_or(0, |b| b.len());
        }
    }

    fn alert(&mut self, title: &str, text: &str) {
        rfd::MessageDialog::new().set_title(title).set_description(text).show();
    }

    fn save(&mut self) {
        let Some(c) = &self.cur else { return };
        if let Err(e) = crate::setup_ui::writable() {
            return self.alert("Save failed", &format!("{e}\nNothing was written."));
        }
        let (path, dir) = (c.path.clone(), c.path.parent().unwrap_or(Path::new(".")).to_path_buf());
        let p = dir.join(format!("{}.d2s", c.save.name()));
        // Phase one: verify everything and run the preflight of every document before writing any.
        if let Err(e) = crate::cli::preflight_save(&self.writer, &path, &c.fp, &c.save).and_then(|()| self.items.preflight_docs(&self.writer)) {
            return self.alert("Save failed", &format!("{e}\nNothing was written."));
        }
        // One backup of the whole set (with the stashes) first, so the newest backup listed for this character is
        // the consistent state before this save. The per-file backups below go to a sub-folder: each of them
        // mixes old and new files, and restoring one would duplicate or lose a moved item.
        if let Err(e) = self.writer.backup(&dir, &stem(&path), &[]) {
            return self.alert("Save failed", &format!("backup failed: {e}\nNothing was written."));
        }
        let w = d2io::Writer { game_check: self.writer.game_check, backup_root: self.writer.backup_root.join(".save") };
        // Phase two: stashes (they receive items) first, then the .d2s; a late failure restores the stashes.
        let done = match self.items.commit_docs(&w) {
            Ok(d) => d,
            Err(e) => return self.alert("Save failed", &e),
        };
        let web = cfg!(target_arch = "wasm32");
        let old = if web && p != path { char_set(&dir, &stem(&path)) } else { vec![] };
        if let Err(e) = write_save(&w, &path, &c.fp, &c.save) {
            let msg = match self.items.rollback(&w, &done) {
                Ok(()) => format!("{e}\nThe stash files were restored from backup; nothing was lost."),
                Err(r) => format!("{e}\nROLLBACK FAILED: {r}\nRestore the stash files from the backup folder before doing anything else."),
            };
            return self.alert("Save failed", &msg);
        }
        let mut files = vec![];
        if web {
            files = char_set(&dir, &stem(&p));
            files.extend(self.items.paths(&done).into_iter().filter(|f| d2host::exists(f)));
            files.sort();
            files.dedup();
        }
        self.items.finish(&done);
        self.dirty = false;
        self.scan();
        self.open(p.clone());
        self.msg = format!("Saved {}", p.display());
        if web {
            platform::offer(&files);
            let n = |v: &[PathBuf]| v.iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ");
            self.msg = format!("Offered {} files for download ({}): check the browser's download bar that all of them arrived, then copy them into your save folder, replacing the old files.", files.len(), n(&files));
            if !old.is_empty() {
                self.msg += &format!(" The character was renamed: delete the old files {}.", n(&old));
            }
        }
    }

    fn restore(&mut self, backup: &Path) {
        let Some(dir) = self.cur.as_ref().and_then(|c| c.path.parent().map(Path::to_path_buf)) else { return };
        match self.writer.restore(backup, &dir) {
            Ok(()) => {
                self.dirty = false;
                self.msg = "Backup restored".into();
                if let Some(p) = self.cur.as_ref().map(|c| c.path.clone()) {
                    self.open(p);
                }
            }
            Err(e) => self.alert("Restore failed", &e.to_string()),
        }
    }
}

/// The files of one character in `dir` (`<stem>.d2s`, `.d2x`, `.key`, `.map`, `.ma*`), for the web download set.
fn char_set(dir: &Path, stem: &str) -> Vec<PathBuf> {
    let pre = format!("{stem}.");
    d2host::read_dir(dir).into_iter().flatten().filter(|f| f.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&pre))).collect()
}

/// The status bar's mode button: its text is the mode, a click opens the Game setup window.
fn status_button(ui: &mut egui::Ui, open: &mut bool) {
    if ui.button(format!("Tables: {}", crate::setup_ui::status())).clicked() {
        *open = !*open;
    }
}

fn diff_picker(ui: &mut egui::Ui, d: &mut usize) {
    ui.horizontal(|ui| DIFFS.iter().enumerate().for_each(|(i, n)| drop(ui.selectable_value(d, i, *n))));
}

/// A seed in 1..=0x7FFF_FFFF from a time-seeded xorshift.
fn random_seed() -> u32 {
    let mut x = d2host::now_nanos() | 1;
    (0..3).for_each(|_| (x ^= x << 13, x ^= x >> 7, x ^= x << 17).0);
    (x % 0x7FFF_FFFF) as u32 + 1
}

fn num(ui: &mut egui::Ui, label: &str, v: &mut i64, range: std::ops::RangeInclusive<i64>) -> bool {
    ui.label(label);
    ui.add(egui::DragValue::new(v).range(range)).changed()
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(ui);
    }
}

impl App {
    #[doc(hidden)] pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let (undo, redo) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z));
            (i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)), redo)
        });
        if let Some(c) = &mut self.cur {
            let (from, to) = if undo { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
            if (undo || redo) && let Some((s, d)) = from.pop() {
                self.dirty = true;
                to.push((std::mem::replace(&mut c.save, s), d.map(|d| self.items.restore(d))));
                c.name_buf = c.save.name();
                self.refresh();
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.close_ok && self.unsaved() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.confirm_discard() {
                self.close_ok = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if let Some(e) = platform::take_error() {
            self.note = e;
        }
        platform::guard(self.unsaved());
        if cfg!(target_arch = "wasm32") {
            egui::Panel::top("web").show(ui, |ui| ui.horizontal_wrapped(|ui| {
                ui.label("Runs in your browser. Your files never leave this computer. Saving downloads the edited file: put it in your save folder (replace the old one) while the game is closed.");
                ui.hyperlink_to("Credits and licenses", "https://github.com/kkuzar/d2rs/tree/main/licenses");
            }));
        }
        let mut to_open = None;
        egui::Panel::left("files").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open folder...").clicked() {
                    platform::pick(Pick::SaveFolder, ui.ctx());
                }
                if let Some(d) = platform::picked(Pick::SaveFolder) {
                    self.note.clear();
                    if cfg!(target_arch = "wasm32") {
                        (self.folders, self.files) = (vec![], vec![]); // the newest pick is the only one listed
                    }
                    self.folders.push(d);
                    self.scan();
                    platform::release(self.cur.as_ref().map(|c| c.path.as_path()));
                }
                if ui.button("Game folder...").on_hover_text(self.items.game_dir().map_or("for item art".into(), |d| d.display().to_string())).clicked() {
                    platform::pick(Pick::GameFolder, ui.ctx());
                }
                if let Some(d) = platform::picked(Pick::GameFolder) {
                    self.note.clear();
                    self.items.set_game_dir(d);
                    // The tables may have changed: reopen the save from disk, or lock it when it has unsaved edits.
                    match self.cur.as_ref().map(|c| c.path.clone()) {
                        Some(p) if !self.unsaved() => self.open(p),
                        Some(_) => {
                            crate::setup_ui::adapt(self.items.game_dir(), None, None);
                            crate::setup_ui::force_readonly("the game folder changed with unsaved edits; save is locked, reopen the file");
                        }
                        None => drop(crate::setup_ui::adapt(self.items.game_dir(), None, None)),
                    }
                    if let Some(c) = &self.cur {
                        self.items.resolve(&c.save);
                    }
                    self.refresh();
                }
                if ui.button("Open file...").clicked() {
                    platform::pick(Pick::SaveFile, ui.ctx());
                }
                to_open = platform::picked(Pick::SaveFile);
                if to_open.is_some() && cfg!(target_arch = "wasm32") {
                    (self.folders, self.files) = (vec![], vec![]);
                }
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for f in &self.files {
                    let on = self.cur.as_ref().is_some_and(|c| &c.path == f);
                    if ui.selectable_label(on, stem(f)).on_hover_text(f.display().to_string()).clicked() {
                        to_open = Some(f.clone());
                    }
                }
            });
        });
        if let Some(p) = to_open {
            self.open(p);
        }

        egui::Panel::bottom("status").show(ui, |ui| match &self.cur {
            Some(c) => {
                ui.horizontal(|ui| {
                    status_button(ui, &mut self.setup_open);
                    ui.label(format!("Bytes left: {}", d2io::MAX_SAVE as i64 - c.size as i64));
                    ui.label(&self.msg);
                });
                if !self.note.is_empty() {
                    ui.colored_label(egui::Color32::RED, &self.note);
                }
                for f in &c.findings {
                    let col = if f.level == Level::Error { egui::Color32::RED } else { egui::Color32::YELLOW };
                    ui.colored_label(col, format!("{:?}: {}", f.level, f.msg));
                }
            }
            None => drop(ui.horizontal(|ui| {
                status_button(ui, &mut self.setup_open);
                ui.label("No file open");
                ui.colored_label(egui::Color32::RED, &self.note);
            })),
        });
        egui::Window::new("Game setup").open(&mut self.setup_open).vscroll(true).show(&ctx, |ui| crate::setup_ui::lines().iter().for_each(|l| drop(ui.label(l))));

        let mut save_clicked = false;
        let mut restore = None;
        let mut edited = false;
        let mut refresh = false;
        let before = self.cur.as_ref().map(|c| (c.save.clone(), (self.tab == 5).then(|| self.items.snapshot())));
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(c) = &mut self.cur else {
                ui.label("Pick a save on the left, or use the Map tab with a typed seed.");
                ui.separator();
                return self.map.show(ui, self.items.game_dir(), None);
            };
            ui.horizontal(|ui| {
                for (i, t) in TABS.iter().enumerate().filter(|(i, _)| cfg!(not(target_arch = "wasm32")) || !matches!(i, 6 | 7)) {
                    ui.selectable_value(&mut self.tab, i, *t);
                }
                ui.separator();
                save_clicked = ui.add_enabled(!has_error(&c.findings) && crate::setup_ui::readonly().is_none(), egui::Button::new("Save")).on_disabled_hover_text(crate::setup_ui::readonly().unwrap_or_default()).clicked();
                ui.label(format!("undo {} / redo {}", self.undo.len(), self.redo.len()));
            });
            ui.separator();
            let s = &mut c.save;
            let sch = Schema::classic();
            let stat = |s: &Save, id| s.get_stat(id).unwrap_or(0);
            match self.tab {
                0 => {
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut c.name_buf);
                        if ui.button("Rename").clicked() {
                            match s.set_name(&c.name_buf) {
                                Ok(()) => edited = true,
                                Err(e) => c.name_buf = format!("{} ({e})", s.name()),
                            }
                        }
                    });
                    let mut cl = usize::from(s.class());
                    egui::ComboBox::from_label("Class").selected_text(game::CLASSES.get(cl).copied().unwrap_or("?")).show_ui(ui, |ui| {
                        for (i, n) in game::CLASSES.iter().enumerate() {
                            ui.selectable_value(&mut cl, i, *n);
                        }
                    });
                    if cl != usize::from(s.class()) {
                        s.set_class(cl as u8);
                        edited = true;
                    }
                    let mut lv = i64::from(s.level());
                    if num(ui, "Level", &mut lv, d2s::limits::char_stat(sch, 12, 0).unwrap_or(1..=99)) {
                        edited |= s.set_level_checked(sch, lv).is_ok();
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Set exp for this level").clicked() {
                            s.set_stat(13, i64::from(game::exp_for_level(u32::from(s.level()))));
                            edited = true;
                        }
                        if ui.button("Set level from exp").clicked() {
                            let l = game::level_for_exp(stat(s, 13) as u32);
                            s.set_level(l as u8);
                            s.set_stat(12, i64::from(l));
                            edited = true;
                        }
                    });
                    for (l, id) in [("Experience", 13), ("Gold", 14), ("Stash gold", 15)] {
                        let mut v = stat(s, id);
                        if num(ui, l, &mut v, d2s::limits::char_stat(sch, id, stat(s, 12)).unwrap_or(0..=0)) {
                            edited |= s.set_stat_checked(sch, id, v).is_ok();
                        }
                    }
                    for (l, bit) in [("Hardcore", d2s::save::HARDCORE), ("Ladder", d2s::save::LADDER), ("Expansion", d2s::save::EXPANSION), ("Died", d2s::save::DIED)] {
                        let mut on = s.status(bit);
                        if ui.checkbox(&mut on, l).changed() {
                            s.set_status(bit, on);
                            edited = true;
                        }
                    }
                    ui.add_enabled_ui(s.ver >= 92, |ui| {
                        ui.group(|ui| {
                            ui.label("Map seed");
                            let mut seed = s.map_seed();
                            ui.horizontal(|ui| {
                                let ch = ui.add(egui::DragValue::new(&mut seed).range(0..=u32::MAX)).changed();
                                ui.label(format!("0x{seed:X}"));
                                let rnd = ui.button("Randomize").clicked();
                                if rnd {
                                    seed = random_seed();
                                }
                                if (ch || rnd) && s.set_map_seed(seed).is_ok() {
                                    edited = true;
                                }
                                if ui.button(format!("Copy `-seed {seed}`")).clicked() {
                                    ctx.copy_text(format!("-seed {seed}"));
                                }
                            });
                            ui.horizontal(|ui| {
                                for (d, n) in DIFFS.iter().enumerate() {
                                    let mut on = s.seed_reuse(d);
                                    if ui.checkbox(&mut on, format!("reuse on {n}")).changed() && s.set_seed_reuse(d, on).is_ok() {
                                        edited = true;
                                    }
                                }
                            });
                        });
                    })
                    .response
                    .on_disabled_hover_text("legacy save: read-only");
                }
                1 => {
                    for (l, id) in [("Strength", 0), ("Energy", 1), ("Dexterity", 2), ("Vitality", 3), ("Stat points", 4), ("Skill points", 5)] {
                        let mut v = stat(s, id);
                        if num(ui, l, &mut v, d2s::limits::char_stat(sch, id, 0).unwrap_or(0..=0)) {
                            edited |= s.set_stat_checked(sch, id, v).is_ok();
                        }
                    }
                    for (l, cur, max) in [("Life", 6, 7), ("Mana", 8, 9), ("Stamina", 10, 11)] {
                        ui.label(format!("{l}: {} / {}", stat(s, cur) >> 8, stat(s, max) >> 8));
                    }
                }
                2 => {
                    ui.checkbox(&mut self.advanced, "advanced (up to 99)");
                    let max = if self.advanced { *d2s::limits::skill_level().end() } else { 20 };
                    for (i, (n, req)) in game::class_skills(usize::from(s.class())).into_iter().enumerate() {
                        let Some(&b) = s.skills.get(2 + i) else { break };
                        ui.horizontal(|ui| {
                            let mut v = i64::from(b);
                            ui.label(format!("{n} (req lvl {req})"));
                            if ui.add(egui::DragValue::new(&mut v).range(0..=max)).changed() {
                                edited |= s.set_skill(i, v).is_ok();
                            }
                        });
                    }
                }
                3 => {
                    diff_picker(ui, &mut self.diff);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, (act, name, _)) in game::QUESTS.iter().enumerate() {
                            if i == 0 || game::QUESTS[i - 1].0 != *act {
                                ui.strong(format!("Act {act}"));
                            }
                            let mut on = game::quest_done(s, self.diff, i);
                            if ui.checkbox(&mut on, *name).changed() {
                                game::set_quest_done(s, self.diff, i, on);
                                edited = true;
                            }
                        }
                        ui.strong("Pending rewards");
                        for (i, p) in game::PENDING.iter().enumerate() {
                            let mut on = game::is_pending(s, self.diff, i);
                            if ui.checkbox(&mut on, p.0).on_hover_text("Warning: pending rewards change how the game treats this quest").changed() {
                                game::set_pending(s, self.diff, i, on);
                                edited = true;
                            }
                        }
                    });
                }
                4 => {
                    diff_picker(ui, &mut self.diff);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for act in 1..=5 {
                            ui.horizontal(|ui| {
                                ui.strong(format!("Act {act}"));
                                if act > 1 && ui.button("Unlock").clicked() {
                                    let ok = rfd::MessageDialog::new().set_title("Unlock act").set_description("Opens the act and all its waypoints. Act 5 may mark the Act 4 boss as killed. Continue?").set_buttons(rfd::MessageButtons::YesNo).show();
                                    if ok == rfd::MessageDialogResult::Yes {
                                        game::unlock_act(s, self.diff, act);
                                        edited = true;
                                    }
                                }
                            });
                            for i in game::ACT_WP[act - 1]..game::ACT_WP[act] {
                                let mut on = game::waypoint(s, self.diff, i) || i == 0;
                                if ui.add_enabled(i != 0, egui::Checkbox::new(&mut on, game::WAYPOINTS[i])).changed() {
                                    game::set_waypoint(s, self.diff, i, on);
                                    edited = true;
                                }
                            }
                        }
                    });
                }
                5 => {
                    let big = d2s::edit::big_stash();
                    edited |= self.items.show(ui, s);
                    refresh |= big != d2s::edit::big_stash();
                }
                #[cfg(not(target_arch = "wasm32"))]
                6 => self.live.show(ui, crate::setup_ui::readonly()),
                8 => self.map.show(ui, self.items.game_dir(), Some((s.map_seed(), s.status(d2s::save::EXPANSION)))),
                _ => {
                    let list = d2io::backups(&self.writer.backup_root, &stem(&c.path));
                    if list.is_empty() {
                        ui.label("No backups yet");
                    }
                    for b in list {
                        ui.horizontal(|ui| {
                            ui.label(b.file_name().unwrap_or_default().to_string_lossy());
                            if ui.button("Restore").clicked() {
                                let shared = d2host::read_dir(&b).into_iter().flatten().any(|e| e.extension().is_some_and(|x| x == "sss"));
                                let text = if shared { "Replace the current character files with this backup?\n\nThe backup contains the shared stash: it is rolled back too, and changes made by other characters since then are lost." } else { "Replace the current character files with this backup?" };
                                let ok = rfd::MessageDialog::new().set_title("Restore backup").set_description(text).set_buttons(rfd::MessageButtons::YesNo).show();
                                if ok == rfd::MessageDialogResult::Yes {
                                    restore = Some(b.clone());
                                }
                            }
                        });
                    }
                }
            }
        });
        if refresh && !edited {
            self.refresh();
        }
        if edited {
            if let Some(b) = before.filter(|_| !self.gesture) {
                self.undo.push(b);
                if self.undo.len() > 200 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.gesture = true;
            self.dirty = true;
            self.refresh();
        } else if !ctx.input(|i| i.pointer.any_down()) {
            self.gesture = false;
        }
        if let Some(b) = restore {
            self.restore(&b);
        }
        if save_clicked {
            self.save();
        }
    }
}
