//! Act builder, warp points and warp pairing (specs/drlg/warps.md A, E and F).
use crate::drlg::{build_with, Level, MapData, MapError, Rect};
use crate::tiles::within6;
use crate::{links, maze};
use std::collections::BTreeMap;

/// One warp point: the room centre in subtiles of a room with a warp (or waypoint) bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Warp {
    pub from_level: u32,
    /// `Vis` of the slot, or the level itself for a waypoint point.
    pub to_level: u32,
    pub pos: (i32, i32),
}

/// `(vis, warp)` of a level: the link builder's node, else the `Levels` row (C5).
type Links = ([i32; 8], [i32; 8]);

/// An act's levels, each built on its own (a failed level does not stop the others), and their link lists.
pub struct Act {
    pub levels: Vec<Result<Level, MapError>>,
    pub links: BTreeMap<u32, Links>,
}

/// E1 LevelWarpPointsBuild: rooms newest first; a room with a waypoint bit or a bit of a slot whose `Warp` is not -1.
pub fn points(level: &Level, (vis, warp): Links) -> Vec<Warp> {
    level
        .all()
        .iter()
        .rev()
        .filter_map(|r| {
            let slot = (0..8).find(|&m| r.flags & (0x10 << m) != 0 && warp[m] != -1);
            (slot.is_some() || r.flags & 0x30000 != 0).then(|| Warp {
                from_level: level.id,
                to_level: slot.map_or(level.id, |m| vis[m] as u32),
                pos: ((r.rect.x + r.rect.w / 2) * 5, (r.rect.y + r.rect.h / 2) * 5),
            })
        })
        .collect()
}

impl Act {
    /// F1 WarpPairLink for a slot `m` whose `Warp` is not -1: `(destination level, index of its room in `Level::all`)`, by rank, then the fallback walk; a seam (`Warp` -1) has no pair (`seam`).
    pub fn pair(&self, from: u32, m: usize) -> Option<(u32, usize)> {
        let (vis, warp) = self.links.get(&from)?;
        let dest = *vis.get(m)? as u32;
        let dl = self.links.get(&dest)?.0;
        let d = self.levels.iter().flatten().find(|l| l.id == dest)?;
        // FindRoom(D, j): the first room of D (newest first) with bit 0x10 << j.
        let find = |j: usize| d.all().iter().rposition(|r| r.flags & (0x10 << j) != 0).map(|r| (dest, r));
        if warp[m] == -1 {
            return None;
        }
        let k = (0..m).filter(|&j| vis[j] as u32 == dest).count();
        let named: Vec<usize> = (0..8).filter(|&j| dl[j] as u32 == from).collect();
        named.get(k).and_then(|&j| find(j)).or_else(|| named.iter().find_map(|&j| find(j)))
    }

    /// F1 with `Warp` -1 (a seam): no warp node; the fallback walk's `FindRoom` instead appends to the near list of the source room `room` every room of the destination
    /// (newest first) with bit `0x10 << j` whose gap to it is below 6 tiles on both axes, `j` the first slot of the destination naming `from` that has any (read as `FindRoom`
    /// reporting what it appended, as it does with a `Warp`; stock levels name a seam neighbour in one slot). `(destination level, index of its room in `Level::all`)`.
    pub fn seam(&self, from: u32, m: usize, room: Rect) -> Vec<(u32, usize)> {
        let Some(&(vis, warp)) = self.links.get(&from) else { return vec![] };
        let dest = vis.get(m).copied().unwrap_or(0) as u32;
        let (Some(dl), Some(d)) = (self.links.get(&dest).map(|l| l.0), self.levels.iter().flatten().find(|l| l.id == dest)) else { return vec![] };
        if warp.get(m) != Some(&-1) {
            return vec![];
        }
        (0..8)
            .filter(|&j| dl[j] as u32 == from)
            .map(|j| d.all().iter().enumerate().rev().filter(|(_, r)| r.flags & (0x10 << j) != 0 && within6(room, r.rect)).map(|(i, _)| (dest, i)).collect::<Vec<_>>())
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    }
}

/// A1 and README step 2 for every level of the 0-based `act` (level 0, "None", has no generator): the link builder first, then each level with its maze rooms cut (9a) and its warp points (E1).
pub fn build_act(md: &MapData, map_seed: u32, diff: u8, expansion: bool, act: u8) -> Result<Act, MapError> {
    let lay = links::layout(md, map_seed, diff, act)?;
    let ids: Vec<u32> = md.t.levels.iter().filter(|l| l.act == i32::from(act) && l.drlg_type != 0).map(|l| l.id as u32).collect();
    let links_of = |id: u32| lay.link.get(&id).copied().or_else(|| md.t.levels.iter().find(|l| l.id as u32 == id).map(|l| (l.vis, l.warp)));
    let levels = ids
        .iter()
        .map(|&id| {
            let mut l = build_with(md, Some(&lay), map_seed, diff, expansion, id)?;
            if l.kind == crate::Kind::Maze {
                l.parts = maze::parts(md, &l)?;
            }
            l.warps = links_of(id).map(|k| points(&l, k)).unwrap_or_default();
            Ok(l)
        })
        .collect();
    Ok(Act { levels, links: ids.iter().filter_map(|&id| links_of(id).map(|k| (id, k))).collect() })
}
