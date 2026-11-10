//! Act III jungle layout on the act seed (specs/drlg/outdoor.md J2): three 64 x 192 rects, their path cells and clearings.
use crate::drlg::{MapError, Rect};
use crate::{maze::overlap, Seed};

/// J2e `g_JungleClearingDefs` by `v >> 4` and `g_JungleSideDefs` by (exit mask, side W E S N).
const CLEAR: [i32; 16] = [256, 575, 576, 577, 578, 579, 580, 0, 581, 582, 583, 0, 584, 0, 0, 0];
const SIDE: [[i32; 4]; 16] = [
    [0; 4], [0, 545, 546, 547], [548, 0, 549, 550], [0, 0, 551, 552], [553, 554, 0, 555], [0, 556, 0, 557], [558, 0, 0, 559], [0, 0, 0, 560],
    [561, 562, 563, 0], [0, 564, 565, 0], [566, 0, 567, 0], [0, 0, 568, 0], [569, 570, 0, 0], [0, 571, 0, 0], [572, 0, 0, 0], [0; 4],
];

fn fatal(what: &'static str) -> MapError {
    MapError::Unsupported(76, what)
}

/// J2: the three jungles by y descending (levels 76, 77, 78) with their `Def` arrays (rows outer); `w`, `h` are level 76's size.
pub fn layout(a: &mut Seed, w: i32, h: i32, town: (i32, i32)) -> Result<Vec<(Rect, Vec<i32>)>, MapError> {
    let (nx, ny) = (w / 32, h / 32);
    // J2a: rect picks on the act seed; `kids` holds the children's directions.
    let mut rec = vec![(Rect { x: town.0, y: town.1 - h, w, h }, 0u32, Vec::<u32>::new())];
    let (mut minx, mut miny, mut maxx, maxy) = (town.0, town.1 - h, town.0 + w, town.1);
    for _ in 0..10_000 {
        if rec.len() == 3 {
            break;
        }
        let k = a.range(rec.len() as i32) as usize;
        let d = a.step() % 5;
        let p = rec[k].0;
        let (dx, dy) = match d { 0 => (0, -h), 1 => (-w, -h / 3), 2 => (w, -h / 3), 3 => (-w, -2 * h / 3), _ => (w, -2 * h / 3) };
        let r = Rect { x: p.x + dx, y: p.y + dy, w, h };
        if rec.iter().any(|j| overlap(j.0, r)) {
            continue;
        }
        rec[k].2.push(d);
        if rec[k].2.len() > 3 {
            return Err(fatal("jungle rect with more than 3 children"));
        }
        rec.push((r, d, vec![]));
        (minx, miny, maxx) = (minx.min(r.x), miny.min(r.y), maxx.max(r.x + r.w));
    }
    if rec.len() < 3 || (maxx - minx) % 32 != 0 || (maxy - miny) % 32 != 0 {
        return Err(fatal("jungle rects off the 32-tile grid"));
    }
    let (gw, gh) = (((maxx - minx) / 32 + 2) as usize, ((maxy - miny) / 32 + 2) as usize);
    let n = gw * gh;
    let at = |x: i32, y: i32| (x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh).then(|| y as usize * gw + x as usize);
    let get = |v: &[i32], p: (i32, i32)| at(p.0, p.1).map_or(0, |i| v[i]);
    let origin: Vec<(i32, i32)> = rec.iter().map(|j| ((j.0.x - minx) / 32 + 1, (j.0.y - miny) / 32 + 1)).collect();
    let mut restarts = 0;
    let res = 'restart: loop {
        restarts += 1;
        if restarts > 1000 {
            return Err(fatal("jungle layout does not settle"));
        }
        let (mut owner, mut order, mut marks, mut res) = (vec![0; n], vec![0; n], vec![0; n], vec![0; n]);
        // J2b: one path per jungle, bottom row up, in creation order; dead-end candidates are the cells beside it.
        for j in 0..3 {
            let (cx0, cy0) = origin[j];
            let bottom = cy0 + ny - 1;
            let mut s = if j == 0 { (a.step() & 1) as i32 } else { (rec[j].1 & 1) as i32 };
            let side = |s: i32| if s != 0 { -1 } else { 1 };
            let mut cnt;
            loop {
                for yy in cy0..cy0 + ny {
                    for xx in cx0..cx0 + nx {
                        if let Some(i) = at(xx, yy) {
                            (owner[i], order[i], marks[i], res[i]) = (j as i32 + 1, 0, 0, 0);
                        }
                    }
                }
                let mut col = cx0 + s;
                let set = |v: &mut Vec<i32>, x: i32, y: i32, val: i32| {
                    if let Some(i) = at(x, y) {
                        v[i] = val;
                    }
                };
                if j != 0 {
                    set(&mut marks, col, bottom, 1);
                    set(&mut marks, col + side(s), bottom, 2);
                }
                for &kd in &rec[j].2 {
                    let (mx, my) = match kd { 0 => (cx0, cy0), 1 => (cx0, cy0 + 3), 2 => (cx0 + 1, cy0 + 3), 3 => (cx0, cy0 + 1), _ => (cx0 + 1, cy0 + 1) };
                    set(&mut marks, mx, my, 1);
                }
                cnt = i32::from(j != 0);
                let (mut run, mut row, mut num) = (0, bottom, 100 * (j as i32 + 1));
                while row >= cy0 {
                    set(&mut order, col, row, num);
                    num += 1;
                    if run != 0 && row != cy0 && a.step().is_multiple_of(3) {
                        (run, col, s) = (0, col + side(s), s ^ 1);
                    } else {
                        run += 1;
                        let o = (col + side(s), row);
                        if run > 1 && row > 1 && get(&marks, o) == 0 {
                            set(&mut marks, o.0, o.1, 2);
                            cnt += 1;
                        }
                        row -= 1;
                    }
                }
                if cnt >= 2 {
                    break;
                }
            }
            while cnt > 3 {
                let t = a.range(cnt);
                let mut k = 0;
                'trim: for yy in cy0..cy0 + ny {
                    for xx in cx0..cx0 + nx {
                        if get(&marks, (xx, yy)) == 2 {
                            if k == t {
                                marks[at(xx, yy).unwrap_or(0)] = 0;
                                break 'trim;
                            }
                            k += 1;
                        }
                    }
                }
                cnt -= 1;
            }
        }
        let nb = |c: usize| {
            let (x, y) = ((c % gw) as i32, (c / gw) as i32);
            [(x, y - 1), (x, y + 1), (x + 1, y), (x - 1, y)]
        };
        let or = |v: &mut Vec<i32>, p: (i32, i32), b: i32| {
            if let Some(i) = at(p.0, p.1) {
                v[i] |= b;
            }
        };
        // J2c: exit masks (N 8, S 4, E 2, W 1) from the link marks and consecutive path numbers.
        for c in 0..n {
            let (q, bits, mut m, mut best) = (nb(c), [8, 4, 2, 1], 0, i32::MAX);
            if marks[c] == 1 {
                for k in 0..4 {
                    let o = get(&order, q[k]);
                    if o != 0 && o < best && get(&owner, q[k]) == owner[c] {
                        (m, best) = (bits[k], o);
                    }
                }
                for k in 0..4 {
                    if m & bits[k] != 0 {
                        or(&mut res, q[k], [4, 8, 1, 2][k]);
                    }
                    if get(&marks, q[k]) == 1 && get(&owner, q[k]) != owner[c] {
                        m |= bits[k];
                    }
                }
            }
            if order[c] != 0 {
                for k in 0..4 {
                    if (order[c] - get(&order, q[k])).abs() == 1 {
                        m |= bits[k];
                    }
                }
            }
            res[c] |= m;
        }
        // J2d: clearings (sides N 0x80, S 0x40, E 0x20, W 0x10), one act draw per grid cell.
        for c in 0..n {
            let r = a.step() & 3;
            let (q, bit, mut v) = (nb(c), [0x80, 0x40, 0x20, 0x10], res[c]);
            let zero = v == 0;
            if marks[c] == 2 {
                let rot = |r: u32| (0..4).map(move |i| ((i + r) & 3) as usize);
                if let Some(k) = rot(r).find(|&k| get(&owner, q[k]) == owner[c] && (1..15).contains(&get(&res, q[k]))) {
                    v |= bit[k];
                }
                if v == 0 {
                    continue 'restart;
                }
                if zero {
                    let found = rot(r).find(|&k| get(&owner, q[k]) != owner[c] && get(&marks, q[k]) == 2).map(|k| v |= bit[k]).is_some();
                    let (t1, r2) = (a.step() & 1, a.step() & 3);
                    if t1 != 0 && !found && let Some(k) = rot(r2).find(|&k| (1..15).contains(&get(&res, q[k])) && v & bit[k] == 0) {
                        v |= bit[k];
                    }
                }
                for k in 0..4 {
                    if v & bit[k] != 0 {
                        or(&mut res, q[k], [0x40, 0x80, 0x10, 0x20][k]);
                    }
                }
                marks[c] = 0;
            }
            res[c] |= v;
        }
        break res;
    };
    // J2e: Defs.
    let mut defs = vec![0; n];
    for c in 0..n {
        let (v, low) = (res[c], res[c] & 15);
        defs[c] = if low == 0 {
            let d = CLEAR[(v >> 4) as usize & 15];
            if v >= 16 && d == 0 {
                return Err(fatal("jungle clearing with three or four sides"));
            }
            d * i32::from(v >= 16)
        } else if v < 16 {
            529 + low
        } else {
            let mut d = low;
            for (k, b) in [0x10, 0x20, 0x40, 0x80].into_iter().enumerate() {
                if v & b != 0 {
                    d = *SIDE.get(d as usize).map(|r| &r[k]).ok_or(fatal("jungle side lookup past the table"))?;
                }
            }
            if d == 0 {
                return Err(fatal("jungle cell with no Def"));
            }
            d
        };
    }
    let mut out: Vec<(Rect, Vec<i32>)> = rec
        .iter()
        .zip(&origin)
        .map(|(j, &(cx0, cy0))| (j.0, (0..ny).flat_map(|y| (0..nx).map(move |x| (x, y))).map(|(x, y)| get(&defs, (cx0 + x, cy0 + y))).collect()))
        .collect();
    out.sort_by_key(|j| -j.0.y);
    Ok(out)
}
