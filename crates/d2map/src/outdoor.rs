//! Outdoor levels: the cell grid, border pass, placements and the act layouts (specs/drlg/outdoor.md).
use crate::drlg::{Kind, Level, MapData, MapError, Rect, Room};
use crate::{links, preset, seeds::room_seed, Seed, SeedTree};
use d2data::{ds1::Ds1, excel::PrestDef};
use std::collections::HashMap;

type Res<T> = Result<T, MapError>;

struct Adj {
    nbr: u32,
    side: u8,
    preset: bool,
    rect: Rect,
}

#[derive(Clone, Copy)]
struct V {
    x: i32,
    y: i32,
    ty: u8,
    fl: u8,
    /// 0..=3 for the rectangle's own corners, -1 for inserted vertices.
    orig: i8,
}

/// E2a: border `Def` rows 0..=12 by style 0..=5 (cliff, wild, desert, mesa, Barricade, Barricade snow); row `1 + di` is an edge, row `k` a corner.
const DEFS: [[i32; 6]; 13] = [
    [0; 6],
    [0, 4, 364, 799, 881, 957],
    [16, 5, 365, 800, 882, 958],
    [17, 6, 366, 801, 883, 959],
    [0, 7, 367, 802, 884, 960],
    [18, 8, 368, 803, 885, 961],
    [19, 9, 369, 804, 886, 962],
    [22, 10, 370, 805, 887, 963],
    [0, 11, 371, 806, 888, 964],
    [0, 12, 372, 807, 889, 965],
    [23, 13, 373, 808, 890, 966],
    [0, 14, 374, 809, 891, 967],
    [0, 15, 375, 810, 892, 968],
];
/// E3 `g_BorderCornerIdx`: rows are the out code -4..=4, columns the in code.
const CORNER: [[i8; 9]; 9] = [
    [1, 9, 9, -1, -1, 1, 8, -1, -1],
    [12, -1, -1, -1, -1, 12, 4, -1, -1],
    [5, 2, 2, 10, -1, -1, -1, -1, 10],
    [1, 9, 9, -1, -1, -1, -1, -1, -1],
    [-1; 9],
    [-1, -1, -1, -1, -1, -1, 11, 11, 3],
    [12, -1, -1, -1, -1, 12, 4, 4, 7],
    [-1, -1, 2, 10, -1, -1, -1, -1, 10],
    [-1, -1, 6, 3, -1, -1, 11, 11, 3],
];
/// E2 `g_A2SeamPairDefs` by `dx + 2dy + 2`.
const A2SEAM: [(i32, i32); 5] = [(373, 372), (372, 375), (0, 0), (373, 374), (374, 375)];
const NBR: [(i32, i32); 8] = [(-1, 0), (0, -1), (0, 1), (1, 0), (-1, -1), (1, 1), (1, -1), (-1, 1)];
/// H3 `g_A1RiverIdx` for border `Def` 4..=15: (River Upper, River Lower).
const RIVER: [(i32, i32); 12] = [(2, 2), (0, 3), (1, 1), (3, 0), (0, 2), (0, 1), (1, 0), (2, 0), (2, 3), (1, 3), (3, 1), (3, 2)];
/// E1 `g_OutSideProbe` by side.
const PROBE: [(i32, i32); 4] = [(-4, 4), (4, -4), (12, 4), (4, 12)];

/// The cell grids, the file rotation and the seed of one outdoor window (D, B).
pub struct Out {
    pub id: u32,
    pub rect: Rect,
    pub flags: u32,
    pub cw: i32,
    pub ch: i32,
    pub def: Vec<i32>,
    pub g0: Vec<u32>,
    pub cell: Vec<u32>,
    pub seed: Seed,
    /// H6e: the Act I road polylines in map tiles (hub end first), for the road tiles of M2.
    pub roads: Vec<Vec<(i32, i32)>>,
    rot: Vec<(i32, i32, i32)>,
    /// `Def` to (cells wide, cells high, `Files`).
    pre: HashMap<i32, (i32, i32, i32)>,
    adj: Vec<Adj>,
    ring: Vec<V>,
    vis: [i32; 8],
    lt: i32,
    act: u8,
    /// Level 1's rect (PlaceFar of the Den of Evil entrance) .
    lv1: Rect,
}

fn fatal(o: &Out, what: &'static str) -> MapError {
    MapError::Unsupported(o.id, what)
}

impl Out {
    /// A zeroed grid; `pre` maps `Def` to `(SizeX, SizeY, Files)` of `LvlPrest`.
    pub fn new(cw: i32, ch: i32, seed: Seed, pre: &[PrestDef]) -> Out {
        let n = (cw.max(0) * ch.max(0)) as usize;
        let pre = pre.iter().map(|p| (p.def, (p.size_x / 8, p.size_y / 8, p.files))).collect();
        let nil = Rect { x: 0, y: 0, w: 0, h: 0 };
        Out { id: 0, rect: nil, flags: 0, cw, ch, def: vec![0; n], g0: vec![0; n], cell: vec![0; n], seed, roads: vec![], rot: vec![], pre, adj: vec![], ring: vec![], vis: [0; 8], lt: 0, act: 0, lv1: nil }
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        ((0..self.cw).contains(&x) && (0..self.ch).contains(&y)).then(|| (y * self.cw + x) as usize)
    }
    pub fn cell_at(&self, x: i32, y: i32) -> u32 {
        self.idx(x, y).map_or(0, |i| self.cell[i])
    }
    pub fn def_at(&self, x: i32, y: i32) -> i32 {
        self.idx(x, y).map_or(0, |i| self.def[i])
    }
    pub fn g0_at(&self, x: i32, y: i32) -> u32 {
        self.idx(x, y).map_or(0, |i| self.g0[i])
    }
    /// Marks a cell for tests of the road-driven placements.
    pub fn or_cell_for_test(&mut self, x: i32, y: i32, b: u32) {
        self.or_cell(x, y, b);
    }
    fn or_cell(&mut self, x: i32, y: i32, b: u32) {
        if let Some(i) = self.idx(x, y) {
            self.cell[i] |= b;
        }
    }
    fn or_g0(&mut self, x: i32, y: i32, b: u32) {
        if let Some(i) = self.idx(x, y) {
            self.g0[i] |= b;
        }
    }
    fn set_cell(&mut self, x: i32, y: i32, v: u32) {
        if let Some(i) = self.idx(x, y) {
            self.cell[i] = v;
        }
    }
    fn free(&self, x: i32, y: i32) -> bool {
        self.idx(x, y).is_some() && self.cell_at(x, y) & 0x1B81 == 0
    }

    /// D2 OutPresetFileRotate.
    fn rotate(&mut self, def: i32) -> i32 {
        let i = self.rot.iter().position(|r| r.0 == def).unwrap_or_else(|| {
            let files = self.pre.get(&def).map_or(0, |p| p.2);
            let c = self.seed.range(files) as i32;
            self.rot.push((def, files, c));
            self.rot.len() - 1
        });
        let r = &mut self.rot[i];
        r.2 = if r.1 > 0 { (r.2 + 1) % r.1 } else { 0 };
        r.2
    }

    /// D1 OutPresetPlace (`file` -1: from the rotation).
    pub fn place_b(&mut self, x: i32, y: i32, def: i32, file: i32, border: bool) {
        let (w, h, _) = self.pre.get(&def).copied().unwrap_or_default();
        let file = if file == -1 { self.rotate(def) } else { file } as u32;
        for yy in y..y + h {
            for xx in x..x + w {
                if let Some(i) = self.idx(xx, yy) {
                    self.cell[i] = (self.cell[i] & !0xF0000) | (file << 16) | 0x200 | u32::from(border && ((4..=15).contains(&def) || (364..=375).contains(&def)));
                    self.def[i] = 0;
                }
            }
        }
        if let Some(i) = self.idx(x, y) {
            self.def[i] = def;
        }
    }
    pub fn place(&mut self, x: i32, y: i32, def: i32, file: i32) {
        self.place_b(x, y, def, file, false);
    }

    /// D3 OutPresetFits.
    pub fn fits(&self, x: i32, y: i32, def: i32, pad: i32, sides: i32) -> bool {
        let (w, h, _) = if def == 0 { (1, 1, 0) } else { self.pre.get(&def).copied().unwrap_or_default() };
        let (x0, y0) = (x - if sides & 8 != 0 { pad } else { 0 }, y - if sides & 1 != 0 { pad } else { 0 });
        let (x1, y1) = (x + w + if sides & 2 != 0 { pad } else { 0 }, y + h + if sides & 4 != 0 { pad } else { 0 });
        (y0..y1).all(|yy| (x0..x1).all(|xx| self.free(xx, yy)))
    }

    /// D4 Shuffle: `2N` level draws.
    pub fn shuffle(&mut self, n: i32, w: i32) -> Vec<(i32, i32)> {
        let mut l: Vec<(i32, i32)> = (0..n.max(0)).map(|i| (i % w.max(1), i / w.max(1))).collect();
        for _ in 0..n.max(0) {
            let (a, b) = (self.seed.range(n) as usize, self.seed.range(n) as usize);
            l.swap(a, b);
        }
        l
    }

    /// D5 PlaceRandom.
    pub fn place_random(&mut self, def: i32, file: i32, pad: i32, sides: i32) -> bool {
        let n = (self.cw - 2) * (self.ch - 2);
        if n <= 0 {
            return false;
        }
        for (x, y) in self.shuffle(n, self.cw - 2) {
            if self.fits(x + 1, y + 1, def, pad, sides) {
                self.place(x + 1, y + 1, def, file);
                return true;
            }
        }
        false
    }

