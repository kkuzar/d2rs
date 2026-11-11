//! Create item: the `add-item` request parser and placement shared by the CLI, and the Items tab "Create item..." window.
use d2s::edit::{free_spot, grid_size, type_chain};
use d2s::itemgen::{Kind, Spec, affix_ok, make};
use d2s::legit::types_ok;
use d2s::props::Roll;
use d2s::tables::{Affix, ItemTables};
use d2s::{Item, Save, Schema};
use eframe::egui;

pub const KINDS: [&str; 6] = ["unique", "set", "runeword", "magic", "rare", "base"];
const STORES: [(&str, u32); 3] = [("inventory", 1), ("stash", 5), ("cube", 4)];

/// Draws values from the item seed (random), or takes every maximum or minimum.
fn rolled(mode: &str, f: impl FnOnce(&mut Roll) -> Result<Item, String>) -> Result<Item, String> {
    let mut seed = d2s::rng::Seed::init(crate::trait_ed::random());
    f(&mut match mode {
        "max" => Roll::Max,
        "min" => Roll::Min,
        "random" => Roll::Seed(&mut seed),
        m => return Err(format!("roll: expected max, min or random, not {m}")),
    })
}

/// Puts `it` in the first free spot of the stores named by `to` (default inventory, stash, cube); changes nothing when none fits.
pub fn put(save: &mut Save, mut it: Item, to: &[&str]) -> Result<(), String> {
    let (sch, exp) = (Schema::classic(), save.status(d2s::save::EXPANSION));
    let b = sch.base(it.view.code).ok_or("unknown base")?;
    for (_, store) in STORES.iter().filter(|s| to.is_empty() || to.contains(&s.0)) {
        if let Some(g) = grid_size(*store, exp)
            && let Some((x, y)) = free_spot(&save.items, sch, *store, g, b.w.max(1), b.h.max(1))
        {
            it.set_pos(0, 0, x, y, *store).map_err(|e| e.to_string())?;
            d2s::edit::push_item(save, it);
            return Ok(());
        }
    }
    Err("no free space in the inventory, stash or cube".into())
}

/// The type chain of the base with item code `c`.
#[doc(hidden)]
pub fn chain(c: &str) -> Option<Vec<&'static str>> {
    let sch = Schema::classic();
    sch.bases.iter().find(|b| String::from_utf8_lossy(b.0).trim() == c).map(|b| type_chain(sch, &b.1.typ))
}

/// Candidate rows (plus `off`) for a name (case-insensitive) or a plain id.
fn ids<'a>(names: impl Iterator<Item = &'a str>, s: &str, off: u32) -> Vec<u32> {
    s.parse().map_or_else(|_| names.enumerate().filter(|n| n.1.eq_ignore_ascii_case(s)).map(|n| n.0 as u32 + off).collect(), |n| vec![n])
}

/// An affix id by name (first row eligible on `chain`) or id; 0 or empty means none.
fn affix(l: &[Affix], s: &str, chain: &[&str], exp: bool, rare: bool) -> Result<u32, String> {
    if s.is_empty() || s == "0" {
        return Ok(0);
    }
    ids(l.iter().map(|a| a.name.as_str()), s, 1).into_iter().find(|&i| l.get((i as usize).wrapping_sub(1)).is_some_and(|a| s.parse::<u32>().is_ok() || affix_ok(a, chain, exp, rare))).ok_or(format!("no eligible affix {s} on this base"))
}

