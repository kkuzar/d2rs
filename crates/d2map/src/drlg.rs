//! Act, level and room model and the level dispatch (specs/drlg/README.md "Algorithm").
use crate::{links, maze, outdoor, preset, tiles::RoomTiles, warps::Warp, Seed, SeedTree};
use d2data::{ds1::Ds1, excel::MapTables};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Maze,
    Preset,
    Outdoor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub rect: Rect,
    /// The DS1 path and its index in the record's file list.
    pub ds1: Option<(String, u32)>,
    pub seed: Seed,
    pub tiles: Option<RoomTiles>,
    /// Maze rooms: the `Def` and the forced file index (-1 for none).
    pub maze: Option<(i32, i32)>,
    /// Outdoor levels: `(def, grid0 flags)` on the first room of each embedded preset.
    pub out: Option<(i32, u32)>,
    /// Room flags: `0x10 << m` warp slot, `0x30000` waypoint, outdoor rooms `grid0 | 0x80000` (warps.md D1).
    pub flags: u32,
    /// The room's DT1 mask (room-tiles.md step 2 note).
    pub mask: u32,
    /// Preset rooms: the instance's `Def` and rect (its origin and size, for the room's view of the DS1).
    pub ps: Option<(i32, Rect)>,
    /// Outdoor rooms: the `LvlSub` type, theme and pass mask the creation roll made (README step 8a).
    pub sub: (i32, i32, u32),
}

impl Room {
    /// A room with no file, flags or mask yet.
    pub fn new(rect: Rect, seed: Seed) -> Room {
        Room { rect, ds1: None, seed, tiles: None, maze: None, out: None, flags: 0, mask: 0, ps: None, sub: (-1, -1, 0) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    pub id: u32,
    pub rect: Rect,
    pub kind: Kind,
    pub rooms: Vec<Room>,
    /// Maze levels: the 8 x 8 preset rooms of README step 9a, in creation order (`rooms` holds the maze rooms the golden digests).
    pub parts: Vec<Room>,
    pub warps: Vec<Warp>,
    pub seed_after: Seed,
    /// Act I road polylines in map tiles (outdoor.md H6e), hub end first; the room prepare rasterises them (M2).
    pub roads: Vec<Vec<(i32, i32)>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    Unsupported(u32, &'static str),
    Data(String),
    ClassicNoActV,
}

pub struct MapData<'a> {
    pub t: &'a MapTables,
    pub ds1: &'a dyn Fn(&str) -> d2data::Result<Ds1>,
    /// From Game.exe; without it the preset levels that roll drops return `Unsupported`.
    pub ids: Option<&'a preset::Ids>,
}

impl Level {
    /// The rooms the game ends with: a maze level's 8 x 8 preset rooms (once `maze::parts` ran), else its own.
    pub fn all(&self) -> &[Room] {
        if self.kind == Kind::Maze { &self.parts } else { &self.rooms }
    }
}

/// False for level 134 (Forgotten Sands): outdoor.md A1 and I1 give its branch as a code reading only (no harness game reaches it), so no golden case or worked example pins its draws.
pub fn verified(level_id: u32) -> bool {
    level_id != 134
}

/// README steps 1-2: seed the level with `Init(base + id)` and dispatch on `DrlgType`; the rect, the town and level 27 indices and the neighbours of levels 28 and 107 come from the act links.
pub fn build_level(md: &MapData, map_seed: u32, diff: u8, expansion: bool, level_id: u32) -> Result<Level, MapError> {
    let act = md.t.levels.iter().find(|l| l.id as u32 == level_id).map_or(0, |l| l.act as u8);
    build_with(md, links::layout(md, map_seed, diff, act).ok().as_ref(), map_seed, diff, expansion, level_id)
}

/// `build_level` over an act layout already made (`None`: the `Levels` offsets only).
pub(crate) fn build_with(md: &MapData, lay: Option<&links::Layout>, map_seed: u32, diff: u8, expansion: bool, level_id: u32) -> Result<Level, MapError> {
    let def = md.t.levels.iter().find(|l| l.id as u32 == level_id).ok_or_else(|| MapError::Data(format!("level {level_id}")))?;
    if def.act == 4 && !expansion {
        return Err(MapError::ClassicNoActV);
    }
    let seed = SeedTree { map_seed }.level_seed(def.act as u8, level_id);
    let d = usize::from(diff.min(2));
    let rect = lay.and_then(|l| l.rect_of(md, level_id, d)).unwrap_or(Rect { x: def.offset_x, y: def.offset_y, w: def.size_x[d], h: def.size_y[d] });
    // C1: the index the link builder leaves on levels 1, 40 and 27; without a Black Marsh draw level 27 keeps its create-time index.
    let ov = lay.and_then(|l| match level_id {
        1 | 40 => l.town.get(&level_id).and_then(|&v| u32::try_from(v).ok()),
        _ => l.idx27.filter(|_| level_id == 27),
    });
    match def.drlg_type {
        2 => preset::build(md, level_id, rect, seed, ov),
        1 => {
            // G2, G3: the neighbour's rect and, for level 28, level 27's file index.
            let q27 = md.t.prest.iter().find(|p| p.level_id == 27);
            let k27 = lay.and_then(|l| l.idx27).or_else(|| q27.map(|p| SeedTree { map_seed }.level_seed(0, 27).range(p.files))).unwrap_or(0) as usize;
            let nb = lay.and_then(|l| l.rect_of(md, if level_id == 28 { 27 } else { 108 }, d)).map(|q| (q, k27));
            maze::build(md, level_id, rect, seed, d as u8, SeedTree { map_seed }.tombs(), nb)
        }
        3 => outdoor::build(md, map_seed, diff, level_id, rect, seed),
        _ => Err(MapError::Data(format!("DrlgType {}", def.drlg_type))),
    }
}