    /// D6 PlaceNear.
    pub fn place_near(&mut self, def: i32, file: i32) -> bool {
        let n = (self.cw - 2) * (self.ch - 2);
        if n <= 0 {
            return false;
        }
        for (x, y) in self.shuffle(n, self.cw - 2) {
            if self.cell_at(x + 1, y + 1) & 0x80 != 0 {
                for (dx, dy) in NBR {
                    let (px, py) = (x + 1 + dx, y + 1 + dy);
                    if self.fits(px, py, def, 0, 0xF) {
                        self.place(px, py, def, file);
                        return true;
                    }
                }
            }
        }
        self.place_random(def, file, 0, 0xF)
    }

    /// D7 PlaceFar.
    pub fn place_far(&mut self, rect: Rect, def: i32, file: i32, pad: i32, sides: i32) -> bool {
        let (ox, oy) = (self.seed.range(self.cw - 2) as i32, self.seed.range(self.ch - 2) as i32);
        // The best starts at distance 0, so a cell on the rect centre never wins.
        let mut best = (0, 0, 0);
        for r in 0..=self.ch - 2 {
            let y = (r + oy) % (self.ch - 2).max(1) + 1;
            for c in 0..=self.cw - 2 {
                let x = (c + ox) % (self.cw - 2).max(1) + 1;
                if self.fits(x, y, def, pad, sides) {
                    let a = (8 * x + self.rect.x + 4 - (rect.x + rect.w / 2)).abs();
                    let b = (8 * y + self.rect.y + 4 - (rect.y + rect.h / 2)).abs();
                    let d = (if a > b { b + 2 * a } else { a + 2 * b }) / 2;
                    if d > best.2 {
                        best = (x, y, d);
                    }
                }
            }
        }
        if best.2 > 0 {
            self.place(best.0, best.1, def, file);
        }
        best.2 > 0
    }

    /// D8 OutCellClear / OutCellCover.
    fn clear(&mut self, x: i32, y: i32, v: u32) {
        if let Some(i) = self.idx(x, y) {
            (self.def[i], self.cell[i]) = (0, v);
        }
    }

    /// F1 OutWaypointPlace.
    pub fn waypoint(&mut self) {
        if self.id == 3 {
            let f = self.vis.iter().position(|&v| v == 2).map_or(0, |k| 0x10u32 << k);
            for y in 0..self.ch {
                for x in 0..self.cw {
                    if self.g0_at(x, y) & f != 0 && self.cell_at(x, y) & 0x400 != 0 {
                        let (x, y) = (x.clamp(1, (self.cw - 2).max(1)), y.clamp(1, (self.ch - 2).max(1)));
                        self.or_g0(x, y, 0x20000);
                        self.or_cell(x, y, 0x800);
                        return;
                    }
                }
            }
        }
        let n = (self.cw - 2) * (self.ch - 2);
        if n <= 0 {
            return;
        }
        if let Some((x, y)) = self.shuffle(n, self.cw - 2).into_iter().find(|&(x, y)| self.free(x + 1, y + 1)) {
            self.or_g0(x + 1, y + 1, 0x10000);
            self.or_cell(x + 1, y + 1, 0x800);
        }
    }

    /// F2 OutShrinesPlace (count 5).
    pub fn shrines(&mut self) {
        let mut r = self.seed.step() & 3;
        let n = (self.cw - 2) * (self.ch - 2);
        if n <= 0 {
            return;
        }
        let mut count = 5;
        for (x, y) in self.shuffle(n, self.cw - 2) {
            if count > 0 && self.free(x + 1, y + 1) {
                self.or_g0(x + 1, y + 1, 0x1000 << r);
                self.or_cell(x + 1, y + 1, 0x1000);
                r = (r + 1) & 3;
                count -= 1;
            }
        }
    }

    /// C3 to C5: the vertex ring with a vertex at each seam end, in cells.
    fn build_ring(&mut self) -> Res<()> {
        let r = Rect { w: self.rect.w - 1, h: self.rect.h - 1, ..self.rect };
        let v = |x, y, orig| V { x, y, ty: 0, fl: 0, orig };
        let mut ring = vec![v(r.x, r.y + r.h, 0), v(r.x, r.y, 1), v(r.x + r.w, r.y, 2), v(r.x + r.w, r.y + r.h, 3)];
        for n in &self.adj {
            let q = Rect { w: n.rect.w - 1, h: n.rect.h - 1, ..n.rect };
            let side = n.side as usize;
            let pos = |ring: &[V], o: usize| ring.iter().position(|w| w.orig == o as i8).unwrap_or(0);
            let (s, e) = (ring[pos(&ring, side)], ring[pos(&ring, (side + 1) % 4)]);
            let ((sv, ev), c, a, b) = match side {
                0 => ((s.y, e.y), -1, q.y + q.h, q.y),
                1 => ((s.x, e.x), 1, q.x, q.x + q.w),
                2 => ((s.y, e.y), 1, q.y, q.y + q.h),
                _ => ((s.x, e.x), -1, q.x + q.w, q.x),
            };
            let mut p = pos(&ring, side);
            let at = |p: usize, ring: &[V], val: i32| if side.is_multiple_of(2) { v(ring[p].x, val, -1) } else { v(val, ring[p].y, -1) };
            if a * c <= sv * c {
                if sv * c > b * c {
                    continue;
                }
            } else {
                if a * c > ev * c {
                    continue;
                }
                ring.insert(p + 1, at(p, &ring, a));
                p += 1;
            }
            ring[p].fl |= if n.preset { 3 } else { 1 };
            if b * c < ev * c {
                ring.insert(p + 1, at(p, &ring, b));
            }
        }
        for w in &mut ring {
            (w.x, w.y) = ((w.x - self.rect.x) / 8, (w.y - self.rect.y) / 8);
        }
        if (0..ring.len()).any(|i| (ring[i].x, ring[i].y) == (ring[(i + 1) % ring.len()].x, ring[(i + 1) % ring.len()].y)) {
            return Err(fatal(self, "vertex merge"));
        }
        self.ring = ring;
        Ok(())
    }

    fn segs(&self) -> Vec<(V, V)> {
        (0..self.ring.len()).map(|i| (self.ring[i], self.ring[(i + 1) % self.ring.len()])).collect()
    }

    /// E1 SeamWarpFlag.
    fn seam_flag(&self, v: V) -> u32 {
        let (x, y) = (v.x, v.y);
        let side = if x == 0 {
            usize::from(y == 0)
        } else if y == 0 {
            1 + usize::from(x == self.cw - 1)
        } else if x == self.cw - 1 {
            2 + usize::from(y == self.ch - 1)
        } else if y == self.ch - 1 {
            3
        } else {
            return 0;
        };
        let (px, py) = (8 * x + self.rect.x + PROBE[side].0, 8 * y + self.rect.y + PROBE[side].1);
        let n = self.adj.iter().find(|n| n.side as usize == side && (n.rect.x..n.rect.x + n.rect.w).contains(&px) && (n.rect.y..n.rect.y + n.rect.h).contains(&py));
        n.and_then(|n| self.vis.iter().position(|&k| k == n.nbr as i32)).map_or(0, |k| 0x10 << k)
    }

    /// E1 OutSeamMark.
    fn seam_mark(&mut self) {
        for (v, n) in self.segs() {
            if v.fl & 1 != 0 {
                let f = self.seam_flag(v);
                let (dx, dy) = ((n.x - v.x).signum(), (n.y - v.y).signum());
                let len = (n.x - v.x).abs().max((n.y - v.y).abs());
                for k in 0..=len {
                    self.or_g0(v.x + k * dx, v.y + k * dy, f);
                    self.or_cell(v.x + k * dx, v.y + k * dy, if v.ty != 0 { 3 } else { 1 });
                }
            }
        }
    }

    fn style(&self, t: u8) -> usize {
        match self.lt {
            2 => usize::from(t == 0),
            16 => 2,
            27 => 3,
            31 => 4 + usize::from(self.id == 117),
            _ => 9,
        }
    }

    /// E3 BorderCornerDef: the `Def` of the corner piece, 0 for none.
    fn corner_def(&self, in_code: i32, out_code: i32, style: usize) -> i32 {
        let k = CORNER[(out_code + 4) as usize][(in_code + 4) as usize];
        if k < 0 { 0 } else { DEFS[k as usize][style] }
    }