/// Builds the item an `add-item` request describes: `<kind>=<name|id>` plus base, roll, ilvl, eth, sockets.
pub fn from_args(save: &Save, kv: &[String]) -> Result<Item, String> {
    let (t, sch, exp) = (ItemTables::get(), Schema::classic(), save.status(d2s::save::EXPANSION));
    let get = |k: &str| kv.iter().find_map(|a| a.split_once('=').filter(|p| p.0 == k).map(|p| p.1));
    let num = |k: &str, d: u32| get(k).map_or(Ok(d), |v| v.parse::<u32>().map_err(|_| format!("{k}: not a number: {v}")));
    let (kind, name) = KINDS.iter().find_map(|k| get(k).map(|v| (*k, v))).ok_or("need one of unique= set= runeword= magic= rare= base=")?;
    let trim = |c: [u8; 4]| String::from_utf8_lossy(&c).trim().to_string();
    let base = get("base").map(str::to_string);
    let need = || base.clone().ok_or(format!("{kind}: base=<code> is required"));
    let chain_of = |c: &str| chain(c).ok_or(format!("unknown base {c}"));
    let mut sockets = num("sockets", 0)?;
    let cands: Vec<(String, Kind)> = match kind {
        "unique" => ids(t.uniques.iter().map(|u| u.name.as_str()), name, 0).into_iter().map(|i| (base.clone().unwrap_or_else(|| t.uniques.get(i as usize).map(|u| trim(u.code)).unwrap_or_default()), Kind::Unique(i))).collect(),
        "set" => ids(t.set_items.iter().map(|u| u.name.as_str()), name, 0).into_iter().map(|i| (base.clone().unwrap_or_else(|| t.set_items.get(i as usize).map(|u| trim(u.code)).unwrap_or_default()), Kind::Set(i))).collect(),
        "runeword" => {
            let key = name.parse::<u32>().map(|n| format!("Runeword{n}")).ok().or_else(|| t.runewords.iter().find(|r| r.name.eq_ignore_ascii_case(name)).map(|r| r.key.clone())).ok_or(format!("no runeword {name}"))?;
            sockets = t.runewords.iter().find(|r| r.key == key).map_or(sockets, |r| r.runes.len() as u32);
            vec![(need()?, Kind::Runeword(key))]
        }
        "magic" => {
            let (p, s) = name.split_once('/').ok_or("magic: expected <prefix>/<suffix>")?;
            let chain = chain_of(&need()?)?;
            let (p, s) = (affix(&t.magic_prefix, p, &chain, exp, false)?, affix(&t.magic_suffix, s, &chain, exp, false)?);
            vec![(need()?, Kind::Magic((p > 0).then_some(p), (s > 0).then_some(s)))]
        }
        "rare" => {
            let (a, b) = name.split_once('/').ok_or("rare: expected <prefix name>/<suffix name>")?;
            let chain = chain_of(&need()?)?;
            let list: Vec<u32> = get("affixes").unwrap_or("").split(',').enumerate().map(|(k, s)| affix(if k % 2 == 0 { &t.magic_prefix } else { &t.magic_suffix }, s, &chain, exp, true)).collect::<Result<_, _>>()?;
            let (ps, ss) = (ids(t.rare_prefix.iter().map(|n| n.name.as_str()), a, 156), ids(t.rare_suffix.iter().map(|n| n.name.as_str()), b, 1));
            ps.iter().flat_map(|&p| ss.iter().map(move |&s| (p, s))).map(|(p, s)| (need().unwrap_or_default(), Kind::Rare { names: [p, s], affixes: list.clone() })).collect()
        }
        _ => vec![(name.to_string(), Kind::Normal)],
    };
    let (ilvl, ethereal) = (num("ilvl", 85)?, get("eth") == Some("1"));
    let mut last = Err(format!("no {kind} named {name}"));
    for (code, kind) in cands {
        let spec = Spec { code, kind, ilvl, ethereal, sockets, expansion: exp };
        last = rolled(get("roll").unwrap_or("random"), |r| make(t, sch, &spec, r).map_err(|e| e.to_string()));
        if last.is_ok() {
            break;
        }
    }
    last
}

#[derive(Default)]
pub struct Create {
    pub open: bool,
    #[doc(hidden)] pub kind: usize,
    #[doc(hidden)] pub search: String,
    #[doc(hidden)] pub row: u32,
    #[doc(hidden)] pub code: String,
    #[doc(hidden)] pub pre: u32,
    #[doc(hidden)] pub suf: u32,
    #[doc(hidden)] pub names: [u32; 2],
    #[doc(hidden)] pub slots: [u32; 6],
    #[doc(hidden)] pub roll: usize,
    #[doc(hidden)] pub ilvl: Option<u32>,
    #[doc(hidden)] pub eth: bool,
    #[doc(hidden)] pub sockets: u32,
    #[doc(hidden)] pub custom: bool,
    #[doc(hidden)] pub key: String,
    #[doc(hidden)] pub built: Option<Result<(Item, Item, Item), String>>,
}

