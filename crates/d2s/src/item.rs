//! The v96 item codec. One symmetric routine (`codec`) drives both reading and writing, so an
//! item re-encodes exactly as it was parsed.
use crate::bits::{err, BitReader, BitWriter, Error, Result};
use crate::schema::Schema;

/// Marks an absent rare/crafted affix slot in `ItemView::qual`.
pub const NONE: u32 = u32::MAX;

/// One stored property: raw stored values (no Save Add or ValShift applied). `vals` has one entry,
/// or several for the grouped stats 17, 48, 50, 52, 54, 57 (id, id+1, ..).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prop {
    pub id: u32,
    pub param: u32,
    pub vals: Vec<u32>,
}

/// Every decoded field of an item, as stored in the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemView {
    /// Save version this item belongs to when it is below 96 (0 = the current layout).
    pub sv: u32,
    pub flags: u32,
    pub version: u32,
    pub location: u32,
    pub body: u32,
    pub x: u32,
    pub y: u32,
    pub store: u32,
    pub code: [u8; 4],
    pub ear_class: u32,
    pub ear_level: u32,
    /// Ear name (flag 16) or personalized name (flag 24), 7-bit chars without the terminator.
    pub name: Vec<u8>,
    pub gold: u32,
    pub quest_diff: u32,
    pub realm: Option<[u32; 3]>,
    pub n_filled: u32,
    pub seed: u32,
    pub ilvl: u32,
    pub quality: u32,
    pub gfx: Option<u32>,
    pub class_affix: Option<u32>,
    /// Quality fields in file order: low/superior [idx]; magic [prefix, suffix]; set/unique [id];
    /// rare/crafted [name1, name2, 6 affix slots (NONE if absent)]; 9 [name1, name2];
    /// normal: charm [affix] or [], body part [v], tome [v].
    pub qual: Vec<u32>,
    pub runeword: u32,
    pub defense: u32,
    pub maxdur: u32,
    pub dur: u32,
    pub quantity: u32,
    pub sockets: u32,
    pub set_lists: u32,
    pub props: Vec<Prop>,
    pub set_props: Vec<Vec<Prop>>,
    pub rw_props: Vec<Prop>,
}

impl ItemView {
    pub fn flag(&self, bit: u32) -> bool {
        self.flags >> bit & 1 == 1
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    /// Exact bytes of this item (from its "JM", padding included), without socketed children.
    pub raw: Vec<u8>,
    pub view: ItemView,
    /// Socketed items stored after this one (`view.n_filled` of them).
    pub children: Vec<Item>,
    /// True when `encode(view)` reproduces `raw` exactly.
    pub editable: bool,
    /// Set by `view_mut`; a dirty item is re-encoded from its view when saving.
    pub dirty: bool,
}

trait Io {
    fn rd(&self) -> bool;
    fn pos(&self) -> usize;
    fn f(&mut self, v: &mut u32, n: u32) -> Result<()>;
}

impl Io for BitReader<'_> {
    fn rd(&self) -> bool {
        true
    }
    fn pos(&self) -> usize {
        self.pos
    }
    fn f(&mut self, v: &mut u32, n: u32) -> Result<()> {
        *v = self.read(n)?;
        Ok(())
    }
}

impl Io for BitWriter {
    fn rd(&self) -> bool {
        false
    }
    fn pos(&self) -> usize {
        self.pos
    }
    fn f(&mut self, v: &mut u32, n: u32) -> Result<()> {
        self.write(*v, n)
    }
}

/// Stored width of stat `id` in item bit streams.
fn sb(sch: &Schema, id: u32, pos: usize) -> Result<u32> {
    match sch.stat(id) {
        Some(s) if s.save_bits > 0 => Ok(s.save_bits),
        _ => err(pos, format!("stat {id} has no save width")),
    }
}

/// Stored width of stat `id` for item version `sv`: the 1.09 column up to version 92.
fn wd(sch: &Schema, id: u32, sv: u32, pos: usize) -> Result<u32> {
    match sch.stat(id) {
        Some(s) if sv != 0 && sv <= 92 && s.bits109 > 0 => Ok(s.bits109),
        Some(s) if (sv == 0 || sv > 92) && s.save_bits > 0 => Ok(s.save_bits),
        _ => err(pos, format!("stat {id} has no save width in version {sv}")),
    }
}

