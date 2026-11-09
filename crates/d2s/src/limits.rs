//! Game value ranges: the single source of truth for every edit (setters, validate, GUI, CLI, d2mem).
//! Ranges are inclusive and never clamp: callers refuse with `check`. Spec = recomp docs/specs.
use crate::game::exp_for_level;
use crate::item::{ItemView, Prop};
use crate::schema::{num, rows};
use crate::Schema;
use std::collections::HashMap;
use std::ops::RangeInclusive as R;
use std::sync::OnceLock;

include!("../data/counts.rs");

/// `Ok` when `v` is in `r`, else "<what> <v> outside <lo>..=<hi>".
pub fn check(what: impl std::fmt::Display, v: i64, r: R<i64>) -> Result<(), String> {
    if r.contains(&v) { Ok(()) } else { Err(format!("{what} {v} outside {}..={}", r.start(), r.end())) }
}

/// Range of character stat `id` as the editor shows it (life, mana and stamina without their 8 fraction bits).
pub fn char_stat(sch: &Schema, id: u32, level: i64) -> Option<R<i64>> {
    let s = sch.stat(id).filter(|s| s.csv_bits > 0)?;
    let b = s.csv_bits;
    Some(match id {
        6..=11 => 0..=(1 << (b - 8)) - 1, // save/d2s.md F1: CSvBits 21 minus the 8 fraction bits
        12 => 1..=99,
        13 => 0..=i64::from(exp_for_level(99)), // experience table, level 99 row
        14 => 0..=level.clamp(1, 99) * 10_000, // save/d2s.md D5: gold above the level maximum resets to 0
        15 => 0..=2_500_000,                   // save/d2s.md D5: bank above its maximum resets to 0
        _ if s.csv_signed => -(1 << (b - 1))..=(1 << (b - 1)) - 1,
        _ => 0..=(1 << b) - 1,
    })
}

/// Base level of a class skill. The byte is 8 bits wide and save/d2s.md G gives no tighter fact, so the game's 0..=99.
pub fn skill_level() -> R<i64> {
    0..=99
}

/// Shown value range of item property `id`: Save Bits and Save Add, and `shown << ValShift` must fit an i32.
pub fn item_prop(sch: &Schema, id: u32) -> Option<R<i64>> {
    let s = sch.stat(id).filter(|s| s.save_bits > 0)?;
    let add = i64::from(s.save_add);
    let sh = s.val_shift.min(31);
    Some((-add).max(i64::from(i32::MIN >> sh))..=((1 << s.save_bits) - 1 - add).min(i64::from(i32::MAX >> sh)))
}

/// Components of a packed property (Encode 2: skill on event, 3: charges); the 16 param bits hold skill 10 and level 6 bits.
pub fn packed(enc: u32) -> Vec<(&'static str, R<i64>)> {
    match enc {
        2 => vec![("skill level", 0..=63), ("chance", 0..=127)],
        3 => vec![("skill level", 0..=63), ("charges", 0..=255), ("max charges", 0..=255)],
        _ => vec![],
    }
}

/// `MaxSock1`, `MaxSock25`, `MaxSock40` of an item type.
fn max_sock(typ: &str, ilvl: u32) -> u32 {
    static T: OnceLock<HashMap<&'static str, [u32; 3]>> = OnceLock::new();
    let t = T.get_or_init(|| rows(include_str!("../data/itemtypes.tsv")).iter().map(|r| (r.get("Code").copied().unwrap_or(""), [num(r, "MaxSock1"), num(r, "MaxSock25"), num(r, "MaxSock40")])).collect());
    t.get(typ).map_or(6, |m| m[usize::from(ilvl > 25) + usize::from(ilvl > 40)])
}

/// Socket count the loader accepts: `min(6, invw*invh, ItemMaxSockets)`; more desyncs the socketed children.
/// sockets-runewords.md F1/F2 and properties.md F5: ItemMaxSockets = min(gemsockets, MaxSock1 / MaxSock25 / MaxSock40 by ilvl).
pub fn sockets(sch: &Schema, code: [u8; 4], ilvl: u32) -> R<u32> {
    let Some(b) = sch.base(code) else { return 0..=6 };
    0..=6.min(b.w * b.h).min(b.gemsockets.min(max_sock(&b.typ, ilvl)))
}

