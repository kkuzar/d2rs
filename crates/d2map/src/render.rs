//! Pictures of a built level: the collision map and the isometric tile picture (dt1.md "Pixel data", ds1.md "Preview" for the layer order).
use crate::drlg::{Level, Rect, Room};
use crate::tiles::{RoomTiles, TileRef};
use d2data::{dt1::Dt1, Palette};
use std::{borrow::Borrow, collections::HashMap, io, path::Path};

/// Row-major RGBA8.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Collision sub-cell edge in pixels (an act picture over the size guard takes 2, then 1).
const SCALE: i32 = 4;
const MAX_BYTES: i64 = 256 << 20;
pub const VOID: [u8; 4] = [24, 24, 32, 255];
pub const WALK: [u8; 4] = [220, 220, 205, 255];
pub const BLOCKED: [u8; 4] = [40, 40, 48, 255];
pub const EDGE: [u8; 4] = [60, 120, 220, 255];
pub const WARP: [u8; 4] = [220, 40, 40, 255];
pub const WAYPOINT: [u8; 4] = [40, 200, 240, 255];
pub const LABEL: [u8; 4] = [240, 200, 40, 255];

fn image(w: i64, h: i64, fill: [u8; 4]) -> io::Result<Image> {
    image_max(w, h, fill, MAX_BYTES)
}

fn image_max(w: i64, h: i64, fill: [u8; 4], max: i64) -> io::Result<Image> {
    if w <= 0 || h <= 0 || w * h * 4 > max {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("image {w} x {h} is empty or over 256 MB")));
    }
    Ok(Image { w: w as u32, h: h as u32, rgba: fill.repeat((w * h) as usize) })
}

impl Image {
    fn put(&mut self, x: i64, y: i64, c: [u8; 4]) {
        if (0..i64::from(self.w)).contains(&x) && (0..i64::from(self.h)).contains(&y) {
            let o = ((y * i64::from(self.w) + x) * 4) as usize;
            self.rgba[o..o + 4].copy_from_slice(&c);
        }
    }

    fn square(&mut self, (cx, cy): (i64, i64), r: i64, c: [u8; 4]) {
        (cy - r..=cy + r).for_each(|y| (cx - r..=cx + r).for_each(|x| self.put(x, y, c)));
    }
}

fn marker(level: &Level, w: &crate::Warp) -> [u8; 4] {
    if w.to_level == level.id { WAYPOINT } else { WARP }
}

/// The bounding rect of rects, in tiles.
fn span(rs: impl Iterator<Item = Rect> + Clone) -> Rect {
    let (x0, y0) = (rs.clone().map(|q| q.x).min().unwrap_or(0), rs.clone().map(|q| q.y).min().unwrap_or(0));
    Rect { x: x0, y: y0, w: rs.clone().map(|q| q.x + q.w).max().unwrap_or(0) - x0, h: rs.map(|q| q.y + q.h).max().unwrap_or(0) - y0 }
}

fn shifted(l: &Level, (dx, dy): (i32, i32)) -> Vec<Rect> {
    l.all().iter().map(|q| Rect { x: q.rect.x + dx, y: q.rect.y + dy, ..q.rect }).collect()
}

/// The tile rect a collision picture of `levels` covers and its pixels per sub-cell.
fn frame(levels: &[Placed]) -> (Rect, i32) {
    let r = span(levels.iter().flat_map(|&(l, o)| shifted(l, o)));
    (r, [SCALE, 2, 1].into_iter().find(|&k| i64::from(r.w * 5 * k) * i64::from(r.h * 5 * k) * 4 <= MAX_BYTES).unwrap_or(1))
}

/// A name to draw over the pictures: `coll` in collision pixels, `iso` in isometric screen coordinates (subtract the picture's origin `lo` from `act_level`; the unscaled picture then has them).
pub struct Label {
    pub text: String,
    pub coll: (f32, f32),
    pub iso: (f32, f32),
    /// A warp destination (else a level name).
    pub warp: bool,
}