/// Field widths of one property before version 93 (the 1.09 forms, see the format notes), None if unknown.
fn old_widths(sch: &Schema, id: u32, sv: u32, pos: usize) -> Result<Vec<u32>> {
    let g = |i: u32| match (sv, i) {
        (..=81, 48) => Ok(6),
        (..=81, 49 | 51 | 55 | 57) => Ok(7),
        (..=81, 58) => Ok(8),
        (..=87, 93 | 96 | 99 | 102 | 105) => Ok(6),
        (..=89, 23) => Ok(5),
        (..=89, 24) => Ok(6),
        (..=89, 74) => Ok(5),
        (..=89, 79) => Ok(8),
        (..=89, 80) => Ok(7),
        _ => wd(sch, i, sv, pos),
    };
    Ok(match id {
        83..=87 | 179 | 180 => vec![3],
        126 => vec![4],
        107..=109 | 181..=187 => vec![14],
        188..=193 => vec![10],
        195..=203 => vec![21],
        204..=213 => vec![if sv <= 73 { 29 } else { 30 }],
        134 if sv <= 89 => vec![16],
        1 if sv <= 85 => vec![g(1)?, g(9)?],
        3 if sv <= 85 => vec![g(3)?, g(7)?],
        _ => (0..group(id)).map(|k| g(id + k)).collect::<Result<_>>()?,
    })
}

/// A fixed list of fields; when reading, `vals` is resized, when writing its length must match.
fn many(io: &mut impl Io, vals: &mut Vec<u32>, w: &[u32]) -> Result<()> {
    if vals.len() != w.len() {
        if !io.rd() {
            return err(io.pos(), "field count mismatch");
        }
        vals.resize(w.len(), 0);
    }
    vals.iter_mut().zip(w).try_for_each(|(x, &n)| io.f(x, n))
}

/// A 1-bit presence flag followed by an `n`-bit value.
fn opt(io: &mut impl Io, v: &mut Option<u32>, n: u32) -> Result<()> {
    let mut p = v.is_some() as u32;
    io.f(&mut p, 1)?;
    *v = if p == 1 {
        let mut x = v.unwrap_or(0);
        io.f(&mut x, n)?;
        Some(x)
    } else {
        None
    };
    Ok(())
}

fn realm(io: &mut impl Io, r: &mut Option<[u32; 3]>, sv: u32) -> Result<()> {
    if sv != 0 && sv <= 86 {
        // No realm bit before version 87.
        return if r.is_none() { Ok(()) } else { err(io.pos(), "realm data does not exist in this version") };
    }
    let mut p = r.is_some() as u32;
    io.f(&mut p, 1)?;
    *r = if p == 1 {
        let mut a = r.unwrap_or_default();
        let n = if sv != 0 && sv <= 93 { 2 } else { 3 };
        a.iter_mut().take(n).try_for_each(|w| io.f(w, 32))?;
        Some(a)
    } else {
        None
    };
    Ok(())
}

/// Zero-terminated string of 7-bit characters.
fn str7(io: &mut impl Io, s: &mut Vec<u8>) -> Result<()> {
    if !io.rd() && s.contains(&0) {
        return err(io.pos(), "name contains a NUL character");
    }
    let mut i = 0;
    loop {
        let mut c = s.get(i).map_or(0, |&c| c as u32);
        io.f(&mut c, 7)?;
        if c == 0 {
            return Ok(());
        }
        if i == s.len() {
            s.push(c as u8);
        }
        i += 1;
    }
}

fn gold(io: &mut impl Io, v: &mut ItemView) -> Result<()> {
    let mut big = (v.gold > 4095) as u32;
    io.f(&mut big, 1)?;
    io.f(&mut v.gold, if big == 1 { 32 } else { 12 })
}

fn group(id: u32) -> u32 {
    match id {
        17 | 48 | 50 | 52 => 2,
        54 | 57 => 3,
        _ => 1,
    }
}

/// A property list: (9-bit id, param, values)* then 0x1FF.
fn props(io: &mut impl Io, l: &mut Vec<Prop>, sch: &Schema, sv: u32) -> Result<()> {
    let mut i = 0;
    loop {
        let mut id = l.get(i).map_or(0x1FF, |p| p.id);
        io.f(&mut id, 9)?;
        if id == 0x1FF {
            return Ok(());
        }
        if i == l.len() {
            l.push(Prop { id, ..Default::default() });
        }
        let Some(s) = sch.stat(id) else { return err(io.pos(), format!("unknown stat {id}")) };
        let p = &mut l[i];
        let old = sv != 0 && sv <= 92;
        if s.param_bits > 0 && !old {
            io.f(&mut p.param, s.param_bits)?;
        }
        let w = if old { old_widths(sch, id, sv, io.pos())? } else { (0..group(id)).map(|k| sb(sch, id + k, io.pos())).collect::<Result<Vec<_>>>()? };
        many(io, &mut p.vals, &w)?;
        i += 1;
    }
}

