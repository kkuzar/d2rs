//! Item builder by choice: any quality of an item with the properties the game could roll (items/properties, affixes, uniques-sets, sockets-runewords).
use crate::bits::{err, Result};
use crate::edit::{new_item, type_chain};
use crate::item::{Item, ItemView, Prop, NONE};
use crate::legit::{runeword_row, types_ok};
use crate::props::{apply, pack, skill_itype, Ctx, Roll};
use crate::rng::Seed;
use crate::schema::{num, rows, txt, Base, Row};
use crate::tables::{Affix, Block, ItemTables, RareName};
use crate::{limits, Schema};
use std::collections::HashMap;
use std::sync::OnceLock;

/// What to build. Affix ids follow the save numbering (row + 1 of the prefix or suffix table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Normal,
    /// LowQualityItems row.
    Low(u32),
    /// QualityItems row.
    Superior(u32),
    /// Prefix id, suffix id (at least one).
    Magic(Option<u32>, Option<u32>),
    /// `names` are the saved ids (156 + RarePrefix row, RareSuffix row + 1); `affixes` are the six slots in file
    /// order prefix, suffix, prefix, suffix, prefix, suffix (0 = empty, at most six entries).
    Rare { names: [u32; 2], affixes: Vec<u32> },
    Crafted { names: [u32; 2], affixes: Vec<u32> },
    /// SetItems row.
    Set(u32),
    /// UniqueItems row.
    Unique(u32),
    /// `RunewordN` key of Runes.txt.
    Runeword(String),
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub code: String,
    pub kind: Kind,
    pub ilvl: u32,
    pub ethereal: bool,
    pub sockets: u32,
    pub expansion: bool,
}

/// The saved runeword field, the inverse of the mapping in `edit::item_name`: string index 0x5000 + N + 26 (N + 25 above 75), 2718 for Runeword22.
pub fn runeword_field(t: &ItemTables, key: &str) -> Option<u32> {
    let d: u32 = key.strip_prefix("Runeword")?.parse().ok().filter(|_| t.runewords.iter().any(|r| r.key == key))?;
    Some(if d == 22 { 2718 } else { 0x5000 + d + if d > 75 { 25 } else { 26 } })
}

/// Stats of the blocks applied so far, the ethereal request of func 23 and the base defense as shown (the `ac%` reset, properties.md D3).
struct Acc {
    stats: Vec<(u32, u32, i32)>,
    eth: bool,
    def: i32,
    maxac: i32,
    /// Sockets given by `sock` properties (stat 194).
    socks: u32,
}

impl Acc {
    fn run(&mut self, t: &ItemTables, sch: &Schema, blocks: &[&Block], ctx: &Ctx, roll: &mut Roll) -> Result<()> {
        // properties.md B: a commented-out code (`*name`) has no Properties row and its block is skipped
        for b in blocks.iter().filter(|b| !b.code.starts_with('*')) {
            let got = apply(t, sch, b, ctx, roll)?;
            self.eth |= t.props.iter().any(|p| p.code.eq_ignore_ascii_case(&b.code) && p.slots.iter().any(|s| s.0 == 23));
            if self.maxac != 0 && got.iter().any(|s| s.0 == 16) {
                self.def = (self.def + 1).max(self.maxac + 1);
            }
            self.socks = got.iter().filter(|s| s.0 == 194).map(|s| s.2 as u32).fold(self.socks, u32::max);
            self.stats.extend(got.into_iter().filter(|s| s.0 != 194));
        }
        Ok(())
    }
}

pub fn affix_ok(a: &Affix, chain: &[&str], exp: bool, rare: bool) -> bool {
    a.spawnable && a.frequency > 0 && (exp || a.version < 100) && (!rare || a.rare) && types_ok(&a.itypes, &a.etypes, chain)
}

/// The QualityItems class column of a base (affixes.md F).
pub(crate) fn quality_class(chain: &[&str]) -> Option<&'static str> {
    const C: [(&str, &str); 10] = [("shie", "shield"), ("boot", "boots"), ("glov", "gloves"), ("belt", "belt"), ("thro", "thrown"), ("scep", "scepter"), ("wand", "wand"), ("staf", "staff"), ("bow", "bow"), ("xbow", "bow")];
    C.iter().find(|c| chain.contains(&c.0)).map(|c| c.1).or_else(|| ["armo", "weap"].iter().find(|c| chain.contains(c)).map(|&c| if c == "armo" { "armor" } else { "weapon" }))
}

/// What a reroll adds to the item `make` assembles: the seed, the unit-seed base stats, staff mods and the automagic row.
#[derive(Default)]
struct Pre {
    seed: Option<u32>,
    extra: Vec<(u32, u32, i32)>,
    auto: Option<usize>,
    /// Current and maximum durability, defense as shown, quantity and picture variant (properties.md A), and the low-quality durability (affixes.md G2).
    dur: Option<(u32, u32)>,
    low: Option<u32>,
    def: Option<i32>,
    qty: Option<u32>,
    gfx: Option<u32>,
}

/// Builds the item `s` describes. Refuses what the game would not make; `roll` draws every ranged value.
pub fn make(t: &ItemTables, sch: &Schema, s: &Spec, roll: &mut Roll) -> Result<Item> {
    build(t, sch, s, roll, &Pre::default())
}

