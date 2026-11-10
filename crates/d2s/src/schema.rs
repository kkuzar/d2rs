//! Game tables (stat widths, item bases) parsed from TSV text; the classic 1.14d set is embedded.
use crate::bits::{err, Result};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// One `ItemStatCost` row. Widths are 0 when the stat is not stored in that place.
#[derive(Debug, Clone, Default)]
pub struct StatDef {
    pub name: String,
    pub save_bits: u32,
    pub save_add: i32,
    pub param_bits: u32,
    pub val_shift: u32,
    pub encode: u32,
    pub csv_bits: u32,
    pub csv_param: u32,
    pub csv_signed: bool,
    pub bits109: u32,
    pub add109: i32,
}

/// One row of armor / weapons / misc, with the parser flags precomputed from the item type tree.
#[derive(Debug, Clone, Default)]
pub struct Base {
    /// 'a' armor, 'w' weapon, 'm' misc.
    pub kind: char,
    pub name: String,
    pub typ: String,
    pub w: u32,
    pub h: u32,
    pub invfile: String,
    /// `uniqueinvfile`, `setinvfile` and the `InvTrans` colour-map file type (1..8, 0 none).
    pub unique_inv: String,
    pub set_inv: String,
    pub inv_trans: u8,
    pub stackable: bool,
    pub quest_diff: bool,
    pub gold: bool,
    pub charm: bool,
    pub body: bool,
    pub tome: bool,
    /// compactsave: stored as a simple (short) item.
    pub compact: bool,
    /// May sit in the belt (potions and scrolls: ItemTypes Beltable).
    pub beltable: bool,
    pub maxstack: u32,
    pub durability: u32,
    pub maxac: u32,
    /// belts.tsv row of a belt (rows = numboxes / 4).
    pub belt: u32,
    /// 0 classic, 100 expansion only.
    pub version: u32,
    /// Quality level (`level`), `magic lvl`, `auto prefix` group and `spawnable`.
    pub level: u32,
    pub magic_lvl: u32,
    pub auto_prefix: u32,
    pub spawnable: bool,
    pub minac: u32,
    pub gemsockets: u32,
    pub nodurability: bool,
    pub quest: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    /// Indexed by stat id.
    pub stats: Vec<StatDef>,
    pub bases: HashMap<[u8; 4], Base>,
    /// Item type code -> ([Equiv1, Equiv2], [BodyLoc1, BodyLoc2]).
    pub types: HashMap<String, ([String; 2], [String; 2])>,
    /// Item type code -> its `InvGfx1..VarInvGfx` pictures (empty when the type has none).
    pub inv_gfx: HashMap<String, Vec<String>>,
}

static ACTIVE: RwLock<Option<&'static Schema>> = RwLock::new(None);

pub(crate) type Row<'a> = HashMap<&'a str, &'a str>;

pub(crate) fn rows(s: &str) -> Vec<Row<'_>> {
    let mut lines = s.lines().map(|l| l.trim_end_matches('\r').split('\t'));
    let head: Vec<&str> = lines.next().map(Iterator::collect).unwrap_or_default();
    lines.map(|c| head.iter().copied().zip(c).collect()).collect()
}

pub(crate) fn num<T: std::str::FromStr + Default>(r: &Row, k: &str) -> T {
    r.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or_default()
}

pub(crate) fn txt<'a>(r: &Row<'a>, k: &str) -> &'a str {
    r.get(k).map_or("", |v| v.trim())
}

/// One txt row with every cell raw, keyed by column name.
pub(crate) type Rec = HashMap<String, String>;

