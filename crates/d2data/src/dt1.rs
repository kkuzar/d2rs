//! DT1 tile sets (version 7.6): tile records with their pixel blocks, and tile pixel assembly.
use crate::{Error, Result};

/// One pixel block: placed at `(x, y)` in tile space; `format` 1 is the isometric 32x15 block, others are RLE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub x: i16,
    pub y: i16,
    pub grid_x: u8,
    pub grid_y: u8,
    pub format: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub orientation: i32,
    pub main: i32,
    pub sub: i32,
    pub rarity: i32,
    pub height: i32,
    pub width: i32,
    pub roof_y: i32,
    pub sound: u8,
    pub anim: u8,
    pub sub_flags: [u8; 25],
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dt1 {
    pub tiles: Vec<Tile>,
}

const ISO_W: [usize; 15] = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];

fn i32_at(b: &[u8], o: usize) -> Result<i32> {
    let s = b.get(o..o.checked_add(4).ok_or(Error("truncated"))?).ok_or(Error("truncated"))?;
    Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn off(b: &[u8], o: usize) -> Result<usize> {
    usize::try_from(i32_at(b, o)?).map_err(|_| Error("negative offset"))
}

impl Dt1 {
    /// Version 7.6 only; any out-of-range offset or truncation is an error.
    pub fn parse(b: &[u8]) -> Result<Dt1> {
        if (i32_at(b, 0)?, i32_at(b, 4)?) != (7, 6) {
            return Err(Error("not dt1 version 7.6"));
        }
        let (count, ptr) = (off(b, 0x10C)?, off(b, 0x110)?);
        let tiles = (0..count).map(|i| tile(b, ptr.checked_add(96 * i).ok_or(Error("bad tile ptr"))?)).collect::<Result<_>>()?;
        Ok(Dt1 { tiles })
    }
}

fn tile(b: &[u8], p: usize) -> Result<Tile> {
    let r = b.get(p..p + 96).ok_or(Error("truncated tile"))?;
    let (bp, bn) = (off(b, p + 0x48)?, off(b, p + 0x50)?);
    let blocks = (0..bn.min(1 << 12)).map(|i| block(b, bp, bp.checked_add(20 * i).ok_or(Error("bad block ptr"))?)).collect::<Result<_>>()?;
    Ok(Tile {
        orientation: i32_at(b, p + 0x14)?,
        main: i32_at(b, p + 0x18)?,
        sub: i32_at(b, p + 0x1C)?,
        rarity: i32_at(b, p + 0x20)?,
        height: i32_at(b, p + 8)?,
        width: i32_at(b, p + 0x0C)?,
        roof_y: i16::from_le_bytes([r[4], r[5]]).into(),
        sound: r[6],
        anim: r[7],
        sub_flags: r[0x28..0x41].try_into().map_err(|_| Error("truncated tile"))?,
        blocks,
    })
}

fn block(b: &[u8], bp: usize, p: usize) -> Result<Block> {
    let h = b.get(p..p + 20).ok_or(Error("truncated block header"))?;
    let (len, d) = (off(b, p + 0x0A)?, bp.checked_add(off(b, p + 0x10)?).ok_or(Error("bad data ptr"))?);
    let data = b.get(d..d.checked_add(len).ok_or(Error("bad data len"))?).ok_or(Error("truncated block data"))?;
    Ok(Block { x: i16::from_le_bytes([h[0], h[1]]), y: i16::from_le_bytes([h[2], h[3]]), grid_x: h[6], grid_y: h[7], format: u16::from_le_bytes([h[8], h[9]]), data: data.to_vec() })
}

/// A block as 32-wide rows of palette indices (0 transparent).
fn decode(bl: &Block) -> Vec<[u8; 32]> {
    let mut rows = vec![[0u8; 32]];
    let d = &bl.data;
    if bl.format == 1 {
        let mut i = 0;
        rows = ISO_W
            .iter()
            .map(|&w| {
                let mut r = [0u8; 32];
                let s = d.get(i..(i + w).min(d.len())).unwrap_or(&[]);
                r[(32 - w) / 2..(32 - w) / 2 + s.len()].copy_from_slice(s);
                i += w;
                r
            })
            .collect();
    } else {
        let (mut i, mut x, mut open) = (0, 0usize, false);
        while i + 1 < d.len() && rows.len() <= 256 {
            let (skip, n) = (d[i] as usize, d[i + 1] as usize);
            i += 2;
            if skip == 0 && n == 0 {
                rows.push([0; 32]);
                (x, open) = (0, false);
                continue;
            }
            (x, open) = (x + skip, true);
            for &v in d.get(i..(i + n).min(d.len())).unwrap_or(&[]) {
                if let (true, Some(r)) = (x < 32, rows.last_mut()) {
                    r[x] = v;
                }
                x += 1;
            }
            i += n;
        }
        if !open && rows.len() > 1 {
            rows.pop();
        }
    }
    rows
}

impl Tile {
    /// `(x0, y0, w, h, indices)`: the bounding box of the tile's blocks, row-major, 0 transparent. A later
    /// block's index 0 never overwrites an earlier pixel. No blocks gives an empty image.
    pub fn pixels(&self) -> (i32, i32, u32, u32, Vec<u8>) {
        let dec: Vec<_> = self.blocks.iter().map(decode).collect();
        if self.blocks.is_empty() {
            return (0, 0, 0, 0, vec![]);
        }
        let ext = |f: fn(i32, i32) -> i32, init: i32, g: &dyn Fn(&Block, usize) -> i32| self.blocks.iter().zip(&dec).fold(init, |a, (b, d)| f(a, g(b, d.len())));
        let (x0, y0) = (ext(i32::min, i32::MAX, &|b, _| b.x.into()), ext(i32::min, i32::MAX, &|b, _| b.y.into()));
        let w = (ext(i32::max, i32::MIN, &|b, _| i32::from(b.x) + 32) - x0) as usize;
        let h = (ext(i32::max, i32::MIN, &|b, n| i32::from(b.y) + n as i32) - y0) as usize;
        let mut out = vec![0u8; w * h];
        for (bl, d) in self.blocks.iter().zip(&dec) {
            for (r, row) in d.iter().enumerate() {
                for (c, &v) in row.iter().enumerate().filter(|&(_, &v)| v != 0) {
                    let o = (bl.y as i32 - y0) as usize + r;
                    let o = o * w + (bl.x as i32 - x0) as usize + c;
                    if out[o] == 0 {
                        out[o] = v;
                    }
                }
            }
        }
        (x0, y0, w as u32, h as u32, out)
    }
}
