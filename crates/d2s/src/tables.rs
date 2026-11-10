//! Item generation tables (uniques, sets, affixes, runewords, properties, quality), parsed from the embedded 1.14d TSVs.
use crate::bits::Result;
use crate::schema::{merge, named, num, rows, txt, Row};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static ACTIVE: RwLock<Option<&'static ItemTables>> = RwLock::new(None);

/// One property slot of a row: `code`, `par`, `min`, `max`.
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub code: String,
    pub par: String,
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Unique {
    pub name: String,
    pub code: [u8; 4],
    pub version: u32,
    pub enabled: bool,
    pub rarity: u32,
    pub lvl: u32,
    pub lvl_req: u32,
    pub props: Vec<Block>,
    /// Own inventory picture and `invtransform` colour (colors.txt row).
    pub inv: String,
    pub inv_trans: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct SetItem {
    pub name: String,
    pub set: String,
    pub code: [u8; 4],
    pub rarity: u32,
    pub lvl: u32,
    pub lvl_req: u32,
    pub add_func: u32,
    pub props: Vec<Block>,
    pub aprops: [Vec<Block>; 5],
    pub inv: String,
    pub inv_trans: Option<u8>,
}

/// A magic prefix, magic suffix or automagic row.
#[derive(Debug, Clone, Default)]
pub struct Affix {
    pub name: String,
    pub version: u32,
    pub spawnable: bool,
    pub rare: bool,
    pub level: u32,
    pub maxlevel: u32,
    pub level_req: u32,
    /// Class index (ama, sor, nec, pal, bar, dru, ass = 0..6) when the affix is class specific.
    pub class_specific: Option<u8>,
    pub frequency: u32,
    pub group: u32,
    pub mods: Vec<Block>,
    pub itypes: Vec<String>,
    pub etypes: Vec<String>,
    /// `transformcolor` (colors.txt row).
    pub color: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct RareName {
    pub name: String,
    pub version: u32,
    pub itypes: Vec<String>,
    pub etypes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Runeword {
    /// `RunewordN`, the Runes.txt row name.
    pub key: String,
    pub name: String,
    pub complete: bool,
    pub server: bool,
    pub itypes: Vec<String>,
    pub etypes: Vec<String>,
    pub runes: Vec<[u8; 4]>,
    pub props: Vec<Block>,
}

/// A Properties.txt row: slots of (func, stat id, set, val).
#[derive(Debug, Clone, Default)]
pub struct PropDef {
    pub code: String,
    pub slots: Vec<(u8, Option<u32>, u32, i32)>,
}

/// A QualityItems.txt row (a magic item's built-in modifiers by item class).
#[derive(Debug, Clone, Default)]
pub struct Quality {
    pub nummods: u32,
    pub mods: Vec<Block>,
    /// (min, max) of ToHit, Dam%, AC%, Dur%.
    pub ranges: [(i32, i32); 4],
    pub effects: Vec<String>,
    /// Item class columns set to 1 (armor, weapon, shield, ...).
    pub classes: Vec<String>,
    pub level: u32,
    pub multiply: u32,
    pub add: u32,
}

#[derive(Debug, Default)]
pub struct ItemTables {
    pub uniques: Vec<Unique>,
    pub set_items: Vec<SetItem>,
    pub magic_prefix: Vec<Affix>,
    pub magic_suffix: Vec<Affix>,
    pub automagic: Vec<Affix>,
    pub rare_prefix: Vec<RareName>,
    pub rare_suffix: Vec<RareName>,
    pub runewords: Vec<Runeword>,
    pub props: Vec<PropDef>,
    pub quality: Vec<Quality>,
    pub low_quality: Vec<String>,
}

const CLASSES: [&str; 7] = ["ama", "sor", "nec", "pal", "bar", "dru", "ass"];
const MOD: [&str; 4] = ["code", "param", "min", "max"];
const BLOCK: [&str; 4] = ["prop", "par", "min", "max"];

/// colors.txt row order: the row a `invtransform` / `transformcolor` code names is the map index.
const COLORS: [&str; 21] = ["whit", "lgry", "dgry", "blac", "lblu", "dblu", "cblu", "lred", "dred", "cred", "lgrn", "dgrn", "cgrn", "lyel", "dyel", "lgld", "dgld", "lpur", "dpur", "oran", "bwht"];

fn color(r: &Row, k: &str) -> Option<u8> {
    COLORS.iter().position(|c| *c == txt(r, k)).map(|c| c as u8)
}

fn code4(s: &str) -> [u8; 4] {
    let mut k = [b' '; 4];
    for (d, b) in k.iter_mut().zip(s.bytes()) {
        *d = b;
    }
    k
}

/// Blocks over numbered columns: `names` maps a slot index to the four column names.
fn blocks(r: &Row, n: usize, names: impl Fn(usize) -> [String; 4]) -> Vec<Block> {
    (1..=n)
        .filter_map(|i| {
            let [c, p, lo, hi] = names(i);
            let code = txt(r, &c);
            (!code.is_empty()).then(|| Block { code: code.into(), par: txt(r, &p).into(), min: num(r, &lo), max: num(r, &hi) })
        })
        .collect()
}

/// Blocks over `n` numbered slots; `fmt` turns a column word and a slot index into a column name.
fn blk(r: &Row, n: usize, words: [&str; 4], fmt: impl Fn(&str, usize) -> String) -> Vec<Block> {
    blocks(r, n, |i| words.map(|w| fmt(w, i)))
}

fn list(r: &Row, k: &str, n: usize) -> Vec<String> {
    (1..=n).map(|i| txt(r, &format!("{k}{i}"))).filter(|v| !v.is_empty()).map(String::from).collect()
}

fn affixes(s: &str) -> Vec<Affix> {
    rows(s)
        .iter()
        .map(|r| Affix {
            name: txt(r, "Name").into(),
            version: num(r, "version"),
            spawnable: txt(r, "spawnable") == "1",
            rare: txt(r, "rare") == "1",
            level: num(r, "level"),
            maxlevel: num(r, "maxlevel"),
            level_req: num(r, "levelreq"),
            class_specific: [txt(r, "classspecific"), txt(r, "class")].iter().find_map(|c| CLASSES.iter().position(|x| x == c)).map(|c| c as u8),
            frequency: num(r, "frequency"),
            group: num(r, "group"),
            mods: blk(r, 3, MOD, |w, i| format!("mod{i}{w}")),
            itypes: list(r, "itype", 7),
            etypes: list(r, "etype", 5),
            color: color(r, "transformcolor"),
        })
        .collect()
}

fn rare_names(s: &str) -> Vec<RareName> {
    rows(s).iter().map(|r| RareName { name: txt(r, "name").into(), version: num(r, "version"), itypes: list(r, "itype", 7), etypes: list(r, "etype", 4) }).collect()
}

fn numbered(w: &str, i: usize) -> String {
    format!("{w}{i}")
}

impl ItemTables {
    /// The 1.14d tables of `crate::data` (empty without `D2RS_TABLES`), or after `set_active` the swapped-in ones (process-wide `RwLock` holding a leaked
    /// `&'static ItemTables`; each swap leaks one set and swaps are rare; earlier references keep the old set).
    pub fn get() -> &'static ItemTables {
        ACTIVE.read().ok().and_then(|g| *g).unwrap_or_else(|| {
            static T: OnceLock<ItemTables> = OnceLock::new();
            T.get_or_init(|| {
                Self::build(&crate::data::table)
            })
        })
    }

    /// Makes `ItemTables::get()` return `t` until `reset_active`.
    pub fn set_active(t: ItemTables) {
        if let Ok(mut g) = ACTIVE.write() {
            *g = Some(Box::leak(Box::new(t)));
        }
    }

    /// Returns `ItemTables::get()` to the embedded tables.
    pub fn reset_active() {
        if let Ok(mut g) = ACTIVE.write() {
            *g = None;
        }
    }

    /// Builds the tables from the game's own txt (`get("UniqueItems")` etc.; `tbl` maps a string key to its display
    /// text) with the transforms of tools/gen_data.py. Fails when a table is missing.
    pub fn from_txt(
        get: impl Fn(&str) -> Option<String>,
        tbl: impl Fn(&str) -> Option<String>,
    ) -> Result<ItemTables> {
        let (g, t) = (
            &get as &dyn Fn(&str) -> Option<String>,
            &tbl as &dyn Fn(&str) -> Option<String>,
        );
        let stat_id: HashMap<String, String> = rows(&merge(g, &["ItemStatCost"], false, &|_| {})?)
            .iter()
            .map(|r| (txt(r, "Stat").into(), txt(r, "ID").into()))
            .collect();
        let stat_ids = |r: &mut crate::schema::Rec| {
            for i in 1..=7 {
                let id = r
                    .get(&format!("stat{i}"))
                    .and_then(|n| stat_id.get(n.trim()))
                    .cloned()
                    .unwrap_or_default();
                r.insert(format!("stat{i}"), id);
            }
        };
        let runes = |r: &mut crate::schema::Rec| {
            let n = r.get("Rune Name").cloned().unwrap_or_default();
            r.insert("*Rune Name".into(), n);
            named(t, "Name", "*Rune Name")(r);
        };
        type Entry<'a> = (&'a str, &'a str, bool, &'a dyn Fn(&mut crate::schema::Rec));
        let by_name: [Entry; 11] = [
            ("uniques", "UniqueItems", true, &named(t, "index", "index")),
            ("setitems", "SetItems", false, &named(t, "index", "index")),
            (
                "magicprefix",
                "MagicPrefix",
                true,
                &named(t, "Name", "Name"),
            ),
            (
                "magicsuffix",
                "MagicSuffix",
                true,
                &named(t, "Name", "Name"),
            ),
            ("automagic", "AutoMagic", true, &named(t, "Name", "Name")),
            ("rareprefix", "RarePrefix", false, &named(t, "name", "name")),
            ("raresuffix", "RareSuffix", false, &named(t, "name", "name")),
            ("runes", "Runes", false, &runes),
            ("properties", "Properties", false, &stat_ids),
            ("qualityitems", "QualityItems", false, &|_| {}),
            ("lowqualityitems", "LowQualityItems", false, &|_| {}),
        ];
        let mut tsv = HashMap::new();
        for (k, f, keep, fix) in by_name {
            tsv.insert(k, merge(g, &[f], keep, fix)?);
        }
        Ok(Self::build(&|n| tsv.get(n).map_or("", |s| s.as_str())))
    }

    fn build<'a>(f: &dyn Fn(&str) -> &'a str) -> ItemTables {
        ItemTables {
            uniques: rows(f("uniques"))
                .iter()
                .map(|r| Unique {
                    name: txt(r, "index").into(),
                    code: code4(txt(r, "code")),
                    version: num(r, "version"),
                    enabled: txt(r, "enabled") == "1",
                    rarity: num(r, "rarity"),
                    lvl: num(r, "lvl"),
                    lvl_req: num(r, "lvl req"),
                    props: blk(r, 12, BLOCK, numbered),
                    inv: txt(r, "invfile").into(),
                    inv_trans: color(r, "invtransform"),
                })
                .collect(),
            set_items: rows(f("setitems"))
                .iter()
                .map(|r| SetItem {
                    name: txt(r, "index").into(),
                    set: txt(r, "set").into(),
                    code: code4(txt(r, "item")),
                    rarity: num(r, "rarity"),
                    lvl: num(r, "lvl"),
                    lvl_req: num(r, "lvl req"),
                    add_func: num(r, "add func"),
                    props: blk(r, 9, BLOCK, numbered),
                    aprops: std::array::from_fn(|i| {
                        let ab = |w: &str, j: usize| format!("a{w}{}{}", i + 1, ["a", "b"][j - 1]);
                        blk(r, 2, BLOCK, ab)
                    }),
                    inv: txt(r, "invfile").into(),
                    inv_trans: color(r, "invtransform"),
                })
                .collect(),
            magic_prefix: affixes(f("magicprefix")),
            magic_suffix: affixes(f("magicsuffix")),
            automagic: affixes(f("automagic")),
            rare_prefix: rare_names(f("rareprefix")),
            rare_suffix: rare_names(f("raresuffix")),
            runewords: rows(f("runes"))
                .iter()
                .map(|r| Runeword {
                    key: txt(r, "Name").into(),
                    name: txt(r, "*Rune Name").into(),
                    complete: txt(r, "complete") == "1",
                    server: txt(r, "server") == "1",
                    itypes: list(r, "itype", 6),
                    etypes: list(r, "etype", 3),
                    runes: list(r, "Rune", 6).iter().map(|c| code4(c)).collect(),
                    props: blocks(r, 7, |i| ["Code", "Param", "Min", "Max"].map(|w| format!("T1{w}{i}"))),
                })
                .collect(),
            props: rows(f("properties"))
                .iter()
                .map(|r| PropDef {
                    code: txt(r, "code").into(),
                    slots: (1..=7)
                        .filter(|i| !txt(r, &format!("func{i}")).is_empty())
                        .map(|i| (num(r, &format!("func{i}")), txt(r, &format!("stat{i}")).parse().ok(), num(r, &format!("set{i}")), num(r, &format!("val{i}"))))
                        .collect(),
                })
                .collect(),
            quality: rows(f("qualityitems"))
                .iter()
                .map(|r| Quality {
                    nummods: num(r, "nummods"),
                    mods: blk(r, 2, MOD, |w, i| format!("mod{i}{w}")),
                    ranges: ["ToHit", "Dam%", "AC%", "Dur%"].map(|k| (num(r, &format!("{k}Min")), num(r, &format!("{k}Max")))),
                    effects: list(r, "effect", 2),
                    classes: ["armor", "weapon", "shield", "thrown", "scepter", "wand", "staff", "bow", "boots", "gloves", "belt"].iter().filter(|k| txt(r, k) == "1").map(|k| k.to_string()).collect(),
                    level: num(r, "level"),
                    multiply: num(r, "multiply"),
                    add: num(r, "add"),
                })
                .collect(),
            low_quality: rows(f("lowqualityitems")).iter().map(|r| txt(r, "Name").into()).collect(),
        }
    }
}