/// The level names (at each level's centre, when `centres`) and the destination name of each warp that leaves its level.
pub fn labels(levels: &[Placed], name: &dyn Fn(u32) -> String, centres: bool) -> Vec<Label> {
    let (r, sc) = frame(levels);
    let at = |text: String, x: f32, y: f32, warp: bool| Label { text, coll: ((x - r.x as f32) * 5.0 * sc as f32, (y - r.y as f32) * 5.0 * sc as f32), iso: ((x - y) * 80.0 + 80.0, (x + y) * 40.0 + 40.0), warp };
    let mut out = vec![];
    for &(l, (dx, dy)) in levels {
        let q = span(shifted(l, (dx, dy)).into_iter());
        out.extend(centres.then(|| at(name(l.id), q.x as f32 + q.w as f32 / 2.0, q.y as f32 + q.h as f32 / 2.0, false)));
        out.extend(l.warps.iter().filter(|w| w.to_level != w.from_level).map(|w| at(name(w.to_level), w.pos.0 as f32 / 5.0 + dx as f32, w.pos.1 as f32 / 5.0 + dy as f32, true)));
    }
    out
}

/// Collision pictures of levels at their act positions; `act` adds a `LABEL` marker at each level's centre. Each level is outlined, warps red, the waypoint cyan.
fn paint(levels: &[Placed], grids: &[&[Vec<u16>]], act: bool) -> io::Result<Image> {
    let (r, sc) = frame(levels);
    let mut img = image(i64::from(r.w * 5 * sc), i64::from(r.h * 5 * sc), VOID)?;
    for (&(level, (dx, dy)), grids) in levels.iter().zip(grids) {
        for (room, g) in level.all().iter().zip(*grids) {
            let (gw, ox, oy) = (room.rect.w * 5, (room.rect.x + dx - r.x) * 5, (room.rect.y + dy - r.y) * 5);
            for (i, &v) in g.iter().enumerate() {
                let (cx, cy) = ((i as i32 % gw + ox) * sc, (i as i32 / gw + oy) * sc);
                let c = if v & 1 != 0 { BLOCKED } else { WALK };
                (0..sc).for_each(|d| (0..sc).for_each(|e| img.put(i64::from(cx + e), i64::from(cy + d), c)));
            }
        }
    }
    let px = |t: i32| i64::from(t * 5 * sc);
    for &(level, (dx, dy)) in levels {
        let q = span(shifted(level, (dx, dy)).into_iter());
        let (x0, y0, x1, y1) = (px(q.x - r.x), px(q.y - r.y), px(q.x + q.w - r.x) - 1, px(q.y + q.h - r.y) - 1);
        (x0..=x1).for_each(|x| [y0, y1].iter().for_each(|&y| img.put(x, y, EDGE)));
        (y0..=y1).for_each(|y| [x0, x1].iter().for_each(|&x| img.put(x, y, EDGE)));
        if act {
            img.square(((x0 + x1) / 2, (y0 + y1) / 2), 6, LABEL);
        }
        for wp in &level.warps {
            img.square((i64::from((wp.pos.0 + dx * 5 - r.x * 5) * sc), i64::from((wp.pos.1 + dy * 5 - r.y * 5) * sc)), 6, marker(level, wp));
        }
    }
    Ok(img)
}

/// Top-down map, `SCALE` pixels per collision sub-cell over the bounding box of the rooms: walkable light, blocked (bit 0) dark, no room void; the edge outlined, warps red, the waypoint cyan.
/// `grids` are the `collision::grid` words of `level.all()`, in that order.
pub fn collision(level: &Level, grids: &[Vec<u16>]) -> io::Result<Image> {
    paint(&[(level, (0, 0))], &[grids], false)
}

/// The same picture of several levels at their placed positions (`grids[i]` for `levels[i]`), outlined one by one, a yellow square (no text drawing: the names are in `act_json`) at each level's centre.
pub fn act_collision(levels: &[Placed], grids: &[Vec<Vec<u16>>]) -> io::Result<Image> {
    paint(levels, &grids.iter().map(Vec::as_slice).collect::<Vec<_>>(), true)
}

/// A level and the tile shift that places it on the picture.
pub type Placed<'a> = (&'a Level, (i32, i32));

/// Tiles between two clusters on the picture.
const GAP: i32 = 16;

