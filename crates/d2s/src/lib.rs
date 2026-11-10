//! Lossless game save codec (classic and expansion, up to 1.14d).

pub mod bits;
mod data;
pub mod edit;
pub mod game;
pub mod item;
pub mod itemgen;
pub mod limits;
pub mod legacy;
pub mod legit;
pub mod plugy;
pub mod props;
pub mod rng;
pub mod save;
pub mod schema;
pub mod tables;
pub mod validate;

pub use bits::{Error, Result};
pub use item::{Item, ItemView, Prop};
pub use save::Save;
pub use schema::Schema;

/// The .d2s checksum: rotl32(sum, 1) + byte over the file with bytes 12..16 as zero (F§4.7).
pub fn checksum(b: &[u8]) -> u32 {
    b.iter().enumerate().fold(0u32, |s, (i, &x)| {
        s.rotate_left(1).wrapping_add(if (12..16).contains(&i) { 0 } else { x as u32 })
    })
}

/// Writes file_size (0x08) then checksum (0x0C) into a v92+ save buffer.
pub fn fix_header(b: &mut [u8]) {
    let len = b.len() as u32;
    b[8..12].copy_from_slice(&len.to_le_bytes());
    let c = checksum(b);
    b[12..16].copy_from_slice(&c.to_le_bytes());
}
