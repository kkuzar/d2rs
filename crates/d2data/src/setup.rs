//! Detects what the game folder is: vanilla 1.14d tables or a mod, plus PlugY and known loaders.
use crate::{DataDir, GameData};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableState {
    Vanilla,
    Modified,
    Missing,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: &'static str,
    pub txt: TableState,
    pub bin: TableState,
    /// A loose file in the game folder's `data` tree supplies the table (it wins over the MPQs).
    pub from_data_dir: bool,
}

#[derive(Debug, Clone)]
pub struct PlugY {
    pub dll: bool,
    pub ini: bool,
    pub big_stash: Option<bool>,
    pub save_files: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Setup {
    pub tables: Vec<TableInfo>,
    pub plugy: Option<PlugY>,
    pub loaders: Vec<String>,
    pub data_dir: bool,
}

impl Setup {
    /// True when any table differs from vanilla 1.14d or a mod loader is present.
    pub fn modded(&self) -> bool {
        !self.format_modified().is_empty() || !self.loaders.is_empty()
    }

    /// Tables whose `.txt` or `.bin` differs from vanilla (missing ones are not counted).
    pub fn format_modified(&self) -> Vec<&'static str> {
        self.tables.iter().filter(|t| t.txt == TableState::Modified || t.bin == TableState::Modified).map(|t| t.name).collect()
    }

    pub fn summary(&self) -> String {
        let mut s = if self.tables.iter().all(|t| t.txt == TableState::Missing && t.bin == TableState::Missing) {
            "no game data".to_string()
        } else if self.modded() {
            let m = self.format_modified();
            let l = if self.loaders.is_empty() { String::new() } else { format!(" [{}]", self.loaders.join(", ")) };
            format!("modded: {} table(s) changed{}{l}", m.len(), if m.is_empty() { String::new() } else { format!(" ({})", m.join(", ")) })
        } else {
            "vanilla 1.14d".to_string()
        };
        if let Some(p) = &self.plugy {
            s += &format!("; PlugY{}", if p.big_stash == Some(true) { " (BigStash)" } else { "" });
        }
        s
    }
}

/// `ActiveBigStash` under `[BIGSTASH]` of a PlugY.ini text; `None` when absent.
pub fn ini_big_stash(text: &str) -> Option<bool> {
    let mut on = false;
    for l in text.lines().map(|l| l.trim().to_ascii_lowercase().replace(' ', "")) {
        if l.starts_with('[') {
            on = l == "[bigstash]";
        } else if on && let Some(v) = l.strip_prefix("activebigstash=") {
            return Some(v.trim_start().starts_with('1'));
        }
    }
    None
}

const LOADERS: [(&str, &[&str]); 7] = [
    ("D2SE", &["D2SE.exe", "D2SE_SETUP.ini"]),
    ("BaseMod", &["BaseMod.dll", "BaseMod.ini"]),
    ("D2Mod", &["D2Mod.dll", "D2Mod.ini"]),
    ("SGD2FreeRes", &["SGD2FreeRes.dll"]),
    ("D2DX", &["D2DX.dll", "d2dx.cfg", "glide3x.dll"]),
    ("D2HD", &["D2HD.dll", "D2HD.mpq"]),
    ("PlugY", &["PlugY.dll"]),
];

fn state(bytes: Option<Vec<u8>>, vanilla: &str) -> TableState {
    match bytes {
        None => TableState::Missing,
        Some(b) if vanilla == format!("{:x}", Sha256::digest(&b)) => TableState::Vanilla,
        Some(_) => TableState::Modified,
    }
}

/// Inspect a game folder (MPQs plus a loose `data` tree) and the save folder (PlugY shared stash files).
pub fn detect(game: Option<&Path>, save_dir: Option<&Path>) -> Setup {
    let dir = game.map(DataDir::new);
    let mut gd = game.and_then(|g| GameData::from_install(g).ok()).unwrap_or_default();
    if let Some(g) = game {
        gd = gd.with_override(DataDir::new(g));
    }
    let loose = |p: &str| dir.as_ref().is_some_and(|d| d.find(p).is_some());
    let tables = VANILLA
        .iter()
        .map(|&(name, txt, bin)| {
            let p = |e: &str| format!("data\\global\\excel\\{name}.{e}");
            TableInfo { name, txt: state(gd.read(&p("txt")), txt), bin: state(gd.read(&p("bin")), bin), from_data_dir: loose(&p("txt")) || loose(&p("bin")) }
        })
        .collect();
    let has = |f: &str| loose(f);
    let ini = dir.as_ref().and_then(|d| d.read("PlugY.ini"));
    let save_files = save_dir.and_then(|s| d2host::read_dir(s).ok()).is_some_and(|r| r.iter().any(|e| e.extension().is_some_and(|x| x.eq_ignore_ascii_case("sss") || x.eq_ignore_ascii_case("d2x"))));
    let (dll, has_ini) = (has("PlugY.dll"), ini.is_some());
    Setup {
        tables,
        plugy: (dll || has_ini || save_files).then(|| PlugY { dll, ini: has_ini, big_stash: ini.and_then(|b| ini_big_stash(&String::from_utf8_lossy(&b))), save_files }),
        loaders: LOADERS.iter().filter(|(_, fs)| fs.iter().any(|f| has(f))).map(|(n, _)| n.to_string()).collect(),
        data_dir: has("data"),
    }
}

