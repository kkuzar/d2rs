//! Item editing helpers for the UI and CLI: placement, move, delete, create, names, base64.
use crate::bits::{err, Result};
use crate::item::{parse_tree, Item, ItemView, Prop, NONE};
use crate::{Save, Schema};
use std::sync::atomic::{AtomicU64, Ordering};

thread_local!(static BIG_STASH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

/// Turns the PlugY BigStash switch on or off for this thread (the UI and CLI thread; tests stay isolated).
pub fn set_big_stash(on: bool) {
    BIG_STASH.set(on);
}

pub fn big_stash() -> bool {
    BIG_STASH.get()
}

pub use d2data::setup::ini_big_stash;

/// True when the item sits (or extends) beyond the vanilla 6x8 stash, so the stash must be a BigStash.
pub fn beyond_vanilla(sch: &Schema, it: &Item) -> bool {
    let (w, h) = size(sch, it);
    it.view.location == 0 && (it.view.x + w > 6 || it.view.y + h > 8)
}

/// (columns, rows) of the inventory, stash and cube from a game's Inventory.txt; `None` where a row is missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grids {
    pub inventory: Option<(u32, u32)>,
    pub stash: Option<(u32, u32)>,
    pub cube: Option<(u32, u32)>,
}

thread_local!(static GRIDS: std::cell::Cell<Grids> = const { std::cell::Cell::new(Grids { inventory: None, stash: None, cube: None }) });

/// Sets the grids `grid_size` uses for this thread (a mod's Inventory.txt); `Grids::default()` restores the vanilla sizes.
pub fn set_grids(g: Grids) {
    GRIDS.set(g);
}

/// Grid sizes from an Inventory.txt text: the class row (`<Class>2` for expansion), `Big Bank Page 1` (expansion) or
/// `Bank Page 1`, and `Transmogrify Box Page 1`, by their `gridX` and `gridY`.
pub fn grid_sizes(inventory_txt: &str, class: u8, expansion: bool) -> Grids {
    let all = crate::schema::rows(inventory_txt);
    let find = |name: &str| all.iter().find(|r| crate::schema::txt(r, "class") == name).map(|r| (crate::schema::num(r, "gridX"), crate::schema::num(r, "gridY")));
    let c = ["Amazon", "Sorceress", "Necromancer", "Paladin", "Barbarian", "Druid", "Assassin"].get(class as usize);
    Grids {
        inventory: c.and_then(|c| find(&format!("{c}{}", if expansion { "2" } else { "" }))),
        stash: find(if expansion { "Big Bank Page 1" } else { "Bank Page 1" }),
        cube: find("Transmogrify Box Page 1"),
    }
}

/// (columns, rows) of inventory (1), cube (4) and stash (5, and each PlugY page): 10x10 with BigStash on.
/// A mod's grids (`set_grids`) replace the vanilla sizes, except that BigStash still wins for the stash.
pub fn grid_size(store: u32, expansion: bool) -> Option<(u32, u32)> {
    let g = GRIDS.get();
    match store {
        1 => g.inventory.or(Some((10, 4))),
        4 => g.cube.or(Some((3, 4))),
        5 if big_stash() => Some((10, 10)),
        5 => g.stash.or(Some((6, if expansion { 8 } else { 4 }))),
        _ => None,
    }
}

fn size(sch: &Schema, it: &Item) -> (u32, u32) {
    sch.base(it.view.code).map_or((1, 1), |b| (b.w.max(1), b.h.max(1)))
}

/// True when a `w` x `h` item at (x, y) is inside the `grid` and overlaps no other location-0 item of `store`.
/// `ignore` is an index into `items` to skip (the item being moved).
#[allow(clippy::too_many_arguments)]
pub fn fits(items: &[Item], sch: &Schema, store: u32, grid: (u32, u32), x: u32, y: u32, w: u32, h: u32, ignore: Option<usize>) -> bool {
    if x + w > grid.0 || y + h > grid.1 {
        return false;
    }
    items.iter().enumerate().filter(|(i, it)| Some(*i) != ignore && it.view.location == 0 && it.view.store == store).all(|(_, it)| {
        let (ow, oh) = size(sch, it);
        let v = &it.view;
        x + w <= v.x || v.x + ow <= x || y + h <= v.y || v.y + oh <= y
    })
}

/// First free top-left spot (row by row) for a `w` x `h` item.
pub fn free_spot(items: &[Item], sch: &Schema, store: u32, grid: (u32, u32), w: u32, h: u32) -> Option<(u32, u32)> {
    (0..grid.1).flat_map(|y| (0..grid.0).map(move |x| (x, y))).find(|&(x, y)| fits(items, sch, store, grid, x, y, w, h, None))
}

