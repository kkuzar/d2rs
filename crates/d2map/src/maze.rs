//! Maze levels: room growth, specials, placement and themes (specs/drlg/maze.md).
use crate::drlg::{Kind, Level, MapData, MapError, Rect, Room};
use crate::{preset, seeds::room_seed, Seed};

type Res<T> = Result<T, MapError>;

/// Constants: `DefFromExits` base by level type, and the types the theme pass runs on.
const BASES: [(i32, i32); 14] = [(3, 0x34), (4, 0x6C), (7, 0xA7), (8, 0xCD), (10, 0x101), (13, 0x12D), (17, 0x19D), (18, 0x1E1), (19, 0x1FD), (22, 0x2F1), (24, 0x298), (25, 0x2C0), (28, 0x344), (34, 0x422)];
const THEMED: [i32; 10] = [3, 4, 7, 8, 10, 13, 17, 22, 24, 25];
/// E5: the hell start entries `(def, dir, idx)`, two per `e / 2`.
const HELL: [(i32, u8, i32); 8] = [(1054, 1, 0), (1053, 3, 1), (1054, 1, 1), (1053, 3, 0), (1055, 0, 1), (1056, 2, 0), (1055, 0, 0), (1056, 2, 1)];
/// D: Temple Down and Waypoint entries `(matchDef, def, dir)`.
const TEMPLE_DOWN: [(i32, i32, u8); 3] = [(1042, 1046, 1), (1043, 1047, 0), (1045, 1048, 3)];
const TEMPLE_WAY: [(i32, i32, u8); 4] = [(1042, 1049, 1), (1043, 1050, 0), (1045, 1052, 3), (1044, 1051, 2)];

struct M {
    r: Rect,
    seed: Seed,
    born: Seed,
    def: i32,
    idx: i32,
    lock: bool,
    adj: Vec<(usize, u8)>,
}

struct G {
    lid: u32,
    lt: i32,
    rooms_n: i32,
    merge: i32,
    size: (i32, i32),
    seed: Seed,
    m: Vec<M>,
    /// Link order: oldest first, so "newest first" walks it in reverse.
    list: Vec<usize>,
    rot: usize,
    tombs: (u32, u32),
}

/// B2: the gap between two rects; they overlap when both gaps are negative.
fn gap(a: Rect, b: Rect) -> (i32, i32) {
    (if a.x < b.x { b.x - a.w - a.x } else { a.x - b.w - b.x }, if a.y < b.y { b.y - a.h - a.y } else { a.y - b.h - b.y })
}

pub(crate) fn overlap(a: Rect, b: Rect) -> bool {
    let (gx, gy) = gap(a, b);
    gx < 0 && gy < 0
}

impl G {
    fn fatal(&self) -> MapError {
        MapError::Unsupported(self.lid, "maze fatal")
    }

    /// B1 RoomCreate: one level draw, the room seed `Init(low)` stepped once.
    fn create(&mut self) -> usize {
        let born = room_seed(&mut self.seed);
        let r = Rect { x: 0, y: 0, w: self.size.0, h: self.size.1 };
        self.m.push(M { r, seed: born, born, def: 0, idx: -1, lock: false, adj: vec![] });
        self.m.len() - 1
    }

    /// RoomFree: unlink from the level list and from every neighbour.
    fn free(&mut self, i: usize) {
        self.list.retain(|&q| q != i);
        self.m.iter_mut().for_each(|m| m.adj.retain(|a| a.0 != i));
    }

    /// B3 Place.
    fn place(&mut self, r: usize, p: usize, d: u8) -> bool {
        let pr = self.m[p].r;
        let (dx, dy) = [(-1, 0), (0, -1), (1, 0), (0, 1), (-1, -1), (1, -1), (1, 1), (-1, 1)][usize::from(d)];
        self.m[r].r.x = pr.x + dx * pr.w;
        self.m[r].r.y = pr.y + dy * pr.h;
        let rr = self.m[r].r;
        let near = self.m[p].adj.iter().map(|a| a.0).chain(self.list.iter().copied().filter(|&q| q != r && q != p));
        !near.into_iter().any(|q| overlap(rr, self.m[q].r))
    }

    /// B4 AdjLink.
    fn link(&mut self, a: usize, b: usize, d: u8) {
        if !self.m[a].adj.iter().any(|x| x.0 == b) {
            self.m[a].adj.push((b, d));
        }
        if !self.m[b].adj.iter().any(|x| x.0 == a) {
            self.m[b].adj.push((a, (d + 2) & 3));
        }
    }