/// The surface of an act: the town, the outdoor levels and every level reached from them by seams (a link slot with a `Vis` level and `Warp` -1), in clusters of seam-linked levels. A cluster keeps the act positions of its levels;
/// the clusters (the town side and the far side reached by a warp, e.g. Act I's Dark Wood) are shifted along x, left to right, to sit `GAP` tiles apart instead of ~2000 tiles. Other warps lead out of the surface (dungeons).
pub fn surface(act: &crate::Act) -> Vec<Placed<'_>> {
    let lv = |id: u32| act.levels.iter().flatten().find(|l| l.id == id);
    let roots = act.levels.iter().flatten().enumerate().filter(|&(i, l)| i == 0 || l.kind == crate::Kind::Outdoor && act.links.get(&l.id).is_some_and(|(v, w)| (0..8).any(|m| v[m] > 0 && w[m] == -1))).map(|(_, l)| l.id);
    let (mut seen, mut clusters): (Vec<u32>, Vec<Vec<&Level>>) = (vec![], vec![]);
    for root in roots.filter(|r| lv(*r).is_some()) {
        if seen.contains(&root) {
            continue;
        }
        let mut ids = vec![root];
        let mut i = 0;
        while let Some(&id) = ids.get(i) {
            i += 1;
            let (vis, warp) = act.links.get(&id).copied().unwrap_or_default();
            let next: Vec<u32> = (0..8).filter(|&m| vis[m] > 0 && warp[m] == -1).map(|m| vis[m] as u32).filter(|v| !ids.contains(v) && lv(*v).is_some()).collect();
            ids.extend(next);
        }
        seen.extend(&ids);
        clusters.push(ids.iter().filter_map(|&i| lv(i)).collect());
    }
    let x = |c: &[&Level], f: fn(i32, i32) -> i32, e: i32| c.iter().map(|l| l.rect.x + e * l.rect.w).reduce(f).unwrap_or(0);
    clusters.sort_by_key(|c| x(c, i32::min, 0));
    let mut edge = i32::MIN;
    let mut out = vec![];
    for c in clusters {
        let dx = if edge == i32::MIN { 0 } else { edge + GAP - x(&c, i32::min, 0) };
        edge = x(&c, i32::max, 1) + dx;
        out.extend(c.into_iter().map(|l| (l, (dx, 0))));
    }
    out
}

/// The warps of `levels` that lead out of them (dungeon entrances; waypoints lead to their own level).
fn entrances<'a>(levels: &'a [Placed]) -> impl Iterator<Item = (&'a crate::Warp, (i32, i32))> {
    levels.iter().flat_map(|&(l, o)| l.warps.iter().map(move |w| (w, o))).filter(|(w, _)| w.to_level != w.from_level && !levels.iter().any(|l| l.0.id == w.to_level))
}

/// `levels` of an act (with their shifts: positions on the picture are rect plus shift), their dungeon entrances and the isometric `scale` (`None`: no isometric picture) as JSON; `name` is the display name of a level id.
pub fn act_json(levels: &[Placed], name: &dyn Fn(u32) -> String, scale: Option<u32>) -> String {
    let lv: Vec<_> = levels.iter().map(|&(l, o)| format!("{{\"id\":{},\"name\":{:?},\"verified\":{},\"rect\":[{},{},{},{}],\"shift\":[{},{}]}}", l.id, name(l.id), crate::verified(l.id), l.rect.x, l.rect.y, l.rect.w, l.rect.h, o.0, o.1)).collect();
    let en: Vec<_> = entrances(levels).map(|(w, o)| format!("{{\"from\":{},\"to\":{},\"name\":{:?},\"x\":{},\"y\":{}}}", w.from_level, w.to_level, name(w.to_level), w.pos.0 + o.0 * 5, w.pos.1 + o.1 * 5)).collect();
    format!("{{\"levels\":[{}],\"entrances\":[{}],\"scale\":{}}}", lv.join(","), en.join(","), scale.map_or("null".into(), |k| k.to_string()))
}

type Pix = (i32, i32, u32, u32, Vec<u8>);

/// The isometric picture: tile `(x, y)` has its origin at `((x - y) * 80, (x + y) * 40)`; floors, then shadows (darkening), then walls row by row (`y` outer, `x` inner), then roofs, skipping hidden entries (entry flag 0x8, from cell bit 31; room-tiles.md 9d)
/// except the lit warp tiles (`RoomTiles::lit`, 10a and 10b: what the game shows when the player comes near), drawn after the other entries of their kind (ds1.md "Preview" layer order,
/// no roof lift). `rooms` pair each room's tiles with the DT1 list their `dt1` indices name. Warps and the waypoint are 24 pixel squares at their room centre.
pub fn level<D: Borrow<Dt1>>(level: &Level, rooms: &[(&RoomTiles, &[D])], pal: &Palette) -> io::Result<Image> {
    match level_fit(level, rooms, pal, MAX_BYTES)? {
        (img, 1) => Ok(img),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "image is over 256 MB")),
    }
}