fn put(raw: &mut [u8], pos: usize, v: u32, n: usize) -> Result<()> {
    if pos + n > raw.len() * 8 {
        return err(pos, "item too short");
    }
    for i in 0..n {
        let (p, bit) = (pos + i, (v >> i & 1) as u8);
        raw[p / 8] = raw[p / 8] & !(1 << (p % 8)) | bit << (p % 8);
    }
    Ok(())
}

impl Item {
    /// Moves the item by patching its 18 location bits in `raw` and in `view`; `dirty` is unchanged, so
    /// even a non-editable item moves losslessly. Children keep their own positions.
    pub fn set_pos(&mut self, location: u32, body: u32, x: u32, y: u32, store: u32) -> Result<()> {
        if self.view.sv != 0 {
            return err(58, "moving items of a legacy version is not supported");
        }
        if location > 7 || body > 15 || x > 15 || y > 15 || store > 7 {
            return err(58, "location field out of range");
        }
        let mut raw = self.raw.clone();
        for (p, v, n) in [(58, location, 3), (61, body, 4), (65, x, 4), (69, y, 4), (73, store, 3)] {
            put(&mut raw, p, v, n)?;
        }
        self.raw = raw;
        let v = &mut self.view;
        (v.location, v.body, v.x, v.y, v.store) = (location, body, x, y, store);
        Ok(())
    }
}

fn remap(save: &mut Save, f: impl Fn(u16) -> u16) {
    if save.ver < 92 {
        return; // the legacy header keeps no item references
    }
    for (o, n) in [(0x38, 16), (0x78, 4)] {
        for i in 0..n {
            let p = o + 4 * i + 2;
            let r = u16::from_le_bytes([save.header[p], save.header[p + 1]]);
            save.header[p..p + 2].copy_from_slice(&f(r).to_le_bytes());
        }
    }
}

/// Removes top-level item `index` (with its children). Header refs to it become 0, higher refs shift down.
pub fn remove_item(save: &mut Save, index: usize) -> Option<Item> {
    if index >= save.items.len() {
        return None;
    }
    let n = index as u32 + 1;
    remap(save, |r| match u32::from(r) {
        x if x == n => 0,
        x if x > n => r - 1,
        _ => r,
    });
    Some(save.items.remove(index))
}

/// Appends a top-level item; existing header refs stay valid.
pub fn push_item(save: &mut Save, item: Item) {
    save.items.push(item);
}

fn rng() -> u32 {
    static S: AtomicU64 = AtomicU64::new(0);
    let mut x = S.load(Ordering::Relaxed);
    if x == 0 {
        x = d2host::now_nanos() | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    S.store(x, Ordering::Relaxed);
    (x >> 16) as u32
}

/// A normal-quality identified item of base `code` (1..4 chars). Simple for compactsave bases, extended otherwise.
/// Like the game's writer it sets flag 23 and item version 101 in an expansion game, 2 in a classic one.
pub fn new_item(sch: &Schema, code: &str, ilvl: u32, expansion: bool) -> Result<Item> {
    let mut c = [b' '; 4];
    if code.is_empty() || code.len() > 4 || !code.is_ascii() {
        return err(0, "bad item code");
    }
    c[..code.len()].copy_from_slice(code.as_bytes());
    let Some(b) = sch.base(c) else { return err(0, format!("unknown item code {code}")) };
    if b.gold || b.quest_diff {
        return err(0, "gold and quest items cannot be created");
    }
    if !expansion && b.version >= 100 {
        return err(0, "expansion-only item in a classic character");
    }
    if !b.compact {
        crate::limits::check("ilvl", ilvl.into(), 1..=99).or_else(|m| err(0, m))?;
    }
    let add = |id| sch.stat(id).map_or(0, |s| s.save_add.max(0) as u32);
    let mut v = ItemView { flags: 1 << 4 | 1 << 23, version: if expansion { 101 } else { 2 }, location: 0, store: 1, code: c, ..Default::default() };
    if b.compact {
        v.flags |= 1 << 21;
    } else {
        (v.seed, v.quality, v.ilvl) = (rng(), 2, ilvl);
        if b.body || b.tome {
            v.qual = vec![0];
        }
        if b.kind == 'a' {
            v.defense = b.maxac + add(31);
        }
        if b.kind != 'm' {
            v.maxdur = b.durability;
            v.dur = b.durability;
        }
        if b.stackable {
            v.quantity = b.maxstack.max(1);
        }
    }
    let raw = Item::encode(&v, sch)?;
    let (it, _) = parse_tree(&raw, 0, sch)?;
    if it.view != v {
        return err(0, format!("new item does not round-trip: {v:?} vs {:?}", it.view));
    }
    Ok(it)
}

fn reseed(it: &mut Item, sch: &Schema) -> Result<()> {
    if !it.view.flag(21) && !it.view.flag(16) {
        it.view.seed = rng();
        it.raw = Item::encode(&it.view, sch)?;
    }
    it.children.iter_mut().try_for_each(|c| reseed(c, sch))
}

/// A copy with a fresh seed (also for its socketed children). Refuses non-editable items.
pub fn duplicate(item: &Item, sch: &Schema) -> Result<Item> {
    if !item.editable || item.children.iter().any(|c| !c.editable) {
        return err(0, "item is not editable");
    }
    let mut d = item.clone();
    reseed(&mut d, sch)?;
    Ok(d)
}

fn pick<'a>(mut l: impl Iterator<Item = &'a String>, i: Option<u32>) -> &'a str {
    i.and_then(|i| l.nth(i as usize)).map_or("", String::as_str)
}

