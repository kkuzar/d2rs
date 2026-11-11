//! Act link builder: level rects, seams and Act I outdoor flags (specs/drlg/warps.md A-C).
use crate::drlg::{MapData, MapError, Rect};
use crate::maze::overlap;
use crate::{jungle, Seed, SeedTree};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Anchor,
    Cycle4,
    Cycle8,
    Rogue,
    Blood,
    Fixed,
    Rocky,
    Valley,
    Steppes,
    Rigid,
    Arreat,
    Tundra,
}
use Pick::*;

/// B4: `(pick, level, anchor index)`; the check is `Chk`.
type Table = &'static [(Pick, u32, i32)];
#[derive(Clone, Copy, PartialEq)]
enum Chk {
    Plain,
    Act1A,
    Act1B,
    None,
}
const T1A: Table = &[(Anchor, 4, -1), (Cycle4, 3, 0), (Blood, 2, 1), (Rogue, 1, 2), (Cycle4, 17, 1)];
const T1B: Table = &[(Anchor, 39, -1), (Anchor, 26, -1), (Fixed, 7, 1), (Cycle4, 6, 2), (Cycle4, 5, 3)];
const T2A: Table = &[(Anchor, 40, -1), (Rocky, 41, 0), (Cycle8, 42, 1), (Cycle8, 43, 2), (Cycle8, 44, 3), (Valley, 45, 4)];
const T4A: Table = &[(Anchor, 103, -1), (Steppes, 104, 0), (Cycle4, 105, 1), (Cycle4, 106, 2)];
const T5A: Table = &[(Anchor, 109, -1), (Anchor, 110, 0), (Rigid, 111, 1), (Arreat, 112, 2)];
const T5B: Table = &[(Tundra, 117, -1)];
const ACTS: [&[(Table, Chk)]; 5] = [
    &[(T1A, Chk::Act1A), (T1B, Chk::Act1B)],
    &[(T2A, Chk::Plain), (&[(Anchor, 46, -1)], Chk::Plain)],
    &[],
    &[(T4A, Chk::Plain), (&[(Anchor, 108, -1)], Chk::Plain)],
    &[(T5A, Chk::None), (T5B, Chk::None)],
];
/// Nonzero indices of `g_LinkRogueSideOk`.
const ROGUE_OK: [i32; 16] = [0, 1, 9, 19, 21, 24, 30, 31, 33, 39, 41, 53, 54, 56, 62, 63];
const ARREAT: [(i32, i32); 4] = [(0, -160), (-96, -64), (-64, -96), (-160, 0)];
/// H0 `g_LinkOutdoorFlags`: `(level, not1, not2, dirI, dirNext, flag)`.
const OUT_FLAGS: [(u32, u32, u32, i32, i32, u32); 15] = [
    (0, 2, 3, 1, 0, 4), (0, 2, 3, 2, 3, 4), (0, 3, 17, 2, 1, 8), (0, 3, 17, 3, 0, 8), (0, 3, 17, 1, 1, 0x10), (0, 3, 17, 3, 3, 0x10),
    (2, 0, 0, 0, 0, 8), (2, 0, 0, 2, 2, 8), (2, 0, 0, 3, 0, 8), (2, 0, 0, 3, 2, 8), (2, 0, 0, 0, 1, 0x400), (2, 0, 0, 1, 1, 0x400),
    (2, 0, 0, 2, 1, 0x200), (2, 0, 0, 2, 2, 0x80), (2, 0, 0, 3, 2, 0x100),
];

/// The act's levels after the builder: rects, the link nodes (`vis`, `warp`) and the outdoor flags.
#[derive(Default)]
pub struct Layout {
    pub rect: BTreeMap<u32, Rect>,
    pub link: BTreeMap<u32, ([i32; 8], [i32; 8])>,
    pub flags: BTreeMap<u32, u32>,
    /// J2e: the jungle `Def` arrays of levels 76 to 78 (rows outer).
    pub jungle: BTreeMap<u32, Vec<i32>>,
    /// The links as they stood when `LevelBorderAdj` ran, where that differs from `link` (Act V).
    pub adj: BTreeMap<u32, ([i32; 8], [i32; 8])>,
    /// C1: the preset file index the builder writes for levels 1 and 40 (the town orientation).
    pub town: BTreeMap<u32, i32>,
    /// C1: level 27's file index, drawn on the act seed when Black Marsh ends on side 1 or 3.
    pub idx27: Option<u32>,
}

