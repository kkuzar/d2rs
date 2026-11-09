//! Optional runtime inspector/editor for the expansion 1.14d (Game.exe, 32-bit, image base 0x400000).
//!
//! EXPERIMENTAL WRITES: `Inspector::set_base_stat` is UNVERIFIED on a real game. The game keeps a base
//! stat array and a cached "full" array (base plus item/skill modifiers) on the player's own stat list,
//! and the client unit is a separate copy of the server unit. We patch an existing entry in both arrays
//! of the unit found through g_ClientPlayer only; the server-side unit (hash table 0x7A5E70) is not
//! touched, so the game may overwrite or ignore the change. Writes are off unless
//! `allow_experimental_writes` is set. Skipped: item codes (they live in the items.txt record indexed by
//! the item's classId, not in the item unit), so items are only counted.
#![allow(clippy::disallowed_methods)]
use sha2::{Digest, Sha256};
use std::io;

mod os;
pub use os::{Process, find_game};

pub const IMAGE_BASE: u32 = 0x400000;
#[doc(hidden)]
pub const G_CLIENT_PLAYER: u32 = 0x7A6A70;
pub const GAME_EXE_SHA256: &str = "631066c1649c4ea9ffe48bf97e24c00bca1f7a6759c21150f1a79982589adaaf";
/// Stats `set_base_stat` may change: str, energy, dex, vit, statpts, newskills, gold, goldbank.
pub const WRITABLE: [u32; 8] = [0, 1, 2, 3, 4, 5, 14, 15];

pub trait Memory {
    fn read(&self, addr: u32, buf: &mut [u8]) -> io::Result<()>;
    fn write(&self, addr: u32, buf: &[u8]) -> io::Result<()>;
    fn read_u32(&self, addr: u32) -> io::Result<u32> {
        let mut b = [0; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn read_i32(&self, addr: u32) -> io::Result<i32> {
        self.read_u32(addr).map(|v| v as i32)
    }
}

/// How well a Game.exe matches the pinned 1.14d build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCheck {
    /// The pinned sha256.
    Exact,
    /// Same size, PE timestamp and section table as 1.14d, different bytes (a patched exe).
    Layout1_14d,
    Unknown,
}

#[doc(hidden)]
pub const EXE_SIZE: usize = 3618792;
#[doc(hidden)]
pub const PE_TIMESTAMP: u32 = 0x574DDFBC;
/// 1.14d section table: (name, virtual size, VA, raw size, raw offset).
#[doc(hidden)]
pub const SECTIONS: [(&[u8; 8], u32, u32, u32, u32); 5] = [
    (b".text\0\0\0", 0x2ca5a1, 0x1000, 0x2cb000, 0x1000),
    (b".rdata\0\0", 0x38a10, 0x2cc000, 0x39000, 0x2cc000),
    (b".data\0\0\0", 0x28fca4, 0x305000, 0x48000, 0x305000),
    (b".rsrc\0\0\0", 0x1988, 0x595000, 0x2000, 0x34d000),
    (b".reloc\0\0", 0x224fe, 0x597000, 0x23000, 0x34f000),
];
// Every absolute address d2mem touches must lie inside .text or .data.
const _: () = assert!(G_CLIENT_PLAYER - IMAGE_BASE >= SECTIONS[2].2 && G_CLIENT_PLAYER - IMAGE_BASE + 4 <= SECTIONS[2].2 + SECTIONS[2].1);

/// Classify the bytes of a Game.exe.
pub fn check_image_bytes(d: &[u8]) -> ImageCheck {
    if Sha256::digest(d).iter().map(|b| format!("{b:02x}")).collect::<String>() == GAME_EXE_SHA256 { return ImageCheck::Exact; }
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let ok = (|| {
        let pe = u32_at(0x3c)? as usize;
        let n = u32::from(u16::from_le_bytes([*d.get(pe + 6)?, *d.get(pe + 7)?]));
        let opt = usize::from(u16::from_le_bytes([*d.get(pe + 20)?, *d.get(pe + 21)?]));
        let t = pe + 24 + opt;
        let same = d.len() == EXE_SIZE && d.get(pe..pe + 4)? == b"PE\0\0" && u32_at(pe + 8)? == PE_TIMESTAMP && n as usize == SECTIONS.len();
        let secs = SECTIONS.iter().enumerate().all(|(i, s)| {
            let o = t + i * 40;
            d.get(o..o + 8) == Some(&s.0[..]) && [u32_at(o + 8), u32_at(o + 12), u32_at(o + 16), u32_at(o + 20)] == [Some(s.1), Some(s.2), Some(s.3), Some(s.4)]
        });
        Some(same && secs)
    })();
    if ok == Some(true) { ImageCheck::Layout1_14d } else { ImageCheck::Unknown }
}

/// Classify the file at `path` (unreadable is Unknown).
pub fn verify_image(path: &std::path::Path) -> ImageCheck {
    std::fs::read(path).map_or(ImageCheck::Unknown, |d| check_image_bytes(&d))
}

/// Writes need the experimental checkbox, a known layout, and for a modified exe the user's confirmation.
pub fn write_allowed(experimental: bool, confirmed: bool, v: ImageCheck) -> bool {
    experimental && match v { ImageCheck::Exact => true, ImageCheck::Layout1_14d => confirmed, ImageCheck::Unknown => false }
}

#[doc(hidden)]
pub fn bad(m: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, m.to_string())
}

