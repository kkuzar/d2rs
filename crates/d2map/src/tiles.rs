//! Room tiles: the DT1 list, tile choice by rarity, the k room-seed draws and the bit-2 merges into built neighbours (specs/drlg/room-tiles.md).
use crate::drlg::{Level, MapData, MapError, Rect, Room};
use crate::{outdoor, Seed};
use d2data::{dt1::Dt1, excel::SubDef};
use std::{collections::HashMap, rc::Rc};

type Res<T> = Result<T, MapError>;

/// One tile entry of a room: its cell, the DT1 in the room's list, the tile in that DT1 and the entry flags (9d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRef {
    pub x: i32,
    pub y: i32,
    pub dt1: u16,
    pub tile: u32,
    pub flags: u32,
}

/// An entry's place in `RoomTiles`: `(list, index)` with list 0 floors, 1 walls, 2 roofs, 3 shadows.
pub type Id = (u8, u32);

/// The tiles of a built room (step 7); `roofs` are the orientation 15 walls; `active` is the active-room seed of step 12 (none without floors or walls);
/// `lit` are the lit warp entries of 10a and 10b, hidden (`0x8`) until the player comes near.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoomTiles {
    pub floors: Vec<TileRef>,
    pub walls: Vec<TileRef>,
    pub roofs: Vec<TileRef>,
    pub shadows: Vec<TileRef>,
    pub active: Option<Seed>,
    pub lit: Vec<Id>,
}

impl RoomTiles {
    pub fn get(&self, id: Id) -> Option<&TileRef> {
        [&self.floors, &self.walls, &self.roofs, &self.shadows].get(usize::from(id.0))?.get(id.1 as usize)
    }

    fn get_mut(&mut self, id: Id) -> Option<&mut TileRef> {
        [&mut self.floors, &mut self.walls, &mut self.roofs, &mut self.shadows].into_iter().nth(usize::from(id.0))?.get_mut(id.1 as usize)
    }
}

/// A room's DT1 list (step 2) with its candidates by key `(orientation, main, sub)`: files in slot order, tiles newest-indexed first.
pub struct Pool {
    pub dt1: Vec<Rc<Dt1>>,
    by_key: HashMap<(i32, i32, i32), Vec<(u16, u32)>>,
}

impl Pool {
    pub fn new(dt1: Vec<Rc<Dt1>>) -> Pool {
        let mut by_key: HashMap<_, Vec<_>> = HashMap::new();
        for (d, f) in dt1.iter().enumerate() {
            for (t, tile) in f.tiles.iter().enumerate().rev() {
                by_key.entry((tile.orientation, tile.main, tile.sub)).or_default().push((d as u16, t as u32));
            }
        }
        Pool { dt1, by_key }
    }

    /// Step 3 PickTile: one draw iff the key has candidates (at most 40) and their rarities sum above 0; `None` is the game's fatal.
    pub fn pick(&self, seed: &mut Seed, o: i32, cell: u32) -> Option<(u16, u32)> {
        let key = (o, ((cell >> 20) & 0x3F) as i32, ((cell >> 8) & 0xFF) as i32);
        let Some(buf) = self.by_key.get(&key).map(|v| &v[..v.len().min(40)]) else { return self.by_key.get(&(10, 0, 0))?.first().copied() };
        let rar = |&(d, t): &(u16, u32)| self.dt1[usize::from(d)].tiles[t as usize].rarity;
        let total = buf.iter().fold(0i32, |a, b| a.wrapping_add(rar(b)));
        let r = if total > 0 { seed.range(total) } else { 0 };
        let (mut i, mut c) = (0, r as i32 + 1);
        if total != 0 {
            while buf.len() - i > 1 && c > 0 {
                c -= rar(&buf[i]);
                i += 1;
            }
        }
        buf.get(i.saturating_sub(1)).copied()
    }

    /// The `(main, sub)` of an entry's tile.
    fn key(&self, e: &TileRef) -> Option<(i32, i32)> {
        self.dt1.get(usize::from(e.dt1))?.tiles.get(e.tile as usize).map(|t| (t.main, t.sub))
    }
}

/// Step 8 `CellBits`: the entry flag bits a cell adds.
fn cell_bits(mut f: u32, c: u32) -> u32 {
    let on = |b: u32| c >> b & 1 != 0;
    for (b, v) in [(7, 0x1), (28, 0x102), (17, 0x40), (16, 0x80), (3, 0x4)] {
        if on(b) {
            f |= v;
        }
    }
    f = if on(31) { f | 0x8 } else { f & !0x8 };
    for (b, v) in [(26, 0x20C), (29, 0x800), (2, 0x2000)] {
        if on(b) {
            f |= v;
        }
    }
    f
}

/// Step 9d `layer(cell)`.
fn layer(c: u32) -> u32 {
    (((c >> 18) & 3) + 1) << 14
}