fn build(t: &ItemTables, sch: &Schema, s: &Spec, roll: &mut Roll, pre: &Pre) -> Result<Item> {
    let mut it = new_item(sch, &s.code, s.ilvl, s.expansion)?;
    let (c, mut v) = (it.view.code, it.view.clone());
    v.seed = pre.seed.unwrap_or(v.seed);
    v.gfx = pre.gfx.or(v.gfx);
    v.quantity = pre.qty.unwrap_or(v.quantity);
    let Some(b) = sch.base(c) else { return err(0, "unknown base") };
    let chain = type_chain(sch, &b.typ);
    let exp = s.expansion;
    let add = sch.stat(31).map_or(0, |d| d.save_add);
    let mut acc = Acc { stats: vec![], eth: false, def: pre.def.unwrap_or(v.defense as i32 - add), maxac: if b.kind == 'a' { b.maxac as i32 } else { 0 }, socks: 0 };
    let ctx = Ctx { ilvl: s.ilvl, base: c };
    let dur = b.kind != 'm' && b.durability > 0 && !b.nodurability;
    if let Some((cur, max)) = pre.dur.filter(|_| dur) {
        (v.dur, v.maxdur) = (cur, max);
    }
    let (mut lists, mut kids, mut rw): (Vec<Vec<Prop>>, Vec<Item>, Vec<Prop>) = (vec![], vec![], vec![]);
    if b.compact && s.kind != Kind::Normal {
        return err(0, "this base has no quality");
    }
    let pick = |a: &[Affix], id: u32, rare: bool| -> Result<Affix> {
        match a.get((id as usize).wrapping_sub(1)) {
            Some(x) if affix_ok(x, &chain, exp, rare) => Ok(x.clone()),
            _ => err(0, format!("affix {id} cannot be on this base")),
        }
    };
    let names_ok = |n: &[u32; 2]| {
        let (p, q) = (t.rare_prefix.get((n[0] as usize).wrapping_sub(156)), t.rare_suffix.get((n[1] as usize).wrapping_sub(1)));
        let ok = |l: &[String], e: &[String], ver: u32| (exp || ver < 100) && types_ok(l, e, &chain);
        p.zip(q).is_some_and(|(p, q)| ok(&p.itypes, &p.etypes, p.version) && ok(&q.itypes, &q.etypes, q.version))
    };
    match &s.kind {
        Kind::Normal => {}
        Kind::Low(i) => {
            if *i as usize >= t.low_quality.len() || !matches!(b.kind, 'a' | 'w') {
                return err(0, "low quality needs a weapon or armour and a LowQualityItems row");
            }
            (v.quality, v.qual) = (1, vec![*i]);
        }
        Kind::Superior(i) => {
            let Some(q) = t.quality.get(*i as usize).filter(|q| quality_class(&chain).is_some_and(|c| q.classes.iter().any(|x| x == c))) else { return err(0, "no such superior row for this base") };
            (v.quality, v.qual) = (3, vec![*i]);
            acc.run(t, sch, &q.mods.iter().collect::<Vec<_>>(), &ctx, roll)?;
        }
        Kind::Magic(p, q) => {
            if p.is_none() && q.is_none() {
                return err(0, "a magic item needs a prefix or a suffix");
            }
            let ids = vec![p.unwrap_or(0), q.unwrap_or(0)];
            let (p, q) = (p.map(|i| pick(&t.magic_prefix, i, false)).transpose()?, q.map(|i| pick(&t.magic_suffix, i, false)).transpose()?);
            if p.as_ref().zip(q.as_ref()).is_some_and(|(p, q)| p.group == q.group) {
                return err(0, "prefix and suffix share an affix group");
            }
            (v.quality, v.qual) = (4, ids);
            acc.run(t, sch, &p.iter().chain(&q).flat_map(|a| &a.mods).collect::<Vec<_>>(), &ctx, roll)?;
        }
        Kind::Rare { names, affixes } | Kind::Crafted { names, affixes } => {
            let n = affixes.iter().filter(|&&a| a != 0).count();
            if affixes.len() > 6 || n == 0 || !names_ok(names) {
                return err(0, "rare items need valid names for this base and one to six affix slots");
            }
            let mut slots = vec![NONE; 6];
            let (mut rows, mut groups) = (vec![], vec![]);
            for (k, &id) in affixes.iter().enumerate().filter(|x| *x.1 != 0) {
                let a = pick(if k % 2 == 0 { &t.magic_prefix } else { &t.magic_suffix }, id, true)?;
                if groups.contains(&a.group) {
                    return err(0, "two affixes share a group");
                }
                (slots[k], groups) = (id, [groups, vec![a.group]].concat());
                rows.push(a);
            }
            (v.quality, v.qual) = (if matches!(s.kind, Kind::Rare { .. }) { 6 } else { 8 }, [names.to_vec(), slots].concat());
            acc.run(t, sch, &rows.iter().flat_map(|a| &a.mods).collect::<Vec<_>>(), &ctx, roll)?;
        }
        Kind::Unique(i) | Kind::Set(i) => {
            let set = matches!(s.kind, Kind::Set(_));
            let i = *i as usize;
            let (code, expansion_row, props, aprops, add_func) = if set {
                let Some(r) = t.set_items.get(i) else { return err(0, "no such set item") };
                (r.code, i >= 62, r.props.iter().collect::<Vec<_>>(), r.aprops.iter().map(|a| a.iter().collect::<Vec<_>>()).collect::<Vec<_>>(), r.add_func)
            } else {
                let Some(r) = t.uniques.get(i) else { return err(0, "no such unique") };
                (r.code, r.version >= 100, r.props.iter().collect(), vec![], 0)
            };
            if code != c {
                return err(0, "that unique or set item belongs to a different base");
            }
            if expansion_row && !exp {
                return err(0, "expansion item in a classic character");
            }
            (v.quality, v.qual) = (if set { 5 } else { 7 }, vec![i as u32]);
            acc.run(t, sch, &props, &ctx, roll)?;
            for (k, a) in aprops.iter().enumerate() {
                if add_func == 0 {
                    acc.run(t, sch, a, &ctx, roll)?;
                } else {
                    let (stats, eth) = (std::mem::take(&mut acc.stats), acc.eth);
                    acc.run(t, sch, a, &ctx, roll)?;
                    let list = pack(sch, &std::mem::replace(&mut acc.stats, stats))?;
                    acc.eth |= eth;
                    if !list.is_empty() {
                        v.set_lists |= 1 << k;
                        lists.push(list);
                    }
                }
            }
        }
        Kind::Runeword(key) => {
            let Some(r) = t.runewords.iter().find(|r| r.key == *key) else { return err(0, format!("no runeword {key}")) };
            if !r.complete || !types_ok(&r.itypes, &r.etypes, &chain) {
                return err(0, format!("{} cannot be made in this base", r.name));
            }
            if s.sockets as usize != r.runes.len() {
                return err(0, format!("{} needs exactly {} sockets", r.name, r.runes.len()));
            }
            let Some(field) = runeword_field(t, key) else { return err(0, "bad runeword key") };
            for (k, code) in r.runes.iter().enumerate() {
                let mut c = new_item(sch, String::from_utf8_lossy(code).trim(), s.ilvl, exp)?;
                c.set_pos(6, 0, k as u32, 0, 0)?;
                kids.push(c);
            }
            let mut rwa = Acc { stats: vec![], eth: false, def: 0, maxac: 0, socks: 0 };
            rwa.run(t, sch, &r.props.iter().collect::<Vec<_>>(), &ctx, roll)?;
            rw = pack(sch, &rwa.stats)?;
            (v.flags, v.runeword, v.n_filled) = (v.flags | 1 << 26, field, kids.len() as u32);
        }
    }
    // save/d2s.md I3, I4: the automagic row r is saved as class_affix r + 1
    if let Some((r, a)) = pre.auto.and_then(|r| Some((r, t.automagic.get(r)?))) {
        acc.run(t, sch, &a.mods.iter().collect::<Vec<_>>(), &ctx, roll)?;
        v.class_affix = Some(r as u32 + 1);
    }
    acc.stats.extend(&pre.extra);
    let s = &Spec { sockets: s.sockets.max(acc.socks), ..s.clone() };
    if s.sockets > *limits::sockets(sch, c, s.ilvl).end() {
        return err(0, format!("sockets {} outside 0..={}", s.sockets, limits::sockets(sch, c, s.ilvl).end()));
    }
    if s.sockets > 0 {
        (v.flags, v.sockets) = (v.flags | 1 << 11, s.sockets);
    }
    if s.ethereal && (!exp || b.quest || !dur) {
        return err(0, "ethereal needs an expansion character and a base with durability that is not a quest item");
    }
    // Quality effects on the base stats, then ethereal (properties.md F3), both on the shown defense.
    // A `dur%` stat raises the current durability to the evaluated maximum (properties.md E13, G).
    let evd = |m: u32| (m as i32 + (m as i32 * acc.stats.iter().filter(|x| x.0 == 75).map(|x| x.2).sum::<i32>() / 100)) as u32;
    if v.quality == 1 {
        acc.def = (acc.def * 75 / 100).max(1);
        if dur {
            v.maxdur = (v.maxdur * 33 / 100).max(1);
            v.dur = pre.low.unwrap_or(v.maxdur);
        }
    }
    if (s.ethereal || acc.eth) && exp && dur {
        v.flags |= 1 << 22;
        (acc.def, v.maxdur) = (acc.def * 3 / 2, v.maxdur / 2 + 1);
        v.dur = evd(v.maxdur);
    } else if dur && acc.stats.iter().any(|x| x.0 == 75) {
        v.dur = evd(v.maxdur);
    }
    if b.kind == 'a' {
        v.defense = (acc.def + add) as u32;
    }
    (v.props, v.set_props, v.rw_props) = (pack(sch, &acc.stats)?, lists, rw);
    (it.view, it.children) = (v, kids);
    finish(t, sch, &mut it)?;
    Ok(it)
}

