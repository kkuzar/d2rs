//! Preset levels: instance, file index and 8x8 room cut (specs/drlg/preset.md, README steps 4-6).
use crate::drlg::{Kind, Level, MapData, MapError, Rect, Room};
use crate::{seeds::room_seed, Seed};
use d2data::{ds1::Ds1, excel::PrestDef, GameData};
use std::{collections::HashMap, path::Path};

const NMONSTATS: u32 = 734;
const NSUPERUNIQUES: u32 = 66;

/// The two tables `MapId` needs: `g_Ds1ObjectIds` and the mapped id of each act's MonPreset list.
pub struct Ids {
    pub obj: [[i32; 150]; 5],
    pub mon: [Vec<i32>; 5],
}

impl Ids {
    /// Read the object table from a 1.14d `Game.exe` and resolve MonPreset.txt (preset.md step 1a, type 1).
    pub fn load(gd: &GameData, exe: &Path) -> Result<Ids, MapError> {
        let data = |e: String| MapError::Data(e);
        let bytes = d2host::read(exe).map_err(|e| data(format!("{}: {e}", exe.display())))?;
        let obj = d2data::exe::ds1_object_ids(&bytes).map_err(|e| data(format!("{}: {}", exe.display(), e.0)))?;
        let col = |file: &str, head: &str| -> Result<Vec<String>, MapError> {
            let b = gd.read(&format!("data\\global\\excel\\{file}.txt")).ok_or_else(|| data(file.to_string()))?;
            let t = String::from_utf8_lossy(&b).into_owned();
            let mut lines = t.lines();
            let i = lines.next().and_then(|h| h.split('\t').position(|c| c == head)).ok_or_else(|| data(format!("{file}.{head}")))?;
            Ok(lines.filter(|l| !l.starts_with("Expansion\t")).map(|l| l.split('\t').nth(i).unwrap_or("").to_lowercase()).collect())
        };
        let (ms, su, mp) = (col("monstats", "Id")?, col("superuniques", "Superunique")?, col("monplace", "code")?);
        let find = |v: &Vec<String>, n: &str, base: u32| v.iter().position(|x| x == n).map(|i| (base + i as u32) as i32);
        let mut mon: [Vec<i32>; 5] = Default::default();
        let (pa, pp) = (col("monpreset", "Act")?, col("monpreset", "Place")?);
        for (a, n) in pa.iter().zip(&pp) {
            let id = find(&ms, n, 0).or_else(|| find(&su, n, NMONSTATS)).or_else(|| find(&mp, n, NMONSTATS + NSUPERUNIQUES)).unwrap_or(-1);
            if let Some(v) = a.trim().parse::<usize>().ok().and_then(|a| mon.get_mut(a.wrapping_sub(1))) {
                v.push(id);
            }
        }
        Ok(Ids { obj, mon })
    }

    /// preset.md step 1a: the mapped `(type, id)` of a DS1 entry, `None` when the game skips it.
    fn map(&self, act: usize, version: u32, o: &d2data::ds1::Ds1Object) -> Option<(i32, u32)> {
        let id = match o.kind {
            1 if version < 5 => -1,
            1 => *self.mon[act].get(usize::try_from(o.id).ok()?)?,
            2 if version >= 6 => if (0..150).contains(&o.id) { self.obj[act][o.id as usize] } else { o.id - 150 },
            _ => o.id,
        };
        u32::try_from(id).ok().map(|id| (o.kind, id))
    }
}

/// preset.md step 3a: `Some(keep)` for a drawing unit `(type, id)` given the draw `r`, `None` for a unit that never draws.
fn keep_test(kind: i32, id: u32) -> Option<fn(u32) -> bool> {
    match (kind, id) {
        (1, 204 | 205 | 371 | 372) => Some(|r| r % 3 == 0),
        (1, i) if i >= NMONSTATS + NSUPERUNIQUES => match i - NMONSTATS - NSUPERUNIQUES {
            33 => Some(|r| r & 3 != 0),
            34 => Some(|r| r & 1 != 0),
            35 => Some(|r| r & 3 == 0),
            _ => None,
        },
        (2, 196 | 261) => Some(|r| r & 1 == 0),
        (2, 581) => Some(|r| r & 3 != 0),
        _ => None,
    }
}

/// preset.md step 3: one draw per drawing unit, walking `(type, id)` units from the list head (reverse file order); returns the keep flags in walk order.
pub fn drop_rolls(seed: &mut Seed, units_head_first: &[(i32, u32)]) -> Vec<bool> {
    units_head_first.iter().map(|&(k, id)| keep_test(k, id).is_none_or(|t| t(seed.step()))).collect()
}

/// Objects.txt classes with `SubClass` bit 6, the waypoints the map scan marks (warps.md D1).
const WAYPOINTS: [u32; 16] = [119, 145, 156, 157, 237, 238, 288, 323, 324, 398, 402, 429, 494, 496, 511, 539];

