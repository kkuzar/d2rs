//! Legit check: does an item hold only what the 1.14d tables could roll (affixes.md B and C, uniques-sets.md, sockets-runewords.md F1).
//! Findings are notes that never block a save, except `load_errors`: what the game deletes or refuses on load.
use crate::edit::type_chain;
use crate::item::{Item, Prop, NONE};
use crate::itemgen::runeword_field;
use crate::props::{range_of, Ctx};
use crate::tables::{Affix, Block, ItemTables, RareName, Runeword};
use crate::{limits, Schema};
use std::collections::{BTreeMap, BTreeSet};

type Want = BTreeMap<(u32, u32), (i32, i32)>;

/// Summed (stat, param) ranges of table blocks.
fn want(t: &ItemTables, sch: &Schema, blocks: &[&Block], ctx: &Ctx) -> Want {
    let mut m = Want::new();
    for (s, p, lo, hi) in blocks.iter().flat_map(|b| range_of(t, sch, b, ctx).unwrap_or_default()) {
        let e = m.entry((s, p)).or_default();
        *e = (e.0 + lo, e.1 + hi);
    }
    m
}

/// Stored props against table ranges: out of range, extra and missing stats; `skip` stats are left alone.
fn diff(sch: &Schema, props: &[Prop], want: &Want, skip: &BTreeSet<u32>) -> Vec<String> {
    let name = |s| sch.stat(s).map_or_else(|| format!("stat{s}"), |d| d.name.clone());
    let have: BTreeMap<(u32, u32), i32> = props
        .iter()
        .flat_map(|p| p.vals.iter().enumerate().map(move |(k, &r)| ((p.id + k as u32, p.param), r as i32 - sch.stat(p.id + k as u32).map_or(0, |d| d.save_add))))
        .filter(|(k, v)| *v != 0 && !skip.contains(&k.0))
        .collect();
    // skill-rand and randclassskill put the roll in the param, so those stats match on the stat alone
    let rolled: BTreeSet<u32> = want.keys().map(|k| k.0).filter(|s| [107, 83].contains(s)).collect();
    let mut o = vec![];
    for (&(s, p), &x) in have.iter().filter(|(k, _)| !rolled.contains(&k.0)) {
        match want.get(&(s, p)) {
            Some(&(lo, hi)) if s == 204 || (lo..=hi).contains(&x) => {}
            Some(&(lo, hi)) => o.push(format!("{} {x} outside the table range {lo}..={hi}", name(s))),
            None => o.push(format!("{} {x} is not a property the table gives this item", name(s))),
        }
    }
    for (&(s, p), &(lo, hi)) in want.iter().filter(|(k, _)| !rolled.contains(&k.0) && !skip.contains(&k.0)) {
        if (lo > 0 || hi < 0) && !have.contains_key(&(s, p)) {
            o.push(format!("{} is missing ({lo}..={hi})", name(s)));
        }
    }
    o
}

/// A type chain that holds one of `l` and none of `e`.
pub fn types_ok(l: &[String], e: &[String], chain: &[&str]) -> bool {
    let has = |l: &[String]| l.iter().any(|x| chain.contains(&x.as_str()));
    has(l) && !has(e)
}

/// sockets-runewords.md D1 to D4 (RUNEWORD_Match, run by the loader): the first complete Runes row whose runes are the
/// children in order, for a non-quest base of quality outside 4..=9 whose socket count equals its child count and
/// whose type fits the row. None makes the game discard an item with the runeword flag (§G).
pub fn runeword_row<'a>(t: &'a ItemTables, sch: &Schema, item: &Item) -> Option<&'a Runeword> {
    let (v, kids) = (&item.view, item.children.iter().map(|c| c.view.code).collect::<Vec<_>>());
    let b = sch.base(v.code).filter(|b| !(4..=9).contains(&v.quality) && !b.quest && !kids.is_empty() && v.sockets as usize == kids.len())?;
    let chain = type_chain(sch, &b.typ);
    t.runewords.iter().find(|r| r.complete && r.runes == kids && types_ok(&r.itypes, &r.etypes, &chain))
}