    /// E2 OutBorderBuild.
    fn border_build(&mut self) -> Res<()> {
        if self.style(0) > 5 {
            return Err(fatal(self, "border style of level type"));
        }
        let segs = self.segs();
        let code = |p: V, d: (i32, i32)| {
            let (a, b) = if p.fl & 2 != 0 { d } else { (2 * d.0, 2 * d.1) };
            let e = |z: i32| z + 2 * z.signum();
            e(a) + b
        };
        for i in 0..segs.len() {
            let (v, n) = segs[i];
            let nn = segs[(i + 1) % segs.len()].1;
            let d = ((n.x - v.x).signum(), (n.y - v.y).signum());
            let len = (n.x - v.x).abs().max((n.y - v.y).abs());
            let mut cv = if v.ty != 0 { 3 } else { 1 };
            let di = match d {
                (-1, 0) => 0,
                (0, -1) => 1,
                (1, 0) => 2,
                _ => 3,
            };
            let e = DEFS[1 + di][self.style(v.ty)];
            if v.fl & 2 == 0 {
                for k in 1..=len {
                    self.place(v.x + k * d.0, v.y + k * d.1, e, -1);
                    self.or_cell(v.x + k * d.0, v.y + k * d.1, cv);
                }
            }
            if v.fl & 1 != 0 && v.fl & 2 == 0 {
                let (mx, my) = (v.x.min(n.x) + len * d.0.abs() / 2, v.y.min(n.y) + len * d.1.abs() / 2);
                match self.act {
                    0 | 4 => {
                        let c = self.cell_at(mx, my);
                        self.set_cell(mx, my, (c & !0xF0000) | if self.id == 17 { 0x40400 } else { 0x30400 });
                    }
                    3 => {
                        let c = self.cell_at(mx, my);
                        self.set_cell(mx, my, (c & !0xF0000) | 0x30400);
                    }
                    1 => {
                        let (a, b) = A2SEAM[(d.0 + 2 * d.1 + 2) as usize];
                        self.place(mx, my, a, -1);
                        self.place(mx + d.0.abs(), my + d.1.abs(), b, -1);
                    }
                    _ => {}
                }
            }
            let t = v.ty | n.ty;
            if t != 0 {
                cv |= 2;
            }
            let dn = ((nn.x - n.x).signum(), (nn.y - n.y).signum());
            let mut c = self.corner_def(code(v, d), code(n, dn), self.style(t));
            if c != 0 {
                if c == 19 {
                    c = if v.ty == 1 { if n.ty == 1 { 19 } else { 20 } } else { 21 };
                }
                self.place(n.x, n.y, c, -1);
                self.or_cell(n.x, n.y, cv);
            }
        }
        self.corner_flood();
        Ok(())
    }

    /// E4 OutCornerFlood.
    fn corner_flood(&mut self) {
        for (xf, yf, dx, dy) in [(0, 0, 1, 1), (1, 0, -1, 1), (0, 1, 1, -1), (1, 1, -1, -1)] {
            let x0 = if xf == 1 { self.cw - 1 } else { 0 };
            let mut y = if yf == 1 { self.ch - 1 } else { 0 };
            // Each pass stops on a bit 0 cell; the bounds guard only keeps a damaged ring from running away.
            while self.idx(x0, y).is_some() && self.cell_at(x0, y) & 1 == 0 {
                let mut x = x0;
                while self.idx(x, y).is_some() && self.cell_at(x, y) & 1 == 0 {
                    self.or_cell(x, y, 0x100);
                    x += dx;
                }
                y += dy;
            }
        }
    }

