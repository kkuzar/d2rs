//! Seed maps: the game-data context and exporter shared by the Map tab and `d2rs cli map`, and the tab itself.
use d2data::{DataDir, GameData, Palette, ds1::Ds1, dt1::Dt1, excel::MapTables};
use d2map::{Act, Level, MapData, MapError, build_act, collision, preset::Ids, render::{self, Image}, tiles::Tiler};
use crate::platform::{self, Pick};
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};

fn s<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

fn why(e: &MapError) -> String {
    match e {
        MapError::ClassicNoActV => "Act V needs an expansion character (this one is classic)".into(),
        e => s(e),
    }
}

fn tile_path(p: &str) -> String {
    format!("data\\global\\tiles\\{}", p.replace('/', "\\"))
}

/// The collision map, and the isometric picture when asked for (its own error: it can be too large).
type ActKey = (u32, u8, bool, u8);
type Iso = Option<Result<(Image, u32), String>>;
/// A name over the pictures: its text, whether it is a warp destination, and its position in the view units of the collision and the isometric picture.
type Lab = (String, bool, [egui::Vec2; 2]);
type Pictures = (Image, Iso, Vec<Lab>);

/// `render::labels` in view units; `lo` is the isometric picture's origin.
fn labs(ls: Vec<render::Label>, lo: (i64, i64)) -> Vec<Lab> {
    ls.into_iter().map(|l| (l.text, l.warp, [egui::vec2(l.coll.0, l.coll.1), egui::vec2(l.iso.0 - lo.0 as f32, l.iso.1 - lo.1 as f32)])).collect()
}

/// The game files the generator needs: MPQs (and a data folder) plus the Game.exe beside them.
pub struct Ctx {
    gd: GameData,
    t: MapTables,
    ids: Ids,
    /// The display `LevelName` of each level.
    names: HashMap<u32, String>,
    /// The last built act, for (seed, diff, expansion, act): Show does not rebuild it.
    cache: Mutex<Option<(ActKey, Arc<Act>)>>,
}

pub struct Job {
    pub seed: u32,
    pub diff: u8,
    pub exp: bool,
    pub out: PathBuf,
    pub acts: Vec<u8>,
    pub level: Option<u32>,
    pub iso: bool,
    /// One picture of the connected surface of each act, instead of its levels.
    pub whole: bool,
}

impl Ctx {
    pub fn load(dir: &Path) -> Result<Ctx, String> {
        let gd = GameData::from_install(dir).map_err(|e| format!("{}: {}", dir.display(), s(e)))?.with_override(DataDir::new(dir));
        let t = MapTables::load(&gd).map_err(s)?;
        let ids = Ids::load(&gd, &dir.join("Game.exe")).map_err(|e| format!("Game.exe: {}", s(e)))?;
        let txt = gd.read("data\\global\\excel\\levels.txt").map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut l = txt.lines();
        let head: Vec<&str> = l.next().unwrap_or("").split('\t').collect();
        let col = |n| head.iter().position(|h| *h == n);
        let (ci, cn, cl) = (col("Id"), col("Name"), col("LevelName"));
        // The display name: `LevelName` through the 1.14d string tables (patch, expansion, then string), else the internal `Name`.
        let tbl = d2data::MergedTbl(["patchstring", "expansionstring", "string"].iter().filter_map(|n| d2data::Tbl::parse(&gd.read(&format!("data\\local\\lng\\eng\\{n}.tbl"))?).ok()).collect());
        let names = l
            .filter_map(|r| {
                let f: Vec<&str> = r.split('\t').collect();
                let shown = cl.and_then(|c| tbl.get(f.get(c)?)).or(f.get(cn?).copied())?.to_string();
                Some((f.get(ci?)?.parse().ok()?, shown))
            })
            .collect();
        Ok(Ctx { gd, t, ids, names, cache: Mutex::new(None) })
    }

    /// The display name (`LevelName`), for the export file names and messages.
    pub fn display(&self, id: u32) -> String {
        self.names.get(&id).cloned().unwrap_or_default()
    }

    pub fn act_levels(&self, a: u8) -> Vec<u32> {
        self.t.levels.iter().filter(|l| l.act == i32::from(a) && l.drlg_type != 0).map(|l| l.id as u32).collect()
    }

