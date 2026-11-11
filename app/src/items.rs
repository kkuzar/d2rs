//! Items tab: grids, paper doll, drag and drop, tooltips, selected item editor, PlugY stash pages.
use d2data::{DataDir, GameData, Palette};
use d2s::edit::{self, fits, free_spot, grid_size};
use d2s::plugy::Stash;
use d2s::item::{ItemView, NONE};
use d2s::tables::{Affix, ItemTables};
use d2s::{Item, Save, Schema};
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[doc(hidden)]
pub const C: f32 = 29.0;
/// (body, col, row, w, h) in cells.
type Slot = (u32, f32, f32, f32, f32);
const EQ: [Slot; 10] = [(1, 3., 0., 2., 2.), (2, 5., 1., 1., 1.), (3, 3., 2., 2., 3.), (4, 0., 0., 2., 4.), (5, 6., 0., 2., 4.), (6, 2., 5., 1., 1.), (7, 5., 5., 1., 1.), (8, 3., 5., 2., 1.), (9, 6., 4., 2., 2.), (10, 0., 4., 2., 2.)];
const MERC: [Slot; 4] = [(1, 2., 0., 2., 2.), (3, 2., 2., 2., 3.), (4, 0., 0., 2., 4.), (5, 4., 0., 2., 4.)];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[doc(hidden)]
pub enum List {
    Player,
    Merc,
    Stash(usize, usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum Ref {
    L(List, usize),
    Golem,
}

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct Cand {
    #[doc(hidden)] pub list: List,
    #[doc(hidden)] pub loc: u32,
    #[doc(hidden)] pub body: u32,
    #[doc(hidden)] pub x: u32,
    #[doc(hidden)] pub y: u32,
    #[doc(hidden)] pub store: u32,
}

enum Kind {
    /// Stored items of this store (0 = any store, used by PlugY pages).
    Grid(u32),
    Belt,
    Equip(Vec<Slot>),
}

#[derive(Default)]
struct St {
    sel: Option<Ref>,
    drag: Option<(Ref, u32, u32)>,
    cand: Option<Cand>,
}

#[doc(hidden)]
pub struct Doc {
    label: String,
    path: PathBuf,
    fp: d2io::Fingerprint,
    #[doc(hidden)] pub stash: Stash,
    #[doc(hidden)] pub changed: bool,
    page: usize,
}

#[derive(Default)]
#[doc(hidden)]
pub struct Art {
    gd: Option<GameData>,
    dd: Option<DataDir>,
    pal: Option<Palette>,
    tex: HashMap<String, Option<egui::TextureHandle>>,
    maps: HashMap<u8, Vec<[u8; 256]>>,
}

impl Art {
    #[doc(hidden)] pub fn new(dir: Option<&Path>) -> Art {
        let mut a = Art::default();
        if let Some(d) = dir {
            a.gd = GameData::from_install(d).ok().map(|g| g.with_override(DataDir::new(d)));
            a.dd = Some(DataDir::new(d));
            a.pal = a.read("data\\global\\palette\\ACT1\\pal.dat").and_then(|b| Palette::from_dat(&b).ok());
        }
        a
    }

    #[doc(hidden)] pub fn read(&self, p: &str) -> Option<Vec<u8>> {
        self.gd.as_ref().and_then(|g| g.read(p)).or_else(|| self.dd.as_ref().and_then(|d| d.read(p)))
    }

    /// The inventory picture `name` (shifted by `shift`) and its native size.
    fn tex(&mut self, ctx: &egui::Context, name: &str, shift: Option<(u8, u8)>) -> Option<(egui::TextureId, egui::Vec2)> {
        if name.is_empty() || self.pal.is_none() {
            return None;
        }
        let key = format!("{name}/{shift:?}");
        if !self.tex.contains_key(&key) {
            let map = shift.and_then(|(f, c)| {
                if !self.maps.contains_key(&f) {
                    let m = self.read(&format!("data\\global\\items\\palette\\{}.dat", SHIFT_FILES[f as usize - 1])).map(|b| d2data::colormaps(&b));
                    self.maps.insert(f, m.unwrap_or_default());
                }
                self.maps[&f].get(c as usize).copied()
            });
            let t = self.read(&format!("data\\global\\items\\{name}.dc6")).and_then(|b| d2data::dc6::decode(&b).ok()).and_then(|f| {
                let f = f.into_iter().next()?;
                let rgba = self.pal.as_ref()?.rgba(&f.pixels, map.as_ref());
                Some(ctx.load_texture(key.clone(), egui::ColorImage::from_rgba_unmultiplied([f.width, f.height], &rgba), Default::default()))
            });
            self.tex.insert(key.clone(), t);
        }
        self.tex[&key].as_ref().map(|t| (t.id(), t.size_vec2()))
    }
}

/// The colour-map files of `Data\Global\Items\Palette`, transform type 1..8 (item-graphics 0003, "Files and maps").
#[doc(hidden)]
pub const SHIFT_FILES: [&str; 8] = ["grey", "grey2", "gold", "brown", "greybrown", "invgrey", "invgrey2", "invgreybrown"];

