//! Live tab: read-only inspection of a running game (version 1.14d) through d2mem, with opt-in experimental writes.
use d2mem::{ImageCheck, Inspector, PlayerView, Process, WRITABLE};
use d2s::Schema;
use eframe::egui;

#[derive(Default)]
pub struct Live {
    insp: Option<Inspector<Process>>,
    view: Option<PlayerView>,
    msg: String,
    exp: bool,
    confirmed: bool,
    check: Option<ImageCheck>,
    /// None: Game.exe path unknown or its hash differs; the reason is shown and writes stay off.
    why: Option<String>,
    stat: usize,
    val: i32,
    base: u32,
}

impl Live {
    fn refresh(&mut self) {
        let Some(i) = &self.insp else { return };
        match i.player() {
            Ok(v) => (self.view, self.msg) = (Some(v), String::new()),
            Err(e) => (self.view, self.msg) = (None, format!("Read failed: {e}")),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, readonly: Option<String>) {
        let sch = Schema::classic();
        ui.label("Read-only by default. Attaching usually needs administrator (Windows) or root (macOS, Linux) rights.");
        ui.horizontal(|ui| {
            if ui.button("Attach").clicked() {
                match d2mem::find_game() {
                    Ok(p) => {
                        self.base = p.base;
                        (self.exp, self.confirmed) = (false, false);
                        self.check = None;
                        self.why = match p.exe_path() {
                            None => Some("the Game.exe path could not be determined".into()),
                            Some(e) => match d2mem::verify_image(&e) {
                                ImageCheck::Unknown => Some(format!("{} is not the 1.14d Game.exe (layout differs or unreadable)", e.display())),
                                c => {
                                    self.check = Some(c);
                                    None
                                }
                            },
                        };
                        self.insp = Some(Inspector::new(p, self.base));
                        self.refresh();
                    }
                    Err(e) => self.msg = format!("Attach failed: {e} (attaching usually needs administrator rights)"),
                }
            }
            if ui.add_enabled(self.insp.is_some(), egui::Button::new("Refresh")).clicked() {
                self.refresh();
            }
        });
        if !self.msg.is_empty() {
            ui.colored_label(egui::Color32::YELLOW, &self.msg);
        }
        if let Some(v) = &self.view {
            ui.label(format!("Name: {}   Class: {}   Image base: {:#x}   Items: {}", v.name, d2s::game::CLASSES.get(v.class as usize).copied().unwrap_or("?"), self.base, v.item_count));
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                egui::Grid::new("live_stats").striped(true).show(ui, |ui| {
                    for &(id, param, val) in &v.full_stats {
                        ui.label(sch.stat(u32::from(id)).map_or_else(|| format!("stat{id}"), |s| s.name.clone()));
                        ui.label(if param != 0 { format!("[{param}]") } else { String::new() });
                        ui.label(if (6..=11).contains(&id) { format!("{} ({val} raw)", val >> 8) } else { val.to_string() });
                        ui.end_row();
                    }
                });
            });
        }
        let Some(i) = &mut self.insp else { return };
        let mut go = false;
        ui.separator();
        if let Some(r) = readonly {
            self.why = Some(format!("the save tables are read-only ({r})"));
        }
        if let Some(w) = &self.why {
            self.exp = false;
            ui.colored_label(egui::Color32::YELLOW, format!("Writes disabled: {w}."));
        }
        ui.add_enabled(self.why.is_none(), egui::Checkbox::new(&mut self.exp, "Experimental writes"));
        if self.check == Some(ImageCheck::Layout1_14d) {
            ui.colored_label(egui::Color32::YELLOW, "This Game.exe is a modified 1.14d (same layout, different bytes). Writes are allowed only if you confirm.");
            ui.add_enabled(self.why.is_none(), egui::Checkbox::new(&mut self.confirmed, "I confirm writes to this modified Game.exe"));
        }
        let ok = self.why.is_none() && d2mem::write_allowed(self.exp, self.confirmed, self.check.unwrap_or(ImageCheck::Unknown));
        i.allow_experimental_writes = ok;
        ui.colored_label(egui::Color32::LIGHT_RED, "Unverified: a write may be lost, may corrupt the running game or crash it. Use only on a throwaway session.");
        if ok {
            let lvl = self.view.as_ref().and_then(|v| v.base_stats.iter().find(|e| e.0 == 12)).map_or(1, |e| i64::from(e.2));
            let range = d2s::limits::char_stat(sch, WRITABLE[self.stat], lvl).unwrap_or(0..=0);
            let name = |id: u32| sch.stat(id).map_or_else(|| format!("stat{id}"), |s| s.name.clone());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("live_stat").selected_text(name(WRITABLE[self.stat])).show_ui(ui, |ui| {
                    for (k, id) in WRITABLE.iter().enumerate() {
                        ui.selectable_value(&mut self.stat, k, name(*id));
                    }
                });
                ui.add(egui::DragValue::new(&mut self.val).range(*range.start() as i32..=*range.end() as i32));
                go = ui.button("Write").clicked();
            });
        }
        if go {
            let m = match self.insp.as_ref().map(|i| i.set_base_stat(WRITABLE[self.stat], self.val)) {
                Some(Ok(())) => "Written".into(),
                Some(Err(e)) => format!("Write failed: {e}"),
                None => String::new(),
            };
            self.refresh();
            if !m.is_empty() {
                self.msg = m;
            }
        }
    }
}
