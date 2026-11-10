//! Read-only MPQ v0 reader (game archives) plus a layered `GameData` view.
//! Supports PKWARE implode sectors, encrypted tables/files and single-unit files.
//! Zlib/Huffman/ADPCM sectors are not supported (`read` returns `None` for them).
use crate::{DataDir, Error, Result};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn crypt() -> &'static [u32; 1280] {
    static T: OnceLock<[u32; 1280]> = OnceLock::new();
    T.get_or_init(|| {
        let (mut t, mut seed) = ([0u32; 1280], 0x100001u32);
        for i in 0..256 {
            for k in 0..5 {
                seed = (seed * 125 + 3) % 0x2AAAAB;
                let hi = (seed & 0xFFFF) << 16;
                seed = (seed * 125 + 3) % 0x2AAAAB;
                t[i + k * 256] = hi | (seed & 0xFFFF);
            }
        }
        t
    })
}

/// Storm name hash; `kind` 0 = table slot, 1/2 = name check, 3 = file key.
#[doc(hidden)]
pub fn hash(name: &str, kind: usize) -> u32 {
    let (mut s1, mut s2) = (0x7FED7FEDu32, 0xEEEEEEEEu32);
    for c in name.bytes().map(|c| c.to_ascii_uppercase()).map(|c| if c == b'/' { b'\\' } else { c }) {
        s1 = crypt()[kind * 256 + c as usize] ^ s1.wrapping_add(s2);
        s2 = (c as u32).wrapping_add(s1).wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
    }
    s1
}

/// Decrypt (or, with `enc`, encrypt) whole little-endian u32 words in place.
#[doc(hidden)]
pub fn crypt_words(buf: &mut [u8], mut key: u32, enc: bool) {
    let mut s2 = 0xEEEEEEEEu32;
    for w in buf.as_chunks_mut::<4>().0 {
        s2 = s2.wrapping_add(crypt()[0x400 + (key & 0xFF) as usize]);
        let x = u32::from_le_bytes(*w);
        let (plain, out) = if enc { (x, x ^ key.wrapping_add(s2)) } else { let p = x ^ key.wrapping_add(s2); (p, p) };
        *w = out.to_le_bytes();
        s2 = plain.wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
        key = ((!key << 21).wrapping_add(0x11111111)) | (key >> 11);
    }
}

fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub struct Mpq {
    path: PathBuf,
    base: u64,
    sector: usize,
    hashes: Vec<u8>,
    blocks: Vec<u8>,
}

impl Mpq {
    pub fn open(path: impl AsRef<Path>) -> Result<Mpq> {
        let path = path.as_ref().to_path_buf();
        let len = d2host::len(&path).map_err(|_| Error("mpq: cannot open"))?;
        let mut h = [0u8; 32];
        let mut base = 0u64;
        loop {
            if base + 32 > len {
                return Err(Error("mpq: header not found"));
            }
            d2host::read_at(&path, base, &mut h).map_err(|_| Error("mpq: read"))?;
            match &h[..4] {
                b"MPQ\x1A" => break,
                b"MPQ\x1B" => base += u64::from(u32at(&h, 8)).max(1),
                _ => base += 512,
            }
        }
        if u16::from_le_bytes([h[12], h[13]]) != 0 {
            return Err(Error("mpq: only format v0 supported"));
        }
        let sector = 512usize << (u16::from_le_bytes([h[14], h[15]]) & 15);
        let (hn, bn) = (u32at(&h, 24) as usize, u32at(&h, 28) as usize);
        if hn == 0 || !hn.is_power_of_two() || hn > 1 << 22 || bn > 1 << 22 {
            return Err(Error("mpq: bad table size"));
        }
        let table = |off: u32, n: usize, key: &str| {
            let mut v = vec![0u8; n * 16];
            d2host::read_at(&path, base + u64::from(off), &mut v).map_err(|_| Error("mpq: table"))?;
            crypt_words(&mut v, hash(key, 3), false);
            Ok::<_, Error>(v)
        };
        let hashes = table(u32at(&h, 16), hn, "(hash table)")?;
        let blocks = table(u32at(&h, 20), bn, "(block table)")?;
        Ok(Mpq { path, base, sector, hashes, blocks })
    }