const CLS: [&str; 7] = ["ama", "sor", "nec", "pal", "bar", "dru", "ass"];

/// affixes.md C4h: a class-specific row is refused only on an item whose type has another class.
pub fn class_ok(a: &Affix, typ: &str) -> bool {
    let cls = itype(typ).and_then(|r| CLS.iter().position(|&c| c == txt(r, "Class")));
    a.class_specific.is_none_or(|c| cls.is_none_or(|k| k == c as usize))
}

/// The ItemTypes Throwable flag of a base type.
pub(crate) fn throwable(typ: &str) -> bool {
    itype(typ).is_some_and(|r| txt(r, "Throwable") == "1")
}

/// An ItemTypes.txt row by code.
pub(crate) fn itype(code: &str) -> Option<&'static Row<'static>> {
    static T: OnceLock<HashMap<&'static str, Row<'static>>> = OnceLock::new();
    T.get_or_init(|| rows(include_str!("../data/itemtypes.tsv")).into_iter().map(|r| (txt(&r, "Code"), r)).collect()).get(code)
}

/// A column of the items table (Weapons, Armor, Misc) for one base.
fn item_col(code: [u8; 4], col: &str) -> i32 {
    static T: OnceLock<HashMap<String, Row<'static>>> = OnceLock::new();
    let t = T.get_or_init(|| rows(include_str!("../data/items.tsv")).into_iter().map(|r| (txt(&r, "code").to_string(), r)).collect());
    t.get(String::from_utf8_lossy(&code).trim()).map_or(0, |r| num(r, col))
}

/// The builders' state: the item seed with the property draws it made (replayed by `build`), the properties applied so far and the base stats of the unit seed.
struct Rl<'a> {
    t: &'a ItemTables,
    sch: &'a Schema,
    b: &'a Base,
    chain: Vec<&'a str>,
    ctx: Ctx,
    exp: bool,
    seed: u32,
    s: Seed,
    us: Seed,
    q: u32,
    alvl: i32,
    keep: Option<&'a Kind>,
    tape: Vec<i32>,
    acc: Acc,
    taken: Vec<u32>,
    pre: Pre,
}