    /// B5 DefFromExits.
    fn defs(&mut self, i: usize) -> Res<()> {
        let mask = self.m[i].adj.iter().fold(0, |m, a| m | [1, 8, 2, 4].get(usize::from(a.1)).copied().unwrap_or(0));
        let corner = |t: [i32; 4]| match mask {
            5 => t[0],
            6 => t[1],
            9 => t[2],
            10 => t[3],
            _ => 0,
        };
        let mut idx = -1;
        let mut def = match self.lt {
            33 => if self.rooms_n == 1 { mask } else { 0x3EA + mask },
            14 => corner([356, 355, 357, 354]),
            15 => corner([360, 359, 361, 358]),
            23 => corner([659, 660, 661, 662]),
            32 => corner([1045, 1044, 1043, 1042]),
            35 => match mask {
                8 => 1053,
                4 => 1054,
                2 => 1055,
                1 => 1056,
                3 => 1057,
                12 => 1058,
                _ => 0,
            },
            t => BASES.iter().find(|b| b.0 == t).map(|b| b.1 + mask).ok_or_else(|| self.fatal())?,
        };
        match (self.lt, self.lid, def) {
            (15, 52, 361) => idx = 2,
            (15, 54, 361 | 359) => idx = 3,
            (23, 84, 662) => def = 664,
            (23, 85, 661) => def = 663,
            _ => {}
        }
        if def != 0 {
            let m = &mut self.m[i];
            (m.def, m.idx, m.lock) = (def, idx, false);
        }
        Ok(())
    }

    /// B6 Merge.
    fn merge(&mut self, r: usize) -> Res<()> {
        if self.m[r].lock {
            return Ok(());
        }
        for p in self.list.clone().into_iter().rev() {
            let (gx, gy) = gap(self.m[r].r, self.m[p].r);
            if self.m[p].lock || gx >= 1 || gy >= 1 || gx == gy || self.m[r].adj.iter().any(|a| a.0 == p) {
                continue;
            }
            if (self.m[p].seed.step() % 1000) as i32 >= self.merge {
                continue;
            }
            let (a, b) = (self.m[p].r, self.m[r].r);
            let d = if a.x == b.x + b.w { 0 } else if b.x == a.x + a.w { 2 } else if a.y == b.y + b.h { 1 } else if b.y == a.y + a.h { 3 } else { 9 };
            if d != 9 {
                self.link(p, r, d);
                self.defs(p)?;
            }
        }
        Ok(())
    }

    /// B7 Grow.
    fn grow(&mut self, p: usize, d: u8) -> Res<Option<usize>> {
        let r = self.create();
        if !self.place(r, p, d) {
            self.free(r);
            return Ok(None);
        }
        self.link(p, r, d);
        self.merge(r)?;
        self.list.push(r);
        self.defs(p)?;
        self.defs(r)?;
        Ok(Some(r))
    }

    /// A grow whose failure is fatal (the next room's parent would be null).
    fn grow_must(&mut self, p: usize, d: u8) -> Res<usize> {
        self.grow(p, d)?.ok_or_else(|| self.fatal())
    }

    /// B8 AddFixed.
    fn add_fixed(&mut self, p: usize, d: u8, def: i32, idx: i32, recalc: bool) -> Res<Option<usize>> {
        let r = self.create();
        if !self.place(r, p, d) {
            self.free(r);
            return Ok(None);
        }
        self.link(p, r, d);
        self.list.push(r);
        if recalc {
            self.defs(p)?;
        }
        self.m[r].lock = true;
        (self.m[r].def, self.m[r].idx) = (def, idx);
        Ok(Some(r))
    }

    /// B9 Probe.
    fn probe(&mut self, r: usize, d: u8) -> Res<bool> {
        if self.m[r].lock || self.m[r].adj.iter().any(|a| a.1 == d) {
            return Ok(false);
        }
        let t = self.create();
        if !self.place(t, r, d) {
            self.free(t);
            return Ok(false);
        }
        self.link(r, t, d);
        self.list.push(t);
        self.defs(t)?;
        self.free(t);
        Ok(true)
    }

