//! Values read at runtime from a user's own game 1.14d `Game.exe` (never shipped with this crate).
use crate::{Error, Result};

const SIZE: usize = 3_618_792;
const STAMP: u32 = 0x574D_DFBC;
const IMAGE_BASE: u32 = 0x40_0000;
const DS1_OBJECT_IDS: u32 = 0x0074_8AD8;
const BAD: Error = Error("not the 1.14d Game.exe: this needs the 1.14d Game.exe");

fn u32_le(b: &[u8], o: usize) -> Result<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or(BAD)
}

/// `len` bytes at virtual address `va` of the 1.14d exe (size and PE timestamp checked), mapped through the PE section headers.
pub fn read_va(exe: &[u8], va: u32, len: usize) -> Result<&[u8]> {
    let pe = u32_le(exe, 0x3C)? as usize;
    if exe.len() != SIZE || exe.get(pe..pe + 4) != Some(b"PE\0\0") || u32_le(exe, pe + 8)? != STAMP {
        return Err(BAD);
    }
    let rva = va - IMAGE_BASE;
    let sections = exe.get(pe + 6..pe + 8).zip(exe.get(pe + 20..pe + 22)).map(|(n, o)| (u16::from_le_bytes([n[0], n[1]]) as usize, u16::from_le_bytes([o[0], o[1]]) as usize)).ok_or(BAD)?;
    let mut at = None;
    for i in 0..sections.0 {
        let s = pe + 24 + sections.1 + i * 40;
        let (size, va, raw) = (u32_le(exe, s + 8)?, u32_le(exe, s + 12)?, u32_le(exe, s + 20)?);
        if (va..va.saturating_add(size)).contains(&rva) {
            at = Some((rva - va + raw) as usize);
        }
    }
    let at = at.ok_or(BAD)?;
    exe.get(at..at + len).ok_or(BAD)
}

/// `g_Ds1ObjectIds` (5 acts x 150 ids, specs/drlg/preset.md step 1a). Accepted only for the 1.14d exe (`[3][5..8]` all 573).
pub fn ds1_object_ids(exe: &[u8]) -> Result<[[i32; 150]; 5]> {
    let b = read_va(exe, DS1_OBJECT_IDS, 5 * 150 * 4)?;
    let mut t = [[0i32; 150]; 5];
    for (i, v) in t.iter_mut().flatten().enumerate() {
        *v = u32_le(b, i * 4)? as i32;
    }
    if t[3][5..8] != [573; 3] {
        return Err(BAD);
    }
    Ok(t)
}