/// The inventory picture of an item and its colour shift (map file type, colour row), item-graphics 0003 sections 2 and 3.
/// Picture: identified set row `invfile`, base `setinvfile`; identified unique row `invfile` (row 0 never), base `uniqueinvfile`;
/// the item type's `InvGfx[min(gfx, VarInvGfx - 1)]`; the base `invfile`. Shift: base `InvTrans` file (types 3 and 4 are never served)
/// times the unique or set row colour, or the first magic or rare affix colour (suffixes, then prefixes), else the automagic one.
#[doc(hidden)]
pub fn inv_picture(sch: &Schema, t: &ItemTables, v: &ItemView) -> (String, Option<(u8, u8)>) {
    let Some(b) = sch.base(v.code) else { return (String::new(), None) };
    let i = v.qual.first().map_or(0, |&x| x as usize);
    let (set, uni) = (t.set_items.get(i).filter(|_| v.quality == 5), t.uniques.get(i).filter(|_| v.quality == 7));
    let own = match v.quality {
        5 if v.flag(4) => [set.map_or("", |s| &s.inv), &b.set_inv],
        7 if v.flag(4) => [uni.filter(|_| i > 0).map_or("", |u| &u.inv), &b.unique_inv],
        _ => ["", ""],
    };
    let var = sch.inv_gfx.get(&b.typ).and_then(|g| g.get((v.gfx.unwrap_or(0) as usize).min(g.len().wrapping_sub(1)))).map_or("", |s| s.as_str());
    let name = own.into_iter().chain([var, &b.invfile]).find(|n| !n.is_empty()).unwrap_or("");
    let affix = |list: &[Affix], k: usize| v.qual.get(k).filter(|&&x| x != NONE).and_then(|&x| list.get((x as usize).wrapping_sub(1))).and_then(|a| a.color);
    let auto = v.class_affix.and_then(|a| t.automagic.get((a as usize).wrapping_sub(1))).and_then(|a| a.color);
    let colour = match v.quality {
        5 => set.and_then(|s| s.inv_trans),
        7 => uni.and_then(|u| u.inv_trans),
        4 => affix(&t.magic_suffix, 1).or(affix(&t.magic_prefix, 0)).or(auto),
        6 => [3, 5, 7].iter().find_map(|&k| affix(&t.magic_suffix, k)).or_else(|| [2, 4, 6].iter().find_map(|&k| affix(&t.magic_prefix, k))).or(auto),
        _ => auto,
    };
    (name.to_string(), colour.filter(|_| matches!(b.inv_trans, 1 | 2 | 5..=8)).map(|c| (b.inv_trans, c)))
}

/// Where the game draws a picture of `size` in the item's cell block: native size, centred.
#[doc(hidden)]
pub fn frame_rect(cell: Rect, size: egui::Vec2) -> Rect {
    Rect::from_center_size(cell.center(), size)
}

pub struct ItemsUi {
    art: Art,
    game_dir: Option<PathBuf>,
    /// The manual BigStash checkbox, and PlugY.ini's `ActiveBigStash` when the game folder has one (it wins).
    manual: bool,
    ini: Option<bool>,
    /// Columns the stash grid drew last frame (tests).
    #[doc(hidden)] pub stash_cols: u32,
    #[doc(hidden)] pub docs: Vec<Doc>,
    st: St,
    swap: bool,
    import: String,
    new_code: String,
    create: crate::create::Create,
    ed: crate::trait_ed::TraitEd,
    pub msg: String,
}

fn cfg_file() -> Option<PathBuf> {
    Some(d2io::default_backup_root()?.parent()?.join("game_dir.txt"))
}

fn big_file() -> Option<PathBuf> {
    Some(cfg_file()?.with_file_name("big_stash.txt"))
}

fn ini_of(dir: Option<&Path>) -> Option<bool> {
    edit::ini_big_stash(&d2host::read_to_string(&dir?.join("PlugY.ini")).ok()?)
}

fn dims(sch: &Schema, it: &Item) -> (u32, u32) {
    sch.base(it.view.code).map_or((1, 1), |b| (b.w.max(1), b.h.max(1)))
}

/// Rows of the belt equipped in body slot 8 (one row with no belt).
fn belt_rows(save: &Save, sch: &Schema) -> u32 {
    save.items.iter().find(|i| i.view.location == 1 && i.view.body == 8).map_or(1, |i| sch.belt_rows(i.view.code))
}

/// True when the item's position is outside its store grid or the belt, so no grid shows it.
fn outside(sch: &Schema, save: &Save, it: &Item) -> bool {
    let (v, (w, h)) = (&it.view, dims(sch, it));
    match v.location {
        0 => grid_size(v.store, save.status(d2s::save::EXPANSION)).is_none_or(|(c, r)| v.x + w > c || v.y + h > r),
        2 => v.x / 4 >= belt_rows(save, sch),
        _ => false,
    }
}

/// An expansion-only base cannot go into a classic character.
fn classic_refuses(sch: &Schema, save: &Save, code: [u8; 4]) -> bool {
    !save.status(d2s::save::EXPANSION) && sch.base(code).is_some_and(|b| b.version >= 100)
}