/// Step 9d `SetWallFlags` without the Orient89Unit call: the layer (not for a shadow), the orientation bit and the cell bits over `f`.
fn wall_flags(f: u32, cell: u32, o: i32) -> u32 {
    cell_bits(f | if o != 13 { layer(cell) } else { 0 } | if o == 14 { 0x4 } else if (8..=11).contains(&o) { 0x2 } else { 0 }, cell)
}

/// Step 9c `g_Bit2OrientRow` (new orientation 0 to 19: a row of `MERGE`, -1 keep, -2 no merge) and `g_Bit2OrientMerge` (stride 7, old orientation 0 to 7) (Constants).
const ROW: [i32; 20] = [-1, 0, 1, 2, -1, 3, 4, 5, -2, -2, -1, -1, -1, -2, -1, -1, -1, -1, -1, -1];
const MERGE: [i32; 43] = [0, 1, 3, 3, 4, 1, 3, 1, 1, 2, 3, 4, 3, 2, 2, 3, 3, 3, 4, 3, 3, 3, 1, 3, 3, 4, 5, 6, 1, 3, 2, 3, 4, 3, 6, 2, 1, 2, 3, 4, 1, 2, 7];

/// Step 11 `g_Orient89Levels` as `(first level, last level, first row, last row)` and `g_Orient89Rows` as `(main, sub, is9, unit id, unit type, dx, dy)` (Constants; `Game.exe` data).
const O89_LEVELS: [(u32, u32, usize, usize); 13] = [(28, 31, 0, 3), (26, 27, 4, 6), (32, 33, 5, 9), (34, 36, 10, 11), (37, 37, 10, 12), (51, 51, 13, 14), (52, 54, 15, 18), (55, 61, 19, 20), (66, 72, 19, 20), (62, 64, 21, 22), (109, 109, 23, 24), (111, 112, 24, 33), (117, 117, 24, 33)];
pub const O89_ROWS: [[i32; 7]; 34] = [
    [7, 0, 1, 14, 2, 5, 0], [7, 0, 0, 13, 2, 0, 5], [5, 0, 1, 16, 2, 0, 0], [5, 0, 0, 15, 2, 0, 0], [6, 0, 1, 27, 2, 5, -2], [4, 0, 1, 24, 2, 1, 2], [4, 0, 0, 23, 2, 0, 0],
    [4, 3, 1, 25, 2, 1, 0], [1, 2, 0, 62, 2, 0, 3], [1, 2, 1, 63, 2, 3, 0], [0, 0, 1, 16, 2, 0, 0], [0, 0, 0, 64, 2, 0, 0], [2, 0, 1, 47, 2, 5, 0], [0, 1, 1, 291, 2, 2, 0],
    [0, 1, 0, 290, 2, 0, 2], [5, 0, 1, 293, 2, 2, 0], [4, 0, 0, 292, 2, 0, 2], [0, 0, 1, 295, 2, 2, 0], [0, 0, 0, 294, 2, 0, 2], [2, 4, 1, 92, 2, 1, 0], [2, 1, 0, 91, 2, 0, 2],
    [0, 1, 1, 229, 2, 0, 0], [0, 1, 0, 230, 2, 0, 0], [3, 3, 0, 449, 2, -2, 4], [2, 1, 0, 435, 1, 1, 2], [2, 1, 1, 435, 1, 2, 1], [2, 6, 0, 435, 1, 1, 1], [2, 2, 0, 433, 1, 0, 1],
    [2, 3, 1, 432, 1, 1, 0], [26, 0, 0, 434, 1, 0, 1], [2, 4, 1, 524, 1, 0, 0], [2, 4, 0, 525, 1, 0, 0], [29, 0, 1, 60, 2, 2, 0], [29, 0, 0, 60, 2, 0, 2],
];

/// Step 11 `Orient89RowFind`: the first row of the level's range whose `(main, sub, is9)` match the cell, else `None`.
pub fn orient89_row(level: u32, cell: u32, is9: bool) -> Option<&'static [i32; 7]> {
    let (m, s) = (((cell >> 20) & 0x3F) as i32, ((cell >> 8) & 0xFF) as i32);
    let r = O89_LEVELS.iter().find(|r| (r.0..=r.1).contains(&level))?;
    O89_ROWS[r.2..=r.3].iter().find(|r| r[..3] == [m, s, i32::from(is9)])
}

/// Step 11 `g_Orient89Levels` as the level records `(level, first row, last row)` the exe holds, for the test against `Game.exe`.
pub fn orient89_levels() -> Vec<(u32, usize, usize)> {
    O89_LEVELS.iter().flat_map(|r| (r.0..=r.1).map(move |l| (l, r.2, r.3))).collect()
}

/// warps.md F1 note: a gap below 6 tiles on both axes (negative where the rects overlap on an axis).
pub fn within6(a: Rect, b: Rect) -> bool {
    let gap = |a: i32, b: i32, aw: i32, bw: i32| if a <= b { b - a - aw } else { a - b - bw };
    gap(a.x, b.x, a.w, b.w) < 6 && gap(a.y, b.y, a.h, b.h) < 6
}