/// Display name: base name, or the unique/set/runeword/magic/rare name (best effort, never fails).
pub fn item_name(item: &Item, sch: &Schema) -> String {
    let v = &item.view;
    let base = sch.base(v.code).map_or_else(|| String::from_utf8_lossy(&v.code).trim().to_string(), |b| b.name.clone());
    let t = crate::tables::ItemTables::get();
    let q = |i: usize| v.qual.get(i).copied().filter(|&x| x != NONE);
    let s = match v.quality {
        _ if v.flag(26) => {
            // The low 12 bits of the stored string index: key RunewordN is N + 26 (N + 25 above 75); 2718 is Runeword22.
            let x = match v.runeword & 0xFFF { 2718 => 48, x => x };
            let key = |k: &str| k.strip_prefix("Runeword").and_then(|d| d.parse::<u32>().ok()).is_some_and(|d| d + if d > 75 { 25 } else { 26 } == x);
            format!("{} {base}", pick(t.runewords.iter().map(|r| &r.name), t.runewords.iter().position(|r| key(&r.key)).map(|j| j as u32)))
        }
        1 => format!("Crude {base}"),
        3 => format!("Superior {base}"),
        4 => [pick(t.magic_prefix.iter().map(|a| &a.name), q(0).and_then(|x| x.checked_sub(1))), &base, pick(t.magic_suffix.iter().map(|a| &a.name), q(1).and_then(|x| x.checked_sub(1)))].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" "),
        5 => format!("{} {base}", pick(t.set_items.iter().map(|a| &a.name), q(0))),
        7 => format!("{} {base}", pick(t.uniques.iter().map(|a| &a.name), q(0))),
        6 | 8 | 9 => {
            let nm = |i| q(i).map_or("", |x| if x >= 156 { pick(t.rare_prefix.iter().map(|a| &a.name), Some(x - 156)) } else { pick(t.rare_suffix.iter().map(|a| &a.name), x.checked_sub(1)) });
            format!("{} {} {base}", nm(0), nm(1))
        }
        _ => base,
    };
    s.trim().to_string()
}

/// Property text with the shown value (stored value minus Save Add) and the stat name.
pub fn prop_text(p: &Prop, sch: &Schema) -> String {
    let name = |id| sch.stat(id).map_or_else(|| format!("stat{id}"), |s| s.name.clone());
    let shown = |k: usize| p.vals.get(k).map_or(0, |&r| i64::from(r) - sch.stat(p.id + k as u32).map_or(0, |s| i64::from(s.save_add)));
    let enc = sch.stat(p.id).map_or(0, |s| s.encode);
    let (skill, lvl) = (p.param >> 6, p.param & 63);
    match enc {
        2 => format!("{} {}% skill {skill} lvl {lvl}", name(p.id), shown(0)),
        3 => format!("{} skill {skill} lvl {lvl} charges {}/{}", name(p.id), shown(0) & 255, shown(0) >> 8),
        _ if p.vals.len() > 1 => format!("{} {}", name(p.id), (0..p.vals.len()).map(|k| format!("{:+}", shown(k))).collect::<Vec<_>>().join(" ")),
        _ if p.param > 0 => format!("{} [{}] {:+}", name(p.id), p.param, shown(0)),
        _ => format!("{} {:+}", name(p.id), shown(0)),
    }
}

