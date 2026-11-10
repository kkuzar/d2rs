//! Lossless v96 save model: clean parts are copied raw, edited parts are re-encoded.
use crate::bits::{err, BitReader, BitWriter, Error, Result};
use crate::item::{parse_list_v, parse_tree_v, write_list, Item};
use crate::schema::Schema;

const HDR: usize = 0x14F;

pub const HARDCORE: u32 = 2;
pub const DIED: u32 = 3;
pub const EXPANSION: u32 = 5;
pub const LADDER: u32 = 6;

/// A character stat: (id, param, value). Values are the stored ones (life/mana/stamina are <<8).
pub type Stat = (u32, i32, i64);

#[derive(Clone, Debug)]
pub struct Corpse {
    /// The 12 bytes the game never reads.
    pub raw: [u8; 12],
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Save {
    /// File version (71..=96). A legacy (< 92) header is 0x82 bytes at the start of this array, the rest is zero.
    pub ver: u32,
    pub header: [u8; HDR],
    /// Quests ("Woo!"), waypoints ("WS") and NPCs (0x01 'w') sections, 0x14F..0x2FD.
    pub blocks: Vec<u8>,
    pub stats: Vec<Stat>,
    gf_raw: Vec<u8>,
    gf_dirty: bool,
    /// "if" + one level byte per skill.
    pub skills: Vec<u8>,
    pub items: Vec<Item>,
    pub corpses: Vec<Corpse>,
    /// "jf" present (expansion trailer).
    pub jf: bool,
    /// Mercenary item list; present exactly when the header merc block is non-zero.
    pub merc: Option<Vec<Item>>,
    /// "kf" section: Some(None) = no golem item, Some(Some(item)) = golem item.
    pub kf: Option<Option<Item>>,
    /// Legacy (< 92) mercenary head, kept raw: v71 `JM` + 6 bytes, else a 0 byte or 0x12 bytes (then `merc` holds its items).
    pub lmerc: Vec<u8>,
    /// Bytes after the last known section, kept verbatim.
    pub tail: Vec<u8>,
}

fn sx(v: u32, n: u32) -> i64 {
    (i64::from(v) << (64 - n)) >> (64 - n)
}

/// Two's complement / unsigned encoding of `v` in `n` bits, None if it does not fit.
fn fit(v: i64, n: u32, signed: bool) -> Option<u32> {
    let ok = if signed { sx(v as u32, n) == v && v >= i64::from(i32::MIN) } else { (0..1i64 << n).contains(&v) };
    ok.then_some((v as u64 & ((1u64 << n) - 1)) as u32)
}

/// The game's rule (D2S_ReadMercHeader): a mercenary exists unless its seed, name and experience are all zero.
fn has_merc(h: &[u8; HDR]) -> bool {
    h[0xB3..0xB9].iter().chain(&h[0xBB..0xBF]).any(|&x| x != 0)
}

fn ctx(e: Error, what: &str) -> Error {
    Error { bit: e.bit, msg: format!("{what}: {}", e.msg) }
}

macro_rules! hdr {
    ($($get:ident $set:ident $t:ty = $off:expr, $lo:expr;)*) => {
        impl Save {$(
            pub fn $get(&self) -> $t {
                let (o, mut a) = (self.at($off, $lo), [0u8; std::mem::size_of::<$t>()]);
                a.copy_from_slice(&self.header[o..o + std::mem::size_of::<$t>()]);
                <$t>::from_le_bytes(a)
            }
            pub fn $set(&mut self, v: $t) {
                let o = self.at($off, $lo);
                self.header[o..o + std::mem::size_of::<$t>()].copy_from_slice(&v.to_le_bytes());
            }
        )*}
    };
}

hdr! {
    progression set_progression u8 = 0x25, 0x19;
    class set_class u8 = 0x28, 0x22;
    level set_level u8 = 0x2B, 0x24;
    // A legacy header has no mercenary block: these hit unused zero padding (writes are dropped).
    merc_dead set_merc_dead u16 = 0xB1, 0x100;
    merc_seed set_merc_seed u32 = 0xB3, 0x104;
    merc_name set_merc_name u16 = 0xB7, 0x108;
    merc_type set_merc_type u16 = 0xB9, 0x10A;
    merc_exp set_merc_exp u32 = 0xBB, 0x10C;
    raw_map_seed set_raw_map_seed u32 = 0xAB, 0x7E;
}

impl Save {
    pub fn map_seed(&self) -> u32 {
        self.raw_map_seed()
    }
    /// Legacy (< 92) seeds are read-only.
    pub fn set_map_seed(&mut self, v: u32) -> Result<()> {
        if self.ver < 92 {
            return err(0, "legacy save: map seed is read-only");
        }
        self.set_raw_map_seed(v);
        Ok(())
    }
    /// Bit 7 of header byte 0xA8 + diff (the low bits hold the act).
    pub fn seed_reuse(&self, diff: usize) -> bool {
        self.ver >= 92 && diff < 3 && self.header[0xA8 + diff] & 0x80 != 0
    }
    pub fn set_seed_reuse(&mut self, diff: usize, on: bool) -> Result<()> {
        if self.ver < 92 || diff > 2 {
            return err(0, "seed reuse needs a 92+ save and difficulty 0..=2");
        }
        let b = &mut self.header[0xA8 + diff];
        *b = *b & 0x7F | u8::from(on) << 7;
        Ok(())
    }
}

impl Save {
    /// Header offset of a field: `a` for 92+ headers, `l` for the 0x82-byte legacy one.
    fn at(&self, a: usize, l: usize) -> usize {
        if self.ver < 92 { l } else { a }
    }

