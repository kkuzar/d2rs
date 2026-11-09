//! The selected-item editor of the Items tab: seed and reroll, per-line values, add/remove property, quality, ilvl, ethereal, sockets.
use d2s::itemgen::{Kind, Trait, bounds, edit_traits, reroll_in_place};
use d2s::tables::{Block, ItemTables};
use d2s::{Item, ItemView, Prop, Schema};
use eframe::egui;

#[derive(Default)]
pub struct TraitEd {
    seen: ItemView,
    bounds: Option<(Item, Item)>,
    dec: String,
    hex: String,
    keep: bool,
    custom: bool,
    search: String,
    code: String,
    par: String,
    val: i32,
}

pub(crate) fn random() -> u32 {
    let n = d2host::now_nanos();
    (n % 1_000_000_000) as u32 ^ (n / 1_000_000_000) as u32
}

/// One property line: a slider for a single-value stat, else its text; the new shown value when moved. The slider spans
/// the line's values (found by stat and param) in the items at every minimum and maximum, `lo` and `hi`, and its own
/// value, or the whole game range when `custom` or when neither holds the line.
pub fn line(ui: &mut egui::Ui, sch: &Schema, p: &Prop, lo: &[Prop], hi: &[Prop], custom: bool) -> Option<i64> {
    let Some(s) = sch.stat(p.id).filter(|s| s.save_bits > 0 && p.vals.len() == 1) else {
        ui.label(d2s::edit::prop_text(p, sch));
        return None;
    };
    let shown = |x: u32| i64::from(x) - i64::from(s.save_add);
    let mut val = shown(p.vals[0]);
    let table: Vec<i64> = [lo, hi].iter().filter_map(|l| l.iter().find(|q| q.id == p.id && q.param == p.param && q.vals.len() == 1)).map(|q| shown(q.vals[0])).collect();
    let (a, b) = table.iter().fold((val, val), |(a, b), &x| (a.min(x), b.max(x)));
    let r = if custom || table.is_empty() { d2s::limits::item_prop(sch, p.id).unwrap_or(0..=0) } else { a..=b };
    let label = if p.param > 0 { format!("{} [{}]", s.name, p.param) } else { s.name.clone() };
    ui.add(egui::Slider::new(&mut val, r).text(label)).changed().then_some(val)
}

impl TraitEd {
    /// Draws the editor for `it`; true when the item changed. A refused edit leaves the item as it was and says why in `msg`.
    pub fn show(&mut self, ui: &mut egui::Ui, it: &mut Item, msg: &mut String) -> bool {
        let (t, sch) = (ItemTables::get(), Schema::classic());
        if self.seen != it.view {
            self.bounds = bounds(t, sch, it);
            (self.dec, self.hex, self.seen) = (it.view.seed.to_string(), format!("{:X}", it.view.seed), it.view.clone());
        }
        let (v, mut edits, mut seed) = (it.view.clone(), vec![], None);
        let editable = it.editable && !v.flag(21) && !v.flag(16);
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Seed");
                if ui.add(egui::TextEdit::singleline(&mut self.dec).desired_width(90.0)).changed() {
                    self.hex = self.dec.parse::<u32>().map_or(String::new(), |n| format!("{n:X}"));
                }
                if ui.add(egui::TextEdit::singleline(&mut self.hex).desired_width(80.0).hint_text("hex")).changed() {
                    self.dec = u32::from_str_radix(&self.hex, 16).map_or(String::new(), |n| n.to_string());
                }
                if ui.button("Apply seed").clicked() {
                    seed = Some(self.dec.parse::<u32>().map_err(|_| "seed: expected a number up to 4294967295".to_string()));
                }
                if ui.add_enabled(crate::setup_ui::gen_off().is_none(), egui::Button::new("Reroll")).clicked() {
                    seed = Some(Ok(random()));
                }
                ui.checkbox(&mut self.keep, "keep unique/set/affixes");
            });
            if v.flag(26) {
                ui.label("runeword lines: seed is not stored in the save");
            }
            ui.checkbox(&mut self.custom, "custom (any value inside the game range)");
            let (lo, hi) = self.bounds.as_ref().map_or((&[][..], &[][..]), |b| (&b.0.view.props[..], &b.1.view.props[..]));
            for p in &v.props {
                ui.horizontal(|ui| {
                    if let Some(val) = line(ui, sch, p, lo, hi, self.custom) {
                        edits.push(Trait::SetProp { stat: p.id, param: p.param, shown: val as i32 });
                    }
                    if ui.small_button("x").on_hover_text("remove").clicked() {
                        edits.push(Trait::RemoveProp { stat: p.id, param: p.param });
                    }
                });
            }
            for p in &v.rw_props {
                ui.label(format!("runeword: {}", d2s::edit::prop_text(p, sch)));
            }
            ui.collapsing("Add property", |ui| {
                ui.text_edit_singleline(&mut self.search).on_hover_text("search");
                let q = self.search.to_lowercase();
                egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                    for d in t.props.iter().filter(|d| !d.code.is_empty() && d.code.to_lowercase().contains(&q)) {
                        ui.selectable_value(&mut self.code, d.code.clone(), &d.code);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(&self.code);
                    ui.add(egui::TextEdit::singleline(&mut self.par).desired_width(50.0).hint_text("par"));
                    ui.add(egui::DragValue::new(&mut self.val).range(-1_000_000..=1_000_000));
                    if ui.button("Add").clicked() {
                        edits.push(Trait::AddProp(Block { code: self.code.clone(), par: self.par.clone(), min: self.val, max: self.val }));
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                let cur = ["", "Low", "Normal", "Superior"].get(v.quality as usize).map_or("keep", |s| s);
                egui::ComboBox::from_label("Quality").selected_text(cur).show_ui(ui, |ui| {
                    for (n, k) in [("Normal", Kind::Normal), ("Low", Kind::Low(0)), ("Superior", Kind::Superior(0))] {
                        if ui.selectable_label(n == cur, n).clicked() {
                            edits.push(Trait::Quality(k));
                        }
                    }
                });
                let mut ilvl = v.ilvl;
                if ui.add(egui::DragValue::new(&mut ilvl).range(1..=99).prefix("ilvl ")).changed() {
                    edits.push(Trait::Ilvl(ilvl));
                }
                let mut sockets = v.sockets;
                let cap = *d2s::limits::sockets(sch, v.code, v.ilvl).end();
                // a runeword's socket count is fixed by its runes (sockets-runewords.md D1)
                if ui.add_enabled(!v.flag(26), egui::DragValue::new(&mut sockets).range(v.n_filled..=cap.max(v.n_filled)).prefix("sockets ")).changed() {
                    edits.push(Trait::Sockets(sockets));
                }
                let mut eth = v.flag(22);
                if ui.checkbox(&mut eth, "ethereal").changed() {
                    edits.push(Trait::Ethereal(eth));
                }
            });
        });
        let f = d2s::legit::check(t, sch, it);
        ui.colored_label(if f.is_empty() { egui::Color32::LIGHT_GREEN } else { egui::Color32::YELLOW }, if f.is_empty() { "Legit".to_string() } else { f.join("; ") });
        let res = match seed {
            Some(Ok(s)) => reroll_in_place(t, sch, it, s, self.keep, v.version >= 100).map_err(|e| e.msg),
            Some(Err(e)) => Err(e),
            None if edits.is_empty() => return false,
            None => edit_traits(t, sch, it, &edits).map_err(|e| e.msg),
        };
        *msg = res.as_ref().err().cloned().unwrap_or_default();
        res.is_ok()
    }
}