    /// Read and decode a file by name (case-insensitive, `\` or `/`). `None` if absent or unsupported.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        let n = self.hashes.len() / 16;
        let (a, b) = (hash(name, 1), hash(name, 2));
        let mut i = hash(name, 0) as usize & (n - 1);
        let blk = loop {
            let e = &self.hashes[i * 16..];
            match u32at(e, 12) {
                0xFFFF_FFFF => return None,
                x if x < 0xFFFF_FFFE && u32at(e, 0) == a && u32at(e, 4) == b => break x as usize,
                _ => i = (i + 1) & (n - 1),
            }
            if i == hash(name, 0) as usize & (n - 1) {
                return None;
            }
        };
        let e = self.blocks.get(blk * 16..blk * 16 + 16)?;
        let (off, csize, fsize, flags) = (u32at(e, 0), u32at(e, 4) as usize, u32at(e, 8) as usize, u32at(e, 12));
        if flags & 0x8000_0000 == 0 || fsize > 1 << 28 || csize > 1 << 28 {
            return None;
        }
        let mut raw = vec![0u8; csize];
        d2host::read_at(&self.path, self.base + u64::from(off), &mut raw).ok()?;
        let mut key = hash(name.rsplit(['/', '\\']).next()?, 3);
        if flags & 0x2_0000 != 0 {
            key = key.wrapping_add(off) ^ fsize as u32;
        }
        let enc = flags & 0x1_0000 != 0;
        let packed = flags & 0x300 != 0;
        let secs: Vec<(usize, usize)> = if flags & 0x0100_0000 != 0 {
            vec![(0, csize)]
        } else {
            let ns = fsize.div_ceil(self.sector);
            if !packed {
                (0..ns).map(|i| (i * self.sector, ((i + 1) * self.sector).min(csize))).collect()
            } else {
                let mut t = raw.get(..(ns + 1) * 4)?.to_vec();
                if enc {
                    crypt_words(&mut t, key.wrapping_sub(1), false);
                }
                (0..ns).map(|i| (u32at(&t, i * 4) as usize, u32at(&t, i * 4 + 4) as usize)).collect()
            }
        };
        let mut out = Vec::with_capacity(fsize);
        for (i, (s, e)) in secs.into_iter().enumerate() {
            let mut d = raw.get(s..e)?.to_vec();
            if enc {
                let k = if flags & 0x0100_0000 != 0 { key } else { key.wrapping_add(i as u32) };
                let whole = d.len() & !3;
                crypt_words(&mut d[..whole], k, false);
            }
            let want = if flags & 0x0100_0000 != 0 { fsize } else { self.sector.min(fsize - i * self.sector) };
            if packed && d.len() < want {
                d = if flags & 0x100 != 0 {
                    explode::explode(&d).ok()?
                } else {
                    match d.split_first()? {
                        (0x08, rest) => explode::explode(rest).ok()?,
                        _ => return None,
                    }
                };
            }
            out.extend_from_slice(&d);
        }
        (out.len() == fsize).then_some(out)
    }
}

/// Layered game data: optional extracted override folder, then patch_d2, d2exp, d2data MPQs.
#[derive(Default)]
pub struct GameData {
    over: Option<DataDir>,
    mpqs: Vec<Mpq>,
}

impl GameData {
    /// Open the MPQs found in a game install folder (names matched ignoring case).
    pub fn from_install(dir: impl AsRef<Path>) -> Result<GameData> {
        let d = DataDir::new(dir.as_ref());
        let mpqs: Vec<Mpq> = ["patch_d2.mpq", "d2exp.mpq", "d2data.mpq"].iter().filter_map(|n| d.find(n)).map(Mpq::open).collect::<Result<_>>()?;
        if mpqs.is_empty() {
            return Err(Error("no game MPQs found"));
        }
        Ok(GameData { over: None, mpqs })
    }

    /// Extracted files in `dir` win over every MPQ.
    pub fn with_override(mut self, dir: DataDir) -> Self {
        self.over = Some(dir);
        self
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.over.as_ref().and_then(|o| o.read(path)).or_else(|| self.mpqs.iter().find_map(|m| m.read(path)))
    }
}