    /// B10 Find.
    fn find(&mut self, d: u8) -> Res<usize> {
        let key = |m: &M| [m.r.x, m.r.y, -m.r.x, -m.r.y][usize::from(d)];
        let mut best: Option<usize> = None;
        for i in self.list.clone().into_iter().rev() {
            if best.is_none_or(|b| key(&self.m[i]) < key(&self.m[b])) && self.probe(i, d)? {
                best = Some(i);
            }
        }
        best.ok_or_else(|| self.fatal())
    }

    /// C1 MazeGrow.
    fn growth(&mut self) -> Res<()> {
        let target = self.rooms_n * if self.lid == self.tombs.0 { 3 } else if self.lid == self.tombs.1 { 2 } else { 1 };
        while (self.list.len() as i32) < target {
            let k = self.seed.range(self.list.len() as i32) as usize;
            let p = self.list[self.list.len() - 1 - k];
            let d = (self.m[p].seed.step() & 3) as u8;
            if !self.m[p].lock {
                self.grow(p, d)?;
            }
        }
        Ok(())
    }

    /// D1, D2 SpecialPlace and Attach for one entry `(matchDef, def, dir)`.
    fn special(&mut self, (want, def, dir): (i32, i32, u8), rotate: bool) -> Res<()> {
        let hit = self.list.iter().rev().copied().find(|&i| !self.m[i].lock && self.m[i].def == want);
        if let Some(i) = hit {
            (self.m[i].lock, self.m[i].def, self.m[i].idx) = (true, def, -1);
        } else {
            for p in self.list.clone().into_iter().rev() {
                if self.m[p].lock {
                    continue;
                }
                let r = self.create();
                if self.place(r, p, dir) {
                    self.link(p, r, dir);
                    self.list.push(r);
                    self.defs(p)?;
                    (self.m[r].lock, self.m[r].def, self.m[r].idx) = (true, def, -1);
                    break;
                }
            }
        }
        if rotate {
            self.rot = (self.rot + 1) & 3;
        }
        Ok(())
    }

    /// D: the table entry at the current rotation, for block base `m`, first record `b`; `exc` for the three exception tables.
    fn sp(&mut self, m: i32, b: i32, exc: bool) -> Res<()> {
        let r = self.rot;
        let off = if exc { [0, 2, 1, 3][r] } else { [3, 1, 2, 0][r] };
        self.special((m + [8, 2, 4, 1][r], b + off, [3, 0, 1, 2][r]), true)
    }

    /// F1: the finisher draw.
    fn draw(&mut self) {
        self.rot = (self.seed.step() & 3) as usize;
    }

    /// E1 Ring.
    fn ring(&mut self, n: usize) -> Res<()> {
        let (r0, mut cur) = (self.list[0], self.list[0]);
        for (d, cnt) in [(1, n - 1), (0, n - 1), (3, n - 1), (2, n - 2)] {
            for _ in 0..cnt {
                cur = self.grow_must(cur, d)?;
            }
        }
        self.link(cur, r0, 2);
        self.defs(cur)?;
        self.defs(r0)
    }

    /// E2 Tee.
    fn tee(&mut self, prev: [i32; 4]) -> Res<()> {
        let r0 = self.list[0];
        let mut r = (self.seed.step() & 3) as u8;
        for _ in 0..3 {
            self.grow(r0, r)?;
            r = (r + 1) & 3;
        }
        (self.m[r0].def, self.m[r0].lock, self.m[r0].idx) = (prev[usize::from(r)], true, -1);
        Ok(())
    }

    /// E3 CatacombsStart.
    fn catacombs(&mut self) -> Res<()> {
        let r0 = self.list[0];
        let def = if self.lid == 34 {
            [1, 2, 3, 0].map(|d| self.grow(r0, d)).into_iter().collect::<Res<Vec<_>>>()?;
            290
        } else if self.seed.step() & 1 == 0 {
            self.grow(r0, 1)?;
            self.grow(r0, 3)?;
            289
        } else {
            self.grow(r0, 0)?;
            self.grow(r0, 2)?;
            288
        };
        (self.m[r0].def, self.m[r0].lock, self.m[r0].idx) = (def, true, -1);
        Ok(())
    }

