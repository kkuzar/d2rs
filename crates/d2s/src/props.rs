//! The property roll (properties.md B to E): turns a table block (code, par, min, max) into stats, and packs stats into stored props.
use crate::bits::{err, Result};
use crate::item::Prop;
use crate::rng::Seed;
use crate::schema::{num, rows, txt, Schema};
use crate::tables::{Block, ItemTables};
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Where rolled values come from: the item seed, or the extremes of every range, or one fixed value.
/// `Rec` draws from the seed and records every value; `Tape` replays them (a builder that interleaves its picks with property rolls).
pub enum Roll<'a> {
    Seed(&'a mut Seed),
    Rec(&'a mut Seed, &'a mut Vec<i32>),
    Tape(&'a mut std::vec::IntoIter<i32>),
    Max,
    Min,
    Fixed(i32),
}

pub struct Ctx {
    pub ilvl: u32,
    pub base: [u8; 4],
}

type Stat = (u32, u32, i32);

impl Roll<'_> {
    /// `Range(n)`: always one draw for a seed.
    fn range(&mut self, n: i32) -> i32 {
        match self {
            Roll::Seed(s) => s.range(n) as i32,
            Roll::Rec(s, t) => {
                t.push(s.range(n) as i32);
                t[t.len() - 1]
            }
            Roll::Tape(t) => t.next().unwrap_or(0),
            Roll::Min => 0,
            Roll::Max => n - 1,
            Roll::Fixed(v) => *v,
        }
    }

    /// D1: no draw when min = max; a reversed pair is swapped.
    fn draw(&mut self, a: i32, b: i32) -> i32 {
        let (lo, hi) = (a.min(b), a.max(b));
        match self {
            Roll::Fixed(v) => *v,
            _ if lo == hi => lo,
            r => lo.wrapping_add(r.range(hi.wrapping_sub(lo).wrapping_add(1))),
        }
    }
}

/// `v <- prev or Roll`.
fn pr(roll: &mut Roll, b: &Block, prev: i32) -> i32 {
    if prev != 0 { prev } else { roll.draw(b.min, b.max) }
}

/// D2: nothing is written for 0; the handler returns the value either way.
fn w(s: u32, p: u32, v: i32) -> (i32, Vec<Stat>) {
    (v, if v != 0 { vec![(s, p, v)] } else { vec![] })
}

/// Weapon damage columns of a base: mindam, maxdam, 2handmindam, 2handmaxdam, minmisdam, maxmisdam (items.tsv).
fn dam(code: [u8; 4]) -> [i32; 6] {
    static D: OnceLock<HashMap<[u8; 4], [i32; 6]>> = OnceLock::new();
    let d = D.get_or_init(|| {
        rows(crate::data::table("items"))
            .iter()
            .filter(|r| txt(r, "kind") == "w")
            .map(|r| {
                let mut k = [b' '; 4];
                txt(r, "code").bytes().zip(&mut k).for_each(|(c, d)| *d = c);
                (k, ["mindam", "maxdam", "2handmindam", "2handmaxdam", "minmisdam", "maxmisdam"].map(|c| num(r, c)))
            })
            .collect()
    });
    d.get(&code).copied().unwrap_or_default()
}

/// Skills.txt keys: lower-case internal name to (id, reqlevel), plus the same by id (skills.tsv).
type Skills = (HashMap<String, i32>, HashMap<i32, i32>, HashMap<i32, &'static str>);

fn skills() -> &'static Skills {
    static S: OnceLock<Skills> = OnceLock::new();
    S.get_or_init(|| {
        let r = rows(crate::data::table("skills"));
        (r.iter().map(|r| (txt(r, "skillkey").to_lowercase(), num(r, "*Id"))).collect(), r.iter().map(|r| (num(r, "*Id"), num(r, "reqlevel"))).collect(), r.iter().map(|r| (num(r, "*Id"), txt(r, "itypea1"))).collect())
    })
}

/// Skills.txt `itypea1` (the item type a skill needs, empty for none); None when the skill has no record.
pub fn skill_itype(id: i32) -> Option<&'static str> {
    skills().2.get(&id).copied()
}

/// The Skills.txt row count; a skill id is its row.
pub fn skill_rows() -> u32 {
    skills().1.len() as u32
}

fn req_level(skill: i32) -> Option<i32> {
    skills().1.get(&skill).copied()
}

