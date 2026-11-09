//! PlugY stash files: personal `.d2x` ("CSTM") and shared `.sss` ("SSS\0"), pages of v96 item lists.
use crate::bits::{Error, Result};
use crate::item::{parse_list, write_list, Item};
use crate::Schema;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Personal { reserved: u32 },
    /// `gold` is `Some` for version "02" files, `None` for version "01".
    Shared { gold: Option<u32> },
}

#[derive(Clone, Debug)]
pub struct Page {
    /// `None` for pre-11.02 files that have no flags word.
    pub flags: Option<u32>,
    pub name: Vec<u8>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Stash {
    pub kind: Kind,
    pub pages: Vec<Page>,
    /// Bytes after the last page, kept verbatim.
    pub trailing: Vec<u8>,
}

fn fail<T>(pos: usize, msg: String) -> Result<T> {
    Err(Error { bit: pos * 8, msg })
}

fn u32_at(b: &[u8], p: usize, what: &str) -> Result<u32> {
    match b.get(p..p + 4) {
        Some(s) => Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]])),
        None => fail(p, format!("truncated {what}")),
    }
}

/// Offset of the item list after a NUL-terminated name starting at `p`, if it begins with "JM".
fn list_after_name(b: &[u8], p: usize) -> Option<usize> {
    let n = b.get(p..)?.iter().position(|&c| c == 0)?;
    (b.get(p + n + 1..p + n + 3) == Some(b"JM")).then_some(p + n + 1)
}

impl Stash {
    pub fn parse(b: &[u8], sch: &Schema) -> Result<Stash> {
        let (kind, mut p) = match (b.get(0..4), b.get(4..6)) {
            (Some(b"CSTM"), Some(b"01")) => (Kind::Personal { reserved: u32_at(b, 6, "header")? }, 10),
            (Some(b"SSS\0"), Some(b"01")) => (Kind::Shared { gold: None }, 6),
            (Some(b"SSS\0"), Some(b"02")) => (Kind::Shared { gold: Some(u32_at(b, 6, "gold")?) }, 10),
            _ => return fail(0, "not a PlugY stash (bad magic or version)".into()),
        };
        let n = u32_at(b, p, "page count")?;
        p += 4;
        let mut pages = vec![];
        for i in 0..n {
            let at = p;
            if b.get(p..p + 2) != Some(b"ST") {
                return fail(at, format!("page {i}: tag ST missing"));
            }
            p += 2;
            // PlugY's loadStash rule: no flags word when "JM" follows the first NUL, else a flags word comes first.
            let (flags, lp) = match list_after_name(b, p) {
                Some(lp) => (None, lp),
                None => match list_after_name(b, p + 4) {
                    Some(lp) => (Some(u32_at(b, p, "page flags")?), lp),
                    None => return fail(at, format!("page {i}: name or item list JM missing")),
                },
            };
            let name = b[p + if flags.is_some() { 4 } else { 0 }..lp - 1].to_vec();
            let (items, np) = parse_list(b, lp, sch).map_err(|e| Error { bit: e.bit, msg: format!("page {i}: {}", e.msg) })?;
            p = np;
            pages.push(Page { flags, name, items });
        }
        Ok(Stash { kind, pages, trailing: b[p..].to_vec() })
    }

    fn encode(&self, sch: &Schema, force: bool) -> Result<Vec<u8>> {
        let mut o = vec![];
        match self.kind {
            Kind::Personal { reserved } => {
                o.extend_from_slice(b"CSTM01");
                o.extend_from_slice(&reserved.to_le_bytes());
            }
            Kind::Shared { gold: None } => o.extend_from_slice(b"SSS\x0001"),
            Kind::Shared { gold: Some(g) } => {
                o.extend_from_slice(b"SSS\x0002");
                o.extend_from_slice(&g.to_le_bytes());
            }
        }
        o.extend_from_slice(&(self.pages.len() as u32).to_le_bytes());
        for (i, pg) in self.pages.iter().enumerate() {
            if pg.name.contains(&0) {
                return fail(o.len(), format!("page {i}: name contains a NUL byte"));
            }
            o.extend_from_slice(b"ST");
            if let Some(f) = pg.flags {
                o.extend_from_slice(&f.to_le_bytes());
            }
            o.extend_from_slice(&pg.name);
            o.push(0);
            let at = o.len();
            write_list(&pg.items, sch, force, &mut o).map_err(|e| Error { bit: at * 8, msg: format!("page {i}: {}", e.msg) })?;
        }
        o.extend_from_slice(&self.trailing);
        Ok(o)
    }

    /// Clean items are copied raw, dirty items re-encoded.
    pub fn to_bytes(&self, sch: &Schema) -> Result<Vec<u8>> {
        self.encode(sch, false)
    }

    /// Checks that a forced re-encode of every item reproduces `bytes` exactly.
    pub fn verify(bytes: &[u8], sch: &Schema) -> Result<()> {
        if Stash::parse(bytes, sch)?.encode(sch, true)? != bytes {
            return fail(0, "forced re-encode differs from input".into());
        }
        Ok(())
    }

    /// Appends an empty page. PlugY loads the flags word over its page record, so a shared page needs bit 0 (isShared).
    pub fn add_page(&mut self, name: &[u8]) -> &mut Page {
        let shared = matches!(self.kind, Kind::Shared { .. }) as u32;
        self.pages.push(Page { flags: Some(shared), name: name.to_vec(), items: vec![] });
        let n = self.pages.len();
        &mut self.pages[n - 1]
    }

    /// Removes page `i`; refuses a non-empty page unless `force`.
    pub fn remove_page(&mut self, i: usize, force: bool) -> Result<Page> {
        match self.pages.get(i) {
            None => fail(0, format!("page {i}: no such page")),
            Some(p) if !p.items.is_empty() && !force => fail(0, format!("page {i}: not empty ({} items)", p.items.len())),
            Some(_) => Ok(self.pages.remove(i)),
        }
    }

    /// Sets shared gold; a v01 file becomes v02 only when `gold` is non-zero. Errors on personal stashes.
    pub fn set_gold(&mut self, gold: u32) -> Result<()> {
        match &mut self.kind {
            Kind::Shared { gold: g } => {
                if gold != 0 || g.is_some() {
                    *g = Some(gold);
                }
                Ok(())
            }
            Kind::Personal { .. } => fail(0, "personal stash has no gold".into()),
        }
    }
}