/// (stat, param, value) with the value as stored (e.g. life is shifted left by 8).
pub type StatEntry = (u16, u16, i32);

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    pub class: u32,
    pub name: String,
    pub base_stats: Vec<StatEntry>,
    pub full_stats: Vec<StatEntry>,
    pub item_count: usize,
}

pub struct Inspector<M: Memory> {
    mem: M,
    base: u32,
    pub allow_experimental_writes: bool,
}

impl<M: Memory> Inspector<M> {
    pub fn new(mem: M, base: u32) -> Self {
        Inspector { mem, base, allow_experimental_writes: false }
    }

    fn unit(&self) -> io::Result<u32> {
        let u = self.mem.read_u32(self.base.wrapping_sub(IMAGE_BASE).wrapping_add(G_CLIENT_PLAYER))?;
        if u == 0 || self.mem.read_u32(u)? != 0 { return Err(bad("no player unit")); }
        Ok(u)
    }

    /// (entries pointer, count) of the array whose header is at `stat list + off`.
    fn array(&self, list: u32, off: u32) -> io::Result<(u32, u32)> {
        let p = self.mem.read_u32(list + off)?;
        let n = self.mem.read_u32(list + off + 4)? & 0xFFFF;
        if (p == 0 && n != 0) || n > 1024 { return Err(bad("bad stat array")); }
        Ok((p, n))
    }

    fn entries(&self, list: u32, off: u32) -> io::Result<Vec<StatEntry>> {
        let (p, n) = self.array(list, off)?;
        (0..n).map(|i| {
            let k = self.mem.read_u32(p + i * 8)?;
            Ok(((k >> 16) as u16, k as u16, self.mem.read_i32(p + i * 8 + 4)?))
        }).collect()
    }

    pub fn player(&self) -> io::Result<PlayerView> {
        let u = self.unit()?;
        let mut nb = [0u8; 16];
        self.mem.read(self.mem.read_u32(u + 0x14)?, &mut nb)?;
        let len = nb.iter().position(|&b| b == 0).ok_or_else(|| bad("name not terminated"))?;
        if len == 0 || !nb[..len].iter().all(|b| b.is_ascii_graphic()) { return Err(bad("unprintable name")); }
        let list = self.mem.read_u32(u + 0x5C)?;
        let mut item_count = 0;
        let inv = self.mem.read_u32(u + 0x60)?;
        let mut it = if inv == 0 { 0 } else { self.mem.read_u32(inv + 0x0C)? };
        while it != 0 && item_count < 4096 {
            if self.mem.read_u32(it)? != 4 { return Err(bad("non-item in inventory chain")); }
            item_count += 1;
            it = self.mem.read_u32(self.mem.read_u32(it + 0x14)? + 0x64)?;
        }
        Ok(PlayerView {
            class: self.mem.read_u32(u + 4)?,
            name: String::from_utf8_lossy(&nb[..len]).into_owned(),
            base_stats: self.entries(list, 0x24)?,
            full_stats: self.entries(list, 0x48)?,
            item_count,
        })
    }

    /// Address of the value of the (stat, param 0) entry in the array at `list + off`.
    fn find(&self, list: u32, off: u32, stat: u32) -> io::Result<u32> {
        let (p, n) = self.array(list, off)?;
        for i in 0..n {
            if self.mem.read_u32(p + i * 8)? == stat << 16 { return Ok(p + i * 8 + 4); }
        }
        Err(bad("stat entry missing"))
    }

    /// Experimental: overwrite an existing whitelisted stat (param 0) in both the base and full arrays.
    /// Refuses values the save cannot hold or the game resets on the next load (gold above level x 10000,
    /// stash gold above 2,500,000), because the game itself saves the value at exit.
    pub fn set_base_stat(&self, stat: u32, value: i32) -> io::Result<()> {
        if !self.allow_experimental_writes { return Err(bad("experimental writes disabled")); }
        if !WRITABLE.contains(&stat) { return Err(bad("stat not whitelisted")); }
        let list = self.mem.read_u32(self.unit()? + 0x5C)?;
        let level = self.entries(list, 0x24)?.iter().find(|e| e.0 == 12).map_or(1, |e| e.2);
        let Some(r) = d2s::limits::char_stat(d2s::Schema::classic(), stat, level.into()) else { return Err(bad("stat has no save range")) };
        d2s::limits::check(format!("stat {stat}"), value.into(), r).map_err(|m| bad(&format!("{m} (the save cannot keep it)")))?;
        let (a, b) = (self.find(list, 0x24, stat)?, self.find(list, 0x48, stat)?);
        self.mem.write(a, &value.to_le_bytes())?;
        self.mem.write(b, &value.to_le_bytes())
    }
}