    pub fn act_of(&self, id: u32) -> Option<u8> {
        self.t.levels.iter().find(|l| l.id as u32 == id && l.drlg_type != 0).map(|l| l.act as u8)
    }

    /// The level's id and name, marked when no golden case pins its draws (`d2map::verified`).
    pub fn label(&self, id: u32) -> String {
        format!("{id} {}{}", self.display(id), if d2map::verified(id) { "" } else { " (unverified)" })
    }

    fn build(&self, seed: u32, diff: u8, exp: bool, a: u8) -> Result<Arc<Act>, String> {
        let key = (seed, diff, exp, a);
        if let Some((k, act)) = self.cache.lock().map_err(s)?.as_ref()
            && *k == key
        {
            return Ok(act.clone());
        }
        let ds = |p: &str| Ds1::parse_game(&self.gd.read(&tile_path(p)).ok_or(d2data::Error("missing"))?);
        let act = Arc::new(build_act(&MapData { t: &self.t, ds1: &ds, ids: Some(&self.ids) }, seed, diff, exp, a).map_err(|e| why(&e))?);
        *self.cache.lock().map_err(s)? = Some((key, act.clone()));
        Ok(act)
    }

    fn pictures(&self, act: &Act, lv: &Level, a: u8, iso: bool) -> Result<Pictures, String> {
        let ds = |p: &str| Ds1::parse_game(&self.gd.read(&tile_path(p)).ok_or(d2data::Error("missing"))?);
        let load = |p: &str| Dt1::parse(&self.gd.read(&tile_path(p)).ok_or(d2data::Error("missing"))?);
        let md = MapData { t: &self.t, ds1: &ds, ids: Some(&self.ids) };
        let warp = act.links.get(&lv.id).ok_or("level has no links")?.1;
        let mut tiler = Tiler::new(&md, &load);
        let built = tiler.level(lv, warp).map_err(s)?;
        let grids = collision::grids(lv, &built);
        let c = render::collision(lv, &grids).map_err(s)?;
        let i = iso.then(|| {
            let pal = self.gd.read(&format!("data\\global\\palette\\ACT{}\\pal.dat", a + 1)).ok_or("no palette")?;
            let rooms: Vec<_> = built.iter().map(|(rt, p)| (rt, &p.dt1[..])).collect();
            render::act_level(&[(lv, (0, 0))], &[&rooms], &Palette::from_dat(&pal).map_err(s)?, 256 << 20).map_err(s)
        });
        let lo = i.as_ref().and_then(|r| r.as_ref().ok()).map_or((0, 0), |r| r.2);
        Ok((c, i.map(|r| r.map(|(img, k, _)| (img, k))), labs(render::labels(&[(lv, (0, 0))], &|id| self.display(id), false), lo)))
    }

    fn view(&self, seed: u32, diff: u8, exp: bool, a: u8, id: u32, iso: bool) -> Result<Pictures, String> {
        let act = self.build(seed, diff, exp, a)?;
        let i = self.act_levels(a).iter().position(|l| *l == id).ok_or("level not in this act")?;
        self.pictures(&act, act.levels[i].as_ref().map_err(why)?, a, iso)
    }