/// E3: the skill level of funcs 11 and 19; None when the skill does not exist.
fn skill_level(b: &Block, par: i32, ilvl: i32) -> Option<i32> {
    let req = req_level(par)?;
    Some(match b.max {
        1.. => b.max,
        0 => ((ilvl - req) / 4 + 1).clamp(1, 20),
        m => match (ilvl - req) / (-((99 - req).max(1) / m)).max(1) {
            l if l <= 0 => 1,
            l => l,
        },
    })
}

/// E1: funcs 5 (min) and 6 (max) write the damage stats of the base columns; the value is returned unclamped.
fn e1(max: bool, v: i32, weapon: bool, c: [i32; 6]) -> (i32, Vec<Stat>) {
    let o = max as usize;
    let (c1, c2, ct) = (c[o], c[2 + o], c[4 + o]);
    let stats = if max { [22, 24, 160] } else { [21, 23, 159] };
    let skip = [weapon && c1 == 0 && c2 != 0, weapon && c2 == 0 && c1 != 0, weapon && c[5] == 0];
    let out = (0..3)
        .filter(|&i| !skip[i])
        .filter_map(|i| {
            let col = [c1, c2, ct][i];
            let x = if col != 0 && col + v <= 0 { if max { -col } else { 1 - col } } else { v };
            (x != 0).then_some((stats[i], 0, x))
        })
        .collect();
    (v, out)
}

/// Applies one table block: the stats `Properties` makes of it, with the values as shown (no Save Add, no ValShift).
pub fn apply(t: &ItemTables, sch: &Schema, b: &Block, ctx: &Ctx, roll: &mut Roll) -> Result<Vec<Stat>> {
    let Some(p) = t.props.iter().find(|p| p.code.eq_ignore_ascii_case(&b.code)) else { return err(0, format!("unknown property {}", b.code)) };
    let par: i32 = if b.par.is_empty() { 0 } else { b.par.parse().ok().or_else(|| skills().0.get(&b.par.to_lowercase()).copied()).map_or_else(|| err(0, format!("property {} par {}", b.code, b.par)), Ok)? };
    let base = sch.base(ctx.base);
    let weapon = base.is_some_and(|x| x.kind == 'w');
    let cols = dam(ctx.base);
    let (mut out, mut prev) = (vec![], 0);
    for (k, &(func, stat, _set, val)) in p.slots.iter().enumerate() {
        let st = stat.unwrap_or(u32::MAX);
        let (r, ws) = match func {
            // E table: Roll, W(stat, 0, v)
            1 | 2 => w(st, 0, roll.draw(b.min, b.max)),
            // E table: v <- prev or Roll
            3 | 8 => w(st, 0, pr(roll, b, prev)),
            // E1: min damage
            5 => e1(false, pr(roll, b, prev), weapon, cols),
            // E1: max damage
            6 => e1(true, pr(roll, b, prev), weapon, cols),
            // E2: damage percent, with the +1 max damage fallback on weapons
            7 => {
                let v = pr(roll, b, prev);
                if weapon && i64::from(cols[1].max(cols[3])) * i64::from(v) / 100 <= 0 {
                    (v, e1(true, 1, weapon, cols).1)
                } else {
                    (v, if v != 0 { vec![(18, 0, v), (17, 0, v)] } else { vec![] })
                }
            }
            // E table: the skill tab packs the class in bits 3 and up
            10 => w(st, (par % 3 + par / 3 * 8) as u32, pr(roll, b, prev)),
            // E3: skill on event, no draw
            11 => skill_level(b, par, ctx.ilvl as i32).map_or((0, vec![]), |l| w(st, par.wrapping_mul(64).wrapping_add(l & 63) as u32, if b.min > 0 { b.min } else { 5 })),
            // E table: the roll is the param, par the value
            12 => w(st, roll.draw(b.min, b.max) as u32, par),
            // E5: the rolled count, clamped by ItemMaxSockets, comes back as stat 194 (the socket field, never a stored prop)
            14 => {
                let cap = (base.map_or(0, |x| x.w * x.h)).min(6);
                if cap == 0 {
                    (0, vec![])
                } else {
                    let n = pr(roll, b, prev.max(0));
                    let cap = cap.min(*crate::limits::sockets(sch, ctx.base, ctx.ilvl).end()) as i32;
                    (0, if cap < 1 { vec![] } else { vec![(194, 0, (if n < 1 { par } else { n }).max(1).min(cap))] })
                }
            }
            // E table: v <- min; stat 21 goes through E1 as func 5
            15 => if st == 21 { e1(false, pr(roll, b, b.min), weapon, cols) } else { w(st, 0, b.min) },
            // E table: v <- max; stat 22 goes through E1 as func 6
            16 => if st == 22 { e1(true, pr(roll, b, b.max), weapon, cols) } else { w(st, 0, b.max) },
            // E table: v <- par (a roll when par is 0); stat 22 goes through E1 as func 6
            17 => {
                let v = if par == 0 { roll.draw(b.min, b.max) } else { par };
                if st == 22 { e1(true, v, weapon, cols) } else { w(st, 0, v) }
            }
            // E4: charges, one draw; set directly with the packed param
            19 => match skill_level(b, par, ctx.ilvl as i32) {
                None => (0, vec![]),
                Some(l) => {
                    let c = match b.min { 0 => 5, m if m < 0 => -m + (-m * l) / 8, m => m }.clamp(1, 255);
                    let r = roll.range(c - c / 8);
                    (1, vec![(st, ((par << 6) + (l & 63)) as u32, ((r + 1 + c / 8) & 0xFF) + c * 256)])
                }
            },
            // E table: indestructible
            20 => w(152, 0, 1),
            // E table: val1 is the param (class or element)
            21 => w(st, val as u32, roll.draw(b.min, b.max)),
            // E table: par is the param
            22 => w(st, par as u16 as u32, roll.draw(b.min, b.max)),
            // E table: makes the item ethereal, writes no stat
            23 => (1, vec![]),
            // E table: Roll, W(stat, 0, v) (the durability base stat follows from the stats)
            13 => w(st, 0, roll.draw(b.min, b.max)),
            // E table: v <- prev or Roll, par is the param
            24 => w(st, par as u32, pr(roll, b, prev)),
            // E table: the roll is the param, val the value
            36 => w(st, roll.draw(b.min, b.max) as u32, val),
            f => return err(0, format!("property {} func {f} unsupported", b.code)),
        };
        out.extend(ws.into_iter().filter(|s| s.0 == 194 || sch.stat(s.0).is_some()));
        if k == 0 {
            prev = r;
        }
    }
    Ok(out)
}