/// Every range violation of one item's fields and properties, as "<what> <value> outside <lo>..=<hi>".
pub fn item_errors(sch: &Schema, v: &ItemView) -> Vec<String> {
    let mut o = vec![];
    let ext = !v.flag(21) && !v.flag(16); // simple items and ears carry none of these fields
    let Some(b) = sch.base(v.code).filter(|_| ext) else { return props_errors(sch, v) };
    let add = |id| sch.stat(id).map_or(0, |s| i64::from(s.save_add));
    let sum = |id: u32| v.props.iter().chain(v.set_props.iter().flatten()).chain(&v.rw_props).filter(|p| p.id == id).filter_map(|p| p.vals.first()).map(|&x| i64::from(x) - add(id)).sum::<i64>();
    let mut c = |w: &str, x: i64, r: R<i64>| o.extend(check(w, x, r).err());
    c("ilvl", v.ilvl.into(), 1..=99); // save/d2s.md I3: the writer clamps ilvl to 1..99
    if b.stackable && (v.sv == 0 || v.sv > 80) {
        // inventory.md G2: min(511, maxstack + stat 254). A tome is never removed when used up (items/use.md D1, D4:
        // stat 70 goes to 0 and the book stays; economy.md B4 prices it at max(1, stat 70)), so 0 is a tome's minimum.
        c("quantity", v.quantity.into(), i64::from(b.typ != "book")..=(i64::from(b.maxstack) + sum(254)).clamp(1, 511));
    }
    if v.flag(11) {
        c("sockets", v.sockets.into(), 0..=i64::from(*sockets(sch, v.code, v.ilvl).end()));
    }
    if b.kind == 'a' {
        c("defense", i64::from(v.defense) - add(31), item_prop(sch, 31).unwrap_or(0..=0));
    }
    if b.kind != 'm' {
        // properties.md F2 and the ItemMaxDurability note: an indestructible item (stat 152) saves 0; a durability
        // modifier raises the maximum by stat 73 and base * stat 75 / 100.
        c("max durability", v.maxdur.into(), i64::from(b.durability > 0 && !b.nodurability && sum(152) == 0)..=255);
        c("durability", v.dur.into(), 0..=i64::from(v.maxdur) * (100 + sum(75)) / 100 + sum(73));
    }
    o.extend(props_errors(sch, v));
    o
}

fn props_errors(sch: &Schema, v: &ItemView) -> Vec<String> {
    let mut o = vec![];
    for p in v.props.iter().chain(v.set_props.iter().flatten()).chain(&v.rw_props) {
        o.extend(prop_errors(sch, p));
    }
    o
}

/// Range violations of one property: each value, the param and the table row it indexes, the components of packed
/// stats, and stat 194 (the socket field) stored as a property.
pub fn prop_errors(sch: &Schema, p: &Prop) -> Vec<String> {
    let mut o = vec![];
    let name = |id| sch.stat(id).map_or_else(|| format!("stat{id}"), |s| s.name.clone());
    let shown = |k: usize| p.vals.get(k).map_or(0, |&r| i64::from(r) - sch.stat(p.id + k as u32).map_or(0, |s| i64::from(s.save_add)));
    for k in 0..p.vals.len() {
        if let Some(r) = item_prop(sch, p.id + k as u32) {
            o.extend(check(name(p.id + k as u32), shown(k), r).err());
        }
    }
    let Some(s) = sch.stat(p.id) else { return o };
    if s.param_bits > 0 {
        o.extend(check(format!("{} param", name(p.id)), p.param.into(), 0..=(1 << s.param_bits) - 1).err());
    }
    if p.id == 194 {
        o.push("item_numsockets is the socket field, never a property (save/d2s.md I3)".into());
    }
    // A param that indexes a game table: a Skills row, a class (and a skill tab 0..=2), a States, MonStats or MonType row.
    let (par, sk) = (i64::from(p.param), i64::from(crate::props::skill_rows()));
    let ids = match p.id {
        97 | 107 | 151 => vec![("skill", par, sk)],
        195..=201 | 204 => vec![("skill", par >> 6, sk)],
        98 => vec![("state", par, STATES.into())],
        155 => vec![("monster", par, MONSTATS.into())],
        179 | 180 => vec![("monster type", par, MONTYPE.into())],
        83 => vec![("class", par, 7)],
        188 => vec![("class", par >> 3, 7), ("skill tab", par & 7, 3)],
        _ => vec![],
    };
    for (w, x, n) in ids {
        o.extend(check(format!("{} {w}", name(p.id)), x, 0..=n - 1).err());
    }
    let vals = match s.encode {
        2 => vec![i64::from(p.param & 63), shown(0)],
        3 => vec![i64::from(p.param & 63), shown(0) & 255, shown(0) >> 8],
        _ => vec![],
    };
    for ((w, r), x) in packed(s.encode).into_iter().zip(vals) {
        o.extend(check(format!("{} {w}", name(p.id)), x, r).err());
    }
    o
}