/// Like `level`, but a picture over `max_bytes` is drawn at the smallest power of two downscale `k` that fits (every k-th pixel in both directions, nearest); returns the image and `k`.
pub fn level_fit<D: Borrow<Dt1>>(level: &Level, rooms: &[(&RoomTiles, &[D])], pal: &Palette, max_bytes: i64) -> io::Result<(Image, u32)> {
    act_level(&[(level, (0, 0))], &[rooms], pal, max_bytes).map(|(img, k, _)| (img, k))
}

/// The isometric picture of several levels (`rooms[i]` for `levels[i]`) on one canvas, drawn like `level_fit` with each level's tiles and warps shifted by its placement; also returns the screen position `lo` of its top-left corner.
pub fn act_level<D: Borrow<Dt1>>(levels: &[Placed], rooms: &[&[(&RoomTiles, &[D])]], pal: &Palette, max_bytes: i64) -> io::Result<(Image, u32, (i64, i64))> {
    let warps = || levels.iter().flat_map(|&(l, o)| l.warps.iter().map(move |w| (w, o, marker(l, w))));
    let mut order: Vec<(u8, bool, TileRef, &Dt1)> = vec![];
    for (&(_, (dx, dy)), rooms) in levels.iter().zip(rooms) {
    for (t, d) in *rooms {
        for (k, l, v) in [(0u8, 0u8, &t.floors), (1, 3, &t.shadows), (2, 1, &t.walls), (3, 2, &t.roofs)] {
            let lit = |i: usize| t.lit.contains(&(l, i as u32));
            order.extend(v.iter().enumerate().filter(|&(i, e)| e.flags & 0x8 == 0 || lit(i)).filter_map(|(i, e)| Some((k, lit(i), TileRef { x: e.x + dx, y: e.y + dy, ..*e }, d.get(usize::from(e.dt1))?.borrow()))));
        }
    }
    }
    order.sort_by_key(|&(k, lit, e, _)| (k, if k >= 2 { (e.y, e.x) } else { (0, 0) }, lit));
    let mut cache: HashMap<(usize, u32), Pix> = HashMap::new();
    let key = |e: TileRef, d: &Dt1| (d as *const Dt1 as usize, e.tile);
    let (mut lo, mut hi) = ((i64::MAX, i64::MAX), (i64::MIN, i64::MIN));
    let mut grow = |x0: i64, y0: i64, x1: i64, y1: i64| (lo, hi) = ((lo.0.min(x0), lo.1.min(y0)), (hi.0.max(x1), hi.1.max(y1)));
    for &(_, _, e, d) in &order {
        let p = cache.entry(key(e, d)).or_insert_with(|| {
            let (x0, y0, w, h, idx) = d.tiles.get(e.tile as usize).map_or((0, 0, 0, 0, vec![]), |t| t.pixels());
            (x0, y0, w, h, pal.rgba(&idx, None))
        });
        if p.2 > 0 && p.3 > 0 {
            let (sx, sy) = (i64::from((e.x - e.y) * 80 + p.0), i64::from((e.x + e.y) * 40 + p.1));
            grow(sx, sy, sx + i64::from(p.2), sy + i64::from(p.3));
        }
    }
    let centre = |w: &crate::Warp, (dx, dy): (i32, i32)| {
        let (x, y) = (w.pos.0 / 5 + dx, w.pos.1 / 5 + dy);
        (i64::from((x - y) * 80 + 80), i64::from((x + y) * 40 + 40))
    };
    for (w, o, _) in warps() {
        let (cx, cy) = centre(w, o);
        grow(cx - 12, cy - 12, cx + 13, cy + 13);
    }
    if lo.0 > hi.0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nothing to draw"));
    }
    let mut sc = 1;
    while (hi.0 - lo.0 + sc - 1) / sc * ((hi.1 - lo.1 + sc - 1) / sc) * 4 > max_bytes && sc < 1 << 20 {
        sc *= 2;
    }
    let mut img = image_max((hi.0 - lo.0 + sc - 1) / sc, (hi.1 - lo.1 + sc - 1) / sc, [0; 4], max_bytes)?;
    for &(k, _, e, d) in &order {
        let Some(p) = cache.get(&key(e, d)) else { continue };
        let (sx, sy) = (i64::from((e.x - e.y) * 80 + p.0) - lo.0, i64::from((e.x + e.y) * 40 + p.1) - lo.1);
        for (i, c) in p.4.as_chunks::<4>().0.iter().enumerate().filter(|(_, c)| c[3] != 0) {
            let (x, y) = (sx + (i as i64 % i64::from(p.2)), sy + (i as i64 / i64::from(p.2)));
            if x % sc != 0 || y % sc != 0 {
                continue;
            }
            let (x, y) = (x / sc, y / sc);
            if !(0..i64::from(img.w)).contains(&x) || !(0..i64::from(img.h)).contains(&y) {
                continue;
            }
            let o = ((y * i64::from(img.w) + x) * 4) as usize;
            if k == 1 {
                let dst = &mut img.rgba[o..o + 4];
                if dst[3] == 0 { dst.copy_from_slice(&[0, 0, 0, 110]) } else { dst[..3].iter_mut().for_each(|v| *v = (u16::from(*v) * 3 / 5) as u8) }
            } else {
                img.rgba[o..o + 4].copy_from_slice(c);
            }
        }
    }
    for (w, o, c) in warps() {
        let (cx, cy) = centre(w, o);
        img.square(((cx - lo.0) / sc, (cy - lo.1) / sc), 12 / sc, c);
    }
    Ok((img, sc as u32, lo))
}