    /// E4 Arcane.
    fn arcane(&mut self) -> Res<()> {
        let r = (self.seed.step() & 3) as i32;
        let (r0, mut arms) = (self.list[0], vec![]);
        for a in 0..4u8 {
            let (a1, a2, a3) = ((a + 1) & 3, (a + 2) & 3, (a + 3) & 3);
            let (mut cur, mut s) = (r0, vec![]);
            for d in [a, a, a3, a, a, a, a, a1] {
                cur = self.grow_must(cur, d)?;
                s.push(cur);
            }
            self.grow(cur, a)?;
            for d in [a1, a2, a2] {
                cur = self.grow_must(cur, d)?;
                s.push(cur);
            }
            self.grow(cur, a3)?;
            for d in [a2, a2] {
                cur = self.grow_must(cur, d)?;
                s.push(cur);
            }
            arms.push(s);
        }
        // Set after all four arms: a later `DefFromExits` would reset the index.
        for (a, s) in arms.iter().enumerate() {
            s.iter().for_each(|&i| self.m[i].idx = (a as i32 + r) & 3);
        }
        self.m[r0].idx = 4;
        Ok(())
    }

    /// E5 HellStart.
    fn hell(&mut self) -> Res<()> {
        let r0 = self.list[0];
        let e = ((self.m[r0].seed.step() & 3) * 2) as usize;
        for (def, dir, idx) in HELL[e..e + 2].iter().copied() {
            self.add_fixed(r0, dir, def, idx, true)?;
        }
        self.fill_around(836, None)
    }

    /// E6 FillAround.
    fn fill_around(&mut self, def: i32, exclude: Option<usize>) -> Res<()> {
        for p in self.list.clone().into_iter().rev().filter(|&p| Some(p) != exclude) {
            for d in 0..8 {
                let r = self.create();
                if self.place(r, p, d) {
                    self.link(p, r, d);
                    self.list.push(r);
                    (self.m[r].lock, self.m[r].def, self.m[r].idx) = (true, def, -1);
                } else {
                    self.free(r);
                }
            }
        }
        Ok(())
    }

    /// F2 Sewers A2 finisher.
    fn sewer_a2(&mut self) -> Res<()> {
        let a = (((self.seed.step() & 1) * 2) | 1) as u8;
        self.rot = (self.seed.step() & 3) as usize;
        match self.lid {
            47 => {
                let p = self.find(1)?;
                let r1 = self.grow_must(p, 1)?;
                let r2 = self.grow_must(r1, 1)?;
                self.add_fixed(r2, 0, 333, 0, true)?;
                let p = self.find(2)?;
                let r1 = self.grow_must(p, 2)?;
                let r2 = self.grow_must(r1, 2)?;
                let dock = self.add_fixed(r2, a, 336, 0, true)?.ok_or_else(|| self.fatal())?;
                let r = self.create();
                if self.place(r, dock, a) {
                    self.link(dock, r, a);
                    self.list.push(r);
                    self.defs(r)?;
                } else {
                    self.free(r);
                }
                self.sp(301, 337, false)
            }
            48 => [332, 345, 337].iter().try_for_each(|&b| self.sp(301, b, false)),
            49 => [332, 341].iter().try_for_each(|&b| self.sp(301, b, false)),
            _ => [332, 349].iter().try_for_each(|&b| self.sp(301, b, false)),
        }
    }

    /// F3 Tomb finisher.
    fn tomb(&mut self) -> Res<()> {
        let p = self.list.iter().rev().find(|&&i| self.m[i].def > 428).map(|&i| self.m[i].def);
        self.rot = match p {
            Some(444) => 3,
            Some(445) => 1,
            Some(446) => 0,
            Some(447) => 2,
            _ => return Err(self.fatal()),
        };
        let id = self.lid;
        let mut picks = vec![];
        if (55..=58).contains(&id) {
            picks.push(448);
        }
        if id == 57 {
            picks.push(476);
        }
        if id == 59 || id == 61 {
            picks.push(472);
        }
        if (66..=72).contains(&id) && id != self.tombs.0 {
            picks.push(472);
        }
        match id {
            59 => picks.extend([464, 452]),
            60 => picks.push(456),
            61 => picks.push(452),
            _ => {}
        }
        if id == self.tombs.0 {
            picks.push(460);
        }
        if id == self.tombs.1 {
            picks.push(468);
        }
        picks.into_iter().try_for_each(|b| self.sp(413, b, false))
    }

    /// F4 Level 92 corners.
    fn corners(&mut self) -> Res<()> {
        for (want, new) in [(0x2C5, 735), (0x2C6, 736), (0x2C9, 737), (0x2CA, 738)] {
            let i = self.list.iter().rev().copied().find(|&i| !self.m[i].lock && self.m[i].def == want).ok_or_else(|| self.fatal())?;
            (self.m[i].lock, self.m[i].def, self.m[i].idx) = (true, new, -1);
        }
        Ok(())
    }