/// warps.md F1 note, `RoomNearListBuild`: the near list of room `i` of `rooms` (the level's rooms in creation order): the rooms, newest first, whose gap to it is below
/// 6 tiles on both axes (itself included), then the exchange sort over adjacent pairs `(d, c)` that swaps them unless `d.x < c.x + c.w` and `d.y < c.y + c.h`, read as
/// n - 1 bubble passes (the test can swap both ways, so the pass count is bounded; the order only matters when two near rooms hold an entry of one kind at one cell). Returns indices.
pub fn near(rooms: &[Room], i: usize) -> Vec<usize> {
    let Some(r) = rooms.get(i).map(|r| r.rect) else { return vec![] };
    let mut v: Vec<usize> = (0..rooms.len()).rev().filter(|&j| within6(r, rooms[j].rect)).collect();
    for _ in 1..v.len() {
        for k in 1..v.len() {
            let (d, c) = (rooms[v[k - 1]].rect, rooms[v[k]].rect);
            if !(d.x < c.x + c.w && d.y < c.y + c.h) {
                v.swap(k - 1, k);
            }
        }
    }
    v
}

/// The grids of one room (step 5 or 6), `(w + 1) x (h + 1)` cells, rows outer.
struct Grids {
    floors: Vec<Vec<u32>>,
    walls: Vec<(Vec<u32>, Vec<u8>)>,
}

/// The built near rooms a room's bit-2 cells scan (9a): the level's rooms built so far and the near-list indices into them, in list order.
struct Near<'b, 'a> {
    built: &'b mut [Build<'a>],
    list: &'b [usize],
}

/// The state of one room's tile build, kept after it for the merges of the rooms built later (9c re-picks step `seed` and rewrite entries).
struct Build<'a> {
    pool: Rc<Pool>,
    md: &'a MapData<'a>,
    seed: Seed,
    level: u32,
    warp: [i32; 8],
    rect: Rect,
    out: RoomTiles,
    /// 9: each linked entry's orientation and `+0x20` link; the kind 0 (walls and shadows) and kind 1 (floors) node heads; 10: the warp nodes' list heads by warp id.
    link: HashMap<Id, (i32, Option<Id>)>,
    heads: [Option<Id>; 2],
    wheads: HashMap<i32, Option<Id>>,
}

impl<'a> Build<'a> {
    fn pick(&mut self, o: i32, cell: u32) -> Res<(u16, u32)> {
        self.pool.pick(&mut self.seed, o, cell).ok_or(MapError::Unsupported(self.level, "tile pick: no candidate and no (10, 0, 0) tile"))
    }

    fn push(&mut self, o: i32, e: TileRef) -> Id {
        let (l, v) = match o {
            0 => (0, &mut self.out.floors),
            13 => (3, &mut self.out.shadows),
            15 => (2, &mut self.out.roofs),
            _ => (1, &mut self.out.walls),
        };
        v.push(e);
        (l, v.len() as u32 - 1)
    }

    /// 9: prepends `id` to the node chain of its kind.
    fn prepend(&mut self, id: Id, o: i32) {
        let k = usize::from(o == 0);
        self.link.insert(id, (o, self.heads[k]));
        self.heads[k] = Some(id);
    }

    /// 10: re-links `id` into the list of the warp node `wid` (its `+0x20` takes the node's head, which cuts the older entries off a node chain it was on).
    fn relink(&mut self, id: Id, o: i32, wid: i32) {
        let h = self.wheads.get(&wid).copied().flatten();
        self.link.insert(id, (o, h));
        self.wheads.insert(wid, Some(id));
    }