impl<'a> Rl<'a> {
    fn flag(&self, k: &str) -> bool {
        itype(&self.b.typ).is_some_and(|r| txt(r, k) == "1")
    }

    /// The affixes.md C5a refusal: a classic stackable or throwable base has no affix and no rare name.
    fn classic_no(&self) -> bool {
        !self.exp && (self.b.stackable || self.flag("Throwable"))
    }

    /// The seed back to `{S, 666}` and nothing applied (uniques-sets.md E2).
    fn reset(&mut self) {
        (self.s, self.tape, self.taken, self.pre.extra, self.pre.auto) = (Seed::init(self.seed), vec![], vec![], vec![], None);
        self.acc = Acc { stats: vec![], eth: false, def: 0, maxac: 0, socks: 0 };
    }

    /// ItemApplyProps on the item seed, recording every draw (properties.md B to E).
    fn props(&mut self, blocks: &[&Block]) -> Result<()> {
        self.acc.run(self.t, self.sch, blocks, &self.ctx, &mut Roll::Rec(&mut self.s, &mut self.tape))
    }

    /// affixes.md C: one pick over `tab` (two draws on a hit); the row index. `auto` is the automagic group, `spawn` the needSpawnable flag.
    fn pick(&mut self, tab: &'a [Affix], must: bool, spawn: bool, auto: u32) -> Option<usize> {
        if self.s.step() & 1 == 0 && !must {
            return None;
        }
        let (mut cand, mut sum) = (vec![], 0);
        for (row, r) in tab.iter().enumerate() {
            let ok = (!spawn || r.spawnable) && (self.exp || r.version < 100) && r.level as i32 <= self.alvl && (r.maxlevel == 0 || r.maxlevel as i32 >= self.alvl);
            let ok = ok && (r.rare || !matches!(self.q, 6 | 8 | 9)) && !self.classic_no() && types_ok(&r.itypes, &r.etypes, &self.chain);
            let ok = ok && (auto == 0 || r.group == auto) && r.frequency > 0 && class_ok(r, &self.b.typ);
            if ok && !self.taken.contains(&r.group) {
                let w = (r.frequency * if self.b.magic_lvl != 0 { r.level } else { 1 }) as i32;
                if cand.len() < 511 {
                    cand.push((row, w));
                }
                sum += w;
            }
        }
        if cand.is_empty() {
            return None;
        }
        let mut x = self.s.range(sum + 1) as i32;
        let k = cand.iter().position(|c| {
            x -= c.1;
            x < 0
        });
        let row = cand[k.unwrap_or(cand.len() - 1)].0;
        self.taken.push(tab[row].group);
        Some(row)
    }

    /// affixes.md E2: PickRareName, one uniform draw over the rows that fit the base.
    fn name(&mut self, tab: &[RareName]) -> Option<u32> {
        let cand: Vec<usize> = (0..tab.len()).filter(|&i| !self.classic_no() && (self.exp || tab[i].version < 100) && types_ok(&tab[i].itypes, &tab[i].etypes, &self.chain)).collect();
        (!cand.is_empty()).then(|| cand[self.s.range(cand.len() as i32) as usize] as u32)
    }

    /// The properties of the affix with save id `id` (`side` 0 prefix, 1 suffix).
    fn mods(&mut self, side: usize, id: u32) -> Result<()> {
        let t = self.t;
        let Some(a) = [&t.magic_prefix, &t.magic_suffix][side].get((id as usize).wrapping_sub(1)) else { return err(0, format!("affix {id} does not exist")) };
        self.props(&a.mods.iter().collect::<Vec<_>>())
    }

    /// affixes.md I: the class skills of a staff, orb, wand, scepter or class helm.
    fn staff(&mut self) {
        let Some(c) = itype(&self.b.typ).and_then(|r| CLS.iter().position(|&c| c == txt(r, "StaffMods"))) else { return };
        let ilvl = self.ctx.ilvl;
        let tier = if ilvl >= 37 && self.exp { 5 } else if ilvl >= 25 { 4 } else if ilvl >= 19 { 3 } else if ilvl >= 12 { 2 } else { 1 };
        let t = self.s.step() % 100;
        let mut have: Vec<i32> = vec![];
        for _ in 0..(t >= 91) as u32 + (t >= 71) as u32 + (t >= 31) as u32 {
            let u = self.s.step() % 100;
            let mut tr = (if u >= 81 { tier + 1 } else if u >= 31 { tier } else if u >= 11 { tier - 1 } else { tier - 2 }).max(1);
            if self.q == 1 {
                tr = tr.min(4);
            }
            let mut skill = 0;
            for _ in 0..6 {
                skill = (self.s.step() % 5) as i32 + (tr - 1) * 5 + [6, 36, 66, 96, 126, 221, 251][c];
                if skill_itype(skill).is_none_or(|ty| ty.is_empty() || self.chain.contains(&ty)) && !have.contains(&skill) {
                    have.push(skill);
                    break;
                }
            }
            let v = if self.q == 1 && self.exp { 0 } else { self.s.step() % 100 };
            self.pre.extra.push((107, skill as u32, if v >= 90 { 3 } else if v >= 60 { 2 } else { 1 }));
        }
    }