    pub fn parse(b: &[u8], sch: &Schema) -> Result<Save> {
        if b.len() < 8 || b[..4] != [0x55, 0xAA, 0x55, 0xAA] {
            return err(0, "not a d2s file");
        }
        let ver = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        if !(71..=96).contains(&ver) {
            return err(32, format!("version {ver} not supported"));
        }
        let hl = if ver < 92 { 0x82 } else { HDR };
        if b.len() < hl + 0x1AE || (hl == 0x82 && b[0x20..0x22] != [0x82, 0]) {
            return err(0, "file or header too short");
        }
        let (gf, mut header) = (hl + 0x1AE, [0u8; HDR]);
        header[..hl].copy_from_slice(&b[..hl]);
        let u16at = |o: usize| b.get(o..o + 2).map_or(0, |x| u16::from_le_bytes([x[0], x[1]]));
        for (o, tag, at, size) in [(hl, &b"Woo!"[..], hl + 8, 298), (hl + 0x12A, b"WS", hl + 0x130, 80), (hl + 0x17A, &[1, b'w'], hl + 0x17C, 52)] {
            // The legacy reader tests the tags only.
            if b.get(o..o + tag.len()) != Some(tag) || (ver >= 92 && u16at(at) != size) {
                return err(o * 8, format!("section marker {tag:?} / size {size} wrong"));
            }
        }
        if b.get(gf..gf + 2) != Some(b"gf") {
            return err(gf * 8, "stats tag gf missing");
        }
        let mut r = BitReader::new(b, gf + 2);
        let mut stats = vec![];
        if ver < 95 {
            // The old mask form: 3 mask bytes below 92, else ceil(header[0x29] / 8); one u32 per set bit.
            let nbits = if ver < 92 { 16 } else { u32::from(b[0x29]) };
            let nb = if ver < 92 { 3 } else { nbits.div_ceil(8) as usize };
            let Some(m) = b.get(gf + 2..gf + 2 + nb) else { return err(gf * 8, "stats mask truncated") };
            // Below 92 the third mask byte is never read by the game (kept raw in gf_raw).
            let mask = m.iter().take(if ver < 92 { 2 } else { nb }).rev().fold(0u64, |a, &x| a << 8 | u64::from(x));
            if mask >> nbits.min(63) != 0 {
                return err(gf * 8, "stats mask has bits above the stat count");
            }
            r.pos = (gf + 2 + nb) * 8;
            for id in (0..nbits).filter(|i| mask >> i & 1 == 1) {
                stats.push((id, 0, i64::from(r.read(32).map_err(|e| ctx(e, "stats"))?)));
            }
        }
        loop {
            if ver < 95 {
                break;
            }
            let id = r.read(9).map_err(|e| ctx(e, "stats"))?;
            if id == 0x1FF {
                break;
            }
            let s = sch.stat(id).filter(|s| s.csv_bits > 0);
            let Some(s) = s else { return err(r.pos, format!("stats: stat {id} cannot be stored")) };
            let param = if s.csv_param > 0 { sx(r.read(s.csv_param)?, s.csv_param) as i32 } else { 0 };
            let v = r.read(s.csv_bits)?;
            stats.push((id, param, if s.csv_signed && s.csv_bits < 32 { sx(v, s.csv_bits) } else { v.into() }));
        }
        r.align().map_err(|e| ctx(e, "stats"))?;
        let mut p = r.byte_pos();
        let gf_raw = b[gf..p].to_vec();
        let n = if ver < 92 { 30 } else { header[0x2A] as usize };
        if b.get(p..p + 2) != Some(b"if") || b.len() < p + 2 + n {
            return err(p * 8, "skills section if missing or truncated");
        }
        let skills = b[p..p + 2 + n].to_vec();
        p += 2 + n;
        let sv = if ver < 96 { ver } else { 0 };
        let (items, np) = parse_list_v(b, p, sch, sv).map_err(|e| ctx(e, "player items"))?;
        p = np;
        if b.get(p..p + 2) != Some(b"JM") || b.len() < p + 4 {
            return err(p * 8, "corpse list missing");
        }
        let nc = u16::from_le_bytes([b[p + 2], b[p + 3]]);
        if nc > 1 {
            return err(p * 8, format!("{nc} corpses (0 or 1 allowed)"));
        }
        p += 4;
        let mut corpses = vec![];
        for i in 0..nc {
            let Some(raw) = b.get(p..p + 12) else { return err(p * 8, "corpse truncated") };
            let (items, np) = parse_list_v(b, p + 12, sch, sv).map_err(|e| ctx(e, &format!("corpse {i} items")))?;
            let mut c = Corpse { raw: [0; 12], items };
            c.raw.copy_from_slice(raw);
            corpses.push(c);
            p = np;
        }
        let (mut merc, mut kf, mut lmerc) = (None, None, vec![]);
        if ver < 92 {
            // Legacy mercenary head; the game also tolerates a file that simply ends here.
            let n = match (ver, b.get(p)) {
                (71, _) => 8,
                (_, Some(0)) => 1,
                (_, Some(_)) => 0x12,
                _ => 0,
            };
            let Some(h) = b.get(p..p + n) else { return err(p * 8, "mercenary head truncated") };
            if ver == 71 && h[..2] != *b"JM" {
                return err(p * 8, "v71 mercenary JM missing");
            }
            lmerc = h.to_vec();
            p += n;
            if n == 0x12 {
                let (l, np) = parse_list_v(b, p, sch, sv).map_err(|e| ctx(e, "mercenary items"))?;
                (merc, p) = (Some(l), np);
            }
        }
        let jf = ver >= 92 && b.get(p..p + 2) == Some(b"jf");
        if jf {
            p += 2;
            if b.get(p..p + 2) == Some(b"JM") {
                let (l, np) = parse_list_v(b, p, sch, sv).map_err(|e| ctx(e, "mercenary items"))?;
                merc = Some(l);
                p = np;
            }
            if b.get(p..p + 2) == Some(b"kf") && b.len() > p + 2 {
                let golem = b[p + 2];
                p += 3;
                if golem > 1 {
                    return err(p * 8 - 8, format!("golem flag {golem}"));
                }
                kf = Some(if golem == 1 {
                    let (it, np) = parse_tree_v(b, p, sch, sv).map_err(|e| ctx(e, "golem item"))?;
                    (p = np, Some(it)).1
                } else {
                    None
                });
            }
        }
        // Only an expansion save (jf) stores the mercenary's items; a classic one has a hireling but no list.
        if jf && has_merc(&header) != merc.is_some() {
            return err(0xAF * 8, "header mercenary block and mercenary item list disagree");
        }
        Ok(Save { ver, header, blocks: b[hl..gf].to_vec(), lmerc, stats, gf_raw, gf_dirty: false, skills, items, corpses, jf, merc, kf, tail: b[p..].to_vec() })
    }