impl Layout {
    /// C4 LevelSetPosition: a placed level's rect, else its `Levels` offset plus the position of its `Depend` level.
    pub fn rect_of(&self, md: &MapData, id: u32, d: usize) -> Option<Rect> {
        self.rect.get(&id).copied().or_else(|| {
            let def = md.t.levels.iter().find(|l| l.id as u32 == id)?;
            let (bx, by) = if def.depend != 0 { self.rect_of(md, def.depend as u32, d).map_or((0, 0), |r| (r.x, r.y)) } else { (0, 0) };
            Some(Rect { x: def.offset_x + bx, y: def.offset_y + by, w: def.size_x[d], h: def.size_y[d] })
        })
    }
}

struct B {
    seed: Seed,
    rect: Vec<Rect>,
    dir: Vec<i32>,
    dir0: Vec<i32>,
    flip: Vec<i32>,
    flip0: Vec<i32>,
    flag: u32,
}

impl B {
    /// B1: the next cycling choice (`n` a power of two), `None` once back at the first.
    fn cycle(&mut self, i: usize, n: i32) -> Option<i32> {
        if self.dir0[i] < 0 {
            self.dir0[i] = (self.seed.step() & (n as u32 - 1)) as i32;
            self.dir[i] = self.dir0[i];
            return Some(self.dir[i]);
        }
        let d = (self.dir[i] + 1) % n;
        (d != self.dir0[i]).then(|| {
            self.dir[i] = d;
            d
        })
    }

    /// B2 `0x00676450`: the `(d, f)` cycle.
    fn cycle_f(&mut self, i: usize) -> Option<(i32, i32)> {
        let (d, f) = if self.dir0[i] < 0 {
            let d = (self.seed.step() & 3) as i32;
            let f = (self.seed.step() & 1) as i32;
            (self.dir0[i], self.flip0[i]) = (d, f);
            (d, f)
        } else {
            let (d, f) = ((self.dir[i] + self.flip[i]) % 4, (self.flip[i] + 1) % 2);
            if (d, f) == (self.dir0[i], self.flip0[i]) {
                return None;
            }
            (d, f)
        };
        (self.dir[i], self.flip[i]) = (d, f);
        Some((d, f))
    }

    /// B2: place entry `i` by its pick; false when exhausted.
    fn pick(&mut self, t: Table, i: usize, off: (i32, i32)) -> bool {
        let (kind, _, a) = t[i];
        let p = if a >= 0 { self.rect[a as usize] } else { Rect { x: 0, y: 0, w: 0, h: 0 } };
        let (mut w, mut h) = (self.rect[i].w, self.rect[i].h);
        let (x, y) = match kind {
            Anchor => off,
            Cycle4 => match self.cycle(i, 4) {
                Some(0) => (p.x - 16, p.y + p.h),
                Some(1) => (p.x - w, p.y - 16),
                Some(2) => (p.x + p.w - w + 16, p.y - h),
                Some(_) => (p.x + p.w, p.y + p.h - h + 16),
                None => return false,
            },
            Cycle8 => match self.cycle(i, 8) {
                Some(0) => (p.x - 8 - w / 2, p.y + p.h),
                Some(1) => (p.x + w / 2 + 8, p.y + p.h),
                Some(2) => (p.x - w, p.y - h / 2 - 8),
                Some(3) => (p.x - w, p.y + h / 2 + 8),
                Some(4) => (p.x - 8 - w / 2, p.y - h),
                Some(5) => (p.x + w / 2 + 8, p.y - h),
                Some(6) => (p.x + p.w, p.y - h / 2 - 8),
                Some(_) => (p.x + p.w, p.y + h / 2 + 8),
                None => return false,
            },
            Rogue | Blood => {
                let Some((d, f)) = self.cycle_f(i) else { return false };
                if kind == Blood {
                    (w, h) = if d & 1 == 1 { (96, 56) } else { (56, 96) };
                }
                let k = if kind == Blood { 16 } else { 0 };
                match (f, d) {
                    (1, 0) => (p.x - k, p.y + p.h),
                    (1, 1) => (p.x - w, if k > 0 { p.y - 16 } else { p.y + 8 }),
                    (1, 2) => (p.x + p.w - w + k, p.y - h),
                    (1, _) => (p.x + p.w, p.y + p.h - h + if k > 0 { 16 } else { -8 }),
                    (_, 0) => (p.x + p.w - w + k, p.y + p.h),
                    (_, 1) => (p.x - w, p.y + p.h - h + if k > 0 { 16 } else { -8 }),
                    (_, 2) => (p.x - k, p.y - h),
                    _ => (p.x + p.w, if k > 0 { p.y - 16 } else { p.y + 8 }),
                }
            }
            Fixed => {
                self.dir[i] = 0;
                (p.x, p.y + p.h)
            }
            Rocky | Arreat => {
                let base = i32::from(kind == Rocky);
                let d = if self.dir0[i] < 0 {
                    self.dir0[i] = (self.seed.step() & 1) as i32 + base;
                    self.dir0[i]
                } else if self.dir[i] == self.dir0[i] {
                    2 * base + 1 - self.dir0[i]
                } else {
                    return false;
                };
                self.dir[i] = d;
                if kind == Rocky {
                    if d == 1 { (p.x - w, p.y) } else { (p.x, p.y - h) }
                } else {
                    (w, h) = if d == 0 { (64, 160) } else { (160, 64) };
                    let (dx, dy) = ARREAT[(d + 2 * self.dir[a as usize]) as usize & 3];
                    (p.x + dx, p.y + dy)
                }
            }
            Valley => match self.cycle(i, 8) {
                Some(0 | 1) => (p.x, p.y + p.h),
                Some(2 | 3) => (p.x - w, p.y),
                Some(4 | 5) => (p.x, p.y - h),
                Some(_) => (p.x + p.w, p.y),
                None => return false,
            },
            Steppes => {
                self.dir[i] = 3;
                if self.seed.step() & 1 == 1 {
                    self.flag = 0x800000;
                    (p.x + p.w, p.y + p.h - h + 8)
                } else {
                    self.flag = 0x400000;
                    (p.x + p.w, p.y - 8)
                }
            }
            Rigid | Tundra => {
                let b = (self.seed.step() & 1) as i32;
                self.dir[i] = b;
                (w, h) = if b == 0 { (64, 160) } else { (160, 64) };
                if kind == Rigid { (p.x - w, p.y + p.h - h - 16) } else { off }
            }
        };
        self.rect[i] = Rect { x, y, w, h };
        true
    }
}