fn items<'a>(save: &'a Save, docs: &'a [Doc], l: List) -> &'a [Item] {
    match l {
        List::Player => &save.items,
        List::Merc => save.merc.as_deref().unwrap_or(&[]),
        List::Stash(d, p) => docs.get(d).and_then(|d| d.stash.pages.get(p)).map_or(&[], |p| &p.items),
    }
}

fn items_mut<'a>(save: &'a mut Save, docs: &'a mut [Doc], l: List) -> Option<&'a mut Vec<Item>> {
    match l {
        List::Player => Some(&mut save.items),
        List::Merc => save.merc.as_mut(),
        List::Stash(d, p) => docs.get_mut(d)?.stash.pages.get_mut(p).map(|p| &mut p.items),
    }
}

fn get_mut<'a>(save: &'a mut Save, docs: &'a mut [Doc], r: Ref) -> Option<&'a mut Item> {
    match r {
        Ref::Golem => save.kf.as_mut()?.as_mut(),
        Ref::L(l, i) => items_mut(save, docs, l)?.get_mut(i),
    }
}

fn tip(it: &Item, sch: &Schema) -> String {
    let v = &it.view;
    let mut s = edit::item_name(it, sch);
    for p in v.props.iter().chain(v.set_props.iter().flatten()).chain(&v.rw_props) {
        s += &format!("\n{}", edit::prop_text(p, sch));
    }
    if v.sockets > 0 {
        s += &format!("\nSockets: {} ({} filled)", v.sockets, it.children.len());
        for c in &it.children {
            s += &format!("\n  {}", edit::item_name(c, sch));
        }
    }
    if v.flag(22) {
        s += "\nEthereal";
    }
    if !it.editable {
        s += "\nread-only (failed self-check)";
    }
    if let Some(r) = crate::setup_ui::gen_off() {
        return s + &format!("\nLegit check unavailable: {r}");
    }
    let f = d2s::legit::check(d2s::tables::ItemTables::get(), sch, it);
    s + &if f.is_empty() { "\nLegit".to_string() } else { format!("\nNot legit: {}", f.join("; ")) }
}

fn qcolor(q: u32) -> Color32 {
    match q {
        4 => Color32::from_rgb(70, 90, 190),
        5 => Color32::from_rgb(40, 150, 60),
        6 => Color32::from_rgb(190, 180, 50),
        7 => Color32::from_rgb(170, 130, 50),
        8 => Color32::from_rgb(210, 120, 40),
        _ => Color32::from_gray(90),
    }
}

fn paint_item(art: &mut Art, ctx: &egui::Context, p: &egui::Painter, r: Rect, it: &Item, sch: &Schema, tint: Color32) {
    let (name, shift) = inv_picture(sch, ItemTables::get(), &it.view);
    match art.tex(ctx, &name, shift) {
        Some((t, size)) => {
            p.rect_filled(r, 0.0, Color32::from_black_alpha(90));
            p.image(t, frame_rect(r, size), Rect::from_min_max(pos2(0., 0.), pos2(1., 1.)), tint);
        }
        None => {
            p.rect_filled(r.shrink(1.0), 2.0, qcolor(it.view.quality).gamma_multiply(tint.a() as f32 / 255.0));
            p.text(r.center(), Align2::CENTER_CENTER, String::from_utf8_lossy(&it.view.code).trim(), FontId::monospace(11.0), Color32::WHITE);
        }
    }
}

struct Draw<'a> {
    art: &'a mut Art,
    st: &'a mut St,
    sch: &'a Schema,
    save: &'a Save,
    docs: &'a [Doc],
}