fn codec(io: &mut impl Io, v: &mut ItemView, sch: &Schema) -> Result<()> {
    let sv = v.sv;
    if sv == 71 {
        return err(io.pos(), "version 71 item records are kept raw");
    }
    let mut tag = 0x4D4A;
    io.f(&mut tag, 16)?;
    if tag != 0x4D4A {
        return err(io.pos() - 16, "item does not start with JM");
    }
    io.f(&mut v.flags, 32)?;
    for (f, n) in [(&mut v.version, 10), (&mut v.location, 3), (&mut v.body, 4), (&mut v.x, 4), (&mut v.y, 4), (&mut v.store, 3)] {
        io.f(f, n)?;
    }
    if v.location == 3 || v.location == 5 {
        return err(io.pos(), "ground items are not supported");
    }
    if v.flag(16) {
        io.f(&mut v.ear_class, 3)?;
        io.f(&mut v.ear_level, 7)?;
        str7(io, &mut v.name)?;
        return realm(io, &mut v.realm, sv);
    }
    for c in &mut v.code {
        let mut x = *c as u32;
        io.f(&mut x, 8)?;
        *c = x as u8;
    }
    // Before version 93 the game reads the code "nec " as "neg ".
    let key = if sv != 0 && sv <= 92 && v.code == *b"nec " { *b"neg " } else { v.code };
    let Some(base) = sch.base(key) else {
        return err(io.pos(), format!("unknown item code {:?}", String::from_utf8_lossy(&v.code)));
    };
    if v.flag(21) {
        if base.gold {
            gold(io, v)?;
        }
        if base.quest_diff && (sv == 0 || sv > 92) {
            io.f(&mut v.quest_diff, sb(sch, 356, io.pos())?)?;
        }
        return realm(io, &mut v.realm, sv);
    }
    io.f(&mut v.n_filled, 3)?;
    io.f(&mut v.seed, 32)?;
    io.f(&mut v.ilvl, 7)?;
    io.f(&mut v.quality, 4)?;
    opt(io, &mut v.gfx, 3)?;
    opt(io, &mut v.class_affix, 11)?;
    match v.quality {
        1 | 3 => many(io, &mut v.qual, &[3])?,
        4 => many(io, &mut v.qual, &[11, 11])?,
        5 | 7 => many(io, &mut v.qual, &[12])?,
        9 => many(io, &mut v.qual, &[8, 8])?,
        6 | 8 => {
            if io.rd() {
                v.qual = vec![NONE; 8];
            } else if v.qual.len() != 8 {
                return err(io.pos(), "rare item needs 8 quality fields");
            }
            for (i, s) in v.qual.iter_mut().enumerate() {
                if i < 2 {
                    io.f(s, 8)?;
                    continue;
                }
                let mut p = (*s != NONE) as u32;
                io.f(&mut p, 1)?;
                if p == 1 {
                    io.f(s, 11)?;
                } else {
                    *s = NONE;
                }
            }
        }
        2 if base.charm => {
            let mut p = !v.qual.is_empty() as u32;
            io.f(&mut p, 1)?;
            many(io, &mut v.qual, &[11][..p as usize])?;
        }
        2 => {
            let w: &[u32] = if base.body { &[10] } else if base.tome { &[5] } else { &[] };
            many(io, &mut v.qual, w)?;
        }
        q => return err(io.pos(), format!("invalid quality {q}")),
    }
    if v.flag(26) {
        io.f(&mut v.runeword, 16)?;
    }
    if v.flag(24) {
        str7(io, &mut v.name)?;
    }
    if v.quality == 5 && sv != 0 && sv <= 84 {
        return err(io.pos(), "set items before version 85 have no set list field");
    }
    realm(io, &mut v.realm, sv)?;
    if base.kind == 'a' {
        io.f(&mut v.defense, wd(sch, 31, sv, io.pos())?)?;
    }
    if base.kind != 'm' {
        io.f(&mut v.maxdur, wd(sch, 73, sv, io.pos())?)?;
        if v.maxdur > 0 {
            // Durability is 8 bits up to version 95.
            io.f(&mut v.dur, if sv != 0 { 8 } else { sb(sch, 72, io.pos())? })?;
        }
    }
    if base.gold {
        gold(io, v)?;
    }
    if base.stackable && (sv == 0 || sv > 80) {
        io.f(&mut v.quantity, 9)?;
    }
    if v.flag(11) {
        io.f(&mut v.sockets, wd(sch, 194, sv, io.pos())?)?;
    }
    if v.quality == 5 {
        io.f(&mut v.set_lists, 5)?;
    }
    props(io, &mut v.props, sch, sv)?;
    let mut n = 0;
    if v.quality == 5 {
        for k in 0..5 {
            if v.set_lists >> k & 1 == 1 {
                if n == v.set_props.len() {
                    if !io.rd() {
                        return err(io.pos(), "missing set bonus list");
                    }
                    v.set_props.push(vec![]);
                }
                props(io, &mut v.set_props[n], sch, sv)?;
                n += 1;
            }
        }
    }
    if !io.rd() && (n != v.set_props.len() || (!v.flag(26) && !v.rw_props.is_empty())) {
        return err(io.pos(), "set or runeword property lists do not match set_lists or the runeword flag");
    }
    if v.flag(26) {
        props(io, &mut v.rw_props, sch, sv)?;
    }
    Ok(())
}

