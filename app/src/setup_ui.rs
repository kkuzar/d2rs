//! Game setup: detects the game folder, picks the tables (vanilla, the mod's, or read-only) and reports it.
use d2data::setup::{Setup, TableState, detect};
use d2data::{DataDir, GameData, MergedTbl, Tbl};
#[cfg(not(target_arch = "wasm32"))]
use d2mem::ImageCheck;
use d2s::tables::ItemTables;
use d2s::validate::validate;
use d2s::{Save, Schema};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The tables that define the save format; a change there needs loadable txt.
const FORMAT: [&str; 6] = ["ItemStatCost", "Weapons", "Armor", "Misc", "ItemTypes", "CharStats"];

#[derive(Clone, Default, PartialEq, Debug)]
pub enum Mode {
    #[default]
    Vanilla,
    Mod,
    ReadOnly(String),
}

#[derive(Clone, Default)]
pub struct Adapt {
    pub setup: Setup,
    pub mode: Mode,
    /// Why item creation, reroll and the legit check are off (the generation tables are unusable).
    pub gen_off: Option<String>,
    game: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    exe: Option<ImageCheck>,
    inv: Option<String>,
}

thread_local!(static CUR: RefCell<Adapt> = RefCell::default());

pub fn readonly() -> Option<String> {
    CUR.with_borrow(|a| match &a.mode {
        Mode::ReadOnly(r) => Some(r.clone()),
        _ => None,
    })
}

/// Why item generation is off; also when no item tables are loaded (no game folder picked).
pub fn gen_off() -> Option<String> {
    CUR.with_borrow(|a| a.gen_off.clone()).or_else(|| ItemTables::get().uniques.is_empty().then(|| "no game folder: pick it (its MPQ files) for item names and to create, reroll or import items".into()))
}

/// Refuses a write while the mode is read-only.
pub fn writable() -> Result<(), String> {
    readonly().map_or(Ok(()), |r| Err(format!("read-only: {r}")))
}

/// Refuses item generation while the generation tables are unusable.
pub fn generation() -> Result<(), String> {
    gen_off().map_or(Ok(()), |r| Err(format!("disabled: {r}")))
}

/// Read-only from now on (the open save no longer matches the tables).
pub fn force_readonly(reason: &str) {
    CUR.with_borrow_mut(|a| a.mode = Mode::ReadOnly(reason.into()));
}

pub fn status() -> String {
    CUR.with_borrow(|a| match &a.mode {
        Mode::Vanilla => "Vanilla".into(),
        Mode::Mod => format!("Mod tables ({})", a.setup.format_modified().join(", ")),
        Mode::ReadOnly(r) => format!("Read-only: {r}"),
    })
}

fn read(gd: &GameData, n: &str) -> Option<String> {
    gd.read(&format!("data\\global\\excel\\{n}.txt")).map(|b| b.iter().map(|&c| c as char).collect())
}