/// B3: the validators.
fn check(c: Chk, t: Table, b: &B, i: usize) -> bool {
    let a = t[i].2;
    if c == Chk::None {
        return true;
    }
    if (0..i).any(|j| j as i32 != a && overlap(b.rect[i], b.rect[j])) {
        return false;
    }
    match c {
        Chk::Act1A if t[i].1 == 1 => {
            let a = a as usize;
            ROGUE_OK.contains(&(b.dir[i] + 4 * b.flip[i] + 8 * b.dir[a] + 32 * b.flip[a]))
        }
        Chk::Act1A if t[i].1 == 17 => !(0..t.len()).any(|j| j != i && t[j].2 == a && b.dir[j] == b.dir[i]),
        Chk::Act1B if i != 0 => {
            let r0 = b.rect[0];
            !overlap(Rect { x: r0.x, y: r0.y - 200, w: r0.w, h: r0.h + 200 }, b.rect[i])
        }
        _ => true,
    }
}

/// C2.
fn link_set(l: &mut Layout, md: &MapData, id: u32, dest: u32, warp: i32) {
    let Some(def) = md.t.levels.iter().find(|d| d.id as u32 == id) else { return };
    let n = l.link.entry(id).or_insert((def.vis, def.warp));
    let k = n.0.iter().position(|&v| v == dest as i32).or_else(|| (0..8).find(|&k| n.0[k] == 0 && n.1[k] == -1));
    if let Some(k) = k {
        n.0[k] = dest as i32;
        n.1[k] = warp;
    }
}

/// C3: a shared edge segment.
fn touch(a: Rect, b: Rect) -> bool {
    let g = |p: i32, q: i32, pw: i32, qw: i32| if p <= q { q - pw - p } else { p - qw - q };
    let (gx, gy) = (g(a.x, b.x, a.w, b.w), g(a.y, b.y, a.h, b.h));
    gx == 0 && gy <= -1 || gy == 0 && gx <= -1
}

/// C3 LevelSeamsByTouch: a walkable link both ways between every touching pair of `lo..=hi`.
fn seams_by_touch(l: &mut Layout, md: &MapData, lo: u32, hi: u32) {
    for a in lo..=hi {
        for z in (lo..=hi).filter(|&z| z != a) {
            if matches!((l.rect.get(&a), l.rect.get(&z)), (Some(&ra), Some(&rz)) if touch(ra, rz)) {
                link_set(l, md, a, z, -1);
            }
        }
    }
}