    /// The entries of the kind's node chain, head first (a chain cut by a re-link ends in the warp list; cycles stop at the link count).
    fn chain(&self, kind: usize) -> impl Iterator<Item = Id> + '_ {
        let (mut cur, mut n) = (self.heads[kind], 0);
        std::iter::from_fn(move || {
            let id = cur?;
            cur = self.link.get(&id).and_then(|l| l.1);
            n += 1;
            (n <= self.link.len()).then_some(id)
        })
    }

    fn entry(&mut self, o: i32, cell: u32, x: i32, y: i32, flags: u32) -> Res<Id> {
        let (dt1, tile) = self.pick(o, cell)?;
        let flags = match o {
            0 => cell_bits(layer(cell), cell),
            13 => cell_bits(0, cell),
            _ => flags,
        };
        Ok(self.push(o, TileRef { x, y, dt1, tile, flags }))
    }

    /// 11 Orient89Unit on entry flags `f` (0 for a hidden cell with no entry): the row of the level and cell whose unit subtile lies inside the room marks the entry (`0x20`) and,
    /// for objects 91 and 92 (the Act II tomb doors), draws `Range(3)`; an entry already marked is skipped.
    fn orient89(&mut self, cell: u32, o: i32, x: i32, y: i32, f: u32) -> u32 {
        let Some(row) = orient89_row(self.level, cell, o == 9).filter(|_| f & 0x20 == 0) else { return f };
        let (sx, sy) = ((x - self.rect.x) * 5 + row[5], (y - self.rect.y) * 5 + row[6]);
        if !(0..self.rect.w * 5).contains(&sx) || !(0..self.rect.h * 5).contains(&sy) {
            return f;
        }
        if row[4] == 2 && (row[3] == 91 || row[3] == 92) {
            self.seed.range(3);
        }
        f | 0x20
    }

    /// 8a AddWall: the wall entry with its flags (9d; Orient89Unit first for orientations 8 and 9), and the orientation 4 partner of an orientation 3; `chain` puts them on the kind-0 node (9b).
    fn wall(&mut self, o: i32, cell: u32, x: i32, y: i32, chain: bool) -> Res<Id> {
        let (dt1, tile) = self.pick(o, cell)?;
        let f = wall_flags(if o == 8 || o == 9 { self.orient89(cell, o, x, y, 0) } else { 0 }, cell, o);
        let id = self.push(o, TileRef { x, y, dt1, tile, flags: f });
        if chain {
            self.prepend(id, o);
        }
        if o == 3 {
            let (dt1, tile) = self.pick(4, cell)?;
            let p = self.push(4, TileRef { x, y, dt1, tile, flags: f });
            if chain {
                self.prepend(p, 4);
            }
        }
        Ok(id)
    }

    /// 10: the warp id of the cell's slot and its LvlWarp row fitted to the orientation; `None` when the level has no such warp.
    fn warp_row(&self, cell: u32, o: i32) -> Option<(i32, i32, i32)> {
        let id = *self.warp.get(((cell >> 20) & 0x3F) as usize)?;
        let d = if o == 11 { b'r' } else { b'l' };
        self.md.t.warp.iter().find(|w| w.id == id && (w.direction == b'b' || w.direction == d)).map(|w| (id, w.lit_version, w.tiles))
    }

    /// 10a WarpTileAdd on the wall entry `id`.
    fn warp_tile(&mut self, cell: u32, o: i32, x: i32, y: i32, id: Id) -> Res<()> {
        let s = (cell >> 8) & 0xFF;
        let last = x - self.rect.x == self.rect.w || y - self.rect.y == self.rect.h;
        let Some((wid, lit, tiles)) = self.warp_row(cell, o).filter(|_| !((s == 0 || s == 4) && last)) else { return Ok(()) };
        self.relink(id, o, wid);
        if lit != 0 {
            let e = self.entry(o, cell | (tiles as u32) << 8, x, y, cell_bits(layer(cell) | 0x2, cell) | 0x8)?;
            self.out.lit.push(e);
        }
        Ok(())
    }

    /// 10b HiddenWarp: four lit floor tiles around the cell, then every floor of the room with tile `(s, sub < 4)` re-linked into the warp node's list.
    fn hidden_warp(&mut self, cell: u32, o: i32, x: i32, y: i32) -> Res<()> {
        if let Some((wid, lit, _)) = self.warp_row(cell, o) && lit != 0 {
            let s = (cell >> 8) & 0xFF;
            for (i, (dx, dy)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
                let c = ((s << 12) | i as u32 | 4) << 8;
                let id = self.entry(0, c, x - 1 + dx, y - 1 + dy, 0)?;
                if let Some(e) = self.out.get_mut(id) {
                    e.flags |= 0x8;
                }
                self.out.lit.push(id);
            }
            let ids: Vec<u32> = self.out.floors.iter().enumerate().filter(|(_, e)| self.pool.key(e).is_some_and(|(m, sub)| m == s as i32 && sub < 4)).map(|(i, _)| i as u32).collect();
            for i in ids {
                self.relink((0, i), 0, wid);
            }
        }
        Ok(())
    }

    /// 9b Bit2New.
    fn bit2_new(&mut self, o: i32, cell: u32, x: i32, y: i32) -> Res<()> {
        let inside = (self.rect.x..self.rect.x + self.rect.w).contains(&x) && (self.rect.y..self.rect.y + self.rect.h).contains(&y);
        if (o == 10 || o == 11) && !inside {
            return Ok(());
        }
        let id = match o {
            0 | 13 => self.entry(o, cell, x, y, 0)?,
            _ => self.wall(o, cell, x, y, true)?,
        };
        if o == 0 || o == 13 {
            self.prepend(id, o);
        }
        if o == 10 || o == 11 {
            self.warp_tile(cell, o, x, y, id)?;
        }
        Ok(())
    }

    /// 9a NeighbourTileScan: the first entry at `(x, y)` on the kind's chain of a built near room (the room is not an orientation 4 partner, a shadow cell takes only a shadow, the layer matches).
    fn scan(nb: &Near, kind: usize, x: i32, y: i32, cell: u32) -> Option<(usize, Id)> {
        nb.list.iter().filter_map(|&j| nb.built.get(j).map(|n| (j, n))).find_map(|(j, n)| {
            if !((n.rect.x..=n.rect.x + n.rect.w).contains(&x) && (n.rect.y..=n.rect.y + n.rect.h).contains(&y)) {
                return None;
            }
            n.chain(kind).find(|&id| {
                let (Some(e), Some(&(eo, _))) = (n.out.get(id), n.link.get(&id)) else { return false };
                e.x == x && e.y == y && eo != 4 && (eo == 13 || cell & 0x0800_0000 == 0) && (e.flags & 0x1C000 == 0 || ((e.flags >> 14) & 7) as i32 - 1 == ((cell >> 18) & 3) as i32)
            }).map(|id| (j, id))
        })
    }

    /// 9c Bit2Merge into entry `id` of the built neighbour `n`: a flagged (`0x1`) entry only takes the flags (Orient89Unit first for orientations 8 and 9); else the orientation remap
    /// (ROW and MERGE, skipped by cell bit 7), the orientation 3 branches, the re-pick on the neighbour's seed and list when the orientation differs or the entry is the blank floor, then the flags.
    fn merge(&mut self, n: &mut Build<'a>, id: Id, mut o: i32, cell: u32, x: i32, y: i32) -> Res<()> {
        let (Some(e), Some(&(eo, next))) = (n.out.get(id).copied(), n.link.get(&id)) else { return Ok(()) };
        if e.flags & 0x1 != 0 {
            if let (8 | 9, Some(e)) = (eo, n.out.get_mut(id)) {
                let f = self.orient89(cell, eo, x, y, e.flags);
                e.flags = wall_flags(f, cell, eo);
            }
            return Ok(());
        }
        if cell & 0x80 == 0 {
            let remap = if o == 8 || o == 9 {
                x != self.rect.x && y != self.rect.y
            } else if (eo == 8 || eo == 9) && (x == n.rect.x || y == n.rect.y) {
                return Ok(());
            } else {
                true
            };
            let tr = ROW.get(o as usize).copied().unwrap_or(-1);
            if remap && tr >= 0 && (0..=7).contains(&eo) {
                o = MERGE[(tr * 7 + eo) as usize];
            } else if remap && tr != -1 {
                return Ok(());
            }
        }
        let mut flags = e.flags;
        if eo != 3 && o == 3 {
            flags |= 0xC008;
            self.wall(3, cell, x, y, true)?;
        }
        if eo == 3 && o != 3 && let Some(nx) = next && let Some(ne) = n.out.get_mut(nx) {
            ne.flags |= 0x8;
        }
        let mut new = (e.dt1, e.tile, eo);
        if o != eo || (eo == 0 && n.pool.key(&e) == Some((30, 0))) {
            let (dt1, tile) = n.pool.pick(&mut n.seed, o, cell).ok_or(MapError::Unsupported(self.level, "tile pick: no candidate and no (10, 0, 0) tile"))?;
            new = (dt1, tile, o);
        }
        if new.2 == 8 || new.2 == 9 {
            flags = self.orient89(cell, new.2, x, y, flags);
        }
        if let Some(e) = n.out.get_mut(id) {
            (e.dt1, e.tile, e.flags) = (new.0, new.1, wall_flags(flags, cell, new.2));
        }
        n.link.insert(id, (new.2, next));
        Ok(())
    }

    /// 8 RoomCellBuild.
    fn cell(&mut self, nb: &mut Near<'_, 'a>, mut cell: u32, o: i32, x: i32, y: i32, filler: bool) -> Res<()> {
        let (m, s) = ((cell >> 20) & 0x3F, (cell >> 8) & 0xFF);
        if (o == 10 || o == 11) && m >= 8 {
            return Ok(());
        }
        if o == 0 && m == 30 && s < 2 {
            cell |= 0x8000_0000;
        }
        let hidden = cell & 0x8000_0000 != 0;
        if hidden && (o == 8 || o == 9) && ![111, 112, 117].contains(&self.level) {
            self.orient89(cell, o, x, y, 0);
            return Ok(());
        }
        if hidden && (o == 10 || o == 11) {
            return self.hidden_warp(cell, o, x, y);
        }
        if cell & 0x4 != 0 {
            let bit2 = if cell & 0x2 != 0 { Some((0, if m == 30 && s < 2 { cell & !0x80 } else { cell })) } else if cell & 0x1 != 0 { Some((o, cell)) } else if cell & 0x0800_0000 != 0 && !hidden { Some((13, cell)) } else { None };
            if let Some((o, c)) = bit2 {
                return match Self::scan(nb, usize::from(o == 0), x, y, c).and_then(|(j, id)| Some((nb.built.get_mut(j)?, id))) {
                    Some((n, id)) => self.merge(n, id, o, c, x, y),
                    None => self.bit2_new(o, c, x, y),
                };
            }
        }
        let inside = (self.rect.x..self.rect.x + self.rect.w).contains(&x) && (self.rect.y..self.rect.y + self.rect.h).contains(&y);
        if cell & 0x2 != 0 {
            self.entry(0, cell, x, y, 0)?;
        } else if filler && inside {
            self.entry(0, if self.level == 74 { 0x01E0_0100 } else { 0x01E0_0000 }, x, y, 0)?;
        }
        if cell & 0x1 != 0 {
            let id = self.wall(o, cell, x, y, false)?;
            if (o == 10 || o == 11) && self.level != 133 {
                self.warp_tile(cell, o, x, y, id)?;
            }
        }
        if cell & 0x0800_0000 != 0 {
            self.entry(13, cell, x, y, 0)?;
        }
        Ok(())
    }

    /// 7c RoomCellsBuild.
    fn cells(&mut self, nb: &mut Near<'_, 'a>, grid: &[u32], orient: Option<&[u8]>, filler: bool, (kx, ky): (bool, bool)) -> Res<()> {
        let (w, h, stride) = (self.rect.w, self.rect.h, self.rect.w as usize + 1);
        for j in 0..h + i32::from(!ky) {
            for i in 0..w + i32::from(!kx) {
                let k = j as usize * stride + i as usize;
                self.cell(nb, grid[k], orient.map_or(0, |o| i32::from(o[k])), self.rect.x + i, self.rect.y + j, filler)?;
            }
        }
        Ok(())
    }

    /// 5d Fits: every stamped cell needs a bare floor cell of the room and no wall.
    fn fits(&self, g: &Grids, sub: &Sub, (x, y): (usize, usize)) -> bool {
        let stride = self.rect.w as usize + 1;
        sub.cells().all(|(i, j, f, w, _, _)| {
            let k = (y + j) * stride + x + i;
            !(f & 2 != 0 || w & 1 != 0) || (g.floors[0][k] & 0x3F0_FF00 == 0 && g.floors[0][k] & 2 != 0 && g.walls[0].0[k] & 1 == 0)
        })
    }

    /// 5d Stamp, with the shadow picks of the prepare.
    fn stamp(&mut self, g: &mut Grids, sub: &Sub, (x, y): (usize, usize)) -> Res<()> {
        let stride = self.rect.w as usize + 1;
        for (i, j, f, w, q, sh) in sub.cells() {
            let k = (y + j) * stride + x + i;
            if f & 2 != 0 {
                g.floors[0][k] = f | 0x80;
            }
            if w & 1 != 0 {
                g.walls[0].0[k] = w;
            }
            if q != 0 {
                g.walls[0].1[k] = q;
            }
            if sh & 0x0800_0000 != 0 {
                self.entry(13, sh, self.rect.x + (x + i) as i32, self.rect.y + (y + j) as i32, 0)?;
            }
        }
        Ok(())
    }

    /// 5a to 5b LvlSubApply: the picks of the records named by `mask` (CheckAll records do not ship).
    fn sub_apply(&mut self, g: &mut Grids, kind: i32, theme: i32, mask: u32) -> Res<()> {
        let recs: Vec<&SubDef> = self.md.t.sub.iter().skip_while(|s| s.kind != kind).take_while(|s| s.kind == kind).collect();
        for rec in recs.iter().enumerate().filter(|&(i, _)| kind != -1 && mask >> i & 1 != 0).map(|(_, r)| r) {
            let ds = (self.md.ds1)(&rec.file).map_err(|e| MapError::Data(format!("{}: {}", rec.file, e.0)))?;
            let th = usize::try_from(theme).unwrap_or(0).min(4);
            if ds.groups.is_empty() || rec.max[th] <= 0 || rec.check_all != 0 {
                continue;
            }
            for _ in 0..rec.max[th] {
                let gr = ds.groups[self.seed.range(ds.groups.len() as i32) as usize];
                let sub = Sub { ds: &ds, g: gr };
                let (dx, dy) = (self.rect.w - gr[2], self.rect.h - gr[3]);
                if dx < 1 || dy < 1 {
                    continue;
                }
                let at = if rec.trials[th] == -1 {
                    let nn = dx * dy;
                    let mut list: Vec<(i32, i32)> = (0..nn).map(|i| (i % dx, i / dx)).collect();
                    for _ in 0..nn {
                        let (a, b) = (self.seed.range(nn) as usize, self.seed.range(nn) as usize);
                        list.swap(a, b);
                    }
                    list.into_iter().map(|(x, y)| ((x + 1) as usize, (y + 1) as usize)).find(|&p| self.fits(g, &sub, p))
                } else {
                    (0..rec.trials[th].max(0)).find_map(|_| {
                        let p = ((self.seed.range(dx) + 1) as usize, (self.seed.range(dy) + 1) as usize);
                        self.fits(g, &sub, p).then_some(p)
                    })
                };
                if let Some(p) = at {
                    self.stamp(g, &sub, p)?;
                }
            }
        }
        Ok(())
    }
}