    /// affixes.md D: prefix pick and properties, then suffix pick and properties. A kept magic kind replaces each pick after its draws.
    fn magic(&mut self) -> Result<Option<Kind>> {
        let (t, kept) = (self.t, if let Some(Kind::Magic(p, s)) = self.keep { Some([*p, *s]) } else { None });
        let p = self.pick(&t.magic_prefix, false, true, 0);
        let kp = kept.map_or(p.map(|r| r as u32 + 1), |k| k[0]);
        if let Some(id) = kp {
            self.mods(0, id)?;
        }
        let q = self.pick(&t.magic_suffix, p.is_none(), true, 0);
        let kq = kept.map_or(q.map(|r| r as u32 + 1), |k| k[1]);
        if let Some(id) = kq {
            self.mods(1, id)?;
        }
        self.staff();
        Ok((kp.is_some() || kq.is_some()).then_some(Kind::Magic(kp, kq)))
    }

    /// affixes.md E (rare) and cube.md I (crafted): the names, the count and the slots.
    fn rare_pick(&mut self, crafted: bool) -> Option<([u32; 2], Vec<u32>)> {
        let t = self.t;
        if !crafted && !self.flag("Rare") {
            return None;
        }
        let (Some(a), Some(b)) = (self.name(&t.rare_prefix), self.name(&t.rare_suffix)) else { return None };
        let mut slots = vec![0u32; 6];
        if crafted {
            let ilvl = self.ctx.ilvl;
            let tier = if ilvl > 70 { 4 } else if ilvl > 50 { 3 } else if ilvl > 30 { 2 } else { 1 };
            let (mut p, mut s) = (0, 0);
            for _ in 0..(self.s.step() % 5).max(tier) {
                let suffix = p == 3 || (s != 3 && self.s.step() & 1 == 1);
                let (n, tab) = if suffix { (&mut s, &t.magic_suffix) } else { (&mut p, &t.magic_prefix) };
                slots[*n * 2 + suffix as usize] = self.pick(tab, true, true, 0).map_or(0, |r| r as u32 + 1);
                *n += 1;
            }
        } else {
            let count = if self.b.typ == "jewl" { 3 + self.s.range(2) as usize } else { [3, 4, 4, 5, 5, 5, 6, 6][(self.s.step() & 7) as usize] };
            let (mut n, mut done, mut i) = ([0usize; 2], [false; 2], 0);
            while i < count && !(done[0] && done[1]) {
                let side = if done[0] { 1 } else if done[1] { 0 } else { (self.s.step() & 1) as usize };
                match self.pick(if side == 1 { &t.magic_suffix } else { &t.magic_prefix }, true, true, 0) {
                    None => done[side] = true,
                    Some(r) => {
                        slots[n[side] * 2 + side] = r as u32 + 1;
                        n[side] += 1;
                        done[side] |= n[side] > 2;
                        i += 1;
                    }
                }
            }
        }
        Some(([156 + a, b + 1], slots))
    }

    /// The rare or crafted picks, then every property. A kept rare or crafted kind replaces the names and slots after their draws.
    fn rare(&mut self, crafted: bool) -> Result<Option<Kind>> {
        let got = self.rare_pick(crafted);
        let (names, affixes) = match self.keep {
            Some(Kind::Rare { names, affixes } | Kind::Crafted { names, affixes }) => (*names, affixes.clone()),
            _ => match got.filter(|g| g.1.iter().any(|&x| x != 0)) {
                Some(g) => g,
                None => return Ok(None),
            },
        };
        for (k, &id) in affixes.iter().enumerate().filter(|x| *x.1 != 0) {
            self.mods(k % 2, id)?;
        }
        Ok(Some(if crafted { Kind::Crafted { names, affixes } } else { Kind::Rare { names, affixes } }))
    }

    /// affixes.md F: QualityItems rows until one fits the base; a kept superior row replaces the pick after the draws.
    fn superior(&mut self) -> Result<Option<Kind>> {
        let t = self.t;
        let n = if self.flag("Throwable") || self.b.nodurability { 4 } else { t.quality.len() };
        let (mut tried, class) = (vec![false; n], quality_class(&self.chain));
        let row = loop {
            let row = loop {
                let r = self.s.range(n as i32) as usize;
                if !tried[r] {
                    break r;
                }
            };
            if class.is_some_and(|c| t.quality[row].classes.iter().any(|x| x == c)) {
                break Some(row);
            }
            tried[row] = true;
            if tried.iter().all(|&x| x) {
                break None;
            }
        };
        let row = if let Some(Kind::Superior(i)) = self.keep { Some(*i as usize) } else { row };
        let Some((row, q)) = row.and_then(|r| Some((r, t.quality.get(r)?))) else { return Ok(None) };
        self.props(&q.mods.iter().collect::<Vec<_>>())?;
        self.staff();
        Ok(Some(Kind::Superior(row as u32)))
    }

    /// affixes.md G: the LowQualityItems row (a kept row replaces it after the draw); its durability comes from the unit seed.
    fn low(&mut self) -> Option<Kind> {
        let row = self.s.range(self.t.low_quality.len() as i32);
        self.staff();
        let row = if let Some(Kind::Low(i)) = self.keep { *i } else { row };
        matches!(self.b.kind, 'a' | 'w').then_some(Kind::Low(row))
    }

    /// uniques-sets.md C: the weighted unique pick (`keep` replaces the row, not the draw), then its properties.
    fn unique(&mut self) -> Result<Option<Kind>> {
        let (t, mut total) = (self.t, 0);
        let cand: Vec<(usize, i32)> = (0..t.uniques.len())
            .filter(|&i| {
                let u = &t.uniques[i];
                (self.exp || u.version < 100) && u.enabled && u.code == self.ctx.base && u.lvl <= self.ctx.ilvl
            })
            .map(|i| {
                total += (t.uniques[i].rarity as i32).max(1);
                (i, total - (t.uniques[i].rarity as i32).max(1))
            })
            .collect();
        let r = if cand.is_empty() { 0 } else { self.s.range(total) as i32 };
        let row = match self.keep {
            Some(Kind::Unique(i)) => *i as usize,
            _ => match cand.iter().rev().find(|c| r >= c.1) {
                Some(c) => c.0,
                None => return Ok(None),
            },
        };
        let Some(u) = t.uniques.get(row) else { return err(0, "no such unique") };
        self.props(&u.props.iter().collect::<Vec<_>>())?;
        Ok(Some(Kind::Unique(row as u32)))
    }