    /// Translate every room.
    fn shift(&mut self, (dx, dy): (i32, i32)) {
        for &i in &self.list {
            self.m[i].r.x += dx;
            self.m[i].r.y += dy;
        }
    }

    /// G2 MazeBarracksPlace: the Court Connect against level 27's rect `q` on the side its file index `k` names, the Next and Forge specials, the shift.
    fn barracks(&mut self, q: Rect, k: usize) -> Res<()> {
        let d = *[2u8, 3, 0].get(k).ok_or_else(|| self.fatal())?;
        let p = self.find(d)?;
        let c = self.add_fixed(p, d, 167, k as i32, true)?.ok_or_else(|| self.fatal())?;
        let (c, (sx, sy)) = (self.m[c].r, self.size);
        let off = match k {
            0 => (q.x - c.x - sx, q.y + q.h / 2 - c.y),
            1 => (q.x + q.w / 2 - c.x - 6, q.y - c.y - sy),
            _ => (q.x + q.w - c.x, q.y + q.h / 2 - c.y + 1),
        };
        self.rot = k;
        let order = if self.seed.step() & 1 != 0 { [198, 202] } else { [202, 198] };
        order.iter().try_for_each(|&b| self.sp(167, b, false))?;
        self.shift(off);
        Ok(())
    }

    /// G3 MazeRiverOfFlame: Lava Warp, two bridges, one forge, the fillers, the shift below level 108's rect `q`.
    fn river(&mut self, q: Rect) -> Res<()> {
        let p = self.find(3)?;
        self.add_fixed(p, 3, 852, -1, true)?;
        let p = self.find(1)?;
        let mut b = self.add_fixed(p, 1, 855, -1, true)?.ok_or_else(|| self.fatal())?;
        for _ in 0..2 {
            let r = self.create();
            if !self.place(r, b, 1) {
                return Err(self.fatal());
            }
            self.link(b, r, 1);
            self.list.push(r);
            (self.m[r].lock, self.m[r].def, self.m[r].idx) = (true, 856, -1);
            b = r;
        }
        let r = (self.seed.step() & 1) as usize;
        self.special([(837, 853, 2), (838, 854, 0)][r], false)?;
        self.fill_around(836, Some(b))?;
        let br = self.m[b].r;
        self.shift((q.x + 2 * br.w - br.x, q.y + q.h - br.y));
        Ok(())
    }

    /// H1 MazeThemes.
    fn themes(&mut self) {
        let Some(&(_, base)) = BASES.iter().find(|b| b.0 == self.lt) else { return };
        if !THEMED.contains(&self.lt) || self.lid == 8 {
            return;
        }
        let mut s = (self.seed.step() % 15) as usize;
        let n = self.list.len() as i32;
        let (mut want, mut tries) = (2.max(n / 5 + 1), 2 * n);
        let mut perm: [usize; 15] = std::array::from_fn(|i| i);
        for _ in 0..15 {
            let a = (self.seed.step() % 15) as usize;
            let b = (self.seed.step() % 15) as usize;
            perm.swap(a, b);
        }
        while want > 0 && tries > 0 {
            let v = base + perm[s] as i32;
            if let Some(i) = self.list.iter().rev().copied().find(|&i| !self.m[i].lock && self.m[i].def == v) {
                (self.m[i].lock, self.m[i].def, self.m[i].idx) = (true, v + 15, -1);
                want -= 1;
            }
            tries -= 1;
            s = (s + 1) % 15;
        }
    }