    /// The collision and isometric pictures and the JSON of an act's connected surface (`render::surface`); `done` counts the levels tiled, out of `total`.
    fn act_pictures(&self, act: &Act, a: u8, iso: bool, cancel: &AtomicBool, done: &AtomicUsize, total: &AtomicUsize) -> Result<(Pictures, String), String> {
        let lvs = render::surface(act);
        if lvs.is_empty() {
            return Err(act.levels.iter().find_map(|l| l.as_ref().err()).map_or("the act has no levels".into(), why));
        }
        total.store(lvs.len(), Relaxed);
        let ds = |p: &str| Ds1::parse_game(&self.gd.read(&tile_path(p)).ok_or(d2data::Error("missing"))?);
        let load = |p: &str| Dt1::parse(&self.gd.read(&tile_path(p)).ok_or(d2data::Error("missing"))?);
        let md = MapData { t: &self.t, ds1: &ds, ids: Some(&self.ids) };
        let mut tiler = Tiler::new(&md, &load);
        let mut built = vec![];
        for &(lv, _) in &lvs {
            if cancel.load(Relaxed) {
                return Err("Cancelled".into());
            }
            built.push(tiler.level(lv, act.links.get(&lv.id).ok_or("level has no links")?.1).map_err(s)?);
            done.fetch_add(1, Relaxed);
        }
        let grids: Vec<_> = lvs.iter().zip(&built).map(|(l, b)| collision::grids(l.0, b)).collect();
        let c = render::act_collision(&lvs, &grids).map_err(s)?;
        let i = iso.then(|| {
            let pal = self.gd.read(&format!("data\\global\\palette\\ACT{}\\pal.dat", a + 1)).ok_or("no palette")?;
            let rooms: Vec<Vec<_>> = built.iter().map(|b| b.iter().map(|(rt, p)| (rt, &p.dt1[..])).collect()).collect();
            render::act_level(&lvs, &rooms.iter().map(Vec::as_slice).collect::<Vec<_>>(), &Palette::from_dat(&pal).map_err(s)?, 256 << 20).map_err(s)
        });
        let ok = i.as_ref().and_then(|r| r.as_ref().ok());
        let json = render::act_json(&lvs, &|id| self.display(id), ok.map(|r| r.1));
        let labels = labs(render::labels(&lvs, &|id| self.display(id), true), ok.map_or((0, 0), |r| r.2));
        Ok(((c, i.map(|r| r.map(|(img, k, _)| (img, k))), labels), json))
    }

    fn view_act(&self, j: &Job, a: u8, cancel: &AtomicBool, done: &AtomicUsize, total: &AtomicUsize) -> Result<Pictures, String> {
        Ok(self.act_pictures(&*self.build(j.seed, j.diff, j.exp, a)?, a, j.iso, cancel, done, total)?.0)
    }

    /// Writes the collision picture and, when built, the isometric one (`_iso_xK` when downscaled by K).
    fn save(&self, f: impl Fn(&str) -> PathBuf, c: &Image, i: Option<Result<(Image, u32), String>>, what: &str) -> Result<(), String> {
        render::write_png(&f("_collision.png"), c).map_err(s)?;
        let Some(i) = i else { return Ok(()) };
        let (img, k) = i.map_err(|e| format!("isometric: {e}"))?;
        if k > 1 {
            println!("{what}: isometric picture downscaled x{k}");
        }
        render::write_png(&f(&if k > 1 { format!("_iso_x{k}.png") } else { "_iso.png".into() }), &img).map_err(|e| format!("isometric: {e}"))
    }

    fn export_act(&self, j: &Job, act: &Act, a: u8, cancel: &AtomicBool) -> Result<(), String> {
        let n = format!("act{}_{}", a + 1, ["normal", "nightmare", "hell"][usize::from(j.diff.min(2))]);
        let ((c, i, _), json) = self.act_pictures(act, a, j.iso, cancel, &AtomicUsize::new(0), &AtomicUsize::new(0))?;
        render::write_atomic(&j.out.join(format!("{n}.json")), json.as_bytes()).map_err(s)?;
        self.save(|ext| j.out.join(format!("{n}{ext}")), &c, i, &n)
    }

    fn export_level(&self, j: &Job, act: &Act, a: u8, id: u32, res: &Result<Level, MapError>) -> Result<(), String> {
        let lv = res.as_ref().map_err(why)?;
        let n: String = self.display(id).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let f = |ext: &str| j.out.join(format!("{id}_{n}{ext}"));
        if !d2map::verified(id) {
            println!("level {id} {n}: unverified (no golden case pins its draws)");
        }
        let (c, i, _) = self.pictures(act, lv, a, j.iso)?;
        render::write_atomic(&f(".json"), render::json(lv).as_bytes()).map_err(s)?;
        self.save(f, &c, i, &format!("level {id} {n}"))
    }