/// Writes `bytes` to `path.tmp`, then renames it over `path`; a failed write removes the temp file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    d2host::write(&tmp, bytes).and_then(|()| d2host::rename(&tmp, path)).inspect_err(|_| drop(d2host::remove_file(&tmp)))
}

/// Writes `path.png.tmp` then renames it over `path`; a failed write removes the temp file.
pub fn write_png(path: &Path, img: &Image) -> io::Result<()> {
    let tmp = path.with_extension("png.tmp");
    let go = || -> io::Result<()> {
        let mut out = vec![];
        let mut enc = png::Encoder::new(&mut out, img.w, img.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&img.rgba)?;
        d2host::write(&tmp, &out)?;
        d2host::rename(&tmp, path)
    };
    go().inspect_err(|_| drop(d2host::remove_file(&tmp)))
}

/// Reads a PNG of any colour type (8 or 16 bit, indexed too) as RGBA; refuses one over the renderer's size guard before allocating it.
pub fn read_png(path: &Path) -> io::Result<Image> {
    let mut dec = png::Decoder::new(io::Cursor::new(d2host::read(path)?));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info()?;
    let (w, h) = (i64::from(r.info().width), i64::from(r.info().height));
    let mut img = image(w, h, [0; 4])?;
    let mut buf = vec![0; r.output_buffer_size().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "image too large"))?];
    let info = r.next_frame(&mut buf)?;
    let n = info.color_type.samples();
    for (d, s) in img.rgba.as_chunks_mut::<4>().0.iter_mut().zip(buf.chunks_exact(n)) {
        d.copy_from_slice(&match n {
            1 => [s[0], s[0], s[0], 255],
            2 => [s[0], s[0], s[0], s[1]],
            3 => [s[0], s[1], s[2], 255],
            _ => [s[0], s[1], s[2], s[3]],
        });
    }
    Ok(img)
}

fn room_json(r: &Room) -> String {
    format!("{{\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"ds1\":{}}}", r.rect.x, r.rect.y, r.rect.w, r.rect.h, r.ds1.as_ref().map_or("null".into(), |d| format!("{:?}", d.0)))
}

/// Rooms, warps and waypoints of a level as JSON (positions in collision sub-cells for warps, tiles for rooms); `verified` is `crate::verified`.
pub fn json(level: &Level) -> String {
    let rooms: Vec<_> = level.all().iter().map(room_json).collect();
    let pts = |wp: bool| -> Vec<String> {
        let f = |w: &&crate::Warp| (w.to_level == level.id) == wp;
        level.warps.iter().filter(f).map(|w| format!("{{\"to\":{},\"x\":{},\"y\":{}}}", w.to_level, w.pos.0, w.pos.1)).collect()
    };
    let r = level.rect;
    format!("{{\"level\":{},\"verified\":{},\"rect\":[{},{},{},{}],\"rooms\":[{}],\"warps\":[{}],\"waypoints\":[{}]}}", level.id, crate::verified(level.id), r.x, r.y, r.w, r.h, rooms.join(","), pts(false).join(","), pts(true).join(","))
}