    /// F: the chain of the level type (start shape, growth, finisher).
    fn chain(&mut self) -> Res<()> {
        let (id, r0) = (self.lid, self.list[0]);
        match self.lt {
            3 => {
                self.growth()?;
                self.draw();
                self.sp(52, 83, false)?;
                self.sp(52, if id == 8 { 95 } else { 91 }, false)?;
                if id == 9 {
                    self.sp(52, 99, false)?;
                }
                if id == 10 {
                    self.sp(52, 87, false)?;
                }
            }
            4 => {
                self.growth()?;
                self.draw();
                self.sp(108, 139, false)?;
                match id {
                    18 => self.sp(108, 147, false)?,
                    19 | 133 => self.sp(108, 151, false)?,
                    21..=24 => self.sp(108, 143, false)?,
                    _ => {}
                }
            }
            7 => {
                self.ring(2)?;
                self.growth()?;
            }
            28 => self.growth()?,
            8 => {
                self.ring(2)?;
                self.growth()?;
                self.draw();
                self.sp(205, 236, false)?;
                match id {
                    29 => self.sp(205, 248, false)?,
                    30 => self.sp(205, 252, false)?,
                    _ => {}
                }
                self.sp(205, if id == 31 { 244 } else { 240 }, false)?;
            }
            10 => {
                self.catacombs()?;
                self.growth()?;
                self.draw();
                self.sp(257, 291, false)?;
                if id == 35 {
                    self.sp(257, 295, false)?;
                }
            }
            13 => {
                self.ring(2)?;
                self.growth()?;
                self.sewer_a2()?;
            }
            14 | 15 | 23 => self.ring(2)?,
            17 if id == 61 => (self.m[r0].lock, self.m[r0].def, self.m[r0].idx) = (true, 480, -1),
            17 => {
                self.tee([447, 444, 446, 445])?;
                self.growth()?;
                self.tomb()?;
            }
            18 => {
                self.ring(2)?;
                self.growth()?;
                self.draw();
                if id == 64 {
                    self.special((485, 509, 1), true)?;
                    self.rot = 3;
                    self.special((482, 505, 2), true)?;
                } else {
                    self.sp(481, 501, false)?;
                }
                self.sp(481, 497, false)?;
            }
            19 => {
                self.arcane()?;
                self.draw();
                self.sp(509, 525, false)?;
            }
            22 | 24 | 25 => {
                self.ring(if self.lt == 25 && id == 92 { 5 } else { 2 })?;
                if id == 92 {
                    self.corners()?;
                }
                self.growth()?;
                self.draw();
                let picks: &[i32] = match (self.lt, id) {
                    (22, 101) => &[784, 792, 788],
                    (22, 100) => &[784, 788],
                    (22, _) => &[784],
                    (24, _) => &[695, 699],
                    _ => &[739, 743],
                };
                let m = [(22, 753), (24, 664), (25, 704)].iter().find(|x| x.0 == self.lt).map_or(0, |x| x.1);
                picks.iter().try_for_each(|&b| self.sp(m, b, false))?;
            }
            32 => {
                self.ring(2)?;
                self.rot = (self.seed.step() % 3) as usize;
                if id != 124 {
                    self.special(TEMPLE_DOWN[self.rot], true)?;
                }
                if id == 123 {
                    self.special(TEMPLE_WAY[self.rot], true)?;
                }
            }
            33 if [114, 116, 119].contains(&id) => {
                let def = match id {
                    114 => if self.seed.range(2) != 0 { 1038 } else { 1039 },
                    116 => 1040,
                    _ => 1041,
                };
                (self.m[r0].lock, self.m[r0].def, self.m[r0].idx) = (true, def, -1);
            }
            33 => {
                self.ring(2)?;
                self.growth()?;
                self.draw();
                for b in [1018, 1022, 1026] {
                    self.sp(1002, b, false)?;
                }
                if id == 115 {
                    self.sp(1002, 1030, false)?;
                }
                if [113, 115, 118].contains(&id) {
                    self.sp(1002, 1034, true)?;
                }
            }
            34 => {
                self.tee([1075, 1077, 1076, 1074])?;
                self.growth()?;
                self.draw();
                self.sp(1058, 1078, true)?;
                if id == 129 {
                    self.sp(1058, 1082, true)?;
                }
            }
            35 => self.hell()?,
            _ => return Err(MapError::Unsupported(id, "maze level type")),
        }
        Ok(())
    }
}

/// The rect enclosing every room.
fn bounds(rs: impl Iterator<Item = Rect>) -> Rect {
    let rs: Vec<Rect> = rs.collect();
    let (x0, y0) = (rs.iter().map(|r| r.x).min().unwrap_or(0), rs.iter().map(|r| r.y).min().unwrap_or(0));
    Rect { x: x0, y: y0, w: rs.iter().map(|r| r.x + r.w).max().unwrap_or(0) - x0, h: rs.iter().map(|r| r.y + r.h).max().unwrap_or(0) - y0 }
}