/// One `LvlSub` group of a DS1: its cells as `(i, j, floor, wall0, orient0, shadow)` (cells outside the DS1 read 0).
struct Sub<'a> {
    ds: &'a d2data::ds1::Ds1,
    g: [i32; 5],
}

impl Sub<'_> {
    fn cells(&self) -> impl Iterator<Item = (usize, usize, u32, u32, u8, u32)> + '_ {
        let raw = |c: Option<&d2data::ds1::Cell>| c.map_or(0, |c| u32::from_le_bytes([c.prop1, c.prop2, c.prop3, c.prop4]));
        let (w, g) = (self.ds.w as usize, self.g);
        (0..g[3].max(0) as usize).flat_map(move |j| (0..g[2].max(0) as usize).map(move |i| (i, j))).map(move |(i, j)| {
            let k = (g[1] as usize + j) * w + g[0] as usize + i;
            (i, j, raw(self.ds.floors.first().and_then(|l| l.get(k))), raw(self.ds.walls.first().and_then(|l| l.get(k))), self.ds.orients.first().and_then(|l| l.get(k)).copied().unwrap_or(0), raw(self.ds.shadow.get(k)))
        })
    }
}

/// The theme fill value of an outdoor level type (Constants).
fn theme_fill(lt: i32, level: u32) -> u32 {
    match (lt, level) {
        (16, _) => 0x100,
        (21, _) => 0x12_0000,
        (22, _) => 0x10_0000,
        (27, _) => 0xA0_0000,
        (28, _) => 0x160_0000,
        (31, 117) => 0x60_0000,
        _ => 0,
    }
}