    /// uniques-sets.md D: the weighted set pick, then the properties and the aprops.
    fn set(&mut self) -> Result<Option<Kind>> {
        let t = self.t;
        let cand: Vec<(usize, i32)> = (0..t.set_items.len())
            .filter(|&i| {
                let s = &t.set_items[i];
                (self.exp || i < 62) && s.lvl <= self.ctx.ilvl && s.code == self.ctx.base && s.set != "Cow King's Leathers"
            })
            .map(|i| (i, (t.set_items[i].rarity as i32).max(1)))
            .collect();
        let total = cand.iter().map(|c| c.1).sum::<i32>();
        let mut r = if total > 0 { self.s.range(total) as i32 } else { 0 };
        let row = match self.keep {
            Some(Kind::Set(i)) => *i as usize,
            _ => match cand.iter().find(|c| {
                r -= c.1;
                r < 0
            }) {
                Some(c) => c.0,
                None => return Ok(None),
            },
        };
        let Some(s) = t.set_items.get(row) else { return err(0, "no such set item") };
        self.props(&s.props.iter().chain(s.aprops.iter().flatten()).collect::<Vec<_>>())?;
        Ok(Some(Kind::Set(row as u32)))
    }

    /// One builder of the `ItemCreateByQuality` switch (uniques-sets.md B5, affixes.md D to H); `None` is a failure.
    fn builder(&mut self, q: u32) -> Result<Option<Kind>> {
        self.q = q;
        match q {
            7 => self.unique(),
            5 => self.set(),
            6 | 8 => self.rare(q == 8),
            4 => self.magic(),
            3 => self.superior(),
            1 => Ok(self.low()),
            _ => {
                self.staff();
                Ok(Some(Kind::Normal))
            }
        }
    }
}

/// Rerolls an item from `seed` like the game's builders do (the item seed is `init(seed)`, base stats come from the unit seed `init(seed)`).
/// `quality` is the quality the drop roll gave (1 low, 2 normal, 3 superior, 4 magic, 5 set, 6 rare, 7 unique, 8 crafted); a failed
/// builder falls back as in uniques-sets.md E. `keep` runs the builder of its kind, makes every selection draw and then
/// replaces each pick with the kept rows, so the values match the game for every seed whose picks land on them; staff
/// mods, ethereal, sockets and automagic are rolled as usual. A kept runeword rolls only its lines, on `{seed, 666}`.
#[allow(clippy::too_many_arguments)]
pub fn roll(t: &ItemTables, sch: &Schema, code: &str, ilvl: u32, quality: u32, keep: Option<&Kind>, seed: u32, expansion: bool) -> Result<Item> {
    let it = new_item(sch, code, ilvl, expansion)?;
    let (c, exp) = (it.view.code, expansion);
    let Some(b) = sch.base(c) else { return err(0, "unknown base") };
    let spec = |kind, ethereal, sockets| Spec { code: code.into(), kind, ilvl, ethereal, sockets, expansion };
    if b.compact || !(1..=8).contains(&quality) {
        return if b.compact { make(t, sch, &spec(Kind::Normal, false, 0), &mut Roll::Min) } else { err(0, "quality must be 1 to 8") };
    }
    let own = itype(&b.typ);
    let tf = |k: &str| own.is_none_or(|r| txt(r, k) == "1");
    if let Some(Kind::Runeword(key)) = keep {
        let n = t.runewords.iter().find(|r| r.key == *key).map_or(0, |r| r.runes.len() as u32);
        return build(t, sch, &spec(Kind::Runeword(key.clone()), false, n), &mut Roll::Seed(&mut Seed::init(seed)), &Pre { seed: Some(seed), ..Pre::default() });
    }
    // uniques-sets.md B4: the overrides of the quality; a kept kind keeps its own
    let mut q = quality;
    if own.is_some_and(|r| txt(r, "Magic") == "1") {
        q = if b.quest { 7 } else if (4..=9).contains(&q) { q } else { 4 };
    }
    q = if own.is_none_or(|r| txt(r, "Rare") != "1") && q == 6 { 4 } else { q };
    q = if tf("Normal") { 2 } else { q };
    let q = keep.map_or(q, |k| match k { Kind::Low(_) => 1, Kind::Normal | Kind::Runeword(_) => 2, Kind::Superior(_) => 3, Kind::Magic(..) => 4, Kind::Set(_) => 5, Kind::Rare { .. } => 6, Kind::Unique(_) => 7, Kind::Crafted { .. } => 8 });
    let ty = own.map(|r| num::<i32>(r, "VarInvGfx")).unwrap_or(0);
    let (chain, dur) = (type_chain(sch, &b.typ), b.kind != 'm' && b.durability > 0 && !b.nodurability);
    let (qlvl, i) = (b.level as i32, ilvl.max(b.level) as i32);
    let alvl = (if b.magic_lvl != 0 { i + b.magic_lvl as i32 } else if i < 99 - qlvl / 2 { i - qlvl / 2 } else { 2 * i - 99 }).clamp(1, 99);
    let mut r = Rl { t, sch, b, chain, ctx: Ctx { ilvl, base: c }, exp, seed, s: Seed::init(seed), us: Seed::init(seed), q, alvl, keep, tape: vec![], acc: Acc { stats: vec![], eth: false, def: 0, maxac: 0, socks: 0 }, taken: vec![], pre: Pre { seed: Some(seed), ..Pre::default() } };
    // properties.md A: the base stats on the unit seed
    let (mins, hi) = (item_col(c, "minstack"), (b.maxstack as i32).min(511));
    let quiver = own.is_some_and(|r| !txt(r, "Quiver").is_empty());
    if quiver || (b.stackable && b.kind != 'a') {
        let sp = item_col(c, "spawnstack");
        let top = if b.kind != 'm' || quiver { hi } else if sp != 0 && sp >= mins { sp } else if mins <= hi { hi } else { mins };
        r.pre.qty = Some((mins + r.us.range(top - mins) as i32).max(1) as u32);
    }
    if b.kind != 'm' {
        let h = b.durability as i32 / 2;
        r.pre.dur = Some(((h + r.us.range(h) as i32).min(255) as u32, b.durability));
    }
    if b.kind == 'a' {
        r.pre.def = Some(b.minac as i32 + r.us.range(b.maxac as i32 - b.minac as i32 + 1) as i32);
    }
    if ty != 0 {
        r.pre.gfx = Some(r.us.range(ty) & 0xFF);
    }
    // uniques-sets.md C to E: the builder, then the fallbacks with their durability multiplier
    let mut kind = r.builder(q)?;
    if kind.is_none() && keep.is_none() {
        let mult = [(7, 3), (5, 2)].iter().find(|x| x.0 == q).map_or(1, |x| x.1);
        r.pre.dur = r.pre.dur.map(|(c, m)| ((c * mult).min(255), (m * mult).min(255)));
        for &f in match q { 7 => &[6, 4, 3, 2][..], 5 | 6 => &[4, 3, 2], 4 => &[3, 2], _ => &[2] } {
            r.reset();
            if let Some(k) = r.builder(f)? {
                kind = Some(k);
                break;
            }
        }
    }
    let Some(kind) = kind else { return err(0, "no builder made the item") };
    if let (Kind::Low(_), true) = (&kind, dur) {
        let m = ((b.durability * 33 / 100) as i32).max(1);
        r.pre.low = Some((m / 2 + r.us.range(m / 2) as i32).max(1) as u32);
    }
    // properties.md F and affixes.md J: the ethereal roll, the socket roll, the automagic pick
    let q = r.q;
    let (mut eth, mut sockets) = (false, 0);
    if exp && q != 8 && !matches!(q, 1 | 5) && dur && !b.quest && !r.acc.stats.iter().any(|x| x.0 == 152) {
        eth = r.s.range(100) < 5;
    }
    let cap = *limits::sockets(sch, c, ilvl).end();
    if (2..=3).contains(&q) && !b.stackable && cap > 0 && (exp || !r.chain.contains(&"tors")) && r.s.range(100) < 33 {
        sockets = seed % cap + 1;
    }
    if exp && !matches!(q, 5 | 7) && b.auto_prefix != 0 && let Some(row) = r.pick(&t.automagic, true, false, b.auto_prefix) {
        r.pre.auto = Some(row);
        r.props(&t.automagic[row].mods.iter().collect::<Vec<_>>())?;
    }
    let mut tape = r.tape.into_iter();
    build(t, sch, &spec(kind, eth, sockets), &mut Roll::Tape(&mut tape), &r.pre)
}

