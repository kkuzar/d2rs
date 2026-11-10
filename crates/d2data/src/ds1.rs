//! DS1 preset maps (versions 3..=18): header, tile-file list, cell layers and the unit list.
//! Substitution layer, groups and NPC paths are validated and skipped (no preset room reads them).
use crate::{Error, Result};

/// One layer cell: the four bytes of the u32, low byte first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub prop1: u8,
    pub prop2: u8,
    pub prop3: u8,
    pub prop4: u8,
}

impl Cell {
    fn raw(self) -> u32 {
        u32::from_le_bytes([self.prop1, self.prop2, self.prop3, self.prop4])
    }
    /// DT1 main index (bits 20..=25).
    pub fn main(self) -> u8 {
        (self.raw() >> 20) as u8 & 0x3F
    }
    /// DT1 sub index (bits 8..=15, as the game reads it).
    pub fn sub(self) -> u8 {
        self.prop2
    }
    /// Bit 31.
    pub fn hidden(self) -> bool {
        self.prop4 & 0x80 != 0
    }
}

/// A placed unit: `kind` 1 monster, 2 object; `x`, `y` in sub-tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1Object {
    pub kind: i32,
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub flags: i32,
}

/// `w` and `h` are the cell grid dimensions (`size_x + 1`, `size_y + 1`); every layer holds `w * h` cells, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1 {
    pub version: u32,
    pub w: u32,
    pub h: u32,
    pub act: u32,
    pub walls: Vec<Vec<Cell>>,
    /// Orientation (low byte) of each wall layer.
    pub orients: Vec<Vec<u8>>,
    pub floors: Vec<Vec<Cell>>,
    pub shadow: Vec<Cell>,
    pub objects: Vec<Ds1Object>,
    pub files: Vec<String>,
    /// Substitution groups `[x, y, w, h, variants]` (empty unless `tag_type` is 1 or 2 and version >= 12).
    pub groups: Vec<[i32; 5]>,
}

struct R<'a>(&'a [u8], usize);

impl R<'_> {
    fn u(&mut self) -> Result<u32> {
        let s = self.0.get(self.1..self.1 + 4).ok_or(Error("truncated"))?;
        self.1 += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i(&mut self) -> Result<i32> {
        Ok(self.u()? as i32)
    }
    /// A count whose entries take at least `min` bytes each.
    fn count(&mut self, min: usize) -> Result<usize> {
        let n = self.u()? as usize;
        if n.saturating_mul(min) > self.0.len() - self.1.min(self.0.len()) {
            return Err(Error("count exceeds file"));
        }
        Ok(n)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.0.get(self.1..self.1.saturating_add(n)).ok_or(Error("truncated"))?;
        self.1 += n;
        Ok(())
    }
    fn layer(&mut self, n: usize) -> Result<Vec<u32>> {
        (0..n).map(|_| self.u()).collect()
    }
    fn cstr(&mut self) -> Result<String> {
        let s = self.0.get(self.1..).ok_or(Error("truncated"))?;
        let e = s.iter().position(|&c| c == 0).ok_or(Error("unterminated string"))?;
        self.1 += e + 1;
        Ok(s[..e].iter().map(|&c| c as char).collect())
    }
}

fn cells(l: Vec<u32>) -> Vec<Cell> {
    l.into_iter().map(|v| Cell { prop1: v as u8, prop2: (v >> 8) as u8, prop3: (v >> 16) as u8, prop4: (v >> 24) as u8 }).collect()
}

impl Ds1 {
    /// Strict: any truncation, bad count or leftover byte is an error.
    pub fn parse(b: &[u8]) -> Result<Ds1> {
        Ds1::parse_with(b, true)
    }

    /// As the game reads a file: a group list that runs past the end reads zeros (`act1/outdoors/trees.ds1`) and bytes after the last list are ignored (`swamp2.ds1`).
    pub fn parse_game(b: &[u8]) -> Result<Ds1> {
        Ds1::parse_with(b, false)
    }

    fn parse_with(b: &[u8], strict: bool) -> Result<Ds1> {
        let mut r = R(b, 0);
        let v = r.u()?;
        if !(3..=18).contains(&v) {
            return Err(Error("unsupported ds1 version"));
        }
        let (sx, sy) = (r.u()? as usize, r.u()? as usize);
        let n = (sx + 1).checked_mul(sy + 1).filter(|&n| sx < 1 << 12 && sy < 1 << 12 && n <= 1 << 20).ok_or(Error("bad ds1 size"))?;
        let act = if v >= 8 { r.u()?.min(4) } else { 0 };
        let tag = if v >= 10 { r.u()? } else { 0 };
        let files = (0..r.count(1)?).map(|_| r.cstr()).collect::<Result<Vec<_>>>()?;
        if (9..=13).contains(&v) {
            r.skip(8)?;
        }
        let walls_n = if v >= 4 { r.count(n * 8)? } else { 1 };
        let floors_n = if v >= 16 { r.count(n * 4)? } else { 1 };
        let (mut walls, mut orients, mut floors) = (vec![], vec![], vec![]);
        let shadow;
        if v >= 4 {
            for _ in 0..walls_n {
                walls.push(cells(r.layer(n)?));
                orients.push(r.layer(n)?.into_iter().map(|c| c as u8).collect());
            }
            for _ in 0..floors_n {
                floors.push(cells(r.layer(n)?));
            }
            shadow = cells(r.layer(n)?);
            if tag == 1 || tag == 2 {
                r.skip(4 * n)?;
            }
        } else {
            walls.push(cells(r.layer(n)?));
            floors.push(cells(r.layer(n)?));
            orients.push(r.layer(n)?.into_iter().map(|c| c as u8).collect());
            r.skip(4 * n)?;
            shadow = cells(r.layer(n)?);
        }
        let mut objects = vec![];
        if v >= 2 {
            for _ in 0..r.count(16)? {
                let (kind, id, x, y) = (r.i()?, r.i()?, r.i()?, r.i()?);
                objects.push(Ds1Object { kind, id, x, y, flags: if v >= 6 { r.i()? } else { 0 } });
            }
        }
        let mut groups = vec![];
        if v >= 12 && (tag == 1 || tag == 2) {
            if v >= 18 {
                r.skip(4)?;
            }
            let g = if v >= 13 { 20 } else { 16 };
            let n = if strict { r.count(g)? } else { (r.u()? as usize).min(1 << 16) };
            let gi = |r: &mut R| if strict { r.i() } else { Ok(r.i().unwrap_or(0)) };
            for _ in 0..n {
                groups.push([gi(&mut r)?, gi(&mut r)?, gi(&mut r)?, gi(&mut r)?, if v >= 13 { gi(&mut r)? } else { 0 }]);
            }
        }
        if (12..=13).contains(&v) {
            let trailer = r.skip(4);
            if strict {
                trailer?;
            }
        }
        if v >= 14 {
            for _ in 0..r.count(12)? {
                let pts = r.count(if v >= 15 { 12 } else { 8 })?;
                r.skip(8)?;
                r.skip(pts * if v >= 15 { 12 } else { 8 })?;
            }
        }
        if strict && r.1 != b.len() {
            return Err(Error("trailing bytes"));
        }
        Ok(Ds1 { version: v, w: sx as u32 + 1, h: sy as u32 + 1, act, walls, orients, floors, shadow, objects, files, groups })
    }
}