    /// Writes every wanted level; returns one message per level that failed (never fatal). Stops when `cancel` is set.
    pub fn export(&self, j: &Job, cancel: &AtomicBool, done: &AtomicUsize) -> Vec<String> {
        if let Err(e) = d2host::create_dir_all(&j.out) {
            return vec![format!("{}: {e}", j.out.display())];
        }
        let mut errs = vec![];
        for &a in &j.acts {
            let all = self.act_levels(a);
            if j.whole {
                let r = self.build(j.seed, j.diff, j.exp, a).and_then(|x| self.export_act(j, &x, a, cancel));
                errs.extend(r.err().map(|e| format!("act {}: {e}", a + 1)));
                done.fetch_add(1, Relaxed);
                continue;
            }
            let want = |id: &u32| j.level.is_none_or(|l| l == *id);
            let fail = |id: u32, e: &str| format!("level {id} {}: {e}", self.display(id));
            let act = match self.build(j.seed, j.diff, j.exp, a) {
                Ok(x) => x,
                Err(e) => {
                    errs.extend(all.iter().filter(|i| want(i)).map(|&i| fail(i, &e)));
                    done.fetch_add(all.iter().filter(|i| want(i)).count(), Relaxed);
                    continue;
                }
            };
            for (&id, res) in all.iter().zip(&act.levels).filter(|(i, _)| want(i)) {
                if cancel.load(Relaxed) {
                    return errs;
                }
                errs.extend(self.export_level(j, &act, a, id, res).err().map(|e| fail(id, &e)));
                done.fetch_add(1, Relaxed);
            }
        }
        errs
    }

    pub fn count(&self, j: &Job) -> usize {
        if j.whole {
            return j.acts.len();
        }
        j.acts.iter().flat_map(|&a| self.act_levels(a)).filter(|i| j.level.is_none_or(|l| l == *i)).count()
    }
}

pub struct Opts {
    pub seed: Option<u32>,
    pub save: Option<String>,
    pub diff: u8,
    pub level: Option<u32>,
    pub act: Option<u8>,
    pub out: PathBuf,
    pub game: Option<PathBuf>,
    pub classic: bool,
    pub no_iso: bool,
}

const USAGE: &str = "usage: d2rs cli map (--seed N | --save f.d2s) --diff 0|1|2 [--level id | --act 1..5] [--classic] [--no-iso] [--game dir] --out dir\n  --game dir or the D2RS_GAME_DIR variable: the game 1.14d folder (MPQs and Game.exe); --classic: no Act V (implied by a classic save)";

/// Parses `map` and its options (`a[0]` is "map").
pub fn parse(a: &[String]) -> Result<Opts, String> {
    let mut o = Opts { seed: None, save: None, diff: 9, level: None, act: None, out: PathBuf::new(), game: None, classic: false, no_iso: false };
    let mut it = a.iter().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().ok_or(format!("{k} needs a value"));
        let num = |v: &String| v.parse::<u32>().map_err(|_| format!("{k}: not a number: {v}"));
        match k.as_str() {
            "--seed" => o.seed = Some(num(v()?)?),
            "--save" => o.save = Some(v()?.clone()),
            "--diff" => o.diff = u8::try_from(num(v()?)?).map_err(s)?,
            "--level" => o.level = Some(num(v()?)?),
            "--act" => o.act = Some(u8::try_from(num(v()?)?).ok().filter(|a| (1..=5).contains(a)).ok_or("--act 1..5")? - 1),
            "--out" => o.out = v()?.into(),
            "--game" => o.game = Some(v()?.into()),
            "--classic" => o.classic = true,
            "--no-iso" => o.no_iso = true,
            _ => return Err(format!("unknown option {k}")),
        }
    }
    if o.seed.is_some() == o.save.is_some() {
        return Err("give exactly one of --seed and --save".into());
    }
    if o.act.is_some() && o.level.is_some() {
        return Err("give --level or --act, not both".into());
    }
    if o.diff > 2 {
        return Err("--diff 0|1|2 is required".into());
    }
    if o.out.as_os_str().is_empty() {
        return Err("--out is required".into());
    }
    Ok(o)
}