impl Draw<'_> {
    /// Items outside every grid, drawn as tiles so they can be dragged back in.
    fn strip(&mut self, ui: &mut egui::Ui) {
        let save = self.save;
        let out: Vec<usize> = (0..save.items.len()).filter(|&i| outside(self.sch, save, &save.items[i])).collect();
        if out.is_empty() {
            return;
        }
        ui.strong("Out of grid");
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            for i in out {
                let (it, r) = (&save.items[i], Ref::L(List::Player, i));
                let (w, h) = dims(self.sch, it);
                let (resp, p) = ui.allocate_painter(vec2(w as f32, h as f32) * C, Sense::click_and_drag());
                paint_item(self.art, &ctx, &p, resp.rect, it, self.sch, if self.st.drag.is_some_and(|d| d.0 == r) { Color32::from_white_alpha(70) } else { Color32::WHITE });
                if self.st.sel == Some(r) {
                    p.rect_stroke(resp.rect, 0.0, Stroke::new(2.0, Color32::YELLOW), StrokeKind::Inside);
                }
                if resp.drag_started() {
                    self.st.drag = Some((r, w, h));
                }
                if resp.drag_started() || resp.clicked() {
                    self.st.sel = Some(r);
                }
                drop(resp.on_hover_text(tip(it, self.sch)));
            }
        });
    }

    fn area(&mut self, ui: &mut egui::Ui, list: List, cols: u32, rows: u32, kind: Kind) {
        let (resp, p) = ui.allocate_painter(vec2(cols as f32, rows as f32) * C, Sense::hover());
        let org = resp.rect.min;
        let ctx = ui.ctx().clone();
        let its = items(self.save, self.docs, list);
        let cell = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(org + vec2(x, y) * C, vec2(w, h) * C);
        let line = Stroke::new(1.0, Color32::from_gray(60));
        match &kind {
            Kind::Equip(sl) => sl.iter().for_each(|s| {
                p.rect_stroke(cell(s.1, s.2, s.3, s.4), 2.0, line, StrokeKind::Inside);
            }),
            _ => (0..rows).flat_map(|y| (0..cols).map(move |x| (x, y))).for_each(|(x, y)| {
                p.rect_stroke(cell(x as f32, y as f32, 1., 1.), 0.0, line, StrokeKind::Inside);
            }),
        }
        let gstore = match (&kind, list) {
            (_, List::Stash(..)) => its.first().map_or(5, |i| i.view.store),
            (Kind::Grid(s), _) => *s,
            _ => 0,
        };
        let slot_rect = |b: u32| match &kind {
            Kind::Equip(sl) => sl.iter().find(|s| s.0 == b).map(|s| cell(s.1, s.2, s.3, s.4)),
            _ => None,
        };
        // Draw items.
        for (i, it) in its.iter().enumerate() {
            let v = &it.view;
            let (w, h) = dims(self.sch, it);
            if list == List::Player && outside(self.sch, self.save, it) {
                continue;
            }
            let rect = match &kind {
                Kind::Grid(s) if v.location == 0 && (*s == 0 || v.store == *s) => cell(v.x as f32, v.y as f32, w as f32, h as f32),
                Kind::Belt if v.location == 2 => cell((v.x % 4) as f32, (v.x / 4) as f32, 1., 1.),
                Kind::Equip(_) if v.location == 1 => match slot_rect(v.body) {
                    Some(s) => Rect::from_center_size(s.center(), vec2(w as f32, h as f32) * C),
                    None => continue,
                },
                _ => continue,
            };
            let r = Ref::L(list, i);
            let r_ui = ui.interact(rect, ui.scope_id().with(("it", list as List, i)), Sense::click_and_drag());
            let dragged = self.st.drag.is_some_and(|d| d.0 == r);
            paint_item(self.art, &ctx, &p, rect, it, self.sch, if dragged { Color32::from_white_alpha(70) } else { Color32::WHITE });
            if self.st.sel == Some(r) {
                p.rect_stroke(rect, 0.0, Stroke::new(2.0, Color32::YELLOW), StrokeKind::Inside);
            }
            if r_ui.drag_started() {
                self.st.drag = Some((r, w, h));
                self.st.sel = Some(r);
            } else if r_ui.clicked() {
                self.st.sel = Some(r);
            }
            if self.st.drag.is_none() {
                drop(r_ui.on_hover_ui(|ui| drop(ui.label(tip(it, self.sch)))));
            }
        }
        // Drop target feedback.
        let (Some((dr, dw, dh)), Some(ptr)) = (self.st.drag, ctx.pointer_latest_pos()) else { return };
        if !resp.rect.contains(ptr) {
            return;
        }
        let ign = match dr {
            Ref::L(l, i) if l == list => Some(i),
            _ => None,
        };
        let rel = ptr - org;
        let dcode = match dr {
            Ref::L(l, i) => items(self.save, self.docs, l).get(i).map(|x| x.view.code),
            Ref::Golem => None,
        };
        let (ok, hl, cand) = match &kind {
            Kind::Grid(_) => {
                let x = ((rel.x - dw as f32 * C / 2.0) / C).round().max(0.0) as u32;
                let y = ((rel.y - dh as f32 * C / 2.0) / C).round().max(0.0) as u32;
                let ok = fits(its, self.sch, gstore, (cols, rows), x, y, dw, dh, ign);
                (ok, cell(x as f32, y as f32, dw as f32, dh as f32), Cand { list, loc: 0, body: 0, x, y, store: gstore })
            }
            Kind::Belt => {
                let (cx, cy) = ((rel.x / C) as u32, (rel.y / C) as u32);
                let belt = dcode.and_then(|c| self.sch.base(c)).is_some_and(|b| b.beltable);
                let ok = belt && cx < 4 && cy < rows && !its.iter().enumerate().any(|(i, it)| Some(i) != ign && it.view.location == 2 && it.view.x == cx + 4 * cy);
                (ok, cell(cx as f32, cy as f32, 1., 1.), Cand { list, loc: 2, body: 0, x: cx + 4 * cy, y: 0, store: 0 })
            }
            Kind::Equip(sl) => {
                let Some(s) = sl.iter().find(|s| cell(s.1, s.2, s.3, s.4).contains(ptr)) else { return };
                let fit = |c| if matches!(list, List::Merc) { edit::merc_slot_ok(self.sch, self.save.merc_type() as u32, c, s.0) } else { edit::slot_ok(self.sch, c, s.0) };
                let ok = dcode.is_some_and(fit) && !its.iter().enumerate().any(|(i, it)| Some(i) != ign && it.view.location == 1 && it.view.body == s.0);
                (ok, cell(s.1, s.2, s.3, s.4), Cand { list, loc: 1, body: s.0, x: s.0, y: 0, store: 0 })
            }
        };
        let ok = ok && !dcode.is_some_and(|c| matches!(list, List::Player | List::Merc) && classic_refuses(self.sch, self.save, c));
        p.rect_stroke(hl, 0.0, Stroke::new(2.0, if ok { Color32::GREEN } else { Color32::RED }), StrokeKind::Inside);
        self.st.cand = ok.then_some(cand);
    }
}