/// Merges the txt tables `files` (each `get(name)`) into one tsv text: blank lines, rows without a first cell
/// (unless `keep`) and `Expansion` rows are dropped as in tools/gen_data.py; `kind` is the file's first letter.
/// `fix` edits each row (aliases, display names); columns it adds are kept.
pub(crate) fn merge(
    get: &dyn Fn(&str) -> Option<String>,
    files: &[&str],
    keep: bool,
    fix: &dyn Fn(&mut Rec),
) -> Result<String> {
    let mut recs: Vec<Rec> = vec![];
    for f in files {
        let Some(t) = get(f) else {
            return err(0, format!("{f}.txt is missing"));
        };
        let t = t
            .lines()
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let h0 = t.split(['\t', '\n']).next().unwrap_or("");
        for r in rows(&t) {
            if r.get(h0)
                .is_some_and(|v| (keep || !v.is_empty()) && *v != "Expansion")
            {
                let mut r: Rec = r.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
                r.insert(
                    "kind".into(),
                    f.chars().next().unwrap_or('m').to_ascii_lowercase().into(),
                );
                fix(&mut r);
                recs.push(r);
            }
        }
    }
    let head: std::collections::BTreeSet<&String> = recs.iter().flat_map(|r| r.keys()).collect();
    let line = |cells: Vec<&str>| cells.join("\t") + "\n";
    let mut out = line(head.iter().map(|h| h.as_str()).collect());
    recs.iter().for_each(|r| {
        out += &line(
            head.iter()
                .map(|h| r.get(*h).map_or("", |v| v.trim()))
                .collect(),
        )
    });
    Ok(out)
}

/// Replaces `dst` by the display text of the string key in `key` (the last line of the text, as gen_data.py does).
pub(crate) fn named<'a>(
    tbl: &'a dyn Fn(&str) -> Option<String>,
    key: &'a str,
    dst: &'a str,
) -> impl Fn(&mut Rec) + 'a {
    move |r| {
        if let Some(t) = r.get(key).and_then(|k| tbl(k)) {
            r.insert(
                dst.into(),
                t.rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .replace('\t', " ")
                    .replace('\r', ""),
            );
        }
    }
}

fn is_a(types: &HashMap<&str, [&str; 2]>, t: &str, target: &str, depth: u32) -> bool {
    t == target
        || (depth < 8 && types.get(t).is_some_and(|e| e.iter().any(|x| !x.is_empty() && is_a(types, x, target, depth + 1))))
}