/// `d2rs cli map`: 0 all written, 1 some level failed or the game files did not load, 2 bad arguments.
pub fn cli(a: &[String]) -> i32 {
    let bad = |e: String| {
        eprintln!("{e}\n{USAGE}");
        2
    };
    let o = match parse(a) {
        Ok(o) => o,
        Err(e) => return bad(e),
    };
    let (seed, exp) = match (&o.save, o.seed) {
        (Some(p), _) => match d2host::read(std::path::Path::new(p)).map_err(s).and_then(|b| d2s::Save::parse(&b, d2s::Schema::classic()).map_err(s)) {
            Ok(v) => (v.map_seed(), v.status(d2s::save::EXPANSION)),
            Err(e) => {
                eprintln!("{p}: {e}");
                return 1;
            }
        },
        (_, n) => (n.unwrap_or(0), true),
    };
    let Some(game) = o.game.clone().or_else(|| std::env::var_os("D2RS_GAME_DIR").map(PathBuf::from)) else { return bad("no game folder".into()) };
    let ctx = match Ctx::load(&game) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let acts = match (o.level.map(|l| ctx.act_of(l)), o.act) {
        (Some(None), _) => return bad(format!("unknown level {}", o.level.unwrap_or(0))),
        (Some(Some(a)), _) | (_, Some(a)) => vec![a],
        _ => (0..5).collect(),
    };
    let j = Job { seed, diff: o.diff, exp: exp && !o.classic, out: o.out, acts, level: o.level, iso: !o.no_iso, whole: o.act.is_some() };
    let errs = ctx.export(&j, &AtomicBool::new(false), &AtomicUsize::new(0));
    println!("map seed {seed}: {} {}, {} failed, written to {}", ctx.count(&j), if j.whole { "acts" } else { "levels" }, errs.len(), j.out.display());
    errs.iter().for_each(|e| eprintln!("{e}"));
    i32::from(!errs.is_empty())
}

enum Msg {
    View(Box<Result<Pictures, String>>),
    Done(Vec<String>),
}

struct Shown {
    coll: egui::TextureHandle,
    iso: Option<egui::TextureHandle>,
    iso_err: Option<String>,
    labels: Vec<Lab>,
    size: [egui::Vec2; 2],
}

pub struct MapUi {
    ctx: Option<(PathBuf, Result<Arc<Ctx>, String>)>,
    seed: String,
    from_save: bool,
    exp: bool,
    diff: u8,
    act: u8,
    level: u32,
    whole: bool,
    labels: bool,
    iso_view: bool,
    iso_export: bool,
    #[doc(hidden)] pub view: View,
    rx: Option<mpsc::Receiver<Msg>>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    errors: Vec<String>,
    status: String,
    shown: Option<Shown>,
    /// Export folder that replaces the folder dialog (tests).
    #[doc(hidden)] pub out: Option<PathBuf>,
    /// PNG that replaces the file dialog (tests).
    #[doc(hidden)] pub png: Option<PathBuf>,
    /// An export (all acts?) waiting for its folder.
    want: Option<bool>,
}

impl Default for MapUi {
    fn default() -> Self {
        MapUi { ctx: None, seed: "4832".into(), from_save: true, exp: true, diff: 0, act: 0, level: 1, whole: false, labels: true, iso_view: false, iso_export: true, view: View::default(), rx: None, cancel: Default::default(), done: Default::default(), total: Default::default(), errors: vec![], status: String::new(), shown: None, out: None, png: None, want: None }
    }
}

/// Keeps every side within what a GPU texture allows by taking every n-th pixel.
#[doc(hidden)]
pub fn texture(ctx: &egui::Context, name: &str, img: &Image, k: f32) -> (egui::TextureHandle, egui::Vec2) {
    let n = (img.w.max(img.h) as usize).div_ceil(ctx.input(|i| i.max_texture_side)).max(1);
    let (w, h) = (img.w as usize, img.h as usize);
    let px: Vec<u8> = (0..h.div_ceil(n)).flat_map(|y| (0..w.div_ceil(n)).flat_map(move |x| (0..4).map(move |k| (y * n * w + x * n) * 4 + k))).map(|i| img.rgba[i]).collect();
    let c = egui::ColorImage::from_rgba_unmultiplied([w.div_ceil(n), h.div_ceil(n)], &px);
    (ctx.load_texture(name, c, egui::TextureOptions::NEAREST), egui::vec2(w as f32 * k, h as f32 * k))
}

impl MapUi {
    fn start(&mut self, ctx: Arc<Ctx>, f: impl FnOnce(&Ctx, &AtomicBool, &AtomicUsize) -> Msg + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        (self.cancel, self.done) = (Arc::default(), Arc::default());
        let (c, d) = (self.cancel.clone(), self.done.clone());
        self.rx = Some(rx);
        d2host::spawn(move || drop(tx.send(f(&ctx, &c, &d))));
    }