/// What the 1.14d loader drops (save/d2s.md I5, sockets-runewords.md G).
fn lost(t: &ItemTables, sch: &Schema, item: &Item) -> Vec<String> {
    let v = &item.view;
    let id = v.qual.first().map_or(0, |&x| x as usize);
    let why = [
        (v.quality == 5 && id >= t.set_items.len(), format!("set id {id} has no SetItems row: the game drops the item")),
        // the game itself writes 0xFFF for a unique with no row and keeps the item
        (v.quality == 7 && id >= t.uniques.len() && id != 0xFFF, format!("unique id {id} is not a UniqueItems row: the game reads it as none")),
        (v.flag(26) && runeword_row(t, sch, item).is_none(), "no complete Runes row matches its runes, sockets, quality and base: the game deletes it".into()),
    ];
    let ext = !v.flag(21) && !v.flag(16) && v.sv != 71;
    why.into_iter().filter(|w| ext && w.0).map(|w| w.1).collect()
}

/// Every reason the game would delete, refuse or overflow this item: `limits::item_errors` and the loader's drops.
pub fn load_errors(t: &ItemTables, sch: &Schema, item: &Item) -> Vec<String> {
    [limits::item_errors(sch, &item.view), lost(t, sch, item)].concat()
}

/// The findings for one item; empty means legit.
pub fn check<'a>(t: &'a ItemTables, sch: &Schema, item: &Item) -> Vec<String> {
    let v = &item.view;
    let Some(b) = sch.base(v.code).filter(|_| !v.flag(21) && !v.flag(16) && v.sv != 71) else { return vec![] };
    let mut o = lost(t, sch, item);
    let (exp, chain) = (v.version >= 100, type_chain(sch, &b.typ));
    let ctx = Ctx { ilvl: v.ilvl, base: v.code };
    let q = |i: usize| v.qual.get(i).copied().filter(|&x| x != NONE);
    if !(1..=99).contains(&v.ilvl) {
        o.push(format!("ilvl {} outside 1..=99", v.ilvl));
    }
    // affixes.md B: the affix level
    let (qlvl, i) = (b.level as i32, v.ilvl.max(b.level) as i32);
    let h = qlvl / 2;
    let alvl = (if b.magic_lvl != 0 { i + b.magic_lvl as i32 } else if i < 99 - h { i - h } else { 2 * i - 99 }).clamp(1, 99);
    // affixes.md C4: the filters of one pick
    let check_affix = |a: Option<&'a Affix>, id: u32, rare: bool| -> (Vec<String>, &'a [Block]) {
        let Some(a) = a else {
            return (vec![format!("affix {id} does not exist")], &[]);
        };
        let why = [
            (!a.spawnable || a.frequency == 0, "cannot spawn"),
            (a.version >= 100 && !exp, "is expansion-only"),
            ((a.level as i32) > alvl || (a.maxlevel != 0 && (a.maxlevel as i32) < alvl), "is outside the level window"),
            (rare && !a.rare, "cannot be on a rare item"),
            (!types_ok(&a.itypes, &a.etypes, &chain), "does not fit this base type"),
            (!crate::itemgen::class_ok(a, &b.typ), "is for another class than this base type"),
        ];
        (why.iter().filter(|w| w.0).map(|w| format!("affix {} (id {id}) {} (alvl {alvl})", a.name, w.1)).collect(), &a.mods)
    };
    let mut blocks: Vec<&'a Block> = vec![];
    let mut skip = BTreeSet::new();
    match v.quality {
        // affixes.md F: a QualityItems row whose class fits the base
        3 => match q(0).and_then(|r| t.quality.get(r as usize).map(|x| (r, x))) {
            None => o.push("superior row does not exist".into()),
            Some((r, x)) => {
                let (class, few) = (crate::itemgen::quality_class(&chain), crate::itemgen::throwable(&b.typ) || b.nodurability);
                if !class.is_some_and(|c| x.classes.iter().any(|y| y == c)) {
                    o.push(format!("superior row {r} does not fit this base"));
                }
                if few && r >= 4 {
                    o.push(format!("superior row {r} is out of reach: a throwable or no-durability base draws rows 0..4"));
                }
                blocks.extend(&x.mods);
            }
        },
        // properties.md and cube.md E8: 9 is the cube's temporary-name quality (tempered), never a drop
        9 => o.push("quality 9 (tempered) is not generated on 1.14d drops".into()),
        4 => {
            for (k, tab) in [&t.magic_prefix, &t.magic_suffix].into_iter().enumerate() {
                if let Some(id) = q(k).filter(|&x| x != 0) {
                    let (m, b) = check_affix(tab.get(id as usize - 1), id, false);
                    o.extend(m);
                blocks.extend(b);
                }
            }
        }
        6 | 8 => {
            let mut name = |l: &[RareName], r: Option<u32>, w: &str| match r.and_then(|x| l.get(x as usize)) {
                Some(n) if (exp || n.version < 100) && types_ok(&n.itypes, &n.etypes, &chain) => {}
                _ => o.push(format!("rare {w} name does not fit this base")),
            };
            name(&t.rare_prefix, q(0).and_then(|x| x.checked_sub(156)), "prefix");
            name(&t.rare_suffix, q(1).and_then(|x| x.checked_sub(1)), "suffix");
            let mut groups = vec![];
            for k in 0..6 {
                let Some(id) = q(2 + k).filter(|&x| x != 0) else { continue };
                let a = if k % 2 == 0 { &t.magic_prefix } else { &t.magic_suffix }.get((id as usize).wrapping_sub(1));
                if a.is_some_and(|a| groups.contains(&a.group)) {
                    o.push(format!("affix {id} shares a group with another affix"));
                }
                groups.extend(a.map(|a| a.group));
                let (m, b) = check_affix(a, id, true);
                o.extend(m);
                blocks.extend(b);
            }
        }
        5 | 7 => {
            let set = v.quality == 5;
            let i = q(0).unwrap_or(u32::MAX) as usize;
            let row = if set { t.set_items.get(i).map(|r| (r.name.as_str(), r.code, r.lvl)) } else { t.uniques.get(i).map(|r| (r.name.as_str(), r.code, r.lvl)) };
            match row {
                None => {}
                Some((n, c, lvl)) => {
                    if c != v.code {
                        o.push(format!("{n} belongs to base {}", String::from_utf8_lossy(&c).trim()));
                    }
                    if lvl > v.ilvl {
                        o.push(format!("{n} needs ilvl {lvl}, the item has {}", v.ilvl));
                    }
                    if set {
                        let r = &t.set_items[i];
                        blocks.extend(&r.props);
                        if r.add_func == 0 {
                            blocks.extend(r.aprops.iter().flatten());
                        } else {
                            let mut lists = v.set_props.iter();
                            for (k, a) in r.aprops.iter().enumerate() {
                                let l = if v.set_lists >> k & 1 == 1 { lists.next().map_or(&[][..], |l| &l[..]) } else { &[] };
                                o.extend(diff(sch, l, &want(t, sch, &a.iter().collect::<Vec<_>>(), &ctx), &skip));
                            }
                        }
                    } else {
                        blocks.extend(&t.uniques[i].props);
                    }
                }
            }
        }
        _ => {}
    }
    // automagic (affixes.md A) adds stats of its group to non-set, non-unique items: leave those stats alone
    // staff mods (affixes.md I) add item_singleskill to bases whose type has a StaffMods class
    let staff = chain.iter().any(|c| crate::itemgen::itype(c).is_some_and(|r| !crate::schema::txt(r, "StaffMods").is_empty()));
    if staff && !matches!(v.quality, 5 | 7) {
        skip.insert(107);
    }
    if b.auto_prefix != 0 && !matches!(v.quality, 5 | 7) {
        let auto: Vec<&Block> = t.automagic.iter().filter(|a| a.group == b.auto_prefix).flat_map(|a| &a.mods).collect();
        skip.extend(want(t, sch, &auto, &ctx).keys().map(|k| k.0));
    }
    if matches!(v.quality, 3..=8) {
        o.extend(diff(sch, &v.props, &want(t, sch, &blocks, &ctx), &skip));
    } else if v.quality == 2 || v.quality == 1 {
        o.extend(diff(sch, &v.props, &Want::new(), &skip));
    }
    // sockets-runewords.md F1: the socket cap
    if v.flag(11) {
        let cap = match v.quality {
            4 => 4,
            6 => 2,
            8 => 3,
            _ => 6,
        };
        let cap = cap.min(*limits::sockets(sch, v.code, v.ilvl).end());
        if v.sockets > cap {
            o.push(format!("{} sockets above the cap {cap}", v.sockets));
        }
    }
    // properties.md F: ethereal needs an expansion item with durability
    if v.flag(22) && (!exp || b.quest || b.kind == 'm' || b.durability == 0 || b.nodurability) {
        o.push("ethereal is not possible on this item".into());
    }
    // sockets-runewords.md G: the row the loader matches (`lost` reports none), the saved name field and the properties
    if let Some(r) = runeword_row(t, sch, item).filter(|_| v.flag(26)) {
        if runeword_field(t, &r.key) != Some(v.runeword) {
            o.push(format!("runeword field {} does not name {}", v.runeword, r.name));
        }
        if r.server {
            o.push(format!("runeword {} is ladder-only: cannot be made in single player", r.name));
        }
        o.extend(diff(sch, &v.rw_props, &want(t, sch, &r.props.iter().collect::<Vec<_>>(), &ctx), &BTreeSet::new()));
    }
    o
}