fn combo(ui: &mut egui::Ui, label: &str, sel: &mut u32, opts: Vec<(u32, String)>) {
    let cur = opts.iter().find(|o| o.0 == *sel).map_or("none".to_string(), |o| o.1.clone());
    egui::ComboBox::from_label(label).selected_text(cur).show_ui(ui, |ui| {
        ui.selectable_value(sel, 0, "none");
        for (i, n) in opts {
            ui.selectable_value(sel, i, n);
        }
    });
}

impl Create {
    fn spec(&self, exp: bool) -> Option<Spec> {
        let t = ItemTables::get();
        let r = self.row as usize;
        let (code, kind) = match self.kind {
            0 => (t.uniques.get(r)?.code, Kind::Unique(self.row)),
            1 => (t.set_items.get(r)?.code, Kind::Set(self.row)),
            2 => (Default::default(), Kind::Runeword(t.runewords.get(r)?.key.clone())),
            3 => (Default::default(), Kind::Magic((self.pre > 0).then_some(self.pre), (self.suf > 0).then_some(self.suf))),
            4 => (Default::default(), Kind::Rare { names: self.names, affixes: self.slots.to_vec() }),
            _ => (Default::default(), Kind::Normal),
        };
        let code = if self.kind < 2 { String::from_utf8_lossy(&code).trim().to_string() } else { self.code.clone() };
        let sockets = if let Kind::Runeword(k) = &kind { t.runewords.iter().find(|x| x.key == *k)?.runes.len() as u32 } else { self.sockets };
        Some(Spec { code, kind, ilvl: self.ilvl.unwrap_or(85), ethereal: self.eth, sockets, expansion: exp })
    }

