//! Game-asset decoders (std only): DC6 sprites, palettes, colour maps, `.tbl` strings.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Decoding error: truncated or malformed input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);
pub type Result<T> = std::result::Result<T, Error>;

fn u32_at(b: &[u8], o: usize) -> Result<usize> {
    let s = b.get(o..o + 4).ok_or(Error("truncated"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

pub mod ds1;
pub mod dt1;
pub mod excel;
pub mod exe;
pub mod mpq;
pub mod setup;
pub use mpq::{GameData, Mpq};

pub mod dc6 {
    use super::*;
    /// One sprite frame: palette indices, row-major top-down, 0 = transparent.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Frame {
        pub width: usize,
        pub height: usize,
        pub pixels: Vec<u8>,
    }

    pub fn decode(b: &[u8]) -> Result<Vec<Frame>> {
        let n = u32_at(b, 0x10)?.saturating_mul(u32_at(b, 0x14)?);
        if n > 1 << 16 {
            return Err(Error("too many frames"));
        }
        (0..n).map(|i| frame(b, u32_at(b, 0x18 + 4 * i)?)).collect()
    }

    fn frame(b: &[u8], p: usize) -> Result<Frame> {
        let (flip, w, h, len) = (u32_at(b, p)?, u32_at(b, p + 4)?, u32_at(b, p + 8)?, u32_at(b, p + 0x1C)?);
        if w.saturating_mul(h) > 1 << 24 {
            return Err(Error("frame too large"));
        }
        let s = b.get(p + 32..p + 32 + len).ok_or(Error("truncated"))?;
        let mut px = vec![0u8; w * h];
        let (mut row, mut x, mut i) = (0, 0, 0);
        while i < s.len() && row < h {
            let c = s[i];
            i += 1;
            match c {
                0x80 => (row, x) = (row + 1, 0),
                0x81..=0xFF => x += (c & 0x7F) as usize,
                _ => {
                    let run = s.get(i..i + c as usize).ok_or(Error("truncated run"))?;
                    i += c as usize;
                    let y = if flip == 0 { h - 1 - row } else { row };
                    for &v in run {
                        if x < w {
                            px[y * w + x] = v;
                        }
                        x += 1;
                    }
                }
            }
        }
        Ok(Frame { width: w, height: h, pixels: px })
    }
}

/// 256-entry palette (`pal.dat`, stored B,G,R).
pub struct Palette(Vec<[u8; 3]>);

impl Palette {
    pub fn from_dat(b: &[u8]) -> Result<Palette> {
        let s = b.get(..768).ok_or(Error("palette needs 768 bytes"))?;
        Ok(Palette(s.chunks(3).map(|c| [c[2], c[1], c[0]]).collect()))
    }

    /// RGBA8 for palette indices, optionally remapped through a colour map; index 0 is transparent.
    pub fn rgba(&self, indices: &[u8], map: Option<&[u8; 256]>) -> Vec<u8> {
        let mut out = Vec::with_capacity(indices.len() * 4);
        for &p in indices {
            let q = map.map_or(p, |m| m[p as usize]);
            let [r, g, b] = self.0[q as usize];
            out.extend_from_slice(&[r, g, b, if p == 0 { 0 } else { 255 }]);
        }
        out
    }
}

/// Item colour maps: 256-byte maps, 21 per `.dat`.
pub fn colormaps(b: &[u8]) -> Vec<[u8; 256]> {
    b.as_chunks::<256>().0.to_vec()
}

/// A `.tbl` string table.
#[derive(Debug, Default, Clone)]
pub struct Tbl(#[doc(hidden)] pub HashMap<String, String>);

impl Tbl {
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn parse(b: &[u8]) -> Result<Tbl> {
        let count = b.get(2..4).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize).ok_or(Error("truncated"))?;
        let nodes = u32_at(b, 4)?;
        let base = 0x15 + 2 * count;
        let mut m = HashMap::new();
        for i in 0..nodes {
            let n = b.get(base + 17 * i..base + 17 * i + 17).ok_or(Error("truncated nodes"))?;
            if n[0] != 0 {
                let (k, v) = (u32_at(n, 7)?, u32_at(n, 11)?);
                m.insert(cstr(b, k)?, cstr(b, v)?);
            }
        }
        Ok(Tbl(m))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

fn cstr(b: &[u8], o: usize) -> Result<String> {
    let s = b.get(o..).ok_or(Error("bad offset"))?;
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    Ok(String::from_utf8(s.to_vec()).unwrap_or_else(|_| s.iter().map(|&c| c as char).collect()))
}

/// Several tables in priority order (first match wins), e.g. patchstring, expansionstring, string.
#[derive(Debug, Default, Clone)]
pub struct MergedTbl(pub Vec<Tbl>);

impl MergedTbl {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find_map(|t| t.get(key))
    }
}

/// A folder holding an extracted MPQ tree or game data folder; case-insensitive file lookup.
pub struct DataDir(pub PathBuf);

impl DataDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        DataDir(root.into())
    }

    /// Resolve a `/` or `\` separated path, ignoring case.
    pub fn find(&self, path: &str) -> Option<PathBuf> {
        let mut cur: PathBuf = self.0.clone();
        for part in path.split(['/', '\\']).filter(|p| !p.is_empty()) {
            let direct = cur.join(part);
            cur = if d2host::exists(&direct) {
                direct
            } else {
                d2host::read_dir(&cur).ok()?.into_iter().find(|e| e.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(part)))?
            };
        }
        Some(cur)
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        d2host::read(&self.find(path)?).ok()
    }

    /// Root folder this helper searches.
    pub fn root(&self) -> &Path {
        &self.0
    }
}