/// Sets the shown value of single-value property `idx` (stored = shown + Save Add). Refuses values that
/// do not fit Save Bits, grouped stats and non-editable items; never clamps.
pub fn set_prop_value(item: &mut Item, idx: usize, shown: i64, sch: &Schema) -> Result<()> {
    if !item.editable {
        return err(0, "item is not editable");
    }
    let Some(p) = item.view.props.get(idx) else { return err(0, "no such property") };
    let Some(s) = sch.stat(p.id).filter(|s| s.save_bits > 0 && p.vals.len() == 1) else { return err(0, "property has no single value") };
    crate::limits::check(&s.name, shown, crate::limits::item_prop(sch, p.id).unwrap_or(0..=0)).or_else(|m| err(0, m))?;
    let raw = shown + i64::from(s.save_add);
    item.view_mut().props[idx].vals[0] = raw as u32;
    Ok(())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The item and its socketed children as standard base64.
pub fn export_item(item: &Item, sch: &Schema) -> Result<String> {
    let mut b = vec![];
    item.write(sch, false, &mut b)?;
    Ok(b.chunks(3).flat_map(|c| {
        let n = c.iter().enumerate().fold(0u32, |a, (i, &x)| a | u32::from(x) << (16 - 8 * i));
        (0..4).map(move |i| if i <= c.len() { B64[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' })
    }).collect())
}

/// Parses `export_item` text (whitespace ignored); all bytes must be consumed. Refuses what the game deletes or
/// refuses on load (`legit::load_errors`).
pub fn import_item(s: &str, sch: &Schema) -> Result<Item> {
    let (mut b, mut acc, mut bits) = (vec![], 0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let Some(v) = B64.iter().position(|&x| x == c) else { return err(0, "invalid base64") };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            b.push((acc >> bits) as u8);
        }
    }
    let (it, end) = parse_tree(&b, 0, sch)?;
    if end != b.len() {
        return err(end * 8, "trailing bytes after the item");
    }
    let errs: Vec<String> = std::iter::once(&it).chain(&it.children).flat_map(|i| crate::legit::load_errors(crate::tables::ItemTables::get(), sch, i)).collect();
    if !errs.is_empty() {
        return err(0, errs.join("; "));
    }
    Ok(it)
}

const BODY: [&str; 10] = ["head", "neck", "tors", "rarm", "larm", "rrin", "lrin", "belt", "feet", "glov"];

/// True when an item of base `code` may be worn in body slot `body` (1..10, 11/12 are the swap hands).
/// The item type chain (Equiv1/Equiv2) is searched breadth first for the first type with BodyLoc entries.
/// The item type and every type it is equivalent to (ItemTypes Equiv1/Equiv2 chain), nearest first.
pub fn type_chain<'a>(sch: &'a Schema, typ: &'a str) -> Vec<&'a str> {
    let mut q = vec![typ];
    let mut i = 0;
    while i < q.len() {
        for e in sch.types.get(q[i]).into_iter().flat_map(|t| t.0.iter()).filter(|e| !e.is_empty()) {
            if !q.contains(&e.as_str()) {
                q.push(e);
            }
        }
        i += 1;
    }
    q
}

pub fn slot_ok(sch: &Schema, code: [u8; 4], body: u32) -> bool {
    let Some(b) = sch.base(code) else { return true };
    let want = match body {
        1..=10 => body,
        11 | 12 => body - 7,
        _ => return false,
    };
    let locs = type_chain(sch, &b.typ).into_iter().find_map(|t| sch.types.get(t).map(|x| &x.1).filter(|bl| bl.iter().any(|x| !x.is_empty())));
    locs.is_some_and(|bl| bl.iter().any(|x| x == BODY[want as usize - 1]))
}

/// Hireling act (1, 2, 3 or 5) of a merc type id (header 0xB9), from hireling.tsv.
pub fn merc_act(merc_type: u32) -> Option<u32> {
    crate::data::table("hireling").lines().skip(1).map(|l| l.split('\t').collect::<Vec<_>>()).find(|c| c.get(1) == Some(&merc_type.to_string().as_str())).and_then(|c| c.get(3)?.parse().ok())
}

/// Mercenary equipment rules (Arreat Summit "Hirelings"): helm and armor for all (barbarian helms only
/// in act 5, no druid pelts); weapon: act 1 bows (no crossbows), act 2 spears/polearms/javelins,
/// acts 3 and 5 swords; shield only in act 3. Amazon/paladin/necromancer-only types are refused.
pub fn merc_slot_ok(sch: &Schema, merc_type: u32, code: [u8; 4], body: u32) -> bool {
    let (Some(b), Some(act)) = (sch.base(code), merc_act(merc_type)) else { return slot_ok(sch, code, body) };
    let t = type_chain(sch, &b.typ);
    let is = |x: &str| t.contains(&x);
    if ["abow", "aspe", "ajav", "ashd", "head", "pelt"].iter().any(|c| is(c)) || (is("phlm") && act != 5) {
        return false;
    }
    match body {
        1 => is("helm"),
        3 => is("tors"),
        4 => match act {
            1 => is("bow"),
            2 => is("spea") || is("pole") || is("jave"),
            _ => is("swor"),
        },
        5 => act == 3 && is("shie"),
        _ => false,
    }
}