/// One edit of an existing item (see `edit_traits`).
#[derive(Clone, Debug)]
pub enum Trait {
    /// Rebuilds the item as this kind on its own seed.
    Quality(Kind),
    Ilvl(u32),
    Ethereal(bool),
    Sockets(u32),
    /// A single-value property by (stat, param) to a shown value.
    SetProp { stat: u32, param: u32, shown: i32 },
    /// A table block applied at its minimum.
    AddProp(Block),
    RemoveProp { stat: u32, param: u32 },
}

/// The kind an item's quality fields describe.
fn kind_of(v: &ItemView) -> Option<Kind> {
    let q = |i: usize| v.qual.get(i).copied().filter(|&x| x != NONE).unwrap_or(0);
    Some(match v.quality {
        1 => Kind::Low(q(0)),
        2 => Kind::Normal,
        3 => Kind::Superior(q(0)),
        4 => Kind::Magic((q(0) > 0).then_some(q(0)), (q(1) > 0).then_some(q(1))),
        5 => Kind::Set(q(0)),
        6 | 8 => {
            let (names, affixes) = ([q(0), q(1)], (2..8).map(q).collect());
            if v.quality == 6 { Kind::Rare { names, affixes } } else { Kind::Crafted { names, affixes } }
        }
        7 => Kind::Unique(q(0)),
        _ => return None,
    })
}

/// The item with every table value at its minimum and at its maximum (the slider bounds of the editor); None when it cannot be rebuilt.
pub fn bounds(t: &ItemTables, sch: &Schema, item: &Item) -> Option<(Item, Item)> {
    let v = &item.view;
    let kind = if v.flag(26) { Kind::Runeword(runeword_row(t, sch, item)?.key.clone()) } else if v.flag(21) || v.flag(16) { return None } else { kind_of(v)? };
    let spec = Spec { code: String::from_utf8_lossy(&v.code).trim().to_string(), kind, ilvl: v.ilvl, ethereal: v.flag(22), sockets: v.sockets, expansion: v.version >= 100 };
    Some((make(t, sch, &spec, &mut Roll::Min).ok()?, make(t, sch, &spec, &mut Roll::Max).ok()?))
}

/// Checks every game range and the loader's drops (`legit::load_errors`), re-encodes and confirms the item reads back the same.
fn finish(t: &ItemTables, sch: &Schema, it: &mut Item) -> Result<()> {
    let errs = crate::legit::load_errors(t, sch, it);
    if !errs.is_empty() {
        return err(0, errs.join("; "));
    }
    let raw = Item::encode(&it.view, sch)?;
    if Item::parse(&raw, 0, sch)?.0.view != it.view {
        return err(0, "item does not round-trip");
    }
    (it.raw, it.dirty) = (raw, true);
    Ok(())
}