    /// The Map tab. `save` is the open save's map seed and expansion flag.
    pub fn show(&mut self, ui: &mut egui::Ui, dir: Option<&Path>, save: Option<(u32, bool)>) {
        let clicked = ui.button("Open PNG...").on_hover_text("view any PNG here, no game files needed").clicked();
        if clicked && self.png.is_none() {
            platform::pick(Pick::Png, ui.ctx());
        }
        if let Some(p) = self.png.clone().filter(|_| clicked).or_else(|| platform::picked(Pick::Png)) {
            match render::read_png(&p) {
                Ok(img) => {
                    let (coll, size) = texture(ui.ctx(), "map-png", &img, 1.0);
                    (self.shown, self.iso_view, self.status) = (Some(Shown { coll, iso: None, iso_err: None, labels: vec![], size: [size; 2] }), false, String::new());
                }
                Err(e) => self.status = format!("{}: {e}", p.display()),
            }
        }
        self.controls(ui, dir, save);
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        let shown = self.shown.as_ref().and_then(|sh| match (&sh.iso, self.iso_view) {
            (Some(t), true) => Some((t.id(), sh.size[1])),
            (None, true) => None,
            _ => Some((sh.coll.id(), sh.size[0])),
        });
        if let (Some(sh), true) = (&self.shown, self.iso_view && shown.is_none()) {
            ui.label(sh.iso_err.as_deref().map_or("Press Show with Isometric selected to build the isometric picture.".into(), |e| format!("Isometric picture failed: {e}")));
        }
        let labels = self.shown.as_ref().filter(|_| self.labels && shown.is_some()).map(|sh| (&sh.labels[..], usize::from(self.iso_view)));
        self.view.show(ui, shown, labels);
    }