    /// Character name (header 0x14, NUL padded).
    pub fn name(&self) -> String {
        let a = self.at(0x14, 8);
        let n = &self.header[a..a + 16];
        String::from_utf8_lossy(&n[..n.iter().position(|&c| c == 0).unwrap_or(n.len())]).into()
    }

    pub fn set_name(&mut self, s: &str) -> Result<()> {
        if s.is_empty() || s.len() > 15 || !s.is_ascii() {
            return err(0x14 * 8, "name must be 1..15 ASCII characters");
        }
        let a = self.at(0x14, 8);
        self.header[a..a + 16].fill(0);
        self.header[a..a + s.len()].copy_from_slice(s.as_bytes());
        Ok(())
    }

    /// Status bit of header byte 0x24: HARDCORE, DIED, EXPANSION or LADDER.
    pub fn status(&self, bit: u32) -> bool {
        self.header[self.at(0x24, 0x18)] >> bit & 1 == 1
    }

    pub fn set_status(&mut self, bit: u32, on: bool) {
        let a = self.at(0x24, 0x18);
        self.header[a] = self.header[a] & !(1 << bit) | (on as u8) << bit;
    }

    pub fn get_stat(&self, id: u32) -> Option<i64> {
        self.stats.iter().find(|s| s.0 == id).map(|s| s.2)
    }