/// B1 and C1 for every table of the 0-based act; Act III (J1) is level 75 at its offset, the jungle layout on the act seed, the Kurast stack and the seams by touch.
pub fn layout(md: &MapData, map_seed: u32, diff: u8, act: u8) -> Result<Layout, MapError> {
    let mut l = Layout::default();
    let lv = |id: u32| md.t.levels.iter().find(|d| d.id as u32 == id).ok_or_else(|| MapError::Data(format!("level {id}")));
    let d = usize::from(diff.min(2));
    let mut act_seed = SeedTree { map_seed }.act_seed(act);
    if act == 2 {
        let (t, j) = (lv(75)?, lv(76)?);
        l.rect.insert(75, Rect { x: t.offset_x, y: t.offset_y, w: t.size_x[d], h: t.size_y[d] });
        let mut p = Rect { x: 0, y: 0, w: 0, h: 0 };
        for (i, (r, defs)) in jungle::layout(&mut act_seed, j.size_x[d], j.size_y[d], (t.offset_x, t.offset_y))?.into_iter().enumerate() {
            l.rect.insert(76 + i as u32, r);
            l.jungle.insert(76 + i as u32, defs);
            p = r;
        }
        // J3 KurastStack: 79 to 83 stacked north of 78, each centred on it.
        let mut acc = 0;
        for id in 79..=83 {
            let (w, h) = (lv(id)?.size_x[d], lv(id)?.size_y[d]);
            acc -= h;
            l.rect.insert(id, Rect { x: p.x + p.w / 2 - w / 2, y: p.y + acc, w, h });
        }
        seams_by_touch(&mut l, md, 75, 83);
        return Ok(l);
    }
    for &(t, c) in ACTS[usize::from(act.min(4))] {
        let n = t.len();
        let mut b = B { seed: act_seed, rect: vec![], dir: vec![-1; n], dir0: vec![-1; n], flip: vec![-1; n], flip0: vec![-1; n], flag: 0 };
        for &(_, id, _) in t {
            let def = lv(id)?;
            b.rect.push(Rect { x: 0, y: 0, w: def.size_x[d], h: def.size_y[d] });
        }
        let mut i = 0;
        let mut steps = 0u32;
        while i < n {
            steps += 1;
            if steps > 100_000 {
                return Err(MapError::Unsupported(t[0].1, "link builder does not settle"));
            }
            let def = lv(t[i].1)?;
            if !b.pick(t, i, (def.offset_x, def.offset_y)) {
                (b.dir[i], b.dir0[i], b.flip[i], b.flip0[i]) = (-1, -1, -1, -1);
                match i.checked_sub(1) {
                    Some(p) => i = p,
                    None => return Err(MapError::Unsupported(t[0].1, "link builder exhausted")),
                }
                continue;
            }
            if check(c, t, &b, i) {
                i += 1;
            }
        }
        for (i, &(_, id, a)) in t.iter().enumerate() {
            l.rect.insert(id, b.rect[i]);
            // C1: the town orientation (level 40 takes Rocky Waste's side) and level 27's index, the builder's one act-seed draw.
            match (id, b.dir[i]) {
                (1, d) => drop(l.town.insert(1, d)),
                (40, _) => drop(l.town.insert(40, b.dir.get(i + 1).copied().unwrap_or(-1))),
                (6, 1) => l.idx27 = Some(2 - (act_seed.step() & 1)),
                (6, 3) => l.idx27 = Some(!act_seed.step() & 1),
                _ => {}
            }
            if act == 0 {
                // H0 `LinkOutdoorFlags`: B.dir[i + 1] is -1 past the last entry.
                let next = b.dir.get(i + 1).copied().unwrap_or(-1);
                let f = OUT_FLAGS.iter().filter(|r| (r.0 == id || r.0 == 0) && id != r.1 && id != r.2 && b.dir[i] == r.3 && next == r.4).fold(0, |f, r| f | r.5);
                if lv(id)?.drlg_type == 3 {
                    *l.flags.entry(id).or_default() |= f;
                }
            }
            if act != 4 && a >= 0 {
                let anchor = t[a as usize].1;
                link_set(&mut l, md, id, anchor, -1);
                link_set(&mut l, md, anchor, id, -1);
            }
        }
        if act == 3 && b.flag != 0 && t[0].1 == 103 {
            *l.flags.entry(104).or_default() |= b.flag;
        }
    }
    if act == 4 {
        seams_by_touch(&mut l, md, 111, 112);
        // A1: Act V builds its adjacency lists here, before the 110 and 109 seams exist (pinned by the level 111 window of worked example 5).
        l.adj = l.link.clone();
        seams_by_touch(&mut l, md, 110, 111);
        seams_by_touch(&mut l, md, 109, 110);
    }
    Ok(l)
}