    fn controls(&mut self, ui: &mut egui::Ui, dir: Option<&Path>, save: Option<(u32, bool)>) {
        let Some(dir) = dir else { return drop(ui.label("The Map tab needs the game 1.14d folder (the MPQs and Game.exe). Use \"Game folder...\" on the left.")) };
        if self.ctx.as_ref().is_none_or(|c| c.0 != dir) {
            self.ctx = Some((dir.into(), Ctx::load(dir).map(Arc::new)));
        }
        let Some((_, Ok(ctx))) = self.ctx.clone() else { return drop(ui.label(format!("Cannot load the game files: {}", self.ctx.as_ref().and_then(|c| c.1.as_ref().err()).map_or("", |e| e)))) };
        while let Some(m) = self.rx.as_ref().and_then(|r| r.try_recv().ok()) {
            self.rx = None;
            match m {
                Msg::View(r) => match *r {
                    Ok((c, i, labels)) => {
                        let (coll, cs) = texture(ui.ctx(), "map-collision", &c, 1.0);
                        let iso = i.as_ref().and_then(|i| i.as_ref().ok()).map(|(i, k)| texture(ui.ctx(), "map-iso", i, *k as f32));
                        self.shown = Some(Shown { coll, size: [cs, iso.as_ref().map_or(cs, |t| t.1)], iso_err: i.and_then(Result::err), labels, iso: iso.map(|t| t.0) });
                        self.status = String::new();
                    }
                    Err(e) => (self.shown, self.status) = (None, e),
                },
                Msg::Done(e) => {
                    let unit = if self.whole { "acts" } else { "levels" };
                    self.status = if self.cancel.load(Relaxed) { format!("Cancelled after {} of {} {unit}, {} failed.", self.done.load(Relaxed), self.total.load(Relaxed), e.len()) } else { format!("Export finished: {} {unit}, {} failed.", self.total.load(Relaxed), e.len()) };
                    self.errors = e;
                    if cfg!(target_arch = "wasm32") {
                        let files = d2host::read_dir(Path::new("/export")).unwrap_or_default();
                        platform::offer(&files);
                        self.status += &format!(" {} files offered for download: check the browser's download bar.", files.len());
                    }
                }
            }
        }
        let busy = self.rx.is_some();
        if busy {
            ui.ctx().request_repaint();
        }
        let from_save = self.from_save && save.is_some();
        ui.horizontal(|ui| {
            if let Some((n, _)) = save {
                ui.checkbox(&mut self.from_save, format!("Seed of the open save ({n})"));
            }
            ui.add_enabled_ui(!from_save, |ui| {
                ui.label("Seed");
                ui.add(egui::TextEdit::singleline(&mut self.seed).desired_width(90.0));
                ui.checkbox(&mut self.exp, "Expansion");
            });
            ["Normal", "Nightmare", "Hell"].iter().zip(0..).for_each(|(n, d)| drop(ui.selectable_value(&mut self.diff, d, *n)));
        });
        let (seed, exp) = match (from_save, save) {
            (true, Some((n, e))) => (Some(n), e),
            _ => (self.seed.trim().parse::<u32>().ok(), self.exp),
        };
        ui.horizontal(|ui| {
            ["I", "II", "III", "IV", "V"].iter().zip(0..).for_each(|(n, a)| drop(ui.selectable_value(&mut self.act, a, format!("Act {n}"))));
            ui.separator();
            ui.selectable_value(&mut self.whole, false, "Level");
            ui.selectable_value(&mut self.whole, true, "Whole act");
            let ids = ctx.act_levels(self.act);
            if !ids.contains(&self.level) {
                self.level = ids.first().copied().unwrap_or(1);
            }
            if !self.whole {
                egui::ComboBox::from_id_salt("map-level").selected_text(ctx.label(self.level)).show_ui(ui, |ui| ids.iter().for_each(|&i| drop(ui.selectable_value(&mut self.level, i, ctx.label(i)))));
            }
        });
        if self.act == 4 && !exp {
            ui.colored_label(ui.visuals().warn_fg_color, "A classic character has no Act V: its levels fail with a named error.");
        }
        let Some(seed) = seed else { return drop(ui.label("Type a seed (a number up to 4294967295).")) };
        let (a, id, diff, iso, out_iso, whole) = (self.act, self.level, self.diff, self.iso_view, self.iso_export, self.whole);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if ui.button("Show").on_hover_text("builds the level or the whole act; the isometric picture only when Isometric is selected (it is large and slow)").clicked() {
                    self.status = "Building...".into();
                    self.total.store(0, Relaxed);
                    let t = self.total.clone();
                    let j = Job { seed, diff, exp, out: PathBuf::new(), acts: vec![a], level: None, iso, whole };
                    self.start(ctx.clone(), move |c, cancel, done| Msg::View(Box::new(if whole { c.view_act(&j, a, cancel, done, &t) } else { c.view(seed, diff, exp, a, id, iso) })));
                }
                ui.selectable_value(&mut self.iso_view, false, "Collision");
                ui.selectable_value(&mut self.iso_view, true, "Isometric");
                ui.checkbox(&mut self.labels, "Labels");
                ui.separator();
                ui.checkbox(&mut self.iso_export, "Isometric in exports");
                for (label, all) in [(if whole { "Export act..." } else { "Export level..." }, false), ("Export all...", true)].into_iter().filter(|(_, all)| !(*all && cfg!(target_arch = "wasm32"))) {
                    if ui.button(label).clicked() {
                        self.want = Some(all);
                        if self.out.is_none() {
                            platform::pick(Pick::ExportFolder, ui.ctx());
                        }
                    }
                }
                if let Some(all) = self.want
                    && let Some(out) = self.out.clone().or_else(|| platform::picked(Pick::ExportFolder))
                {
                    self.want = None;
                    if cfg!(target_arch = "wasm32") {
                        d2host::unmount(&out);
                    }
                    let j = Job { seed, diff, exp, out, acts: if all { (0..5).collect() } else { vec![a] }, level: (!all && !whole).then_some(id), iso: out_iso, whole };
                    (self.errors, self.status) = (vec![], String::new());
                    self.total.store(ctx.count(&j), Relaxed);
                    self.start(ctx.clone(), move |c, cancel, done| Msg::Done(c.export(&j, cancel, done)));
                }
            });
            if busy && self.total.load(Relaxed) > 0 {
                ui.add(egui::ProgressBar::new(self.done.load(Relaxed) as f32 / self.total.load(Relaxed) as f32).show_percentage());
                if ui.button("Cancel").clicked() {
                    self.cancel.store(true, Relaxed);
                }
            }
        });
        if !self.errors.is_empty() {
            egui::CollapsingHeader::new(format!("{} levels failed", self.errors.len())).show(ui, |ui| self.errors.iter().for_each(|e| drop(ui.label(e))));
        }
    }
}