/// Moves the item to `c` (out of its list for a cross-list move); returns whether anything changed.
/// The position is patched on a copy first, so an item that cannot move (legacy) stays where it was.
#[doc(hidden)]
pub fn apply(save: &mut Save, docs: &mut [Doc], from: Ref, c: Cand) -> bool {
    let Ref::L(fl, fi) = from else { return false };
    let Some(mut it) = items(save, docs, fl).get(fi).cloned() else { return false };
    if it.set_pos(c.loc, c.body, c.x, c.y, c.store).is_err() || items_mut(save, docs, c.list).is_none() {
        return false;
    }
    for l in [fl, c.list] {
        if let List::Stash(d, _) = l {
            docs[d].changed = true;
        }
    }
    if fl == c.list {
        if let Some(v) = items_mut(save, docs, fl) {
            v[fi] = it;
        }
        return true;
    }
    match fl {
        List::Player => drop(edit::remove_item(save, fi)),
        _ => drop(items_mut(save, docs, fl).map(|v| v.remove(fi))),
    }
    if c.list == List::Player {
        edit::push_item(save, it);
    } else if let Some(v) = items_mut(save, docs, c.list) {
        v.push(it);
    }
    true
}

/// Puts a new item in the first free spot of `list` (the player's inventory, then stash, for `List::Player`).
fn place(save: &mut Save, docs: &mut [Doc], sch: &Schema, list: List, mut it: Item) -> Result<(), String> {
    let (w, h) = dims(sch, &it);
    if !matches!(list, List::Stash(..)) && classic_refuses(sch, save, it.view.code) {
        return Err("expansion-only item: this character is classic".into());
    }
    let exp = save.status(d2s::save::EXPANSION);
    let (store, grid) = match list {
        List::Stash(..) => (items(save, docs, list).first().map_or(5, |i| i.view.store), grid_size(5, exp).unwrap_or((10, 10))),
        _ => (1, (10, 4)),
    };
    let mut spot = free_spot(items(save, docs, list), sch, store, grid, w, h).map(|s| (store, s));
    if spot.is_none() && list == List::Player {
        spot = grid_size(5, exp).and_then(|g| free_spot(&save.items, sch, 5, g, w, h)).map(|s| (5, s));
    }
    let Some((store, (x, y))) = spot else { return Err("no free spot".into()) };
    it.set_pos(0, 0, x, y, store).map_err(|e| e.to_string())?;
    match list {
        List::Player => edit::push_item(save, it),
        l => items_mut(save, docs, l).ok_or("no such list")?.push(it),
    }
    if let List::Stash(d, _) = list {
        docs[d].changed = true;
    }
    Ok(())
}

impl Default for ItemsUi {
    fn default() -> Self {
        Self::new()
    }
}

impl ItemsUi {
    pub fn new() -> Self {
        let dir = cfg_file().and_then(|f| d2host::read_to_string(&f).ok()).map(|s| PathBuf::from(s.trim())).filter(|p| d2host::is_dir(p));
        let manual = big_file().and_then(|f| d2host::read_to_string(&f).ok()).is_some_and(|s| s.trim() == "1");
        edit::set_big_stash(manual);
        ItemsUi { art: Art::new(dir.as_deref()), manual, ini: ini_of(dir.as_deref()), stash_cols: 0, game_dir: dir, docs: vec![], st: St::default(), swap: false, import: String::new(), new_code: "hp1".into(), create: Default::default(), ed: Default::default(), msg: String::new() }
    }

    pub fn set_game_dir(&mut self, dir: PathBuf) {
        if let Some(f) = cfg_file() {
            let _ = d2host::write(&f, dir.to_string_lossy().as_bytes());
        }
        self.art = Art::new(Some(&dir));
        self.ini = ini_of(Some(&dir));
        self.game_dir = Some(dir);
    }

    /// Selects the player's item `i`.
    #[doc(hidden)]
    pub fn select(&mut self, i: usize) {
        self.st.sel = Some(Ref::L(List::Player, i));
    }

    pub fn game_dir(&self) -> Option<&Path> {
        self.game_dir.as_deref()
    }

    /// Picks the BigStash switch: PlugY.ini, else a stash item beyond 6x8 (announced), else the checkbox.
    pub fn resolve(&mut self, save: &Save) {
        let sch = Schema::classic();
        let pages = self.docs.iter().flat_map(|d| &d.stash.pages).flat_map(|p| &p.items);
        let auto = save.items.iter().filter(|i| i.view.store == 5).chain(pages).any(|i| edit::beyond_vanilla(sch, i));
        let on = self.ini.unwrap_or(auto || self.manual);
        if on && !edit::big_stash() && auto && self.ini.is_none() && !self.manual {
            self.msg = "PlugY BigStash detected from a stash item beyond 6x8: stash is 10x10".into();
        }
        edit::set_big_stash(on);
    }