    /// G1 OutBorderSubApply: every record of `ty` as a rewrite of the cell grid.
    fn border_sub(&mut self, md: &MapData, ty: i32, base: i32) -> Res<()> {
        for rec in md.t.sub.iter().skip_while(|r| r.kind != ty).take_while(|r| r.kind == ty) {
            let ds = (md.ds1)(&rec.file).map_err(|e| MapError::Data(format!("{}: {}", rec.file, e.0)))?;
            // G2 BorderSubRecord
            if ds.groups.is_empty() {
                continue;
            }
            let n = ds.groups.len();
            let s = if rec.bord_type == 0 { self.seed.range(n as i32) as usize } else { 0 };
            for i in 0..n {
                let ok = self.sub_group(rec.grid_size, rec.bord_type, &ds, ds.groups[(s + i) % n], ty, base)?;
                if ok && rec.bord_type == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    /// G3 BorderSubGroup; positions snap down to multiples of `GridSize` (L5: the `Barricade` records, `Type` 12, use the Act V `Def` map).
    fn sub_group(&mut self, gs: i32, bord: i32, ds: &Ds1, g: [i32; 5], ty: i32, base: i32) -> Res<bool> {
        let off = if ty == 1 && self.flags & 0xC != 0 { -1 } else { 1 };
        let (nx, ny) = (self.cw - g[2] * gs + off, self.ch - g[3] * gs + 1);
        if nx * ny <= 0 {
            return Ok(false);
        }
        let skip22 = ty == 1 && (2..=7).contains(&self.id) && nx < 6 && ny < 6;
        for (px, py) in self.shuffle(nx * ny, nx) {
            if skip22 && (px, py) == (2, 2) {
                continue;
            }
            let p = (px - px % gs, py - py % gs);
            if self.sub_fits(p, ds, g, (gs, base, ty == 12))? {
                let v = self.seed.range(g[4]) as i32;
                self.sub_stamp(p, ds, g, v + 1, (gs, base, ty == 12))?;
                if bord <= 1 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// L5 BarricadeSubDefMap: the `Def` a Barricade stamp cell names (-5 the wildcard); no row is fatal.
    fn a5_map(&self, w: u32) -> Res<i32> {
        let (main, sub) = ((w >> 20) as i32 & 0x3F, (w >> 8) as i32 & 0xFF);
        let r = A5MAP.iter().find(|r| r.0 == main && (r.1..=r.2).contains(&sub)).ok_or_else(|| fatal(self, "Barricade stamp cell with no Def row"))?;
        Ok(if self.id == 117 { r.4 } else { r.3 } - r.1 + sub)
    }

    /// G4: the group cell `(gx, gy)` of variant `k` as `(wall, floor)`.
    fn sub_cells(ds: &Ds1, g: [i32; 5], k: i32, gx: i32, gy: i32) -> (u32, u32) {
        let i = ((g[1] + gy) as u32 * ds.w + (g[0] + (g[2] + 1) * k + gx) as u32) as usize;
        let c = |v: Option<&d2data::ds1::Cell>| v.map_or(0, |c| u32::from_le_bytes([c.prop1, c.prop2, c.prop3, c.prop4]));
        (c(ds.walls.first().and_then(|w| w.get(i))), c(ds.floors.first().and_then(|w| w.get(i))))
    }

    /// G4 BorderSubFits: a wall cell (the wildcard 63 included) must not be a seam opening; Act V maps every cell through L5. `sub` is `(GridSize, baseDef, Act V)`.
    fn sub_fits(&self, (px, py): (i32, i32), ds: &Ds1, g: [i32; 5], (gs, base, a5): (i32, i32, bool)) -> Res<bool> {
        for gy in 0..g[3] {
            for gx in 0..g[2] {
                let (w, f) = Self::sub_cells(ds, g, 0, gx, gy);
                let (x, y) = (px + gs * gx, py + gs * gy);
                let ok = if a5 {
                    let m = self.a5_map(w)?;
                    m == -5 || m == self.def_at(x, y) && self.cell_at(x, y) & 0x400 == 0
                } else if w & 1 != 0 {
                    let sub = (w >> 8) as i32 & 0xFF;
                    (sub == 63 || self.def_at(x, y) == base + sub - 1) && self.cell_at(x, y) & 0x400 == 0
                } else {
                    f & 2 == 0 || self.fits(x, y, 0, 0, 0)
                };
                if !ok {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn sub_stamp(&mut self, (px, py): (i32, i32), ds: &Ds1, g: [i32; 5], k: i32, (gs, base, a5): (i32, i32, bool)) -> Res<()> {
        for gy in 0..g[3] {
            for gx in 0..g[2] {
                let (w, f) = Self::sub_cells(ds, g, k, gx, gy);
                let (x, y) = (px + gs * gx, py + gs * gy);
                if w & 1 != 0 {
                    let d = if a5 { self.a5_map(w)? } else { base + ((w >> 8) as i32 & 0xFF) - 1 };
                    // Sub 63 is the wildcard: the cell keeps its piece.
                    if d != -5 && (w >> 8) & 0xFF != 63 {
                        self.place_b(x, y, d, 0, true);
                    }
                } else if f & 2 != 0 {
                    self.clear(x, y, 0);
                } else {
                    self.clear(x, y, 0x100);
                }
            }
        }
        Ok(())
    }

    /// J5 OutFillRandom: a shuffle of every cell, one `Range` per cell visited, `max` 0 for no limit.
    fn fill(&mut self, lo: i32, hi: i32, max: i32) {
        let mut n = 0;
        for (x, y) in self.shuffle(self.cw * self.ch, self.cw) {
            let d = lo + self.seed.range(hi - lo + 1) as i32;
            if self.fits(x, y, d, 0, 0) {
                self.place(x, y, d, -1);
                n += 1;
                if max > 0 && n >= max {
                    return;
                }
            }
        }
    }
}

/// A1 ring cell index of the segment cells, for the roads.
const HUB: [(i32, i32); 4] = [(-1, 0), (0, 1), (0, -1), (1, 0)];
/// H6d `g_A1SearchTurns`, flat, and `g_DirOctant` column 0.
const TURNS: [i32; 16] = [0, 1, 2, 3, 0, 1, 1, 1, 3, 2, 1, 2, 0, 3, 2, 1];
const OCT: [i32; 25] = [5, 4, 4, 4, 3, 6, 5, 4, 3, 2, 6, 6, 6, 2, 2, 6, 7, 0, 1, 2, 7, 0, 0, 0, 1];
const JITTER: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

struct Node {
    h: i32,
    g: i32,
    pos: (i32, i32),
    tried: i32,
    row: usize,
    ptr: usize,
    dir: i32,
    parent: Option<usize>,
    child: Option<usize>,
}

/// H6d A1PathSearch from `s` to `t` on the cell grid; the cells from the target back to the start.
fn road_search(o: &Out, s: (i32, i32), t: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    let h = |p: (i32, i32)| {
        let (a, b) = ((p.0 - t.0).abs(), (p.1 - t.1).abs());
        a.min(b) + 2 * a.max(b)
    };
    let dir_of = |p: (i32, i32)| {
        let (mut dx, mut dy) = (t.0 - p.0, t.1 - p.1);
        if dx.abs() >= 2 * dy.abs() {
            dy = if dy < 0 { -1 } else { dy & 1 };
        } else if dy.abs() >= 2 * dx.abs() {
            dx = if dx < 0 { -1 } else { dx & 1 };
        }
        (OCT[(5 * dx.clamp(-2, 2) + dy.clamp(-2, 2) + 12) as usize] / 2) & 3
    };
    if (s.0 - t.0).abs() + (s.1 - t.1).abs() <= 1 {
        return Some(vec![s, t]);
    }
    let (h0, turn) = (h(s), |row: usize, ptr: usize| TURNS.get(row * 4 + ptr).copied().unwrap_or(0));
    let (mut bound, top) = (h0 + h0 / 2, h0 + h0 / 2 + 35);
    loop {
        let mut pool = vec![Node { h: h0, g: 0, pos: s, tried: -1, row: 0, ptr: 0, dir: dir_of(s), parent: None, child: None }];
        let mut node = 0usize;
        let found = 'search: loop {
            if pool[node].pos == t {
                break Some(node);
            }
            let n = &pool[node];
            let np = (n.pos.0 + [1, 0, -1, 0][n.dir as usize], n.pos.1 + [0, 1, 0, -1][n.dir as usize]);
            let on_chain = |mut i: Option<usize>| {
                while let Some(k) = i {
                    if pool[k].pos == np {
                        return true;
                    }
                    i = pool[k].parent;
                }
                false
            };
            let open = np == t || (o.idx(np.0, np.1).is_some() && o.cell_at(np.0, np.1) & 0x200 == 0 && !on_chain(Some(node)));
            if open && h(np) + n.g + 2 <= bound {
                let (g, nd) = (n.g + 2, n.dir);
                let c = match n.child {
                    Some(c) => c,
                    None => {
                        if pool.len() >= 900 {
                            break 'search None;
                        }
                        pool.push(Node { h: 0, g: 0, pos: (0, 0), tried: 0, row: 0, ptr: 0, dir: 0, parent: Some(node), child: None });
                        pool[node].child = Some(pool.len() - 1);
                        pool.len() - 1
                    }
                };
                let td = dir_of(np);
                let row = ((nd - td) & 3) as usize;
                pool[c] = Node { h: h(np), g, pos: np, tried: 0, row, ptr: 0, dir: (turn(row, 0) + td) & 3, parent: Some(node), child: pool[c].child };
                node = c;
                continue;
            }
            // Backtrack
            let mut k = node;
            if pool[k].tried < 4 {
                pool[k].ptr += 1;
                pool[k].dir = (turn(pool[k].row, pool[k].ptr) + pool[k].dir) & 3;
            }
            pool[k].tried += 1;
            while pool[k].tried == 3 {
                let Some(p) = pool[k].parent else { break 'search None };
                k = p;
                pool[k].ptr += 1;
                pool[k].dir = (turn(pool[k].row, pool[k].ptr) + pool[k].dir) & 3;
                pool[k].tried += 1;
            }
            node = k;
        };
        bound += 5;
        if pool.len() > 899 {
            return None;
        }
        if let Some(mut i) = found {
            let mut path = vec![];
            loop {
                path.push(pool[i].pos);
                match pool[i].parent {
                    Some(p) => i = p,
                    None => return Some(path),
                }
            }
        }
        if bound >= top {
            return None;
        }
        let _ = pool[0].h;
    }
}

impl Out {
    /// H1 A1CliffMark. The spec text ("non-seam up/right runs") is not the rule; golden l4 pins the code's walk: a run starts only at a
    /// vertex `v` turning W-to-N or N-to-E whose own and preceding segments are both non-seam, follows N and E segments with no seam at
    /// either end, and is marked up to the last vertex whose segment ends at a non-seam N-to-E or E-to-S turn; the walk stops once it
    /// passes the head again.
    fn cliff_mark(&mut self) {
        let (ring, n) = (self.ring.clone(), self.ring.len());
        let at = |i: usize| ring[i % n];
        let ns = |i: usize| at(i).fl & 1 == 0;
        let (mut v, mut p, mut done) = (0, n - 1, false);
        loop {
            let (mut w, n0) = (v, v + 1);
            if ns(v) && ns(p) && ((at(v).x < at(n0).x && at(v).y < at(p).y) || (at(n0).y < at(v).y && at(v).x < at(p).x)) {
                let mut last = None;
                loop {
                    done |= w % n == 0;
                    let m = w + 1;
                    if at(w).y < at(m).y || at(m).x < at(w).x || !ns(w) || !ns(m) {
                        break;
                    }
                    if (at(w).x < at(m).x && at(m).y < at(m + 1).y) || (at(m).y < at(w).y && at(m).x < at(m + 1).x) {
                        last = Some(w);
                    }
                    w = m;
                    if w % n == v % n {
                        break;
                    }
                }
                if let Some(last) = last {
                    (v..=last).for_each(|q| self.ring[q % n].ty = 1);
                    self.flags |= 0x20;
                }
            }
            v = (w + 1) % n;
            if done || v == 0 {
                return;
            }
            p = w % n;
        }
    }

    /// H3 A1RiverColumnClear.
    fn river_clear(&self, x: i32) -> bool {
        (0..self.ch).all(|y| self.cell_at(x, y) & 2 == 0 && self.cell_at(x + 1, y) & 2 == 0)
    }

    /// H3 A1RiverPlace.
    fn river_place(&mut self, x: i32) -> Res<()> {
        for y in 0..self.ch {
            for (col, def) in [(x, 26), (x + 1, 27)] {
                let (p, c) = (self.def_at(col, y), self.cell_at(col, y));
                let idx = if p == 0 {
                    if c & 0x100 != 0 { 0 } else { 3 }
                } else if p == 7 && c & 0xF0000 == 0x30000 {
                    3
                } else {
                    let r = RIVER.get((p - 4) as usize).ok_or_else(|| fatal(self, "river column over a cliff piece"))?;
                    if def == 26 { r.0 } else { r.1 }
                };
                self.place(col, y, def, idx);
            }
        }
        if self.flags & 0x14 != 0 {
            // A1BridgePlace
            let s = self.seed.range(self.ch - 2) as i32;
            for k in 0..(self.ch - 2).max(0) {
                let y = (k + s) % (self.ch - 2) + 1;
                // Flag 0x4 (the east river) tests only the cell west of the river.
                let beside = self.free(x - 1, y) && (self.flags & 4 != 0 || self.free(x + 2, y));
                if beside && (self.cell_at(x, y) >> 16) & 0xF == 3 && (self.cell_at(x + 1, y) >> 16) & 0xF == 3 {
                    self.place(x, y, 28, 1);
                    self.place(x + 1, y, 28, 2 + i32::from(self.flags & 4 != 0));
                    break;
                }
            }
        }
        Ok(())
    }

    /// H2 A1RiverEastAndCave.
    fn river_east_and_cave(&mut self) -> Res<()> {
        if self.flags & 0xC != 0 && self.river_clear(self.cw - 2) {
            self.river_place(self.cw - 2)?;
        }
        if self.flags & 0x20 != 0 && self.flags & 0x40 == 0 {
            let r = self.seed.step();
            let (cw, ch) = (self.cw, self.ch);
            // Columns outer swaps the bounds too (`x` over `ch`, `y` over `cw`); only square levels reach it on stock data.
            let order: Vec<(i32, i32)> = if r & 1 == 0 { (0..ch).flat_map(|y| (0..cw).map(move |x| (x, y))).collect() } else { (0..ch).flat_map(|x| (0..cw).map(move |y| (x, y))).collect() };
            let hit = order.into_iter().find(|&(x, y)| matches!(self.def_at(x, y), 16 | 17)).ok_or_else(|| fatal(self, "cliff cave not found"))?;
            self.place(hit.0, hit.1, if self.def_at(hit.0, hit.1) == 16 { 25 } else { 24 }, -1);
            self.flags |= 0x40;
        }
        if self.flags & 0x1C != 0 && self.flags & 0x40 == 0 {
            let r = self.seed.step() & 3;
            let x = if r & 1 == 1 { 3 } else { self.cw - if self.flags & 0x10 != 0 { 4 } else { 5 } };
            let y = if r >> 1 == 1 { 3 } else { self.ch - 4 };
            self.place(x, y, 51 + i32::from(self.id == 2), -1);
            self.flags |= 0x40;
        }
        Ok(())
    }

    /// H4 A1RiverMidTransCave.
    fn river_mid_trans_cave(&mut self) -> Res<()> {
        if self.flags & 0x10 != 0 && self.river_clear(self.cw / 2 - 1) {
            self.river_place(self.cw / 2 - 1)?;
        }
        let (cw, ch, f) = (self.cw, self.ch, self.flags);
        for (bit, x, y, def, file) in [(0x80, 0, 0, 3, 1), (0x100, cw - 7, 0, 3, 2), (0x200, 0, 1, 2, 1), (0x400, 0, ch - 6, 2, 1)] {
            if f & bit != 0 {
                self.place(x, y, def, file);
            }
        }
        if self.flags & 0x40 == 0 {
            let ok = if self.id == 2 { self.place_far(self.lv1, 52, -1, 1, 0xF) } else { self.place_random(51, -1, 1, 0xF) };
            if !ok {
                return Err(fatal(self, "cave entrance does not fit"));
            }
            self.flags |= 0x40;
        }
        Ok(())
    }

    /// H6 A1Paths: roads from the level's endpoints to a hub; marks the cells and makes the jitter draws.
    fn paths(&mut self) {
        let (lx, ly) = (self.rect.x, self.rect.y);
        let mut a: Vec<(i32, i32, i32)> = vec![];
        for n in &self.adj {
            let r = n.rect;
            if n.nbr == 1 {
                let (x, y) = [(59, 19), (29, 35), (4, 22), (29, 3)][n.side as usize];
                a.push((r.x + x, r.y + y, i32::from(n.side)));
            } else if n.nbr == 26 {
                a.push((r.x + 27, r.y + 13, 1));
            }
        }
        for x in 0..self.cw {
            for y in 0..self.ch {
                let (d, f) = (self.def_at(x, y), (self.cell_at(x, y) >> 16) & 0xF);
                let dir = match (d, f) {
                    (4, 3) => 3,
                    (5, 3) => 0,
                    (6, 3) => 1,
                    (7, 3) => 2,
                    (24, _) => 1,
                    (25, _) => 0,
                    (28, 1) if x == self.cw - 2 => 2,
                    (51 | 52, f) => i32::from(f != 0),
                    _ => 4,
                };
                if dir != 4 {
                    a.push((lx + 3 + 8 * x, ly + 3 + 8 * y, dir));
                }
            }
        }
        let snap = |p: (i32, i32, i32)| {
            let (mut rx, mut ry) = (p.0 - lx, p.1 - ly);
            match p.2 {
                0 => rx = rx / 8 * 8 + 11,
                1 => ry = ry / 8 * 8 + 11,
                2 => rx = rx / 8 * 8 - 5,
                3 => ry = ry / 8 * 8 - 5,
                _ => {}
            }
            (rx + lx, ry + ly, p.2)
        };
        let n = a.len();
        // H6c A1PathHub
        let mut bridge = None;
        if self.flags & 0x10 != 0 {
            let x = self.cw / 2 - 1;
            bridge = (1..=self.cw - 2).find(|&y| self.def_at(x, y) == 28 && (self.cell_at(x, y) >> 16) & 0xF == 1).map(|y| (x, y));
        }
        let d: Vec<(i32, i32, i32)> = if let Some((bx, by)) = bridge {
            let (x, y) = (lx + 3 + 8 * bx, ly + 3 + 8 * by);
            a.iter().map(|p| if p.0 > x { (x + 8, y, 0) } else { (x, y, 2) }).collect()
        } else if n > 0 {
            let (cx, cy) = if n == 1 { (self.cw / 2, self.ch / 2) } else { (a.iter().map(|p| p.0 - lx).sum::<i32>() / (8 * n as i32), a.iter().map(|p| p.1 - ly).sum::<i32>() / (8 * n as i32)) };
            let (mut hx, mut hy) = (cx, cy);
            'hub: for r in 0..8 {
                for (dx, dy) in HUB {
                    (hx, hy) = (cx + dx * r, cy + dy * r);
                    if hx >= 0 && hx < self.cw && hy >= 0 && hy < self.ch && self.free(hx, hy) {
                        break 'hub;
                    }
                }
            }
            vec![(lx + 3 + 8 * hx, ly + 3 + 8 * hy, 4); n]
        } else {
            vec![]
        };
        let cellof = |p: (i32, i32, i32)| ((p.0 - lx) / 8, (p.1 - ly) / 8);
        for i in 0..n {
            let (bp, c) = (snap(a[i]), snap(d[i]));
            // H6e A1PathJitter runs only on a found path: a failed search draws nothing.
            let Some(path) = road_search(self, cellof(bp), cellof(c)) else { continue };
            for &(x, y) in &path {
                self.or_cell(x, y, 0x80);
            }
            let mut r = (self.seed.step() & 3) as usize;
            let mut pts: Vec<(i32, i32)> = path.iter().map(|&(x, y)| (lx + 8 * x + 3, ly + 8 * y + 3)).collect();
            for q in pts.iter_mut().take(path.len() - 1).skip(1) {
                let (s1, s2) = (self.seed.step() as i32, self.seed.step() as i32);
                *q = (q.0 + ((s1 & 1) + 2) * JITTER[r].0, q.1 + ((s2 & 1) + 2) * JITTER[r].1);
                r = (r + 1) & 3;
            }
            let last = pts.len() - 1;
            (pts[0], pts[last]) = ((c.0, c.1), (bp.0, bp.1));
            if d[i].2 != 4 {
                pts.insert(0, (d[i].0, d[i].1));
            }
            pts.push((a[i].0, a[i].1));
            self.roads.push(pts);
        }
    }

    /// H7 A1Areas.
    fn areas(&mut self) {
        let one_or_two = |o: &mut Out, d: i32, s: bool| {
            let r = o.seed.step();
            o.place_near(d, -1);
            if r & 3 == 0 {
                o.place_near(d, -1);
            }
            if s && o.seed.step() & 1 != 0 {
                o.place_near(49, -1);
            }
        };
        // `Near(d)`, `Rand(d)` and `OneOrTwo(d, s)` of the table, as (kind, def, s).
        let seq: &[(u8, i32, bool)] = match self.id {
            2 => &[(0, 46, false), (2, 47, false), (1, 29, false), (1, 30, false)],
            3 => &[(2, 48, true), (1, 44, false), (1, 29, false), (1, 30, false)],
            4 => &[(0, 160, false), (0, 45, false), (1, 162, false), (2, 47, true), (2, 42, false), (1, 31, false)],
            5 => &[(1, 161, false), (1, 41, false), (1, 40, false), (2, 48, true), (2, 43, false), (1, 29, false), (1, 30, false)],
            6 => &[(1, 163, false), (1, 38, false), (1, 39, false), (2, 47, true), (2, 42, false), (1, 29, false), (1, 30, false)],
            7 => &[(2, 48, true), (2, 43, false), (1, 31, false)],
            39 => &[(1, 50, false), (1, 46, false), (1, 31, false), (1, 38, false), (1, 39, false), (1, 29, false), (1, 30, false)],
            17 => {
                self.place(1, 1, 108, -1);
                return;
            }
            _ => &[],
        };
        for &(k, d, s) in seq {
            match k {
                0 => drop(self.place_near(d, -1)),
                1 => drop(self.place_random(d, -1, 0, 0xF)),
                _ => one_or_two(self, d, s),
            }
        }
    }

    /// H1 OutdoorAct1.
    fn act1(&mut self, md: &MapData) -> Res<()> {
        let id = self.id;
        if ![2, 3, 17].contains(&id) {
            self.cliff_mark();
        }
        self.seam_mark();
        self.border_build()?;
        if (2..=7).contains(&id) {
            self.border_sub(md, 0, 4)?;
            self.river_east_and_cave()?;
            self.border_sub(md, 1, 4)?;
            self.border_sub(md, 2, 4)?;
            self.river_mid_trans_cave()?;
            self.border_sub(md, 3, 4)?;
            self.paths();
        }
        if id == 39 {
            for t in 0..4 {
                self.border_sub(md, t, 4)?;
            }
        }
        if (3..=6).contains(&id) {
            self.waypoint();
        }
        if (2..=7).contains(&id) {
            self.shrines();
        }
        self.areas();
        Ok(())
    }
}


/// I3 `g_A2CliffLayouts`: `(def, file, x, y)`, five pieces per layout; -1 takes the rotation.
const A2CLIFF: [[(i32, i32, i32, i32); 5]; 8] = [
    [(376, 1, 0, 4), (378, -1, 2, 4), (377, -1, 4, 4), (377, -1, 6, 4), (376, 2, 8, 4)],
    [(376, 1, 0, 4), (377, -1, 2, 4), (378, -1, 4, 4), (377, -1, 6, 4), (376, 2, 8, 4)],
    [(376, 1, 0, 4), (377, -1, 2, 4), (377, -1, 4, 4), (378, -1, 6, 4), (376, 2, 8, 4)],
    [(376, 2, 8, 4), (377, -1, 6, 4), (382, -1, 4, 4), (381, -1, 4, 6), (379, 2, 4, 8)],
    [(376, 2, 8, 4), (378, -1, 6, 4), (382, -1, 4, 4), (380, -1, 4, 6), (379, 2, 4, 8)],
    [(379, 1, 4, 0), (381, -1, 4, 2), (380, -1, 4, 4), (380, -1, 4, 6), (379, 2, 4, 8)],
    [(379, 1, 4, 0), (380, -1, 4, 2), (381, -1, 4, 4), (380, -1, 4, 6), (379, 2, 4, 8)],
    [(379, 1, 4, 0), (380, -1, 4, 2), (380, -1, 4, 4), (381, -1, 4, 6), (379, 2, 4, 8)],
];
/// I3 `g_A2KingTombCliff`.
const A2TOMB: [(i32, i32, i32, i32); 9] = [(384, 0, 8, 0), (383, 2, 6, 0), (383, 1, 4, 0), (383, 0, 2, 0), (387, 0, 0, 0), (385, 0, 0, 2), (385, 1, 0, 4), (385, 2, 0, 6), (386, 0, 0, 8)];
/// K1 `g_A4MesaBase` and `g_A4PitsBase`.
const MESA: [i32; 3] = [812, 817, 823];
const PITS: [i32; 3] = [828, 832, 832];

impl Out {
    /// I4 PlaceList.
    fn place_list(&mut self, md: &MapData, defs: &[i32], per_file: bool) {
        let n = defs.len();
        let mut k = self.seed.range(n as i32) as usize;
        for _ in 0..n {
            if per_file {
                let files = md.t.prest.iter().find(|p| p.def == defs[k]).map_or(0, |p| p.files);
                for i in 0..files {
                    self.place_random(defs[k], i, 0, 0xF);
                }
            } else {
                self.place_random(defs[k], -1, 0, 0xF);
            }
            k = (k + 1) % n;
        }
    }

    /// I1 OutdoorAct2.
    fn act2(&mut self, md: &MapData) -> Res<()> {
        let id = self.id;
        self.seam_mark();
        self.border_build()?;
        let subs = |o: &mut Out| -> Res<()> { [2, 1, 3].into_iter().try_for_each(|t| o.border_sub(md, t, 364)) };
        let special = |o: &mut Out, def: i32| if o.place_random(def, -1, 0, 0xF) { Ok(()) } else { Err(fatal(o, "special area does not fit")) };
        if id == 41 {
            // I6 TownTransition
            if let Some(n) = self.adj.iter().find(|n| n.nbr == 40) {
                if n.side == 3 {
                    self.place(0, self.ch - 1, 363, -1);
                } else {
                    self.place(self.cw - 1, 0, 362, -1);
                }
            }
        } else if id == 46 {
            for (d, f, x, y) in A2TOMB {
                self.place(x, y, d, f);
            }
        } else if (42..=44).contains(&id) {
            // I3 Cliff
            let k = (self.seed.step() & 7) as usize;
            for (d, f, x, y) in A2CLIFF[k] {
                self.place(x, y, d, f);
            }
        }
        if id == 46 || id == 134 {
            self.place(4, 4, 394, -1);
        }
        if id != 45 {
            subs(self)?;
        }
        if id <= 45 {
            special(self, [388, 388, 390, 412, 389][(id - 41) as usize])?;
        }
        match id {
            43 => self.place_list(md, &[396, 397], false),
            44 => self.place_list(md, &[413, 408, 409, 410], false),
            _ => {}
        }
        if (42..=44).contains(&id) {
            self.waypoint();
        }
        // I1: the shrines come after the fills on level 134 only.
        if id != 45 && id != 134 {
            self.shrines();
        }
        // The fill lists (perFile 0 first, then perFile 1).
        let (a, b, twice): (&[i32], &[i32], i32) = match id {
            41 => (&[395, 411, 401, 402, 399, 398, 403], &[], 0),
            42 => (&[395, 411, 400, 398], &[404, 405, 406, 407], 1),
            43 => (&[411, 399, 398, 403], &[395], 2),
            44 => (&[395, 400, 398, 404, 405], &[411], 2),
            46 | 134 => (&[401, 402, 406, 407, 403], &[392, 393], 1),
            _ => (&[], &[], 0),
        };
        if !a.is_empty() {
            self.place_list(md, a, false);
        }
        for _ in 0..twice {
            self.place_list(md, b, true);
        }
        if id == 134 {
            self.shrines();
        }
        Ok(())
    }

    /// K1 OutdoorAct4.
    fn act4(&mut self, md: &MapData) -> Res<()> {
        self.seam_mark();
        if self.id == 108 {
            // K2 grid
            for i in 0..25 {
                let def = match i {
                    7 => 861,
                    11 => 858,
                    12 => 862,
                    13 => 859,
                    17 => 860,
                    22 => 857,
                    _ => 836,
                };
                self.place((i % 5) * 3, (i / 5) * 3, def, -1);
            }
            return Ok(());
        }
        self.border_build()?;
        if self.flags & 0x400000 != 0 {
            self.place(0, 1, 798, -1);
        }
        if self.flags & 0x800000 != 0 {
            self.place(0, 4, 798, -1);
        }
        for t in 1..=3 {
            self.border_sub(md, t, 799)?;
        }
        // MesaFills
        let r = |o: &mut Out, def: i32, n: i32| (0..n).for_each(|_| _ = o.place_random(def, -1, 0, 0xF));
        let i = (self.id - 104) as usize;
        if self.id == 106 {
            r(self, 811, 1);
        }
        let (a, p) = (MESA[i], PITS[i]);
        r(self, a, 1);
        for (k, n) in [(1, 2), (2, 2), (3, 2)] {
            r(self, a + k, n);
        }
        if self.id == 105 {
            r(self, 822, 1);
        }
        r(self, a + 4, 4);
        r(self, p, 1);
        for (k, n) in [(1, 2), (2, 2), (3, 4)] {
            r(self, p + k, n);
        }
        Ok(())
    }

    /// L1 SiegeStrips (level 110).
    fn siege_strips(&mut self) {
        for k in 0..15 {
            self.place(28 - 2 * k, 0, 865 + k, 0);
        }
    }

    /// J4 OutdoorAct3: the jungle cells (76 to 78), the Kurast rings and fills (79 to 81), the bridge (82) and Travincal (83).
    fn act3(&mut self, defs: &[i32], bit: bool) -> Res<()> {
        self.seam_mark();
        let (id, cw, ch) = (self.id, self.cw, self.ch);
        match id {
            76..=78 => {
                // OutJungleCells: blocks of 4 x 4 cells; the Head (76, last row) and the Tail (78, row 0) take an explicit index.
                let (nx, ny) = (cw / 4, ch / 4);
                let clr = defs.iter().filter(|&&d| d > 574).count();
                let r = self.seed.range(if clr == 3 { 6 } else { 2 }) as usize;
                let (mut e, mut k) = (0usize, 0usize);
                for j in 0..ny {
                    if id == 76 && j == ny - 1 || id == 78 && j == 0 {
                        let (d, probe) = if id == 76 { (573, (nx * ny - 1) as usize) } else { (574, 1) };
                        self.place(0, 4 * j, d, i32::from(defs.get(probe) == Some(&0)));
                        e += nx as usize;
                        continue;
                    }
                    for i in 0..nx {
                        let (mut d, mut idx) = (defs.get(e).copied().unwrap_or(0), -1);
                        e += 1;
                        if d > 574 {
                            d += 10 * (id as i32 - 76);
                            idx = *PERM.get(r).and_then(|p| p.get(k)).ok_or_else(|| fatal(self, "jungle with more than 3 clearings"))?;
                            k += 1;
                        }
                        if d != 0 {
                            self.place(4 * i, 4 * j, d, idx);
                        }
                    }
                }
            }
            79..=81 => {
                // J5a: the ring with its gates on the act-seed bit (`b` the N piece; S, E, W, NE, NW, SE, SW follow; gates +8).
                let (b, mid) = ([605, 619, 636][(id - 79) as usize], (cw - 1) / 2);
                let (ng, sg) = match id {
                    79 => (if bit { 1 } else { cw - 2 }, mid),
                    80 => if bit { (cw - 2, 1) } else { (1, cw - 2) },
                    _ => (mid, if bit { cw - 2 } else { 1 }),
                };
                let row = |o: &mut Out, y: i32, d: i32, g: i32, wide: bool| {
                    let mut x = 1;
                    while x < cw - 1 {
                        o.place(x, y, d + 8 * i32::from(x == g), -1);
                        x += 1 + i32::from(wide && x == g);
                    }
                };
                if id == 80 {
                    for x in 1..cw - 1 {
                        self.place(x, 0, b + 8 * i32::from(x == ng), -1);
                        self.place(x, ch - 1, b + 1 + 8 * i32::from(x == sg), -1);
                    }
                } else {
                    row(self, 0, b, ng, id == 81);
                    row(self, ch - 1, b + 1, sg, id == 79);
                }
                for y in 1..ch - 1 {
                    self.place(cw - 1, y, b + 2, -1);
                    self.place(0, y, b + 3, -1);
                }
                for (x, y, d) in [(0, 0, b + 5), (cw - 1, 0, b + 4), (0, ch - 1, b + 7), (cw - 1, ch - 1, b + 6)] {
                    self.place(x, y, d, -1);
                }
                // J5: the fixed pieces, PlaceRandom and the fills.
                if id != 79 {
                    let (p, q, y) = if id == 80 { (629, 630, 3) } else { (646, 647, ch - 4) };
                    self.place(3, y, p, 0);
                    self.place(cw - 4, y, p, 1);
                    self.place_random(q, 0, 0, 0xF);
                    self.place_random(q, 1, 0, 0xF);
                }
                self.place_random(631, 0, 0, 0xF);
                let [a, lo, hi, c] = [[618, 616, 617, 615], [635, 633, 634, 632], [651, 649, 650, 648]][(id - 79) as usize];
                self.fill(a, a, 4);
                self.fill(lo, hi, 0);
                self.fill(c, c, 0);
            }
            82 => self.place(0, 0, 652, 0),
            _ => (0..6).for_each(|k| self.place([0, 2, 6][k % 3], 4 * (k as i32 / 3), 653 + k as i32, -1)),
        }
        Ok(())
    }

    /// L1 OutdoorAct5 for the Barricade levels 111, 112 and 117.
    fn act5(&mut self, md: &MapData) -> Res<()> {
        self.seam_mark();
        let (id, cw, ch) = (self.id, self.cw, self.ch);
        let (fam, o) = (4 + usize::from(id == 117), 76 * i32::from(id == 117));
        let e2 = |z: i32| z + 2 * z.signum();
        // L2 BarricadeBorderRing: 2 x 2-cell pieces on even cells, the corner replacing the last edge piece, two 0x400 blocks per seam.
        for i in 0..self.ring.len() {
            let (v, n, nn) = (self.ring[i], self.ring[(i + 1) % self.ring.len()], self.ring[(i + 2) % self.ring.len()]);
            let (d, d2) = (((n.x - v.x).signum(), (n.y - v.y).signum()), ((nn.x - n.x).signum(), (nn.y - n.y).signum()));
            let di = match d { (-1, 0) => 0, (0, -1) => 1, (1, 0) => 2, _ => 3 };
            let (tx, ty) = (n.x & !1, n.y & !1);
            if v.fl & 2 == 0 {
                let (mut x, mut y) = (v.x & !1, v.y & !1);
                while (x, y) != (tx, ty) && self.idx(x + 2 * d.0, y + 2 * d.1).is_some() {
                    (x, y) = (x + 2 * d.0, y + 2 * d.1);
                    self.place(x, y, DEFS[1 + di][fam], -1);
                    self.or_cell(x, y, 1);
                }
            }
            if v.fl & 1 != 0 {
                let (gx, gy) = ((v.x.max(n.x) - 4 * d.0.abs()) & !1, (v.y.max(n.y) - 4 * d.1.abs()) & !1);
                self.or_cell(gx, gy, 0x400);
                self.or_cell(gx + 2 * d.0.abs(), gy + 2 * d.1.abs(), 0x400);
            }
            let c = self.corner_def(e2(2 * d.0) + 2 * d.1, e2(2 * d2.0) + 2 * d2.1, fam);
            if c != 0 {
                self.place(tx, ty, c, -1);
                self.or_cell(tx, ty, 1);
            }
        }
        if id == 111 {
            self.or_cell(cw - 2, ch - 4, 0x400);
            self.or_cell(cw - 2, ch - 3, 0x400);
        }
        // L3 BarricadeRavine: down the east column and along the bottom row, each ring piece's outgoing step.
        let (mut x, mut y) = (cw - 2, 0);
        while (x, y) != (0, ch - 2) {
            let k = self.def_at(x, y) - 881 - o;
            let s = STEP.get(k as usize).filter(|_| k >= 0).ok_or_else(|| fatal(self, "ravine walk off the ring"))?;
            self.place(x, y, 893 + o + k, -1);
            (x, y) = (x + 2 * s.0, y + 2 * s.1);
            if self.idx(x, y).is_none() {
                return Err(fatal(self, "ravine walk off the grid"));
            }
        }
        self.place(cw - 2, 0, 906 + o, -1);
        self.place(0, ch - 2, 905 + o, -1);
        // L4 gates: each edge's first 0x400 cell; the caves of `g_A5CaveEntries`; the `To Siege` pieces of 111.
        let open = |o: &Out, x: i32, y: i32| o.cell_at(x, y) & 0x400 != 0;
        for (xs, ys, d) in [(0..cw, 0..1, 909), (0..cw, ch - 2..ch - 1, 908), (0..1, 0..ch, 910), (cw - 2..cw - 1, 0..ch, 907)] {
            if let Some((x, y)) = xs.flat_map(|x| ys.clone().map(move |y| (x, y))).find(|&(x, y)| open(self, x, y)) {
                self.place(x, y, d, -1);
            }
        }
        for &(lv, file, far, tall, wide) in &CAVES {
            if lv == id {
                if self.rect.h < self.rect.w {
                    self.place(if far { cw - 2 } else { 0 }, 2, wide, file);
                } else {
                    self.place(2, if far { ch - 2 } else { 0 }, tall, file);
                }
            }
        }
        if id == 111 {
            self.place(cw - 2, ch - 4, 880, -1);
            self.place(cw - 2, ch - 6, 896, -1);
        }
        self.border_sub(md, 12, 0)?;
        if id == 111 {
            // L6 BarricadePrisons: up to 90 random probes, then two draws and a full scan.
            let placed = |o: &Out, x: i32, y: i32| if o.cell_at(x, y) & 0x200 != 0 { o.def_at(x, y) } else { 0 };
            let hit = |o: &mut Out, x: i32, y: i32| {
                let d = placed(o, x, y);
                let ok = (915..=922).contains(&d);
                if ok {
                    o.place(x, y, d + 16, -1);
                }
                i32::from(ok)
            };
            let mut n = 0;
            for _ in 0..90 {
                if n > 2 {
                    break;
                }
                let (x, y) = (2 * self.seed.range(cw / 2) as i32, 2 * self.seed.range(ch / 2) as i32);
                n += hit(self, x, y);
            }
            let (ox, oy) = (self.seed.range(cw / 2) as i32, self.seed.range(ch / 2) as i32);
            for (r, c) in (0..ch).flat_map(|r| (0..cw).map(move |c| (r, c))) {
                if n > 2 {
                    break;
                }
                n += hit(self, (c + 2 * ox) % cw, (r + 2 * oy) % ch);
            }
            if n < 3 {
                return Err(fatal(self, "fewer than 3 prisons"));
            }
        }
        // L7 BarricadeTablePlace: only the last call of a required row is tested.
        for &(lv, tall, wide, file, count, req) in &A5TABLE {
            if lv == id {
                let d = if self.rect.w < self.rect.h { tall } else { wide };
                let ok = (0..count).fold(false, |_, _| self.place_random(d, file, 0, 0xF));
                if (count <= 0 || !ok) && req {
                    return Err(fatal(self, "required Barricade piece does not fit"));
                }
            }
        }
        Ok(())
    }
}

/// J4 `g_JungleClearingPerm`: the file index of each clearing by the `Range(6)` row.
const PERM: [[i32; 3]; 6] = [[0, 1, 2], [1, 0, 2], [0, 2, 1], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
/// L3 `g_A5RavineStep` by ring row `k`; L4 `g_A5CaveEntries` `(level, file, far, defTall, defWide)`; L5 `g_A5SubDefMap` `(main, subLo, subHi, defDirt, defSnow)`.
const STEP: [(i32, i32); 12] = [(-1, 0), (0, -1), (1, 0), (0, 1), (0, -1), (1, 0), (0, 1), (-1, 0), (-1, 0), (0, -1), (1, 0), (0, 1)];
const CAVES: [(u32, i32, bool, i32, i32); 3] = [(112, 0, false, 913, 914), (117, 0, true, 983, 984), (117, 0, false, 985, 986)];
const A5MAP: [(i32, i32, i32, i32, i32); 10] = [(49, 1, 16, 915, 987), (49, 31, 46, 915, 987), (48, 1, 1, 883, 959), (48, 2, 3, 881, 957), (48, 4, 4, 884, 960), (48, 5, 5, 895, 971), (48, 6, 7, 893, 969), (48, 8, 8, 896, 972), (48, 30, 30, 0, 0), (48, 31, 31, -5, -5)];
/// L7 `g_A5PlaceTable` `(level, defTall, defWide, file, count, required)`.
const A5TABLE: [(u32, i32, i32, i32, i32, bool); 15] = [
    (111, 955, 956, 0, 1, true), (112, 955, 956, 0, 1, true), (117, 955, 956, 1, 1, true), (112, 953, 953, -1, 1, true), (117, 954, 954, -1, 1, true),
    (111, 944, 947, -1, 1, false), (111, 942, 945, -1, 4, false), (111, 943, 946, -1, 4, false), (112, 941, 941, -1, 1, false), (112, 939, 939, -1, 1, false),
    (112, 940, 940, -1, 5, false), (117, 948, 948, -1, 4, false), (117, 949, 949, -1, 4, false), (117, 950, 950, -1, 4, false), (117, 951, 951, -1, 3, false),
];

/// A: the window of one outdoor level, ending before `OutdoorRoomsCreate`; `seed` is the re-seeded level seed.
pub fn window(md: &MapData, map_seed: u32, diff: u8, level_id: u32, seed: Seed) -> Res<Out> {
    let def = md.t.levels.iter().find(|l| l.id as u32 == level_id).ok_or_else(|| MapError::Data(format!("level {level_id}")))?;
    let act = def.act as u8;
    let supported = match act {
        0 => [2, 3, 4, 5, 6, 7, 17, 39].contains(&level_id),
        1 => (41..=46).contains(&level_id),
        2 => (76..=83).contains(&level_id),
        3 => [104, 105, 106, 108].contains(&level_id),
        _ => [110, 111, 112, 117, 134].contains(&level_id),
    };
    if !supported {
        return Err(MapError::Unsupported(level_id, "outdoor level without a generator"));
    }
    let lay = links::layout(md, map_seed, diff, act)?;
    // warps.md C4: the builder's rect, else the `Levels` offset (level 134, which no link table places).
    let rect = lay.rect_of(md, level_id, usize::from(diff.min(2))).ok_or(MapError::Unsupported(level_id, "level rect from act links"))?;
    let mut o = Out::new(rect.w / 8, rect.h / 8, seed, &md.t.prest);
    (o.id, o.rect, o.lt, o.act, o.flags) = (level_id, rect, def.level_type, act, lay.flags.get(&level_id).copied().unwrap_or(0));
    o.lv1 = lay.rect.get(&1).copied().unwrap_or(rect);
    let (vis, warp) = lay.link.get(&level_id).copied().unwrap_or((def.vis, def.warp));
    o.vis = vis;
    let (vis, warp) = lay.adj.get(&level_id).copied().unwrap_or((vis, warp));
    let range = [1..=17, 40..=46, 75..=83, 103..=106, 111..=112][usize::from(act.min(4))].clone();
    if range.contains(&level_id) {
        for k in 0..8 {
            if warp[k] == -1 && vis[k] != 0 {
                let nbr = vis[k] as u32;
                let (r, p) = (lay.rect.get(&nbr).copied(), md.t.levels.iter().find(|l| l.id as u32 == nbr));
                let (Some(r), Some(p)) = (r, p) else { continue };
                let side = if rect.x == r.x + r.w { 0 } else if r.x == rect.x + rect.w { 2 } else if rect.y == r.y + r.h { 1 } else if r.y == rect.y + rect.h { 3 } else { continue };
                let a = Adj { nbr, side, preset: p.drlg_type == 2, rect: r };
                // C2 LevelAdjInsert: side ascending, then W by y ascending, N by x descending, E by y descending, S by x ascending;
                // the code compares against the head only while the list holds one node.
                let less = |n: &Adj, m: &Adj| n.side < m.side || n.side == m.side && match n.side { 0 => m.rect.y > n.rect.y, 1 => m.rect.x < n.rect.x, 2 => m.rect.y < n.rect.y, _ => m.rect.x > n.rect.x };
                let at = match o.adj.len() { 0 => 0, 1 => usize::from(!less(&a, &o.adj[0])), l => (1..l).find(|&k| less(&a, &o.adj[k])).unwrap_or(l) };
                o.adj.insert(at, a);
            }
        }
    }
    o.build_ring()?;
    // A1: level 134 takes the Act II generator; its branch is a code reading with no harness game (`verified`).
    match if level_id == 134 { 1 } else { act } {
        0 => o.act1(md)?,
        1 => o.act2(md)?,
        2 => o.act3(lay.jungle.get(&level_id).map_or(&[][..], |v| v), SeedTree { map_seed }.act3_bit())?,
        3 => o.act4(md)?,
        _ if level_id == 110 => o.siege_strips(),
        _ => o.act5(md)?,
    }
    Ok(o)
}

/// A and step 8: the window, then `OutdoorRoomsCreate` (README step 8); `seed_after` is the level seed at the end of the window, before the rooms' own draws.
pub fn build(md: &MapData, map_seed: u32, diff: u8, level_id: u32, _rect: Rect, seed: Seed) -> Result<Level, MapError> {
    let mut o = window(md, map_seed, diff, level_id, seed)?;
    let seed_after = o.seed;
    let def = md.t.levels.iter().find(|l| l.id as u32 == level_id).ok_or_else(|| MapError::Data(format!("level {level_id}")))?;
    // Constants: the outdoor room DT1 mask by level type.
    let base_mask = match def.level_type {
        2 => 0x44103,
        16 | 22 | 27 | 28 => 1,
        21 => 4,
        30 | 31 => 0x11,
        _ => 0,
    };
    let mut rooms = vec![];
    for cy in 0..o.ch {
        for cx in 0..o.cw {
            let c = o.cell_at(cx, cy);
            let (x, y) = (o.rect.x + 8 * cx, o.rect.y + 8 * cy);
            if c & 0x200 != 0 {
                let d = o.def_at(cx, cy);
                if let Some(p) = md.t.prest.iter().find(|p| p.def == d).filter(|_| d != 0) {
                    let mut rs = preset::instance(md, level_id, p, Rect { x, y, w: 0, h: 0 }, Some((c >> 16) & 0xF), o.g0_at(cx, cy), &mut o.seed)?;
                    if let Some(r) = rs.first_mut() {
                        r.out = Some((d, o.g0_at(cx, cy)));
                    }
                    rooms.extend(rs);
                }
            } else if c & 0x100 == 0 {
                let mut r = Room::new(Rect { x, y, w: 8, h: 8 }, room_seed(&mut o.seed));
                r.flags = o.g0_at(cx, cy) | 0x80000;
                r.mask = base_mask;
                // 8a LvlSubRoll: one room draw per record of the SubType, on a copy of the room seed.
                if def.sub_type != -1 && def.sub_theme != -1 {
                    let mut rs = r.seed;
                    r.sub = (def.sub_type, def.sub_theme, 0);
                    for (i, rec) in md.t.sub.iter().skip_while(|s| s.kind != def.sub_type).take_while(|s| s.kind == def.sub_type).enumerate() {
                        if ((rs.step() % 100) as i32) < rec.prob.get(def.sub_theme as usize).copied().unwrap_or(0) {
                            r.sub.2 |= 1 << i;
                            r.mask |= rec.dt1_mask;
                        }
                    }
                }
                rooms.push(r);
            }
        }
    }
    Ok(Level { id: level_id, rect: o.rect, kind: Kind::Outdoor, rooms, parts: vec![], warps: vec![], seed_after, roads: o.roads })
}

/// M2 `g_RoadCodes` by the 3 x 3 neighbour mask (7 NE, 6 E, 5 SE, 4 N, 3 S, 2 NW, 1 W, 0 SW); 0 writes nothing.
const ROAD_CODES: [u8; 256] = [
    0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15, 21, 21, 8, 8, 10, 38, 8, 8, 40, 20,
    0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15, 21, 21, 8, 8, 10, 38, 8, 8, 40, 20,
    13, 13, 7, 7, 13, 13, 13, 7, 4, 4, 11, 37, 4, 4, 11, 43, 3, 3, 12, 12, 3, 3, 39, 39, 9, 9, 2, 43, 9, 9, 44, 26,
    13, 13, 7, 7, 13, 13, 13, 7, 23, 23, 41, 17, 23, 23, 41, 17, 3, 3, 12, 12, 3, 3, 39, 39, 42, 42, 46, 42, 42, 42, 33, 31,
    0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15, 21, 21, 8, 8, 10, 38, 8, 8, 35, 20,
    0, 0, 16, 16, 0, 0, 16, 16, 14, 14, 6, 19, 14, 14, 6, 19, 15, 15, 5, 5, 15, 15, 21, 21, 8, 8, 10, 38, 8, 8, 40, 20,
    13, 13, 7, 7, 13, 13, 13, 7, 4, 4, 11, 37, 4, 4, 11, 37, 18, 18, 35, 35, 18, 18, 22, 22, 36, 36, 45, 34, 36, 36, 28, 29,
    13, 13, 7, 7, 13, 13, 13, 7, 23, 23, 41, 17, 23, 23, 41, 17, 18, 18, 35, 35, 18, 18, 22, 22, 24, 24, 25, 32, 24, 24, 30, 1,
];

/// M1 and M2: the 9 x 9 floor grid of an Act I out room at its prepare, `0x40002` on the 8 x 8 and `code << 8 | 0x82` on the road cells of the
/// polylines crossing the room's 11 x 11 road grid (M3 `GridLineOp`, width 2; no draw).
pub fn road_floor(roads: &[Vec<(i32, i32)>], r: Rect) -> Vec<u32> {
    let mut floor: Vec<u32> = (0..81).map(|i| if i % 9 < 8 && i / 9 < 8 { 0x40002 } else { 0 }).collect();
    let mut g = [[0u8; 11]; 11];
    let mut put = |px: i32, py: i32| {
        if let (Ok(gx), Ok(gy)) = (usize::try_from(px - r.x + 1), usize::try_from(py - r.y + 1))
            && let Some(c) = g.get_mut(gy).and_then(|row| row.get_mut(gx))
        {
            *c = 1;
        }
    };
    for p in roads {
        for s in p.windows(2) {
            // The minor axis steps when the error exceeds the whole major length; both ends are drawn.
            let ((mut x, mut y), (dx, dy)) = (s[0], (s[1].0 - s[0].0, s[1].1 - s[0].1));
            let (sx, sy, ax, ay) = (if dx < 0 { -1 } else { 1 }, if dy < 0 { -1 } else { 1 }, dx.abs(), dy.abs());
            let (major, minor) = (ax.max(ay), ax.min(ay));
            let mut err = 0;
            for k in 0..=major {
                if k > 0 {
                    if ax < ay { y += sy } else { x += sx }
                    err += minor;
                    if err > major {
                        if ax < ay { x += sx } else { y += sy }
                        err -= major;
                    }
                }
                for w in 0..2 {
                    if ax < ay { put(x + w, y) } else { put(x, y + w) }
                }
            }
        }
    }
    let b = |x: usize, y: usize| u32::from(g[y][x] != 0);
    for x in 1..=9 {
        for y in (1..=9).rev() {
            if g[y][x] == 0 {
                continue;
            }
            let m = b(x + 1, y - 1) << 7 | b(x + 1, y) << 6 | b(x + 1, y + 1) << 5 | b(x, y - 1) << 4 | b(x, y + 1) << 3 | b(x - 1, y - 1) << 2 | b(x - 1, y) << 1 | b(x - 1, y + 1);
            let c = u32::from(ROAD_CODES[m as usize]);
            if m != 0 && c != 0 {
                floor[(y - 1) * 9 + x - 1] = c << 8 | 0x82;
            }
        }
    }
    floor
}