fn ctx(e: Error, what: String) -> Error {
    Error { bit: e.bit, msg: format!("{what}: {}", e.msg) }
}

impl Item {
    /// Parses one item at byte `pos` (without its socketed children); returns it and the next offset.
    pub fn parse(buf: &[u8], pos: usize, sch: &Schema) -> Result<(Item, usize)> {
        Item::parse_v(buf, pos, sch, 0)
    }

    /// Like `parse` for a save of version `sv` (0 = current). Version 71 records are kept raw and non-editable.
    pub fn parse_v(buf: &[u8], pos: usize, sch: &Schema, sv: u32) -> Result<(Item, usize)> {
        if sv == 71 {
            return crate::legacy::v71_item(buf, pos);
        }
        let mut r = BitReader::new(buf, pos);
        let mut view = ItemView { sv, ..Default::default() };
        codec(&mut r, &mut view, sch)?;
        r.align()?;
        let end = r.byte_pos();
        let raw = buf[pos..end].to_vec();
        let editable = Item::encode(&view, sch).is_ok_and(|e| e == raw);
        Ok((Item { raw, view, children: vec![], editable, dirty: false }, end))
    }

    pub fn encode(view: &ItemView, sch: &Schema) -> Result<Vec<u8>> {
        let mut w = BitWriter::default();
        codec(&mut w, &mut view.clone(), sch)?;
        w.align();
        Ok(w.buf)
    }

    pub fn view_mut(&mut self) -> &mut ItemView {
        self.dirty = true;
        &mut self.view
    }

    /// Appends this item and its children. Clean items copy `raw`; dirty ones (or `force`) re-encode.
    /// A dirty item that failed its self-check (`!editable`) is refused: re-encoding it would lose data.
    pub fn write(&self, sch: &Schema, force: bool, out: &mut Vec<u8>) -> Result<()> {
        if self.view.n_filled as usize != self.children.len() {
            return err(out.len() * 8, "n_filled does not match the number of socketed items");
        }
        if self.dirty && !self.editable {
            return err(out.len() * 8, "item failed its self-check and cannot be re-encoded");
        }
        // Legacy items that failed their self-check are copied raw even when forced.
        if self.dirty || (force && (self.view.sv == 0 || self.editable)) {
            out.extend(Item::encode(&self.view, sch)?);
        } else {
            out.extend_from_slice(&self.raw);
        }
        self.children.iter().try_for_each(|c| c.write(sch, force, out))
    }
}

/// Parses one item at `pos` followed by its `n_filled` socketed children.
pub fn parse_tree(buf: &[u8], pos: usize, sch: &Schema) -> Result<(Item, usize)> {
    parse_tree_v(buf, pos, sch, 0)
}

pub fn parse_tree_v(buf: &[u8], pos: usize, sch: &Schema, sv: u32) -> Result<(Item, usize)> {
    let (mut it, mut p) = Item::parse_v(buf, pos, sch, sv)?;
    for j in 0..it.view.n_filled {
        let (c, np) = Item::parse_v(buf, p, sch, sv).map_err(|e| ctx(e, format!("socket {j}")))?;
        it.children.push(c);
        p = np;
    }
    Ok((it, p))
}

/// Parses an item list at `pos`: "JM", u16 count, then the items with their socketed children.
pub fn parse_list(buf: &[u8], pos: usize, sch: &Schema) -> Result<(Vec<Item>, usize)> {
    parse_list_v(buf, pos, sch, 0)
}

pub fn parse_list_v(buf: &[u8], pos: usize, sch: &Schema, sv: u32) -> Result<(Vec<Item>, usize)> {
    if buf.get(pos..pos + 2) != Some(b"JM") || buf.len() < pos + 4 {
        return err(pos * 8, "item list tag JM missing");
    }
    let n = u16::from_le_bytes([buf[pos + 2], buf[pos + 3]]);
    let (mut p, mut items) = (pos + 4, vec![]);
    for i in 0..n {
        let (it, np) = parse_tree_v(buf, p, sch, sv).map_err(|e| ctx(e, format!("item {i}")))?;
        p = np;
        items.push(it);
    }
    Ok((items, p))
}

pub fn write_list(items: &[Item], sch: &Schema, force: bool, out: &mut Vec<u8>) -> Result<()> {
    let Ok(n) = u16::try_from(items.len()) else { return err(out.len() * 8, "more than 65535 items in one list") };
    out.extend_from_slice(b"JM");
    out.extend_from_slice(&n.to_le_bytes());
    items.iter().enumerate().try_for_each(|(i, it)| it.write(sch, force, out).map_err(|e| ctx(e, format!("item {i}"))))
}