    /// Sets (or adds, keeping id order) a stat; marks the stats section dirty. Range is checked on write.
    pub fn set_stat(&mut self, id: u32, value: i64) {
        self.gf_dirty = true;
        match self.stats.iter_mut().find(|s| s.0 == id) {
            Some(s) => s.2 = value,
            None => self.stats.insert(self.stats.partition_point(|s| s.0 < id), (id, 0, value)),
        }
    }

    /// Range-checked `set_stat` (see `limits::char_stat`): life, mana and stamina take the shown value (8 fraction bits added).
    pub fn set_stat_checked(&mut self, sch: &Schema, id: u32, value: i64) -> Result<()> {
        let level = self.get_stat(12).unwrap_or(self.level().into());
        let Some(r) = crate::limits::char_stat(sch, id, level) else { return err(0, format!("stat {id} cannot be stored")) };
        crate::limits::check(sch.stat(id).map_or_else(String::new, |s| s.name.clone()), value, r).or_else(|m| err(0, m))?;
        self.set_stat(id, if (6..=11).contains(&id) { value << 8 } else { value });
        Ok(())
    }

    /// Range-checked level: the header byte and stat 12 together.
    pub fn set_level_checked(&mut self, sch: &Schema, level: i64) -> Result<()> {
        self.set_stat_checked(sch, 12, level)?;
        self.set_level(level as u8);
        Ok(())
    }

    /// Sets the base level of the class's `i`-th skill, within `limits::skill_level`.
    pub fn set_skill(&mut self, i: usize, level: i64) -> Result<()> {
        crate::limits::check(format!("skill {i} level"), level, crate::limits::skill_level()).or_else(|m| err(0, m))?;
        let Some(b) = self.skills.get_mut(2 + i) else { return err(0, format!("no skill {i}")) };
        *b = level as u8;
        Ok(())
    }

    /// Every item in the file, socketed children included.
    pub fn all_items(&self) -> Vec<&Item> {
        let tops = self.items.iter().chain(self.corpses.iter().flat_map(|c| &c.items)).chain(self.merc.iter().flatten()).chain(self.kf.iter().flatten());
        tops.flat_map(|i| std::iter::once(i).chain(&i.children)).collect()
    }

    fn encode_stats(&self, sch: &Schema) -> Result<Vec<u8>> {
        let mut w = BitWriter::default();
        w.bytes(b"gf");
        for &(id, param, v) in &self.stats {
            let s = sch.stat(id).filter(|s| s.csv_bits > 0);
            let Some(s) = s else { return err(w.pos, format!("stat {id} cannot be stored")) };
            let bad = || Error { bit: w.pos, msg: format!("value {v} (param {param}) of stat {id} does not fit") };
            let (pv, vv) = (fit(param.into(), s.csv_param.max(1), true), fit(v, s.csv_bits, s.csv_signed && s.csv_bits < 32));
            let (Some(pv), Some(vv)) = (pv, vv) else { return Err(bad()) };
            w.write(id, 9)?;
            if s.csv_param > 0 {
                w.write(pv, s.csv_param)?;
            }
            w.write(vv, s.csv_bits)?;
        }
        w.write(0x1FF, 9)?;
        Ok(w.buf)
    }