impl Schema {
    /// The core classic 1.14d tables (`crate::data`).
    ///
    /// After `set_active` it returns the swapped-in game tables instead, for every thread (a process-wide `RwLock`
    /// holding a leaked `&'static Schema`; each swap leaks one schema, and swaps are rare). A swap does not affect
    /// references taken earlier, so re-check an open save after swapping.
    pub fn classic() -> &'static Schema {
        ACTIVE
            .read()
            .ok()
            .and_then(|g| *g)
            .unwrap_or_else(Self::builtin)
    }

    /// Makes `Schema::classic()` return `s` until `reset_active`.
    pub fn set_active(s: Schema) {
        if let Ok(mut g) = ACTIVE.write() {
            *g = Some(Box::leak(Box::new(s)));
        }
    }

    /// Returns `Schema::classic()` to the embedded tables.
    pub fn reset_active() {
        if let Ok(mut g) = ACTIVE.write() {
            *g = None;
        }
    }

    fn builtin() -> &'static Schema {
        static S: OnceLock<Schema> = OnceLock::new();
        S.get_or_init(|| {
            Schema::from_tsv(
                crate::data::table("isc"),
                crate::data::table("items"),
                crate::data::table("itemtypes"),
            )
            .unwrap_or_default()
        })
    }

    /// Builds a schema from the game's own txt tables (`get("ItemStatCost")` etc. gives a table's text, `tbl` a string key's
    /// display text) with the transforms of tools/gen_data.py. Fails when a table is missing. A mod's save widths
    /// are used as they are (any stat), so its saves are read and written with its own format.
    pub fn from_txt(
        get: impl Fn(&str) -> Option<String>,
        tbl: impl Fn(&str) -> Option<String>,
    ) -> Result<Schema> {
        let isc = merge(&get, &["ItemStatCost"], false, &|_| {})?;
        let items = merge(
            &get,
            &["Weapons", "Armor", "Misc"],
            false,
            &named(&tbl, "namestr", "name"),
        )?;
        Schema::from_tsv(&isc, &items, &merge(&get, &["ItemTypes"], false, &|_| {})?)
    }

    /// Builds a schema from the three TSV texts (a user data folder can override the embedded ones).
    pub fn from_tsv(isc: &str, items: &str, itemtypes: &str) -> Result<Schema> {
        let mut s = Schema::default();
        for r in rows(isc) {
            let id: usize = num(&r, "ID");
            if id > 1024 {
                return err(0, format!("stat id {id} out of range"));
            }
            if s.stats.len() <= id {
                s.stats.resize(id + 1, StatDef::default());
            }
            let d = StatDef {
                name: txt(&r, "Stat").into(),
                save_bits: num(&r, "Save Bits"),
                save_add: num(&r, "Save Add"),
                param_bits: num(&r, "Save Param Bits"),
                val_shift: num(&r, "ValShift"),
                encode: num(&r, "Encode"),
                csv_bits: num(&r, "CSvBits"),
                csv_param: num(&r, "CSvParam"),
                csv_signed: num::<u32>(&r, "CSvSigned") != 0,
                bits109: num(&r, "1.09-Save Bits"),
                add109: num(&r, "1.09-Save Add"),
            };
            // Bit fields are at most 32 bits wide; a wider table would desync or overflow the codec.
            if [d.save_bits, d.param_bits, d.csv_bits, d.csv_param, d.bits109].iter().any(|&w| w > 32) {
                return err(0, format!("stat {id}: a width above 32 bits"));
            }
            s.stats[id] = d;
        }
        let trows = rows(itemtypes);
        let st = |r: &Row, k: &str| txt(r, k).to_string();
        s.types = trows.iter().map(|r| (st(r, "Code"), ([st(r, "Equiv1"), st(r, "Equiv2")], [st(r, "BodyLoc1"), st(r, "BodyLoc2")]))).collect();
        s.inv_gfx = trows.iter().map(|r| (st(r, "Code"), (1..=num::<usize>(r, "VarInvGfx").min(6)).map(|i| st(r, &format!("InvGfx{i}"))).collect())).collect();
        let types: HashMap<&str, [&str; 2]> = trows.iter().map(|r| (txt(r, "Code"), [txt(r, "Equiv1"), txt(r, "Equiv2")])).collect();
        for r in rows(items) {
            let code = txt(&r, "code");
            if code.is_empty() || code.len() > 4 {
                continue;
            }
            let mut key = [b' '; 4];
            key[..code.len()].copy_from_slice(code.as_bytes());
            let (t1, t2) = (txt(&r, "type"), txt(&r, "type2"));
            let any = |target| is_a(&types, t1, target, 0) || is_a(&types, t2, target, 0);
            s.bases.insert(
                key,
                Base {
                    kind: txt(&r, "kind").chars().next().unwrap_or('m'),
                    name: txt(&r, "name").into(),
                    typ: t1.into(),
                    w: num(&r, "invwidth"),
                    h: num(&r, "invheight"),
                    invfile: txt(&r, "invfile").into(),
                    unique_inv: txt(&r, "uniqueinvfile").into(),
                    set_inv: txt(&r, "setinvfile").into(),
                    inv_trans: num(&r, "InvTrans"),
                    stackable: txt(&r, "stackable") == "1",
                    quest_diff: txt(&r, "questdiffcheck") == "1",
                    gold: any("gold"),
                    charm: any("char"),
                    body: any("body"),
                    tome: any("book") || any("scro"),
                    compact: txt(&r, "compactsave") == "1",
                    beltable: any("poti") || any("elix") || any("scro"),
                    maxstack: num(&r, "maxstack"),
                    durability: num(&r, "durability"),
                    maxac: num(&r, "maxac"),
                    belt: num(&r, "belt"),
                    version: num(&r, "version"),
                    level: num(&r, "level"),
                    magic_lvl: num(&r, "magic lvl"),
                    auto_prefix: num(&r, "auto prefix"),
                    spawnable: txt(&r, "spawnable") == "1",
                    minac: num(&r, "minac"),
                    gemsockets: num(&r, "gemsockets"),
                    nodurability: txt(&r, "nodurability") == "1",
                    quest: txt(&r, "quest") == "1",
                },
            );
        }
        Ok(s)
    }

    pub fn stat(&self, id: u32) -> Option<&StatDef> {
        self.stats.get(id as usize)
    }

    /// Rows (of 4 slots) of the belt in `code`, from belts.tsv.
    pub fn belt_rows(&self, code: [u8; 4]) -> u32 {
        let n = self.base(code).and_then(|b| crate::data::table("belts").lines().nth(b.belt as usize + 1)?.split('\t').nth(1)?.parse::<u32>().ok());
        n.map_or(1, |n| (n / 4).max(1))
    }

    pub fn base(&self, code: [u8; 4]) -> Option<&Base> {
        self.bases.get(&code)
    }
}
