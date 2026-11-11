//! The collision grid of a room from its tiles and its near rooms' (specs/drlg/collision.md A and D).
use crate::drlg::{Level, Room};
use crate::tiles::{self, Pool, RoomTiles};
use d2data::dt1::Dt1;
use std::{borrow::Borrow, rc::Rc};

/// A2 and A3: `w * 5` by `h * 5` u16 words, row-major; each tile of `near` (the room's own tiles and its active near rooms', each with its DT1 list) whose top-left sub-tile
/// is inside the room ORs its 25 bytes (byte row 4 to map row 0) and the entry-flag bits (`0x02` to `0x10`, `0x40` to `0x01`, `0x80` to `0x04`) over its 5 x 5 block (D).
pub fn grid<D: Borrow<Dt1>>(room: &Room, near: &[(&RoomTiles, &[D])]) -> Vec<u16> {
    let (w, h) = (room.rect.w * 5, room.rect.h * 5);
    let mut g = vec![0u16; (w * h).max(0) as usize];
    for (t, dt1) in near {
        for e in t.floors.iter().chain(&t.walls).chain(&t.roofs).chain(&t.shadows) {
            let (sx, sy) = ((e.x - room.rect.x) * 5, (e.y - room.rect.y) * 5);
            let Some(tile) = dt1.get(usize::from(e.dt1)).and_then(|d| d.borrow().tiles.get(e.tile as usize)) else { continue };
            if !(0..w).contains(&sx) || !(0..h).contains(&sy) {
                continue;
            }
            let v = u16::from(e.flags & 0x02 != 0) << 4 | u16::from(e.flags & 0x40 != 0) | u16::from(e.flags & 0x80 != 0) << 2;
            for r in 0..5 {
                for c in 0..5 {
                    g[((sy + r) * w + sx + c) as usize] |= u16::from(tile.sub_flags[((4 - r) * 5 + c) as usize]) | v;
                }
            }
        }
    }
    g
}

/// A1 and A2 for every room of `level` in creation order: its own tiles and those of the rooms built before it on its near list (`tiles::near`), as `Tiler::level` built them.
pub fn grids(level: &Level, built: &[(RoomTiles, Rc<Pool>)]) -> Vec<Vec<u16>> {
    let rooms = level.all();
    let at = |j: usize| built.get(j).map(|(t, p)| (t, &p.dt1[..]));
    rooms.iter().enumerate().map(|(i, r)| grid(r, &std::iter::once(i).chain(tiles::near(rooms, i).into_iter().filter(|&j| j < i)).filter_map(at).collect::<Vec<_>>())).collect()
}