fn reroll(t: &ItemTables, sch: &Schema, item: &mut Item, seed: u32, keep: Option<&Kind>, exp: bool) -> Result<()> {
    let v = item.view.clone();
    if !item.editable || v.flag(21) || v.flag(16) || v.sv != 0 {
        return err(0, "item is not editable or has no seed");
    }
    let code = String::from_utf8_lossy(&v.code).trim().to_string();
    let mut n = if v.flag(26) {
        // sockets-runewords.md E: the lines draw on the completing rune's seed {seed, 666}; a rune is a simple item with
        // no seed field, so that seed is not stored. Nothing else changes.
        let Some(r) = runeword_row(t, sch, item) else { return err(0, "no Runes row matches this runeword") };
        let mut n = item.clone();
        n.view_mut().rw_props = roll(t, sch, &code, v.ilvl, 2, Some(&Kind::Runeword(r.key.clone())), seed, exp)?.view.rw_props;
        n
    } else {
        let mut n = roll(t, sch, &code, v.ilvl, v.quality, keep, seed, exp)?;
        let w = n.view_mut();
        (w.location, w.body, w.x, w.y, w.store, w.version, w.realm) = (v.location, v.body, v.x, v.y, v.store, v.version, v.realm);
        if v.flag(24) {
            (w.flags, w.name) = (w.flags | 1 << 24, v.name.clone());
        }
        if v.n_filled > w.sockets {
            return err(0, "reroll would remove filled sockets");
        }
        w.n_filled = v.n_filled;
        n.children = std::mem::take(&mut item.children);
        n
    };
    finish(t, sch, &mut n)?;
    *item = n;
    Ok(())
}

/// Rerolls `item` from `seed` like the game would, in place: location, store, position, personalized name and socketed
/// items stay; the sockets come from the roll (Err if fewer than `n_filled`). `keep_kind` keeps the unique or set row, or
/// the affixes (see `roll`). A runeword rerolls only its lines, on `{seed, 666}`; that seed is not stored in the save.
pub fn reroll_in_place(t: &ItemTables, sch: &Schema, item: &mut Item, seed: u32, keep_kind: bool, expansion: bool) -> Result<()> {
    let keep = keep_kind.then(|| kind_of(&item.view)).flatten();
    reroll(t, sch, item, seed, keep.as_ref(), expansion)
}

/// Applies `edits` in order, all or none. Refuses any result outside the game ranges (`limits`); legit findings stay warnings.
pub fn edit_traits(t: &ItemTables, sch: &Schema, item: &mut Item, edits: &[Trait]) -> Result<()> {
    if !item.editable {
        return err(0, "item is not editable");
    }
    let mut n = item.clone();
    for e in edits {
        let v = n.view.clone();
        let exp = v.version >= 100;
        match e {
            Trait::Quality(k) if !v.flag(26) => reroll(t, sch, &mut n, v.seed, Some(k), exp)?,
            Trait::Quality(_) => return err(0, "a runeword item has no quality to change"),
            Trait::Ilvl(i) => n.view_mut().ilvl = *i,
            Trait::Sockets(k) if *k < v.n_filled => return err(0, format!("sockets {k} below the {} socketed items", v.n_filled)),
            Trait::Sockets(k) => {
                let w = n.view_mut();
                (w.sockets, w.flags) = (*k, w.flags & !(1 << 11) | u32::from(*k > 0) << 11);
            }
            Trait::Ethereal(on) if v.flag(22) != *on => {
                let b = sch.base(v.code).filter(|b| exp && b.kind != 'm' && b.durability > 0 && !b.nodurability && !b.quest);
                let Some(b) = b else { return err(0, "ethereal needs an expansion character and a base with durability that is not a quest item") };
                let add = sch.stat(31).map_or(0, |d| d.save_add);
                let w = n.view_mut();
                let def = w.defense as i32 - add;
                if *on {
                    (w.defense, w.maxdur) = (if b.kind == 'a' { (def * 3 / 2 + add) as u32 } else { w.defense }, w.maxdur / 2 + 1);
                } else {
                    (w.defense, w.maxdur) = (if b.kind == 'a' { ((2 * def + 2) / 3 + add) as u32 } else { w.defense }, if w.quality == 1 { (b.durability * 33 / 100).max(1) } else { b.durability });
                }
                (w.flags, w.dur) = (w.flags ^ 1 << 22, w.dur.min(w.maxdur));
            }
            Trait::Ethereal(_) => {}
            Trait::SetProp { stat, param, shown } => {
                let Some(i) = v.props.iter().position(|p| p.id == *stat && p.param == *param) else { return err(0, "no such property") };
                crate::edit::set_prop_value(&mut n, i, i64::from(*shown), sch)?;
            }
            Trait::AddProp(b) => {
                let stats = apply(t, sch, b, &Ctx { ilvl: v.ilvl, base: v.code }, &mut Roll::Min)?;
                let new = pack(sch, &stats)?;
                if new.is_empty() || new.iter().any(|p| v.props.iter().any(|q| q.id == p.id && q.param == p.param)) {
                    return err(0, "the item already has that property, or it adds nothing");
                }
                n.view_mut().props.extend(new);
            }
            Trait::RemoveProp { stat, param } => {
                let w = n.view_mut();
                let l = w.props.len();
                w.props.retain(|p| !(p.id == *stat && p.param == *param));
                if w.props.len() == l {
                    return err(0, "no such property");
                }
            }
        }
    }
    finish(t, sch, &mut n)?;
    *item = n;
    Ok(())
}