/// warps.md D1: the warp-slot and waypoint bits of each 8 x 8 cell of a DS1 (`Scan` set), keyed by cell.
fn scan_flags(ds1: &Ds1, ids: &Ids) -> HashMap<(i32, i32), u32> {
    let mut g: HashMap<(i32, i32), u32> = HashMap::new();
    let w = ds1.w.max(1) as usize;
    for (cells, ors) in ds1.walls.iter().zip(&ds1.orients) {
        for (i, (c, &o)) in cells.iter().zip(ors).enumerate() {
            if (o == 10 || o == 11) && c.main() < 8 && (c.sub() == 0 || c.sub() == 4 || c.hidden()) {
                *g.entry(((i % w) as i32 / 8, (i / w) as i32 / 8)).or_default() |= 0x10 << c.main();
            }
        }
    }
    for o in &ds1.objects {
        if matches!(ids.map(ds1.act.min(4) as usize, ds1.version, o), Some((2, id)) if WAYPOINTS.contains(&id)) {
            *g.entry((o.x / 5 / 8, o.y / 5 / 8)).or_default() |= 0x30000;
        }
    }
    g
}

/// README step 4: the file index draw, one level draw iff `Files >= 1`.
pub fn file_draw(p: &PrestDef, seed: &mut Seed) -> u32 {
    seed.range(p.files)
}

/// README step 5 for one instance of record `p` over `at` (the record's size when set) with file index `idx`: the unit drop rolls (5a), the room cut (5b) and each room's flags and DT1 mask.
#[allow(clippy::too_many_arguments)]
pub fn rooms(md: &MapData, level_id: u32, p: &PrestDef, at: Rect, idx: u32, flags: u32, single: bool, seed: &mut Seed) -> Result<Vec<Room>, MapError> {
    let name = p.file.get(idx as usize).filter(|n| !n.is_empty()).ok_or_else(|| MapError::Data(format!("level {level_id} def {} file {idx}", p.def)))?;
    let mut grid = HashMap::new();
    // Step 5a: Scan or Pops rolls the drawing units on the level seed (preset.md step 2a), walking the list from the last entry.
    if p.scan != 0 || p.pops != 0 {
        let ids = md.ids.ok_or(MapError::Unsupported(level_id, "preset unit drop rolls need Game.exe"))?;
        let ds1 = (md.ds1)(name).map_err(|e| MapError::Data(format!("{name}: {}", e.0)))?;
        let units: Vec<_> = ds1.objects.iter().rev().filter_map(|o| ids.map(ds1.act.min(4) as usize, ds1.version, o)).collect();
        drop_rolls(seed, &units);
        if p.scan != 0 {
            grid = scan_flags(&ds1, ids);
        }
    }
    let (w, h) = if p.size_x != 0 && p.size_y != 0 { (p.size_x, p.size_y) } else { (at.w, at.h) };
    // Step 5b: rows outer, columns inner, one level draw per room; a single room covers the instance and reads cell (0, 0).
    let cuts: Vec<(i32, i32)> = if single { vec![(0, 0)] } else { (0..h).step_by(8).flat_map(|y| (0..w).step_by(8).map(move |x| (x, y))).collect() };
    Ok(cuts
        .into_iter()
        .map(|(x0, y0)| {
            let rect = if single { Rect { x: at.x, y: at.y, w, h } } else { Rect { x: at.x + x0, y: at.y + y0, w: (w - x0).min(8), h: (h - y0).min(8) } };
            let mut r = Room::new(rect, room_seed(seed));
            (r.ds1, r.mask, r.flags, r.ps) = (Some((name.clone(), idx)), p.dt1_mask, flags | grid.get(&(x0 / 8, y0 / 8)).copied().unwrap_or(0), Some((p.def, Rect { x: at.x, y: at.y, w, h })));
            r
        })
        .collect())
}

/// README steps 4-5: the file index draw (`ov` replaces the drawn index), then the rooms.
pub fn instance(md: &MapData, level_id: u32, p: &PrestDef, at: Rect, ov: Option<u32>, flags: u32, seed: &mut Seed) -> Result<Vec<Room>, MapError> {
    let drawn = file_draw(p, seed);
    rooms(md, level_id, p, at, ov.unwrap_or(drawn), flags, false, seed)
}

/// README steps 3-6 for a preset level (`Def` of its record, one instance over the level rect); `ov` is the index the act links left (levels 1, 27, 40).
pub fn build(md: &MapData, level_id: u32, lrect: Rect, mut seed: Seed, ov: Option<u32>) -> Result<Level, MapError> {
    let p = md.t.prest.iter().find(|p| p.level_id as u32 == level_id).ok_or_else(|| MapError::Data(format!("no LvlPrest for level {level_id}")))?;
    let rooms = instance(md, level_id, p, lrect, ov, 0, &mut seed)?;
    Ok(Level { id: level_id, rect: lrect, kind: Kind::Preset, rooms, parts: vec![], warps: vec![], seed_after: seed, roads: vec![] })
}