    /// Loads the PlugY stash files next to the save.
    pub fn load(&mut self, save_path: &Path, save: &Save) {
        let sch = Schema::classic();
        let dir = save_path.parent().unwrap_or(Path::new("."));
        let stem = save_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        self.docs.clear();
        self.st = St::default();
        self.msg.clear();
        for (label, name) in [("Personal stash", format!("{stem}.d2x")), ("Shared stash", "_LOD_SharedStashSave.sss".into()), ("Shared stash (HC)", "_LOD_HC_SharedStashSave.sss".into())] {
            let path = dir.join(name);
            let Ok(b) = d2host::read(&path) else { continue };
            match (Stash::verify(&b, sch), Stash::parse(&b, sch), d2io::fingerprint(&path)) {
                (Ok(()), Ok(stash), Ok(fp)) => self.docs.push(Doc { label: label.into(), path, fp, stash, changed: false, page: 0 }),
                _ => self.msg = format!("{} is unreadable or fails the self-check, ignored", path.display()),
            }
        }
        self.resolve(save);
    }

    pub fn dirty_docs(&self) -> bool {
        self.docs.iter().any(|d| d.changed)
    }

    /// The stash documents for the undo stack (a move between the .d2s and a stash changes both).
    pub fn snapshot(&self) -> Vec<(Stash, bool)> {
        self.docs.iter().map(|d| (d.stash.clone(), d.changed)).collect()
    }

    /// Puts an undo snapshot back and returns the state it replaced (for redo).
    pub fn restore(&mut self, s: Vec<(Stash, bool)>) -> Vec<(Stash, bool)> {
        self.st = St::default();
        self.docs.iter_mut().zip(s).map(|(d, (st, ch))| (std::mem::replace(&mut d.stash, st), std::mem::replace(&mut d.changed, ch))).collect()
    }

    /// Phase one: encode and verify every changed document and run the writer preflight; writes nothing.
    pub fn preflight_docs(&self, w: &d2io::Writer) -> Result<(), String> {
        let sch = Schema::classic();
        for d in self.docs.iter().filter(|d| d.changed) {
            let bytes = d.stash.to_bytes(sch).map_err(|e| e.to_string())?;
            w.preflight(&d.path, &d.fp, &bytes, &|b| Stash::verify(b, sch).map_err(|e| e.to_string())).map_err(|e| format!("{}: {e}", d.path.display()))?;
        }
        Ok(())
    }

    /// Phase two: commits every changed document. On failure the documents already written are restored.
    /// Returns the (document, report) pairs so the caller can roll back or finish.
    pub fn commit_docs(&mut self, w: &d2io::Writer) -> Result<Vec<(usize, d2io::CommitReport)>, String> {
        let sch = Schema::classic();
        let mut done = vec![];
        let mut fail = None;
        for (i, d) in self.docs.iter().enumerate().filter(|(_, d)| d.changed) {
            let r = d.stash.to_bytes(sch).map_err(|e| e.to_string()).and_then(|bytes| w.commit(&d.path, &d.fp, &bytes, |b| Stash::verify(b, sch).map_err(|e| e.to_string())).map_err(|e| format!("{}: {e}", d.path.display())));
            match r {
                Ok(r) => done.push((i, r)),
                Err(e) => {
                    fail = Some(e);
                    break;
                }
            }
        }
        match fail {
            None => Ok(done),
            Some(e) => Err(match self.rollback(w, &done) {
                Ok(()) => format!("{e}. Nothing was changed: earlier stash writes were restored from backup."),
                Err(r) => format!("{e}. ROLLBACK FAILED ({r}); restore manually from the backup folder."),
            }),
        }
    }

    /// Restores committed documents from the backups their reports name.
    pub fn rollback(&mut self, w: &d2io::Writer, done: &[(usize, d2io::CommitReport)]) -> Result<(), String> {
        let mut err = vec![];
        for (i, r) in done.iter().rev() {
            let d = &mut self.docs[*i];
            if let Err(e) = w.restore(&r.backup, d.path.parent().unwrap_or(Path::new("."))) {
                err.push(format!("{}: {e}", d.path.display()));
            }
            if let Ok(fp) = d2io::fingerprint(&d.path) {
                d.fp = fp;
            }
        }
        if err.is_empty() { Ok(()) } else { Err(err.join("; ")) }
    }

    /// Marks committed documents clean.
    /// The files the committed documents were written to.
    pub fn paths(&self, done: &[(usize, d2io::CommitReport)]) -> Vec<PathBuf> {
        done.iter().map(|(i, _)| self.docs[*i].path.clone()).collect()
    }

    pub fn finish(&mut self, done: &[(usize, d2io::CommitReport)]) {
        for (i, r) in done {
            (self.docs[*i].fp, self.docs[*i].changed) = (r.fingerprint.clone(), false);
        }
    }