/// Pan and zoom state of the picture area: `pan` is where the image's top-left corner sits inside the area, `zoom` screen points per `size` unit.
#[derive(Default)]
#[doc(hidden)]
pub struct View {
    #[doc(hidden)] pub zoom: f32,
    #[doc(hidden)] pub pan: egui::Vec2,
    /// The image drawn last frame and the area it was drawn in; a different image starts fitted.
    #[doc(hidden)] pub tex: Option<egui::TextureId>,
    #[doc(hidden)] pub area: egui::Vec2,
    #[doc(hidden)] pub origin: egui::Pos2,
}

const HINT: &str = "Pick a level and press Show, or Open PNG...";

impl View {
    fn fit(&mut self, size: egui::Vec2) {
        self.zoom = (self.area.x / size.x).min(self.area.y / size.y).clamp(ZOOM.0, ZOOM.1);
        self.pan = (self.area - size * self.zoom) / 2.0;
    }

    /// Controls, legend and the picture (`None` shows the hint): drag pans, the wheel zooms around the cursor.
    fn show(&mut self, ui: &mut egui::Ui, shown: Option<(egui::TextureId, egui::Vec2)>, labels: Option<(&[Lab], usize)>) {
        ui.horizontal(|ui| {
            if let Some((_, size)) = shown {
                if ui.button("Fit").clicked() {
                    self.fit(size);
                }
                if ui.button("1:1").clicked() {
                    (self.zoom, self.pan) = (1.0, (self.area - size) / 2.0);
                }
                ui.add(egui::Slider::new(&mut self.zoom, ZOOM.0..=ZOOM.1).logarithmic(true).text("Zoom"));
            }
            let colours = [("walkable", render::WALK), ("blocked", render::BLOCKED), ("no room", render::VOID), ("edge", render::EDGE), ("level centre", render::LABEL), ("warp", render::WARP), ("waypoint", render::WAYPOINT)];
            for (name, c) in colours {
                let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), egui::Sense::hover());
                ui.painter().rect(r, 2.0, egui::Color32::from_rgb(c[0], c[1], c[2]), ui.visuals().widgets.noninteractive.fg_stroke, egui::StrokeKind::Inside);
                ui.label(name);
            }
        });
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        (self.area, self.origin) = (rect.size(), rect.min);
        let Some((id, size)) = shown else { return drop(ui.put(rect, egui::Label::new(HINT))) };
        if self.tex != Some(id) {
            (self.tex, self.zoom) = (Some(id), 0.0);
            self.fit(size);
        }
        self.pan += resp.drag_delta();
        if let Some(c) = resp.hover_pos().map(|p| p - rect.min) {
            let z = (self.zoom * ui.input(|i| i.smooth_scroll_delta.y * 0.002).exp()).clamp(ZOOM.0, ZOOM.1);
            self.pan = c - (c - self.pan) * (z / self.zoom);
            self.zoom = z;
        }
        let img = egui::Rect::from_min_size(rect.min + self.pan, size * self.zoom);
        ui.painter_at(rect).image(id, img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        // Names as label widgets (the painter's text is not in the accessibility tree), clipped to the picture area.
        let mut layer = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        layer.set_clip_rect(rect);
        for (text, warp, pos) in labels.map_or(&[][..], |l| l.0) {
            let p = rect.min + self.pan + pos[labels.map_or(0, |l| l.1)] * self.zoom - if *warp { egui::vec2(0.0, 14.0) } else { egui::Vec2::ZERO };
            if rect.expand(100.0).contains(p) {
                let colour = if *warp { egui::Color32::from_rgb(255, 150, 130) } else { egui::Color32::WHITE };
                let t = egui::RichText::new(text).size(13.0).color(colour).background_color(egui::Color32::from_black_alpha(170));
                let w = layer.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(13.0), colour).size() + egui::vec2(4.0, 2.0);
                layer.put(egui::Rect::from_center_size(p, w), egui::Label::new(t).selectable(false));
            }
        }
    }
}

const ZOOM: (f32, f32) = (0.005, 16.0);