/// Sha256 (hex) of each table's `.txt` and `.bin` in vanilla 1.14d, from a verified install.
const VANILLA: &[(&str, &str, &str)] = &[
    ("ItemStatCost", "ee7fa82b48e0c4f27a566334b2da13cf1c83f5c4c0439552084a231cb5938375", "969fa476c0d9c3c4cfa03f99a0abd12e6c15ffee8cb9f70a96e3b85d3600e544"),
    ("Weapons", "05a938e3fc0b8974bc722bfa8974f44e45e99d19c6877613ff731104b12b3536", "70f6677a8b2c019795d172dd9c90deea4663249504b85c65121da9c917e00983"),
    ("Armor", "d8a4827d52807652f8de33d1b9b335ab8bdd40d1b60b5cbabce24ff9987b30fa", "25dd0a0080b8bf7c44a22f8b5265ff9a0e580fd34eb5718bbfbae0cd1533e7cb"),
    ("Misc", "f7fa37ae4f7cdb3f5337a28a55b3876fc27aef066822b3268933e56b636b50f0", "4c0b0e6c2f888847b7a71225d15caf8708c1dd33a9cc8420b6cbbf594963bc1b"),
    ("ItemTypes", "d399dc5d0f3a3e1a4f1eb611dabef8311b7025392c416f58dc26baff93af3550", "233068c5ff96101cd9c460ddf6b7274286e4bd7967dc8168ddda80c3908826b3"),
    ("UniqueItems", "161a5ac42bbcc281e29647822c5a119ac593cef3d631b2ef693b4e5075eab46c", "29328c76a0881f57a268ceba1eefeb6728720881085593faea47e3d7aa708769"),
    ("SetItems", "5696730cf84564f1cb815fb362fba13f23e22f5176e854730733519048f711be", "b793de221bf5ebe94e4f8fdbab6b0c4a6ce0f5b1ba8f4ff78f12dc3d851272ce"),
    ("Sets", "e6694bc603f1f29efdfa981c3f747708e11bb0410786fa8bf5ab634f60f6b822", "abec75e33edd8ffaf1e28c04a4e79adc8e47198f5bf9a552fa057ba4cb220aa4"),
    ("Runes", "f8869655f58f5759919d088b3557e9e0615265f11ee70571a3c15334f95fc935", "7866bdb5aa0fa63670f346e82b532b8260e4f175728fdbdbc856f6806b2a6661"),
    ("MagicPrefix", "f58b05c7b5db4464cbce1087b73a6f99592be56ea955fe60dd901fcddb56c561", "cdcbf46c2e32c39006b224d2590041a076d45bc768fd68d09706607a1493f20b"),
    ("MagicSuffix", "47d4399809c3f123f338deebb139139b0f775f47c435e286954457b0cd47de21", "ec44c7bcff0323661170832452a9996759acc7673f15b6dc7fe07998a9f36206"),
    ("RarePrefix", "529caeee9493a331bd693b8de4e5eaecac271b93f25117ae28da222fc787b5f4", "3138a4e84b612611cfe6d929e5de8e800e417c3f26ee9e98ee203f4d41d49617"),
    ("RareSuffix", "4652d644dff63c5f7086e747911d86a21175f4dcd4718813ac2607fcf6da5ffe", "00f0a4867541e5ce458ad2f7eb829a1a5fef7809b8348230657da5e632dc7185"),
    ("AutoMagic", "9c0ea5edcb02422fcfc6f7274a5667057d1a4a205102bb31cb2e102157aeb050", "1838a9581ca8118690cd8b8b8b938c9fb87087a6192dd1a91e27411888d3385c"),
    ("Properties", "1cea2a360d804559528e7cfdb4dc30f7e33950d074846ef66e49fbc142bacd18", "5dc97008fb367ff1f4301d784835a93e9e4e0379cd42b1faf013a1f7582dcc3f"),
    ("QualityItems", "3d62dde3248648d1de89a994241ab7b85f3de412ddde2076b553fac693699df4", "cf7d88e236c86c27840dc5b484f9fc0ab43c0e75b85d3d3f65e9cdd9f82d7292"),
    ("LowQualityItems", "66a8a59dd88e0bed8130a08d803a057f9331f575a7c9a28f0aa622b34f098108", "aaeb6f0eab7d768ac48ecf2fcca82fced595b0529467c2e6cbdd9d38aa3821e3"),
    ("Gems", "157925b4d12502eed32139ca7a246b6f1f7bfca3ea2291239def4bbf8a222bf0", "6b25b05224c2695caa23aa22a52d2fde736edd595d5800e8065e456327ba8508"),
    ("Belts", "55d680040663a8e6930863fc7687fe041b72c04cb98f97737794cc5bc0b41fb5", "cf8296500985694bc0327b5249a47bbe14558b65f956bd7a36b115f1a670e473"),
    ("CharStats", "bfd28714a573384da0575c6525b3733c82e3bf97f47d0f3e1b8552d137bff51f", "52e1903468b6ba50392545b1e61578b239b9b98a89d21e1ebf156988094541a6"),
    ("Experience", "a6f0f717bfdf4ab36ac2b55d5aa67b9a6e04d32c650facffbfcb00ed7e505c6d", "2cffe98258e40e45ed241747d5f4801f16904058302269378495f4cdc98e0e5d"),
    ("Skills", "465c28e83657540e0d7ae5b0de826d5657be7fba1aa539a85c3ca3aeea34b498", "0b57ba62b6b5c74be6a579adb9327641ecfe880ebc0c217d341c9d5bba89043b"),
    ("SkillDesc", "cd1cd11cb1d85a5dda7a4e666f6e15dceacfd121e4e839e2842d0e25c44ed993", "52c96055cf6016b3cd2b45ba51106f1e13a089bd92be691d0d93770981b54740"),
    ("Hireling", "c743a6489456bd2c29143ae55312fda8fb6acb226d55a637c2c8f54a555afa78", "540ec2865297d57a4faa0793476858cb7226aa27a457b3ac2d9845151b9d95f7"),
    ("Inventory", "45e9e49143b9f9c4fa5c8b4eef44b7113a2cd87ddd73a02b47a9503a1ca5524c", "1f294b790787957ef4e293e7a8dfddc0aa8200a5d29e06aa3e1232693024f049"),
    ("MonStats", "a15e1cb876bd9af1e8f54199c9619226cf4835c45c799aed2d432901487ecb7c", "b8c6b72df4c7825a43da33d3e533f2e7e14b268b01c6236e82ab2d800fc75584"),
    ("MonType", "755edd9bbe12e8427373d1725e0bcea62f31fca0a75871936057cca432960faa", "81c4732b29d441a64cdf2bcd0c157cd71fa872f29f1765b4ea01ec65a2d2ce6f"),
    ("States", "898806c2b69e55e316ecfed48dde95aa2933829b94fdad234bdabc27bdbc23e1", "534886c9d40229282d69128d30b1c1457a73a21976ba0ca22546bd4a8f373291"),
    ("Levels", "34add892965a7b38f08c7346ffe7a41ede6668b9d5b28277493ddf157e55da0b", "4af188176b8830afa78ee5f31558e6fd3b6c4a6c77fd9b4016490611a18e0cba"),
    ("LvlPrest", "c2f6a891f07145fcffd8755b6fd163f86b6c3363fc45e9c6fe133436f7ae3f9e", "bdb6b98a648de5fde9cf1e587a28140786960d7fceba4b8064ef5584ec199c30"),
    ("LvlMaze", "b9e04882f357b20ecfc0f166d7699c93119d1aed545503f2f85866c0a89b4dc3", "4da089a98d38235ab2b66bcfffde11f1a7e179c84f796e36a7f37db0b29fc80a"),
    ("LvlSub", "d3dc9dc70066f819ff9e0c1b433b327952789ed7ad3e5eadccf7160280ddc64f", "de461fd72b8b72ecce452587e408d9aef84eb679c53f2c78a398cb55b17a0247"),
    ("LvlTypes", "4d8a6cc08afd6c66a493b677b1601182695a437dcc0ab5fedecc379537e5c84c", "9399f78364f96ef4bbd3971f808ada15af810dde65c76e9b9132128c8e2c3ece"),
    ("LvlWarp", "ee95bd2015140049ab497f44b6de5e7e4b43a8434524baf817776e735dbf4021", "89f88d0695a521c485a78195a2a93bd8cc195b8d11bcf05b76376c1a5076e7db"),
];