    /// Draws the tab; returns true when the .d2s or a stash document was edited.
    pub fn show(&mut self, ui: &mut egui::Ui, save: &mut Save) -> bool {
        let sch = Schema::classic();
        let exp = save.status(d2s::save::EXPANSION);
        let ctx = ui.ctx().clone();
        let mut edited = false;
        let slots: Vec<Slot> = EQ.iter().map(|s| (if self.swap && s.0 >= 4 && s.0 <= 5 { s.0 + 7 } else { s.0 }, s.1, s.2, s.3, s.4)).collect();
        self.st.cand = None;
        let br = belt_rows(save, sch);
        let mut toggled = false;
        egui::ScrollArea::both().show(ui, |ui| {
            let mut d = Draw { art: &mut self.art, st: &mut self.st, sch, save, docs: &self.docs };
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.strong("Equipment");
                    ui.checkbox(&mut self.swap, "Weapon swap (slots 11/12)");
                    d.area(ui, List::Player, 8, 6, Kind::Equip(slots));
                    ui.strong("Belt");
                    d.area(ui, List::Player, 4, br, Kind::Belt);
                });
                ui.vertical(|ui| {
                    ui.strong("Inventory");
                    d.area(ui, List::Player, 10, 4, Kind::Grid(1));
                    ui.strong("Cube");
                    d.area(ui, List::Player, 3, 4, Kind::Grid(4));
                    if d.save.merc.is_some() {
                        ui.strong("Mercenary");
                        d.area(ui, List::Merc, 6, 5, Kind::Equip(MERC.to_vec()));
                    }
                });
                ui.vertical(|ui| {
                    let (c, r) = grid_size(5, exp).unwrap_or((6, 4));
                    ui.strong("Stash");
                    if ui.checkbox(&mut self.manual, "PlugY BigStash (10x10 stash)").changed() {
                        toggled = true;
                        if let Some(f) = big_file().filter(|_| !cfg!(test)) {
                            let _ = d2host::write(&f, if self.manual { b"1" } else { b"0" });
                        }
                    }
                    d.area(ui, List::Player, c, r, Kind::Grid(5));
                    self.stash_cols = c;
                    if let Some(Some(g)) = &d.save.kf {
                        ui.strong("Golem item");
                        let (resp, p) = ui.allocate_painter(vec2(C, C) * 2.0, Sense::click());
                        paint_item(d.art, &ctx, &p, resp.rect, g, sch, Color32::WHITE);
                        if resp.clicked() {
                            d.st.sel = Some(Ref::Golem);
                        }
                        drop(resp.on_hover_text(tip(g, sch)));
                    }
                });
            });
            d.strip(ui);
            ui.separator();
            let mut docs_ui = vec![];
            for (di, doc) in self.docs.iter().enumerate() {
                let p = doc.page.min(doc.stash.pages.len().saturating_sub(1));
                docs_ui.push((di, p));
            }
            for (di, p) in docs_ui {
                let doc = &mut self.docs[di];
                ui.horizontal(|ui| {
                    ui.strong(format!("{}{}", doc.label, if doc.changed { " *" } else { "" }));
                    for (i, pg) in doc.stash.pages.iter().enumerate() {
                        let n = String::from_utf8_lossy(&pg.name);
                        if ui.selectable_label(i == p, if n.is_empty() { format!("{}", i + 1) } else { n.to_string() }).clicked() {
                            doc.page = i;
                        }
                    }
                    if ui.button("+ page").clicked() {
                        doc.stash.add_page(b"");
                        (doc.changed, edited) = (true, true);
                    }
                    if ui.button("- page").clicked() {
                        match doc.stash.remove_page(p, false) {
                            Ok(_) => (doc.changed, edited) = (true, true),
                            Err(e) => self.msg = e.to_string(),
                        }
                    }
                });
                if p < doc.stash.pages.len() {
                    let mut d = Draw { art: &mut self.art, st: &mut self.st, sch, save, docs: &self.docs };
                    let (c, r) = grid_size(5, exp).unwrap_or((10, 10));
                    d.area(ui, List::Stash(di, p), c, r, Kind::Grid(0));
                }
            }
        });
        if toggled {
            self.resolve(save);
        }
        // Drag release.
        if let Some((from, ..)) = self.st.drag
            && ctx.input(|i| i.pointer.any_released())
        {
            if let Some(c) = self.st.cand.take() {
                edited |= apply(save, &mut self.docs, from, c);
                self.st.sel = None;
            }
            self.st.drag = None;
        } else if let (Some((r, ..)), Some(ptr)) = (self.st.drag, ctx.pointer_latest_pos()) {
            if let Some(it) = items(save, &self.docs, match r { Ref::L(l, _) => l, Ref::Golem => List::Player }).get(match r { Ref::L(_, i) => i, Ref::Golem => usize::MAX }) {
                let (w, h) = dims(sch, it);
                let rect = Rect::from_center_size(ptr, vec2(w as f32, h as f32) * C);
                let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("drag")));
                paint_item(&mut self.art, &ctx, &p, rect, it, sch, Color32::WHITE);
            }
            ctx.request_repaint();
        }
        if self.create.open {
            let added = self.create.show(&ctx, save, &mut self.msg);
            edited |= added;
        }
        egui::Panel::right("selected").show(ui, |ui| edited |= self.selected(ui, save, &ctx));
        edited
    }

    fn selected(&mut self, ui: &mut egui::Ui, save: &mut Save, ctx: &egui::Context) -> bool {
        let sch = Schema::classic();
        let mut edited = false;
        let mut act = 0;
        let sel = self.st.sel;
        let mut changed = false;
        if let Some(r) = sel {
            match get_mut(save, &mut self.docs, r) {
                None => self.st.sel = None,
                Some(it) => {
                    ui.strong(edit::item_name(it, sch));
                    let v = &it.view;
                    ui.label(format!("quality {}  ilvl {}  flags {:#x}  sockets {}", v.quality, v.ilvl, v.flags, v.sockets));
                    if !it.editable {
                        ui.colored_label(Color32::YELLOW, "read-only (failed self-check)");
                    }
                    changed |= self.ed.show(ui, it, &mut self.msg);
                    ui.horizontal_wrapped(|ui| {
                        for (i, n) in ["Delete", "Duplicate", "Export"].iter().enumerate() {
                            if ui.button(*n).clicked() {
                                act = i + 1;
                            }
                        }
                    });
                }
            }
        } else {
            ui.label("Click an item to select it");
        }
        if changed {
            if let Some(List::Stash(d, _)) = r_list(sel) {
                self.docs[d].changed = true;
            }
            edited = true;
        }
        if let (Some(r), true) = (sel, act > 0) {
            let l = r_list(sel).unwrap_or(List::Player);
            let copy = get_mut(save, &mut self.docs, r).map(|i| i.clone());
            match (act, copy) {
                (1, _) => {
                    match r {
                        Ref::L(List::Player, i) => drop(edit::remove_item(save, i)),
                        Ref::L(List::Merc, i) => drop(save.merc.as_mut().map(|m| m.remove(i))),
                        Ref::L(List::Stash(d, p), i) => {
                            self.docs[d].stash.pages[p].items.remove(i);
                            self.docs[d].changed = true;
                        }
                        Ref::Golem => save.kf = Some(None),
                    }
                    edited = true;
                    self.st.sel = None;
                }
                (2, Some(it)) => match edit::duplicate(&it, sch).map_err(|e| e.to_string()).and_then(|c| place(save, &mut self.docs, sch, if matches!(l, List::Stash(..)) { l } else { List::Player }, c)) {
                    Ok(()) => {
                        edited = true;
                        if sch.base(it.view.code).is_some_and(|b| b.quest_diff || b.typ == "ques") {
                            self.msg = "Warning: a quest item was duplicated; the game may not expect two of it.".into();
                        }
                    }
                    Err(e) => self.msg = e,
                },
                (3, Some(it)) => match edit::export_item(&it, sch) {
                    Ok(s) => {
                        ctx.copy_text(s);
                        self.msg = "Item copied as base64".into();
                    }
                    Err(e) => self.msg = e.to_string(),
                },
                _ => {}
            }
        }
        ui.separator();
        ui.label("Import (base64)");
        ui.text_edit_singleline(&mut self.import);
        if ui.button("Import").clicked() {
            match crate::setup_ui::generation().and_then(|()| edit::import_item(&self.import, sch).map_err(|e| e.to_string())).and_then(|it| place(save, &mut self.docs, sch, List::Player, it)) {
                Ok(()) => edited = true,
                Err(e) => self.msg = e,
            }
        }
        ui.separator();
        let mut bases: Vec<_> = sch.bases.iter().filter(|(_, b)| !b.gold && !b.quest_diff).map(|(c, b)| (b.name.clone(), String::from_utf8_lossy(c).trim().to_string())).collect();
        bases.sort();
        let cur = bases.iter().find(|b| b.1 == self.new_code).map_or(self.new_code.clone(), |b| b.0.clone());
        egui::ComboBox::from_label("New item").selected_text(cur).show_ui(ui, |ui| {
            for (n, c) in &bases {
                ui.selectable_value(&mut self.new_code, c.clone(), format!("{n} ({c})"));
            }
        });
        let gen_on = crate::setup_ui::gen_off().is_none();
        if ui.add_enabled(gen_on, egui::Button::new("Create item...")).clicked() {
            self.create.open = true;
        }
        if ui.add_enabled(gen_on, egui::Button::new("Create")).clicked() {
            match edit::new_item(sch, &self.new_code, 85, save.status(d2s::save::EXPANSION)).map_err(|e| e.to_string()).and_then(|it| place(save, &mut self.docs, sch, List::Player, it)) {
                Ok(()) => edited = true,
                Err(e) => self.msg = e,
            }
        }
        if !self.msg.is_empty() {
            ui.label(&self.msg);
        }
        edited
    }
}

fn r_list(r: Option<Ref>) -> Option<List> {
    match r? {
        Ref::L(l, _) => Some(l),
        Ref::Golem => None,
    }
}