    /// The item for the current choices with the chosen roll, and the items at every minimum and maximum (slider bounds).
    #[doc(hidden)] pub fn build(&mut self, exp: bool, force: bool) {
        let Some(s) = self.spec(exp) else { return };
        let key = format!("{s:?}{}", self.roll);
        if key == self.key && !force {
            return;
        }
        self.key = key;
        let (t, sch) = (ItemTables::get(), Schema::classic());
        let mk = |m: &str| rolled(m, |r| make(t, sch, &s, r).map_err(|e| e.to_string()));
        self.built = Some(mk(["max", "min", "random"][self.roll]).and_then(|a| Ok((a, mk("min")?, mk("max")?))));
    }

    /// Draws the window; returns true when an item was added to `save`.
    pub fn show(&mut self, ctx: &egui::Context, save: &mut Save, msg: &mut String) -> bool {
        let (t, sch, exp) = (ItemTables::get(), Schema::classic(), save.status(d2s::save::EXPANSION));
        let (mut open, mut added) = (self.open, false);
        egui::Window::new("Create item").open(&mut open).default_width(420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, k) in KINDS.iter().enumerate() {
                    if ui.selectable_value(&mut self.kind, i, *k).clicked() {
                        (self.row, self.code, self.key) = (0, String::new(), String::new());
                    }
                }
            });
            let names: Vec<String> = match self.kind {
                0 => t.uniques.iter().map(|u| u.name.clone()).collect(),
                1 => t.set_items.iter().map(|u| u.name.clone()).collect(),
                _ => t.runewords.iter().filter(|r| r.complete).map(|r| r.name.clone()).collect(),
            };
            let all: Vec<u32> = (0..t.runewords.len() as u32).filter(|&i| t.runewords[i as usize].complete).collect();
            if self.kind < 3 {
                ui.text_edit_singleline(&mut self.search).on_hover_text("search");
                let q = self.search.to_lowercase();
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    for (k, n) in names.iter().enumerate() {
                        let row = if self.kind == 2 { all[k] } else { k as u32 };
                        if n.to_lowercase().contains(&q) && !n.is_empty() {
                            ui.selectable_value(&mut self.row, row, n);
                        }
                    }
                });
            }
            let chain = chain(&self.code);
            if self.kind >= 2 {
                let rw = t.runewords.get(self.row as usize).filter(|_| self.kind == 2);
                let mut bases: Vec<_> = sch.bases.iter().filter(|(_, b)| !b.gold && !b.quest_diff && !b.compact && (exp || b.version < 100)).filter(|(_, b)| rw.is_none_or(|r| types_ok(&r.itypes, &r.etypes, &type_chain(sch, &b.typ)))).map(|(c, b)| (b.name.clone(), String::from_utf8_lossy(c).trim().to_string())).collect();
                bases.sort();
                egui::ComboBox::from_label("Base").selected_text(bases.iter().find(|b| b.1 == self.code).map_or("pick a base", |b| &b.0)).show_ui(ui, |ui| {
                    for (n, c) in bases {
                        ui.selectable_value(&mut self.code, c.clone(), format!("{n} ({c})"));
                    }
                });
            }
            if let Some(chain) = chain.as_ref().filter(|_| self.kind == 3 || self.kind == 4) {
                let rare = self.kind == 4;
                let opts = |l: &[Affix]| l.iter().enumerate().filter(|(_, a)| affix_ok(a, chain, exp, rare)).map(|(i, a)| (i as u32 + 1, a.name.clone())).collect::<Vec<_>>();
                let (pre, suf) = (opts(&t.magic_prefix), opts(&t.magic_suffix));
                if rare {
                    combo(ui, "Name 1", &mut self.names[0], t.rare_prefix.iter().enumerate().filter(|(_, n)| types_ok(&n.itypes, &n.etypes, chain)).map(|(i, n)| (i as u32 + 156, n.name.clone())).collect());
                    combo(ui, "Name 2", &mut self.names[1], t.rare_suffix.iter().enumerate().filter(|(_, n)| types_ok(&n.itypes, &n.etypes, chain)).map(|(i, n)| (i as u32 + 1, n.name.clone())).collect());
                    for k in 0..6 {
                        combo(ui, &format!("Affix {}", k + 1), &mut self.slots[k], if k % 2 == 0 { pre.clone() } else { suf.clone() });
                    }
                } else {
                    combo(ui, "Prefix", &mut self.pre, pre);
                    combo(ui, "Suffix", &mut self.suf, suf);
                }
            }
            ui.horizontal(|ui| {
                let mut lv = self.ilvl.unwrap_or(85);
                ui.add(egui::DragValue::new(&mut lv).range(1..=99).prefix("ilvl "));
                self.ilvl = Some(lv);
                ui.add_enabled(self.kind != 2, egui::DragValue::new(&mut self.sockets).range(0..=6).prefix("sockets "));
                ui.checkbox(&mut self.eth, "ethereal");
                ui.selectable_value(&mut self.roll, 0, "max");
                ui.selectable_value(&mut self.roll, 1, "min");
                ui.selectable_value(&mut self.roll, 2, "random");
                ui.checkbox(&mut self.custom, "custom");
            });
            self.build(exp, ui.add_enabled(crate::setup_ui::gen_off().is_none(), egui::Button::new("Reroll")).clicked());
            match &mut self.built {
                None => drop(ui.label("pick an item")),
                Some(Err(e)) => drop(ui.colored_label(egui::Color32::LIGHT_RED, e.as_str())),
                Some(Ok((it, lo, hi))) => {
                    ui.strong(d2s::edit::item_name(it, sch));
                    // the min, max and chosen rolls can hold different lines (properties.md E2: the +1 max damage fallback),
                    // so each line finds its bounds by stat and param
                    for w in 0..2 {
                        let (l, h) = if w == 0 { (&lo.view.props, &hi.view.props) } else { (&lo.view.rw_props, &hi.view.rw_props) };
                        for i in 0..if w == 0 { it.view.props.len() } else { it.view.rw_props.len() } {
                            let p = if w == 0 { &it.view.props[i] } else { &it.view.rw_props[i] };
                            let add = sch.stat(p.id).map_or(0, |s| i64::from(s.save_add));
                            if let Some(val) = crate::trait_ed::line(ui, sch, p, l, h, self.custom) {
                                let v = it.view_mut();
                                (if w == 0 { &mut v.props } else { &mut v.rw_props })[i].vals[0] = (val + add) as u32;
                            }
                        }
                    }
                    let f = d2s::legit::check(t, sch, it);
                    ui.label(if f.is_empty() { "Legit".to_string() } else { f.join("; ") });
                    if ui.button("Add").clicked() {
                        let errs = d2s::limits::item_errors(sch, &it.view);
                        *msg = match (errs.is_empty(), put(save, it.clone(), &[])) {
                            (false, _) => errs.join("; "),
                            (_, Err(e)) => e,
                            _ => {
                                added = true;
                                format!("Added {}", d2s::edit::item_name(it, sch))
                            }
                        };
                    }
                }
            }
            ui.label(&*msg);
        });
        self.open = open;
        added
    }
}