/// Loads DT1 files once and builds the room tiles of a level (steps 1 to 12; the DT1 lists, DS1 files and pools are shared between rooms).
pub struct Tiler<'a> {
    md: &'a MapData<'a>,
    load: &'a dyn Fn(&str) -> d2data::Result<Dt1>,
    files: HashMap<String, Rc<Dt1>>,
    pools: HashMap<(i32, u32), Rc<Pool>>,
}

impl<'a> Tiler<'a> {
    /// `load` reads a DT1 by its path under `data\global\tiles` (the `LvlTypes` cell).
    pub fn new(md: &'a MapData<'a>, load: &'a dyn Fn(&str) -> d2data::Result<Dt1>) -> Tiler<'a> {
        Tiler { md, load, files: HashMap::new(), pools: HashMap::new() }
    }

    /// Step 2 BuildDt1List: the level type's files by mask bit, then the three fixed files.
    pub fn pool(&mut self, level_type: i32, mask: u32) -> Res<Rc<Pool>> {
        if let Some(p) = self.pools.get(&(level_type, mask)) {
            return Ok(p.clone());
        }
        let ty = self.md.t.types.iter().find(|t| t.id == level_type).ok_or_else(|| MapError::Data(format!("level type {level_type}")))?;
        let names = (0..32).filter(|i| mask >> i & 1 != 0).filter_map(|i| ty.file.get(i)).filter(|f| !f.is_empty()).map(String::as_str);
        let list: Vec<&str> = names.chain(["Act1/Outdoors/Blank.dt1", "Act1/Barracks/InvisWal.dt1", "Act1/Barracks/Warp.dt1"]).collect();
        let mut dt1 = vec![];
        for n in list {
            if !self.files.contains_key(n) {
                let d = (self.load)(n).map_err(|e| MapError::Data(format!("{n}: {}", e.0)))?;
                self.files.insert(n.to_string(), Rc::new(d));
            }
            dt1.push(self.files[n].clone());
        }
        let p = Rc::new(Pool::new(dt1));
        self.pools.insert((level_type, mask), p.clone());
        Ok(p)
    }

    /// Steps 5 to 7 and 12 of room `room` of `level` built alone (no neighbour holds an entry, so every bit-2 cell is new); `warp` are the level's warp ids (the link list's, else the `Levels` row's).
    /// Returns the tiles and the DT1 list their `dt1` indices name.
    pub fn room(&mut self, level: &Level, warp: [i32; 8], room: &Room) -> Res<(RoomTiles, Rc<Pool>)> {
        let b = self.build(level, warp, room, &mut Near { built: &mut [], list: &[] })?;
        Ok((b.out, b.pool))
    }

    /// Steps 1 to 12 for every room of `level` in creation order, each room's bit-2 cells merging into the entries of the rooms built before it on its near list (9, `near`);
    /// the activation order is an input of the game (reimplementation notes), and this is the creation order. Returns each room's tiles and DT1 list in `Level::all` order.
    pub fn level(&mut self, level: &Level, warp: [i32; 8]) -> Res<Vec<(RoomTiles, Rc<Pool>)>> {
        let rooms = level.all();
        let mut built: Vec<Build<'a>> = Vec::with_capacity(rooms.len());
        for (i, room) in rooms.iter().enumerate() {
            let list: Vec<usize> = near(rooms, i).into_iter().filter(|&j| j < i).collect();
            let b = self.build(level, warp, room, &mut Near { built: &mut built, list: &list })?;
            built.push(b);
        }
        Ok(built.into_iter().map(|b| (b.out, b.pool)).collect())
    }

    fn build(&mut self, level: &Level, warp: [i32; 8], room: &Room, nb: &mut Near<'_, 'a>) -> Res<Build<'a>> {
        let md = self.md;
        let def = md.t.levels.iter().find(|l| l.id as u32 == level.id).ok_or_else(|| MapError::Data(format!("level {}", level.id)))?;
        let pool = self.pool(def.level_type, room.mask)?;
        let mut b = Build { pool, md, seed: Seed::init(room.seed.low), level: level.id, warp, rect: room.rect, out: RoomTiles::default(), link: HashMap::new(), heads: [None; 2], wheads: HashMap::new() };
        let (w, h) = (room.rect.w as usize, room.rect.h as usize);
        let stride = w + 1;
        let border = |k: usize| {
            let (i, j) = (k % stride, k / stride);
            i == 0 || j == 0 || i == w || j == h
        };
        if let Some((pdef, ps)) = room.ps {
            // 6 PresetRoomPrepare and 7b PresetRoomTilesBuild.
            let p = md.t.prest.iter().find(|p| p.def == pdef).ok_or_else(|| MapError::Data(format!("level {} def {pdef}", level.id)))?;
            let name = &room.ds1.as_ref().ok_or(MapError::Data("preset room without a file".into()))?.0;
            let ds = (md.ds1)(name).map_err(|e| MapError::Data(format!("{name}: {}", e.0)))?;
            let (ox, oy) = ((room.rect.x - ps.x) as usize, (room.rect.y - ps.y) as usize);
            let view = |layer: &[d2data::ds1::Cell]| -> Vec<u32> {
                (0..stride * (h + 1)).map(|k| (k % stride + ox, k / stride + oy)).map(|(i, j)| if i < ds.w as usize && j < ds.h as usize { layer.get(j * ds.w as usize + i).map_or(0, |c| u32::from_le_bytes([c.prop1, c.prop2, c.prop3, c.prop4])) } else { 0 }).collect()
            };
            let oview = |layer: &[u8]| -> Vec<u8> { (0..stride * (h + 1)).map(|k| (k % stride + ox, k / stride + oy)).map(|(i, j)| if i < ds.w as usize && j < ds.h as usize { layer.get(j * ds.w as usize + i).copied().unwrap_or(0) } else { 0 }).collect() };
            let with = |mut v: Vec<u32>, layer: u32, edge: bool| {
                v.iter_mut().enumerate().for_each(|(k, c)| *c |= layer << 18 | if edge && border(k) { 0x84 } else { 0 });
                v
            };
            let floors: Vec<Vec<u32>> = ds.floors.iter().enumerate().map(|(i, l)| with(view(l), i as u32, true)).collect();
            let walls: Vec<(Vec<u32>, Vec<u8>)> = ds.walls.iter().zip(&ds.orients).enumerate().map(|(i, (l, o))| (with(view(l), i as u32, i == 0), oview(o))).collect();
            let shadow = with(view(&ds.shadow), 0, true);
            let kill = |a: i32, b: i32| p.kill_edge != 0 && a == b;
            let k = (kill(room.rect.x + room.rect.w, ps.x + ps.w), kill(room.rect.y + room.rect.h, ps.y + ps.h));
            for (f, g) in floors.iter().enumerate() {
                b.cells(nb, g, None, f == 0 && p.fill_blanks != 0, k)?;
            }
            for (g, o) in &walls {
                b.cells(nb, g, Some(o), false, k)?;
            }
            b.cells(nb, &shadow, None, false, k)?;
        } else {
            // 5 OutRoomPrepare (M1 grids, the LvlSub passes, the theme fill, the borders) and 7a OutRoomTilesBuild.
            let roads = outdoor::road_floor(&level.roads, room.rect);
            let floor: Vec<u32> = if def.act == 0 { roads } else { (0..stride * stride).map(|k| if k % stride < w && k / stride < h { 0x40002 } else { 0 }).collect() };
            let mut g = Grids { floors: vec![floor], walls: vec![(vec![0; stride * stride], vec![0; stride * stride])] };
            for (kind, theme, mask) in [(def.sub_waypoint, 0, room.flags >> 16 & 3), (def.sub_shrine, 0, room.flags >> 12 & 0xF), room.sub] {
                b.sub_apply(&mut g, kind, theme, mask)?;
            }
            let v = theme_fill(def.level_type, level.id);
            for (k, c) in g.floors[0].iter_mut().enumerate() {
                if *c & 0x3F0_FF80 == 0 {
                    *c |= v;
                }
                if border(k) {
                    *c |= 0x4;
                }
            }
            g.walls[0].0.iter_mut().enumerate().filter(|&(k, _)| border(k)).for_each(|(_, c)| *c |= 0x4);
            b.cells(nb, &g.walls[0].0, Some(&g.walls[0].1), false, (false, false))?;
            b.cells(nb, &g.floors[0], None, false, (false, false))?;
        }
        // 12 ActivationDraw.
        if !b.out.floors.is_empty() || !b.out.walls.is_empty() {
            b.seed.step();
            b.out.active = Some(Seed::init(b.seed.low));
        }
        Ok(b)
    }
}