/// Detects the setup of `game`, activates the tables it needs and checks `bytes` (the save about to open) against them.
/// Always call before parsing a save; the result is also kept for `status`, `readonly`, `gen_off` and `lines`.
pub fn adapt(game: Option<&Path>, save_dir: Option<&Path>, bytes: Option<&[u8]>) -> Adapt {
    Schema::reset_active();
    ItemTables::reset_active();
    d2s::edit::set_grids(Default::default());
    let setup = detect(game, save_dir);
    #[cfg(not(target_arch = "wasm32"))]
    let exe = game.and_then(|g| DataDir::new(g).find("Game.exe")).map(|p| d2mem::verify_image(&p));
    let mut a = Adapt { mode: Mode::Vanilla, gen_off: None, game: game.map(Path::to_path_buf), #[cfg(not(target_arch = "wasm32"))] exe, inv: None, setup };
    let bad: Vec<_> = a.setup.tables.iter().filter(|t| t.txt == TableState::Modified || t.bin == TableState::Modified).collect();
    let unloadable: Vec<_> = bad.iter().filter(|t| t.txt != TableState::Modified).map(|t| t.name).collect();
    if let Some(g) = game {
        let gd = GameData::from_install(g).unwrap_or_default().with_override(DataDir::new(g));
        let m = MergedTbl(["patchstring", "expansionstring", "string"].iter().filter_map(|n| Tbl::parse(&gd.read(&format!("data\\local\\lng\\eng\\{n}.tbl"))?).ok()).collect());
        let tbl = |k: &str| m.get(k).map(String::from);
        if bad.is_empty() {
            // vanilla: the item tables (uniques, sets, affixes, runewords, ...) come from the game folder
            if let Err(e) = ItemTables::from_txt(|n| read(&gd, n), tbl).map(ItemTables::set_active) && ItemTables::get().uniques.is_empty() {
                a.gen_off = Some(format!("the game folder's item tables do not load: {e}"));
            }
            CUR.set(a.clone());
            return a;
        }
        a.inv = read(&gd, "Inventory");
        let (fmt, gen_bad): (Vec<_>, Vec<_>) = unloadable.iter().partition(|n| FORMAT.contains(n));
        let why = |names: &[&&str], what: &str| format!("{} changed only as compiled .bin, which d2rs cannot read{what}", names.iter().map(|n| **n).collect::<Vec<_>>().join(", "));
        if !fmt.is_empty() {
            let r = why(&fmt, " (the save format is unknown)");
            (a.mode, a.gen_off) = (Mode::ReadOnly(r.clone()), Some(r));
        } else {
            a.mode = Mode::Mod;
            let sch = Schema::from_txt(|n| read(&gd, n), tbl);
            match sch {
                Ok(s) if bad.iter().any(|t| FORMAT.contains(&t.name)) => Schema::set_active(s),
                Err(e) => a.mode = Mode::ReadOnly(format!("the modified tables do not load: {e}")),
                _ => {}
            }
            match ItemTables::from_txt(|n| read(&gd, n), tbl) {
                _ if !gen_bad.is_empty() => a.gen_off = Some(why(&gen_bad, "")),
                Ok(t) => ItemTables::set_active(t),
                Err(e) => a.gen_off = Some(format!("the item tables do not load: {e}")),
            }
        }
        if a.mode == Mode::Mod && let Some(b) = bytes {
            let sch = Schema::classic();
            let ok = Save::verify(b, sch).is_ok() && Save::parse(b, sch).is_ok_and(|s| !crate::cli::has_error(&validate(&s, sch, &s.name())));
            if !ok {
                Schema::reset_active();
                ItemTables::reset_active();
                a.mode = Mode::ReadOnly("this save does not round-trip with the mod tables".into());
            }
        }
    }
    CUR.set(a.clone());
    a
}

/// Applies the game's Inventory.txt grid sizes for `save` (mod tables only).
pub fn grids(save: &Save) {
    CUR.with_borrow(|a| {
        if a.mode == Mode::Mod && let Some(t) = &a.inv {
            d2s::edit::set_grids(d2s::edit::grid_sizes(t, save.class(), save.status(d2s::save::EXPANSION)));
        }
    });
}

/// The report shown in the Game setup window and by `d2rs cli setup`.
pub fn lines() -> Vec<String> {
    CUR.with_borrow(|a| {
        let s = &a.setup;
        let mut v = vec![format!("Mode: {}", status()), format!("Game folder: {}", a.game.as_ref().map_or("not set".into(), |g| g.display().to_string()))];
        if let Some(r) = &a.gen_off {
            v.push(format!("Item creation, reroll and legit are off: {r}"));
        }
        #[cfg(not(target_arch = "wasm32"))]
        v.push(format!("Game.exe: {}", match a.exe { Some(ImageCheck::Exact) => "the 1.14d build", Some(ImageCheck::Layout1_14d) => "1.14d layout, modified bytes", Some(ImageCheck::Unknown) => "not 1.14d or unreadable", None => "not found" }));
        v.push(match &s.plugy {
            Some(p) => format!("PlugY: dll {}, ini {}, BigStash {}, stash files {}", yn(p.dll), yn(p.ini), p.big_stash.map_or("unknown", |b| if b { "on" } else { "off" }), yn(p.save_files)),
            None => "PlugY: not found".into(),
        });
        v.push(format!("Loaders: {}", if s.loaders.is_empty() { "none".into() } else { s.loaders.join(", ") }));
        v.push(format!("Summary: {}", s.summary()));
        v.push("Tables:".into());
        v.extend(s.tables.iter().map(|t| format!("  {}: txt {:?}, bin {:?}{}", t.name, t.txt, t.bin, if t.from_data_dir { " (data folder)" } else { "" })));
        v
    })
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// `d2rs cli setup [--game dir] [file.d2s]`.
pub fn cli(args: &[String]) -> Result<(), String> {
    let game = match args {
        [f, d, ..] if f == "--game" => Some(PathBuf::from(d)),
        _ => std::env::var_os("D2RS_GAME_DIR").map(PathBuf::from),
    };
    let file = args.iter().rev().find(|a| a.ends_with(".d2s"));
    let bytes = file.map(|f| d2host::read(Path::new(f)).map_err(|e| format!("{f}: {e}"))).transpose()?;
    adapt(game.as_deref(), file.and_then(|f| Path::new(f).parent()), bytes.as_deref());
    lines().iter().for_each(|l| println!("{l}"));
    Ok(())
}
