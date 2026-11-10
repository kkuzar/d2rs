//! Game tables by name (tools/gen_data.py format). The core tables (save format, bases, item types, skills,
//! experience, belts, hirelings) ship in `core/`; the item generation tables (uniques, sets, affixes, runewords,
//! properties, qualities) come from the user's game (`ItemTables::from_txt`) and are empty here.
//! `D2RS_TABLES` names a folder of generated tables (maintainers, private tests); its files win over `core/`.
use std::{collections::HashMap, path::Path, sync::OnceLock};

pub(crate) fn table(name: &str) -> &'static str {
    static DIR: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let d = std::env::var("D2RS_TABLES").unwrap_or_default();
        d2host::read_dir(Path::new(&d)).unwrap_or_default().into_iter().filter_map(|p| {
            let s: &'static str = d2host::read_to_string(&p).ok()?.leak();
            Some((p.file_stem()?.to_str()?.to_string(), s))
        }).collect()
    });
    dir.get(name).copied().unwrap_or(match name {
        "isc" => include_str!("../core/isc.tsv"),
        "items" => include_str!("../core/items.tsv"),
        "itemtypes" => include_str!("../core/itemtypes.tsv"),
        "skills" => include_str!("../core/skills.tsv"),
        "experience" => include_str!("../core/experience.tsv"),
        "belts" => include_str!("../core/belts.tsv"),
        "hireling" => include_str!("../core/hireling.tsv"),
        _ => "",
    })
}
