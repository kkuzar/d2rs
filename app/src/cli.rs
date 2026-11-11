//! Command line: info, verify, roundtrip, export-json, set. Every write goes through validate + Writer::commit.
use d2s::validate::{Finding, Level, validate};
use d2s::{Save, Schema};
use std::path::Path;

pub fn has_error(f: &[Finding]) -> bool {
    f.iter().any(|x| x.level == Level::Error)
}

pub fn stem(p: &Path) -> String {
    p.file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

fn encode(save: &Save) -> Result<Vec<u8>, String> {
    d2s::validate::encode_checked(save, Schema::classic(), &save.name()).map_err(|f| f.iter().filter(|x| x.level == Level::Error).map(|x| x.msg.clone()).collect::<Vec<_>>().join("; "))
}

fn verifier(new: String) -> impl Fn(&[u8]) -> Result<(), String> {
    move |b| {
        let sch = Schema::classic();
        Save::verify(b, sch).map_err(|e| e.to_string())?;
        let s = Save::parse(b, sch).map_err(|e| e.to_string())?;
        if has_error(&validate(&s, sch, &new)) { Err("validation failed after write".into()) } else { Ok(()) }
    }
}

/// Phase one of a save: encode, verify and run the writer's preflight without writing anything.
pub fn preflight_save(w: &d2io::Writer, path: &Path, opened: &d2io::Fingerprint, save: &Save) -> Result<(), String> {
    crate::setup_ui::writable()?;
    let bytes = encode(save)?;
    w.preflight(path, opened, &bytes, &verifier(save.name())).map_err(|e| e.to_string())
}

/// Validate, serialize and commit `save` over `path` (or as a rename when the header name differs from the file stem).
pub fn write_save(w: &d2io::Writer, path: &Path, opened: &d2io::Fingerprint, save: &Save) -> Result<(), String> {
    crate::setup_ui::writable()?;
    let (old, new) = (stem(path), save.name());
    let bytes = encode(save)?;
    let verify = verifier(new.clone());
    let dir = path.parent().unwrap_or(Path::new("."));
    let r = if old == new { w.commit(path, opened, &bytes, verify) } else { w.rename_character(dir, &old, &new, &bytes, opened, verify) };
    r.map(drop).map_err(|e| e.to_string())
}

#[doc(hidden)]
pub fn load(p: &str) -> Result<(Save, Vec<u8>), String> {
    let b = d2host::read(std::path::Path::new(p)).map_err(|e| format!("{p}: {e}"))?;
    crate::setup_ui::adapt(std::env::var_os("D2RS_GAME_DIR").map(std::path::PathBuf::from).as_deref(), Path::new(p).parent(), Some(&b));
    let s = Save::parse(&b, Schema::classic()).map_err(|e| format!("{p}: {e}"))?;
    crate::setup_ui::grids(&s);
    d2s::edit::set_big_stash(big_stash(p, &s, std::env::var_os("D2RS_GAME_DIR")));
    Ok((s, b))
}

/// PlugY BigStash: `PlugY.ini` in `D2RS_GAME_DIR`, else a stash item beyond 6x8 in the save or its PlugY files.
#[doc(hidden)]
pub fn big_stash(p: &str, s: &Save, game_dir: Option<std::ffi::OsString>) -> bool {
    let sch = Schema::classic();
    let ini = game_dir.and_then(|d| d2host::read_to_string(&std::path::Path::new(&d).join("PlugY.ini")).ok());
    ini.and_then(|t| d2s::edit::ini_big_stash(&t)).unwrap_or_else(|| {
        let dir = std::path::Path::new(p).parent().unwrap_or(std::path::Path::new("."));
        let files = [format!("{}.d2x", stem(std::path::Path::new(p))), "_LOD_SharedStashSave.sss".into(), "_LOD_HC_SharedStashSave.sss".into()];
        let pages = files.iter().filter_map(|f| d2host::read(&dir.join(f)).ok()).filter_map(|b| d2s::plugy::Stash::parse(&b, sch).ok()).flat_map(|st| st.pages).flat_map(|pg| pg.items);
        s.items.iter().filter(|i| i.view.store == 5).cloned().chain(pages).any(|i| d2s::edit::beyond_vanilla(sch, &i))
    })
}

fn info(p: &str) -> Result<(), String> {
    let (s, b) = load(p)?;
    let flags: Vec<_> = [("hardcore", d2s::save::HARDCORE), ("died", d2s::save::DIED), ("expansion", d2s::save::EXPANSION), ("ladder", d2s::save::LADDER)].iter().filter(|f| s.status(f.1)).map(|f| f.0).collect();
    println!("name: {}\nclass: {}\nlevel: {}\nversion: {}\nstatus: {}\ngold: {}\nitems: {}\nmerc: {}\nsize: {}/{}\nmap seed: {} (0x{:X}), reuse: {}", s.name(), d2s::game::CLASSES.get(usize::from(s.class())).copied().unwrap_or("?"), s.level(), s.ver, flags.join(","), s.get_stat(14).unwrap_or(0), s.all_items().len(), if s.merc.is_some() { "yes" } else { "no" }, b.len(), d2io::MAX_SAVE, s.map_seed(), s.map_seed(), ["N", "NM", "H"].iter().enumerate().filter(|(d, _)| s.seed_reuse(*d)).map(|x| *x.1).collect::<Vec<_>>().join("/"));
    for (i, it) in s.items.iter().enumerate() {
        println!("item {i}: {} (ilvl {}, seed {})", d2s::edit::item_name(it, Schema::classic()), it.view.ilvl, it.view.seed);
    }
    for f in validate(&s, Schema::classic(), &stem(Path::new(p))) {
        println!("{:?}: {}", f.level, f.msg);
    }
    Ok(())
}

#[doc(hidden)]
pub fn verify(p: &str) -> Result<(), String> {
    let (s, b) = load(p)?;
    Save::verify(&b, Schema::classic()).map_err(|e| format!("{p}: {e}"))?;
    let f = validate(&s, Schema::classic(), &stem(Path::new(p)));
    f.iter().for_each(|x| println!("{p}: {:?}: {}", x.level, x.msg));
    if has_error(&f) { Err(format!("{p}: validation errors")) } else { Ok(()) }
}

#[doc(hidden)]
pub fn roundtrip(p: &str) -> Result<(), String> {
    let (s, b) = load(p)?;
    if s.to_bytes(Schema::classic()).map_err(|e| e.to_string())? == b { Ok(()) } else { Err(format!("{p}: round trip differs")) }
}

#[doc(hidden)]
pub fn json(p: &str) -> Result<String, String> {
    let (s, _) = load(p)?;
    let stats: Vec<_> = (0..16).filter_map(|i| s.get_stat(i).map(|v| format!("\"{i}\":{v}"))).collect();
    let items: Vec<_> = s.all_items().iter().map(|i| {
        let v = &i.view;
        format!("{{\"code\":\"{}\",\"location\":{},\"store\":{},\"x\":{},\"y\":{},\"quality\":{}}}", String::from_utf8_lossy(&v.code).trim(), v.location, v.store, v.x, v.y, v.quality)
    }).collect();
    Ok(format!("{{\"name\":\"{}\",\"class\":{},\"level\":{},\"stats\":{{{}}},\"items\":[{}]}}", s.name(), s.class(), s.level(), stats.join(","), items.join(",")))
}

pub fn apply(s: &mut Save, key: &str, val: &str) -> Result<(), String> {
    if key == "name" {
        return s.set_name(val).map_err(|e| e.to_string());
    }
    if key == "seed-reuse" {
        let (d, on) = val.split_once(':').ok_or("seed-reuse: expected <0|1|2>:<0|1>")?;
        let (d, on) = (d.parse().map_err(|_| "seed-reuse: bad difficulty")?, on == "1");
        return s.set_seed_reuse(d, on).map_err(|e| e.to_string());
    }
    let n: i64 = val.parse().map_err(|_| format!("{key}: not a number: {val}"))?;
    if key == "seed" {
        return s.set_map_seed(u32::try_from(n).map_err(|_| "seed out of range")?).map_err(|e| e.to_string());
    }
    let id = match key {
        "str" => 0, "energy" | "ene" => 1, "dex" => 2, "vit" => 3, "statpts" => 4, "newskills" => 5,
        "exp" => 13, "gold" => 14, "goldbank" => 15,
        "level" => {
            return s.set_level_checked(Schema::classic(), n).map_err(|e| e.msg);
        }
        _ => return Err(format!("unknown key {key}")),
    };
    s.set_stat_checked(Schema::classic(), id, n).map_err(|e| e.msg)
}

fn set(w: &d2io::Writer, args: &[String]) -> Result<(), String> {
    let [p, kv @ ..] = args else { return Err("usage: set <file> key=value...".into()) };
    let (mut s, _) = load(p)?;
    let path = Path::new(p);
    let opened = d2io::fingerprint(path).map_err(|e| e.to_string())?;
    for a in kv {
        let (k, v) = a.split_once('=').ok_or(format!("expected key=value: {a}"))?;
        apply(&mut s, k, v)?;
    }
    write_save(w, path, &opened, &s)
}

fn add_item(w: &d2io::Writer, args: &[String]) -> Result<(), String> {
    let [p, kv @ ..] = args else { return Err("usage: add-item <file> <unique|set|runeword|magic|rare|base>=<name|id> [base=<code>] [roll=max|min|random] [ilvl=N] [eth=1] [sockets=N] [to=inventory|stash|cube]".into()) };
    let (mut s, _) = load(p)?;
    crate::setup_ui::generation()?;
    let path = Path::new(p);
    let opened = d2io::fingerprint(path).map_err(|e| e.to_string())?;
    let it = crate::create::from_args(&s, kv)?;
    let to = kv.iter().find_map(|a| a.strip_prefix("to=")).into_iter().collect::<Vec<_>>();
    crate::create::put(&mut s, it, &to)?;
    write_save(w, path, &opened, &s)
}

/// The item `idx` (as `info` lists it) of the save in `args`, edited by `f`, written back; nothing is written on an error.
fn on_item(w: &d2io::Writer, args: &[String], usage: &str, f: impl FnOnce(&mut d2s::Item, bool, &[String]) -> Result<(), String>) -> Result<(), String> {
    let [p, i, kv @ ..] = args else { return Err(usage.into()) };
    let (mut s, _) = load(p)?;
    crate::setup_ui::generation()?;
    let path = Path::new(p);
    let opened = d2io::fingerprint(path).map_err(|e| e.to_string())?;
    let exp = s.status(d2s::save::EXPANSION);
    let it = i.parse::<usize>().ok().and_then(|i| s.items.get_mut(i)).ok_or(format!("{usage}: no item {i}"))?;
    f(it, exp, kv)?;
    write_save(w, path, &opened, &s)
}

fn reroll(w: &d2io::Writer, args: &[String]) -> Result<(), String> {
    on_item(w, args, "usage: reroll <file> <item index> [seed=N] [keep=kind|quality]", |it, exp, kv| {
        let get = |k: &str| kv.iter().find_map(|a| a.strip_prefix(k).and_then(|v| v.strip_prefix('=')));
        let seed = get("seed").map_or_else(|| Ok(crate::trait_ed::random()), |v| v.parse::<u32>().map_err(|_| format!("seed: not a number: {v}")))?;
        let keep = match get("keep") {
            None | Some("quality") => false,
            Some("kind") => true,
            Some(k) => return Err(format!("keep: expected kind or quality, not {k}")),
        };
        d2s::itemgen::reroll_in_place(d2s::tables::ItemTables::get(), Schema::classic(), it, seed, keep, exp).map_err(|e| e.msg)
    })
}

/// One `item-set` argument as a trait edit.
fn trait_of(a: &str) -> Result<d2s::itemgen::Trait, String> {
    use d2s::itemgen::Trait;
    let (k, v) = a.split_once('=').ok_or(format!("expected key=value: {a}"))?;
    let n = |x: &str| x.parse::<u32>().map_err(|_| format!("not a number: {x}"));
    let pair = |x: &str| x.split_once(':').map_or_else(|| Ok((n(x)?, 0)), |(s, p)| Ok::<_, String>((n(s)?, n(p)?)));
    Ok(match k {
        "eth" => Trait::Ethereal(v == "1"),
        "sockets" => Trait::Sockets(n(v)?),
        "ilvl" => Trait::Ilvl(n(v)?),
        "remove" => pair(v).map(|(stat, param)| Trait::RemoveProp { stat, param })?,
        "add" => {
            let p: Vec<&str> = v.split(':').collect();
            let val = p.last().and_then(|x| x.parse::<i32>().ok()).ok_or("add: expected <code>[:par]:<value>")?;
            Trait::AddProp(d2s::tables::Block { code: p[0].into(), par: if p.len() > 2 { p[1].into() } else { String::new() }, min: val, max: val })
        }
        _ => {
            let (stat, param) = pair(k)?;
            Trait::SetProp { stat, param, shown: v.parse().map_err(|_| format!("not a number: {v}"))? }
        }
    })
}

fn item_set(w: &d2io::Writer, args: &[String]) -> Result<(), String> {
    on_item(w, args, "usage: item-set <file> <item index> <stat>[:param]=<value> | add=<code>[:par]:<value> | remove=<stat>[:param] | eth=0|1 | sockets=N | ilvl=N ...", |it, _, kv| {
        let edits = kv.iter().map(|a| trait_of(a)).collect::<Result<Vec<_>, _>>()?;
        d2s::itemgen::edit_traits(d2s::tables::ItemTables::get(), Schema::classic(), it, &edits).map_err(|e| e.msg)
    })
}

/// `verify --legit`: the legit findings of every item.
fn legit(p: &str) -> Result<(), String> {
    let (s, _) = load(p)?;
    crate::setup_ui::generation()?;
    let (t, sch) = (d2s::tables::ItemTables::get(), Schema::classic());
    let mut bad = 0;
    for it in s.all_items() {
        let f = d2s::legit::check(t, sch, it);
        bad += usize::from(!f.is_empty());
        f.iter().for_each(|m| println!("{p}: {}: {m}", d2s::edit::item_name(it, sch)));
    }
    if bad == 0 { Ok(()) } else { Err(format!("{p}: {bad} items not legit")) }
}

pub fn run(a: &[String]) -> i32 {
    // The GUI's per-folder backup root, so its Backups tab lists (and can restore) what `set` backed up.
    let dir = a.get(1).map(|f| Path::new(f).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")));
    let root = dir.and_then(d2io::backup_root_for).or_else(d2io::default_backup_root).unwrap_or("backups".into());
    let w = d2io::Writer { game_check: d2io::game_running, backup_root: root };
    run_with(&w, a)
}

/// Map seed (a number or a .d2s file) with each act's level seed base, the Act II tombs and the Act III bit.
#[doc(hidden)]
pub fn seed_report(arg: &str) -> Result<String, String> {
    let map_seed = match arg.parse() {
        Ok(n) => n,
        Err(_) => load(arg)?.0.map_seed(),
    };
    let t = d2map::SeedTree { map_seed };
    let bases: String = (1..=5).map(|a| format!("act {a} level seed base: {}\n", t.level_seed_base(a - 1))).collect();
    let (a, b) = t.tombs();
    Ok(format!("map seed: {map_seed}\n{bases}act 2 tombs: {a} {b}\nact 3 bit: {}\n", u8::from(t.act3_bit())))
}

pub fn run_with(w: &d2io::Writer, a: &[String]) -> i32 {
    if a.first().is_some_and(|c| c == "map") {
        return crate::map::cli(a);
    }
    let files = || a.get(1..).unwrap_or_default();
    let rs: Vec<Result<(), String>> = match a.first().map(String::as_str) {
        Some("info") => files().iter().take(1).map(|p| info(p)).collect(),
        Some("verify") if files().first().is_some_and(|f| f == "--legit") => files()[1..].iter().map(|p| legit(p)).collect(),
        Some("verify") => files().iter().map(|p| verify(p)).collect(),
        Some("roundtrip") => files().iter().map(|p| roundtrip(p)).collect(),
        Some("export-json") => files().iter().take(1).map(|p| json(p).map(|j| println!("{j}"))).collect(),
        Some("set") => vec![set(w, files())],
        Some("add-item") => vec![add_item(w, files())],
        Some("reroll") => vec![reroll(w, files())],
        Some("item-set") => vec![item_set(w, files())],
        Some("setup") => vec![crate::setup_ui::cli(files())],
        Some("seed") => files().iter().take(1).map(|p| seed_report(p).map(|r| print!("{r}"))).collect(),
        _ => vec![Err("usage: d2rs cli info|verify|roundtrip|export-json|set|add-item|reroll|item-set|seed|setup|map <file> ...".into())],
    };
    rs.iter().filter_map(|r| r.as_ref().err()).for_each(|e| eprintln!("{e}"));
    i32::from(rs.iter().any(Result::is_err))
}
