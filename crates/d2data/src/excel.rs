//! Level tables (`Levels`, `LvlPrest`, `LvlMaze`, `LvlSub`, `LvlTypes`, `LvlWarp`) read from the tab-separated
//! `.txt` files by column name. Each `*Def` holds the columns the DRLG needs; blank or non-numeric cells are 0.
use crate::{Error, GameData, Result};

/// One data row with its header: cells looked up by column name.
struct Row<'a>(&'a [&'a str], Vec<&'a str>);

impl Row<'_> {
    fn s(&self, k: &str) -> String {
        self.0.iter().position(|h| *h == k).and_then(|i| self.1.get(i)).map_or(String::new(), |s| s.to_string())
    }
    fn n(&self, k: &str) -> i32 {
        self.s(k).trim().parse::<i64>().unwrap_or(0) as i32
    }
    fn a<const N: usize>(&self, f: impl Fn(usize) -> String) -> [i32; N] {
        std::array::from_fn(|i| self.n(&f(i)))
    }
}

fn rows<T>(b: &[u8], f: impl Fn(&Row) -> T) -> Vec<T> {
    let text = String::from_utf8_lossy(b);
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    lines.filter(|l| !l.is_empty() && !l.starts_with("Expansion\t")).map(|l| f(&Row(&head, l.split('\t').collect()))).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelDef {
    pub id: i32,
    pub act: i32,
    /// Normal, nightmare, hell.
    pub size_x: [i32; 3],
    pub size_y: [i32; 3],
    pub offset_x: i32,
    pub offset_y: i32,
    pub depend: i32,
    /// 1 maze, 2 preset, 3 outdoor.
    pub drlg_type: i32,
    pub level_type: i32,
    pub sub_type: i32,
    pub sub_theme: i32,
    pub sub_waypoint: i32,
    pub sub_shrine: i32,
    pub vis: [i32; 8],
    pub warp: [i32; 8],
    pub position: i32,
    pub save_monsters: i32,
    pub warp_dist: i32,
    pub mon_wndr: i32,
}

impl LevelDef {
    pub fn parse(b: &[u8]) -> Vec<LevelDef> {
        rows(b, |r| LevelDef {
            id: r.n("Id"),
            act: r.n("Act"),
            size_x: [r.n("SizeX"), r.n("SizeX(N)"), r.n("SizeX(H)")],
            size_y: [r.n("SizeY"), r.n("SizeY(N)"), r.n("SizeY(H)")],
            offset_x: r.n("OffsetX"),
            offset_y: r.n("OffsetY"),
            depend: r.n("Depend"),
            drlg_type: r.n("DrlgType"),
            level_type: r.n("LevelType"),
            sub_type: r.n("SubType"),
            sub_theme: r.n("SubTheme"),
            sub_waypoint: r.n("SubWaypoint"),
            sub_shrine: r.n("SubShrine"),
            vis: r.a(|i| format!("Vis{i}")),
            warp: r.a(|i| format!("Warp{i}")),
            position: r.n("Position"),
            save_monsters: r.n("SaveMonsters"),
            warp_dist: r.n("WarpDist"),
            mon_wndr: r.n("MonWndr"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrestDef {
    pub def: i32,
    pub level_id: i32,
    pub populate: i32,
    pub logicals: i32,
    pub outdoors: i32,
    pub animate: i32,
    pub kill_edge: i32,
    pub fill_blanks: i32,
    pub size_x: i32,
    pub size_y: i32,
    pub auto_map: i32,
    pub scan: i32,
    pub pops: i32,
    pub pop_pad: i32,
    pub files: i32,
    /// `File1..File6`, empty when blank or `0`.
    pub file: Vec<String>,
    pub dt1_mask: u32,
}

impl PrestDef {
    pub fn parse(b: &[u8]) -> Vec<PrestDef> {
        rows(b, |r| PrestDef {
            def: r.n("Def"),
            level_id: r.n("LevelId"),
            populate: r.n("Populate"),
            logicals: r.n("Logicals"),
            outdoors: r.n("Outdoors"),
            animate: r.n("Animate"),
            kill_edge: r.n("KillEdge"),
            fill_blanks: r.n("FillBlanks"),
            size_x: r.n("SizeX"),
            size_y: r.n("SizeY"),
            auto_map: r.n("AutoMap"),
            scan: r.n("Scan"),
            pops: r.n("Pops"),
            pop_pad: r.n("PopPad"),
            files: r.n("Files"),
            file: (1..=6).map(|i| r.s(&format!("File{i}"))).map(|f| if f == "0" { String::new() } else { f }).collect(),
            dt1_mask: r.n("Dt1Mask") as u32,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MazeDef {
    pub level: i32,
    /// Normal, nightmare, hell.
    pub rooms: [i32; 3],
    pub size_x: i32,
    pub size_y: i32,
    pub merge: i32,
}

impl MazeDef {
    pub fn parse(b: &[u8]) -> Vec<MazeDef> {
        rows(b, |r| MazeDef {
            level: r.n("Level"),
            rooms: [r.n("Rooms"), r.n("Rooms(N)"), r.n("Rooms(H)")],
            size_x: r.n("SizeX"),
            size_y: r.n("SizeY"),
            merge: r.n("Merge"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubDef {
    pub kind: i32,
    pub file: String,
    pub check_all: i32,
    pub bord_type: i32,
    pub grid_size: i32,
    pub dt1_mask: u32,
    pub prob: [i32; 5],
    pub trials: [i32; 5],
    pub max: [i32; 5],
}

impl SubDef {
    pub fn parse(b: &[u8]) -> Vec<SubDef> {
        rows(b, |r| SubDef {
            kind: r.n("Type"),
            file: r.s("File"),
            check_all: r.n("CheckAll"),
            bord_type: r.n("BordType"),
            grid_size: r.n("GridSize"),
            dt1_mask: r.n("Dt1Mask") as u32,
            prob: r.a(|i| format!("Prob{i}")),
            trials: r.a(|i| format!("Trials{i}")),
            max: r.a(|i| format!("Max{i}")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    pub id: i32,
    /// `File 1..File 32`, empty when blank or `0`.
    pub file: Vec<String>,
}

impl TypeDef {
    pub fn parse(b: &[u8]) -> Vec<TypeDef> {
        rows(b, |r| TypeDef {
            id: r.n("Id"),
            file: (1..=32).map(|i| r.s(&format!("File {i}"))).map(|f| if f == "0" { String::new() } else { f }).collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarpDef {
    pub id: i32,
    pub lit_version: i32,
    pub tiles: i32,
    /// First byte of `Direction` (`b'b'`, `b'l'`, `b'r'`), 0 when blank.
    pub direction: u8,
}

impl WarpDef {
    pub fn parse(b: &[u8]) -> Vec<WarpDef> {
        rows(b, |r| WarpDef { id: r.n("Id"), lit_version: r.n("LitVersion"), tiles: r.n("Tiles"), direction: r.s("Direction").bytes().next().unwrap_or(0) })
    }
}

/// The six level tables, rows in file order (the `Expansion` marker row is dropped).
#[derive(Debug, Clone)]
pub struct MapTables {
    pub levels: Vec<LevelDef>,
    pub prest: Vec<PrestDef>,
    pub maze: Vec<MazeDef>,
    pub sub: Vec<SubDef>,
    pub types: Vec<TypeDef>,
    pub warp: Vec<WarpDef>,
}

impl MapTables {
    /// An absent file gives an `Error` naming its path.
    pub fn load(gd: &GameData) -> Result<MapTables> {
        fn tab<T>(gd: &GameData, path: &'static str, f: fn(&[u8]) -> Vec<T>) -> Result<Vec<T>> {
            gd.read(path).map(|b| f(&b)).filter(|v| !v.is_empty()).ok_or(Error(path))
        }
        Ok(MapTables {
            levels: tab(gd, "data\\global\\excel\\levels.txt", LevelDef::parse)?,
            prest: tab(gd, "data\\global\\excel\\lvlprest.txt", PrestDef::parse)?,
            maze: tab(gd, "data\\global\\excel\\lvlmaze.txt", MazeDef::parse)?,
            sub: tab(gd, "data\\global\\excel\\lvlsub.txt", SubDef::parse)?,
            types: tab(gd, "data\\global\\excel\\lvltypes.txt", TypeDef::parse)?,
            warp: tab(gd, "data\\global\\excel\\lvlwarp.txt", WarpDef::parse)?,
        })
    }
}
