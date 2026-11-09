//! Version 71 (1.00-1.06) item records: fixed-size, kept raw and non-editable.
use crate::bits::{err, Result};
use crate::item::{Item, ItemView};

/// Parses one version-71 record at `pos` ("JM" + record, socketed items follow separately).
/// The record form is chosen by flags 0x100000 (extended) and 0x200000 (compact), 0x10000 is the ear.
pub fn v71_item(buf: &[u8], pos: usize) -> Result<(Item, usize)> {
    if buf.get(pos..pos + 2) != Some(b"JM") {
        return err(pos * 8, "item does not start with JM");
    }
    let Some(fl) = buf.get(pos + 2..pos + 6) else { return err(pos * 8, "item record truncated") };
    let flags = u32::from_le_bytes([fl[0], fl[1], fl[2], fl[3]]);
    let (ear, ext, compact) = (flags & 0x10000 != 0, flags & 0x100000 != 0, flags & 0x200000 != 0);
    let len = match (ext, compact, ear) {
        (false, ..) => 25,
        (true, true, true) => 24,
        (true, true, false) => 13,
        (true, false, _) => 29,
    };
    let Some(rec) = buf.get(pos + 2..pos + 2 + len) else { return err(pos * 8, "item record truncated") };
    let g = |at: usize, n: usize| (0..n).fold(0u32, |a, i| a | u32::from(rec[(at + i) / 8] >> ((at + i) % 8) & 1) << i);
    let mut v = ItemView { sv: 71, flags, ..Default::default() };
    // (body, socket count, ilvl, quality, mode, x, y, page) bit offsets of the form, 0 = absent.
    let o = match (ext, compact, ear) {
        (false, _, false) => [32, 37, 40, 65, 62, 105, 110, 187],
        (false, _, true) => [32, 0, 0, 0, 47, 50, 55, 68],
        (true, true, _) => [42, 0, 0, 0, 47, 50, 55, 58],
        (true, false, _) => [74, 79, 82, 97, 94, 137, 142, 219],
    };
    v.body = g(o[0], 5);
    v.location = g(o[4], 3);
    (v.x, v.y, v.store) = (g(o[5], 5), g(o[6], 3), g(o[7], 8));
    if o[1] > 0 {
        (v.n_filled, v.ilvl, v.quality) = (g(o[1], 3), g(o[2], 12), g(o[3], 4));
    }
    if ext {
        v.version = g(32, 10);
        let c = match (compact, ear) {
            (false, _) => Some(g(42, 32)),
            (true, false) => Some(g(66, 32)),
            _ => None,
        };
        v.code = c.map_or(*b"ear ", u32::to_le_bytes);
    } else if ear {
        v.code = *b"ear ";
    }
    Ok((Item { raw: buf[pos..pos + 2 + len].to_vec(), view: v, children: vec![], editable: false, dirty: false }, pos + 2 + len))
}