/// A1 MazeLevelBuild with G1 placement; levels 28 and 107 place against a neighbour: `nb` is the rect of level 27 with its file index, or of level 108.
pub fn build(md: &MapData, level_id: u32, lrect: Rect, seed: Seed, diff: u8, tombs: (u32, u32), nb: Option<(Rect, usize)>) -> Result<Level, MapError> {
    let lt = md.t.levels.iter().find(|l| l.id as u32 == level_id).map_or(0, |l| l.level_type);
    let mz = md.t.maze.iter().find(|m| m.level as u32 == level_id).ok_or_else(|| MapError::Data(format!("no LvlMaze for level {level_id}")))?;
    let mut g = G { lid: level_id, lt, rooms_n: mz.rooms[usize::from(diff.min(2))], merge: mz.merge, size: (mz.size_x, mz.size_y), seed, m: vec![], list: vec![], rot: 0, tombs };
    let r0 = g.create();
    g.m[r0].r.x = lrect.x + (lrect.w - mz.size_x) / 2;
    g.m[r0].r.y = lrect.y + (lrect.h - mz.size_y) / 2;
    g.list.push(r0);
    g.chain()?;
    let lrect = match (level_id, nb) {
        (28 | 107, None) => return Err(MapError::Unsupported(level_id, "neighbour level rect from act links")),
        (28, Some((q, k))) => g.barracks(q, k).map(|()| None)?,
        (107, Some((q, _))) => g.river(q).map(|()| None)?,
        _ => Some(lrect),
    };
    let bb = bounds(g.list.iter().map(|&i| g.m[i].r));
    // G1 MazeLevelPlace: the bounding box moves to the level origin.
    let lrect = match lrect {
        Some(l) => {
            if l.w < bb.w || l.h < bb.h {
                return Err(MapError::Unsupported(level_id, "MazeLevelPlace fatal"));
            }
            g.shift((l.x - bb.x, l.y - bb.y));
            l
        }
        None => bb,
    };
    g.themes();
    let rooms = g.list.iter().map(|&i| Room { maze: Some((g.m[i].def, g.m[i].idx)), ..Room::new(g.m[i].r, g.m[i].born) }).collect();
    Ok(Level { id: level_id, rect: lrect, kind: Kind::Maze, rooms, parts: vec![], warps: vec![], seed_after: g.seed, roads: vec![] })
}

/// README 9b `MAZE_DEF_BASE`: the types whose rooms rotate their file index.
const ROT_BASES: [(i32, i32); 15] = [(3, 0x34), (4, 0x6C), (7, 0xA7), (8, 0xCD), (10, 0x101), (13, 0x12D), (17, 0x19D), (18, 0x1E1), (22, 0x2F1), (24, 0x298), (25, 0x2C0), (28, 0x344), (33, 0x3EA), (34, 0x422), (35, 0x41C)];

/// README 9a and 9b: every maze room, newest first, becomes a preset instance (8 x 8 rooms, or one room up to 12 x 12) on the level seed after the window.
pub fn parts(md: &MapData, lvl: &Level) -> Result<Vec<Room>, MapError> {
    let lt = md.t.levels.iter().find(|l| l.id == lvl.id as i32).map_or(0, |l| l.level_type);
    let (mut s, mut rot, mut out) = (lvl.seed_after, Vec::<(i32, i32, i32)>::new(), vec![]);
    for m in lvl.rooms.iter().rev() {
        let (def, forced) = m.maze.ok_or(MapError::Data("maze room without a def".into()))?;
        let p = md.t.prest.iter().find(|p| p.def == def).ok_or_else(|| MapError::Data(format!("level {} maze def {def}", lvl.id)))?;
        let mut idx = preset::file_draw(p, &mut s) as i32;
        // 9b MazePresetFile.
        match ROT_BASES.iter().find(|b| b.0 == lt) {
            _ if forced != -1 => idx = forced,
            Some(&(_, b)) if b < def && def < b + 16 => {
                let n = match rot.iter().position(|n| n.0 == def) {
                    Some(n) => n,
                    None => {
                        rot.push((def, p.files, s.range(p.files) as i32));
                        rot.len() - 1
                    }
                };
                rot[n].2 = (rot[n].2 + 1) % rot[n].1.max(1);
                idx = rot[n].2;
            }
            _ => {}
        }
        out.extend(preset::rooms(md, lvl.id, p, m.rect, idx as u32, 0, m.rect.w <= 12 && m.rect.h <= 12, &mut s)?);
    }
    Ok(out)
}