    /// The old mask form of the stats section (versions below 95).
    fn encode_stats_old(&self) -> Result<Vec<u8>> {
        let nbits = if self.ver < 92 { 16 } else { u32::from(self.header[0x29]) };
        let nb = if self.ver < 92 { 3 } else { nbits.div_ceil(8) as usize };
        let (mut m, mut vals): (Vec<u8>, Vec<(u32, u32)>) = (vec![0u8; nb], vec![]);
        if self.ver < 92 {
            m[2] = self.gf_raw.get(4).copied().unwrap_or(0);
        }
        for &(id, _, v) in &self.stats {
            let Ok(v) = u32::try_from(v) else { return err(0, format!("value {v} of stat {id} does not fit 32 bits")) };
            if id >= nbits || vals.iter().any(|x| x.0 == id) {
                return err(0, format!("stat {id} cannot be stored in the version {} stats form", self.ver));
            }
            m[id as usize / 8] |= 1 << (id % 8);
            vals.push((id, v));
        }
        vals.sort_unstable();
        Ok([&b"gf"[..], &m, &vals.iter().flat_map(|v| v.1.to_le_bytes()).collect::<Vec<_>>()].concat())
    }

    fn encode(&self, sch: &Schema, force: bool) -> Result<Vec<u8>> {
        let legacy = self.ver < 92;
        let want = if self.ver < 96 { self.ver } else { 0 };
        if self.all_items().iter().any(|i| i.view.sv != want) {
            return err(0, format!("an item of another version cannot be written into a version {} save", self.ver));
        }
        let merc_ok = legacy || if self.jf { has_merc(&self.header) == self.merc.is_some() } else { self.merc.is_none() && self.kf.is_none() };
        if !merc_ok {
            return err(0xAF * 8, "mercenary header, mercenary item list and jf/kf trailer disagree");
        }
        let mut o = self.header[..if legacy { 0x82 } else { HDR }].to_vec();
        o.extend_from_slice(&self.blocks);
        if force || self.gf_dirty {
            let st = if self.ver < 95 { self.encode_stats_old() } else { self.encode_stats(sch) };
            o.extend(st.map_err(|e| ctx(e, "stats"))?);
        } else {
            o.extend_from_slice(&self.gf_raw);
        }
        o.extend_from_slice(&self.skills);
        write_list(&self.items, sch, force, &mut o).map_err(|e| ctx(e, "player items"))?;
        o.extend_from_slice(b"JM");
        o.extend_from_slice(&(self.corpses.len() as u16).to_le_bytes());
        for c in &self.corpses {
            o.extend_from_slice(&c.raw);
            write_list(&c.items, sch, force, &mut o).map_err(|e| ctx(e, "corpse items"))?;
        }
        o.extend_from_slice(&self.lmerc);
        if self.jf {
            o.extend_from_slice(b"jf");
        }
        if let Some(m) = &self.merc {
            write_list(m, sch, force, &mut o).map_err(|e| ctx(e, "mercenary items"))?;
        }
        if let Some(g) = &self.kf {
            o.extend_from_slice(&[b'k', b'f', g.is_some() as u8]);
            if let Some(it) = g {
                it.write(sch, force, &mut o).map_err(|e| ctx(e, "golem item"))?;
            }
        }
        o.extend_from_slice(&self.tail);
        if !legacy {
            crate::fix_header(&mut o);
        }
        Ok(o)
    }

    /// The file bytes: clean parts raw, edited parts re-encoded, then size and checksum fixed.
    pub fn to_bytes(&self, sch: &Schema) -> Result<Vec<u8>> {
        self.encode(sch, false)
    }

    /// Open-time self-check: parse, re-encode everything from the decoded model, compare with the input.
    pub fn verify(b: &[u8], sch: &Schema) -> Result<()> {
        let o = Save::parse(b, sch)?.encode(sch, true)?;
        match o.iter().zip(b).position(|(x, y)| x != y) {
            Some(i) => err(i * 8, "forced re-encode differs from the input"),
            None if o.len() != b.len() => err(o.len().min(b.len()) * 8, "forced re-encode has a different length"),
            None => Ok(()),
        }
    }
}