/// (stat, param, lo, hi) of a block: `apply` at both extremes, entries summed per (stat, param).
pub fn range_of(t: &ItemTables, sch: &Schema, b: &Block, ctx: &Ctx) -> Result<Vec<(u32, u32, i32, i32)>> {
    let mut m: BTreeMap<(u32, u32), (i32, i32)> = BTreeMap::new();
    for (i, mut roll) in [Roll::Min, Roll::Max].into_iter().enumerate() {
        for (s, p, v) in apply(t, sch, b, ctx, &mut roll)?.into_iter().filter(|x| x.0 != 194) {
            let e = m.entry((s, p)).or_default();
            if i == 0 { e.0 += v } else { e.1 += v }
        }
    }
    Ok(m.into_iter().map(|((s, p), (a, b))| (s, p, a.min(b), a.max(b))).collect())
}

/// First stat and member count of the grouped stats.
const GROUPS: [(u32, u32); 6] = [(17, 2), (48, 2), (50, 2), (52, 2), (54, 3), (57, 3)];

/// Stored props from shown stats: duplicates add up, grouped stats share one prop, values are stored as shown + Save Add.
pub fn pack(sch: &Schema, stats: &[Stat]) -> Result<Vec<Prop>> {
    let mut m: BTreeMap<(u32, u32), Vec<i32>> = BTreeMap::new();
    for &(s, p, v) in stats {
        let (g, n) = GROUPS.iter().copied().find(|&(g, n)| (g..g + n).contains(&s)).unwrap_or((s, 1));
        let e = m.entry((g, p)).or_insert_with(|| vec![0; n as usize]);
        e[(s - g) as usize] += v;
    }
    let mut out = vec![];
    for ((id, param), shown) in m.into_iter().filter(|(_, v)| v.iter().any(|&x| x != 0)) {
        if id == 194 {
            return err(0, "sockets are the socket field, never a property (save/d2s.md I3)");
        }
        let pd = sch.stat(id).map_or(0, |d| d.param_bits);
        if u64::from(param) >> pd != 0 {
            return err(0, format!("stat {id}: param {param} does not fit {pd} bits"));
        }
        let mut vals = vec![];
        for (k, v) in shown.into_iter().enumerate() {
            let d = match sch.stat(id + k as u32) {
                Some(d) if d.save_bits > 0 => d,
                _ => return err(0, format!("stat {} is not stored", id + k as u32)),
            };
            let raw = i64::from(v) + i64::from(d.save_add);
            if !(0..1i64 << d.save_bits).contains(&raw) {
                return err(0, format!("stat {}: value {v} does not fit {} bits", id + k as u32, d.save_bits));
            }
            vals.push(raw as u32);
        }
        out.push(Prop { id, param, vals });
    }
    Ok(out)
}
