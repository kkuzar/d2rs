//! Validation: the 1.14d load checks in game order and the editor hazard rules (plan 3.6, F§5, F§7).
use crate::game::level_for_exp;
use crate::save::{DIED, EXPANSION, HARDCORE};
use crate::{Save, Schema};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub level: Level,
    pub msg: String,
}

fn u16at(b: &[u8], o: usize) -> usize {
    b.get(o..o + 2).map_or(usize::MAX, |x| u16::from_le_bytes([x[0], x[1]]).into())
}

/// All findings for `save`; `file_stem` is the save's file name without `.d2s` (it must equal the header name).
pub fn validate(save: &Save, sch: &Schema, file_stem: &str) -> Vec<Finding> {
    let mut out = vec![];
    let mut add = |level, msg: String| out.push(Finding { level, msg });
    let (e, w) = (Level::Error, Level::Warn);
    let legacy = save.ver < 92;
    if !(71..=96).contains(&save.ver) {
        add(e, format!("version {} is not supported", save.ver));
    } else if save.ver < 96 {
        add(w, format!("legacy version {}: experimental editing", save.ver));
    }
    let name = save.name();
    if !name.eq_ignore_ascii_case(file_stem) {
        add(e, format!("name {name:?} differs from the file name {file_stem:?} (the game refuses it)"));
    }
    let seps = name.chars().filter(|c| matches!(c, '-' | '_')).count();
    let ok = name.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, '-' | '_'));
    if !(2..=15).contains(&name.len()) || !ok || seps > 1 || name.starts_with(['-', '_']) || name.ends_with(['-', '_']) {
        add(e, format!("name {name:?} breaks the rules: 2..15 letters, at most one - or _, not first or last"));
    }
    if save.status(HARDCORE) && save.status(DIED) {
        add(e, "hardcore character marked dead is refused".into());
    }
    if !legacy && save.status(EXPANSION) != save.jf {
        add(e, "expansion status bit and the jf/kf trailer disagree".into());
    }
    if !legacy && save.jf && save.kf.is_none() {
        add(e, "jf section without the kf (golem) section: the game refuses this file".into());
    }
    if save.class() > 6 {
        add(e, format!("class {} is invalid (7 crashes the game, more is refused)", save.class()));
    }
    for d in 0..3 {
        let m = u16at(&save.blocks, 298 + 8 + 24 * d);
        if ![0, 0x101, 0x102].contains(&m) {
            add(e, format!("waypoint marker of difficulty {d} is {m:#x}"));
        }
    }
    for s in &save.stats {
        if sch.stat(s.0).is_none_or(|d| d.csv_bits == 0) {
            add(e, format!("stat {} cannot be stored", s.0));
        }
    }
    if save.corpses.len() > 1 {
        add(e, "more than one corpse".into());
    }
    match save.to_bytes(sch) {
        Ok(b) if b.len() > 8192 => add(e, format!("file is {} bytes, the game loads at most 8192", b.len())),
        Ok(_) => {}
        Err(x) => add(e, format!("cannot encode: {} (bit {})", x.msg, x.bit)),
    }
    let level = save.get_stat(12).unwrap_or(save.level().into());
    let lv = |id| crate::limits::char_stat(sch, id, level);
    let hdr = save.get_stat(12).is_none().then_some((12, level));
    for (id, v) in hdr.into_iter().chain(save.stats.iter().map(|s| (s.0, s.2))) {
        let v = if (6..=11).contains(&id) { v >> 8 } else { v };
        if let (Some(r), Some(d)) = (lv(id), sch.stat(id)) {
            // Gold and stash gold above their maximum are reset to 0 by the game (save/d2s.md D5).
            crate::limits::check(&d.name, v, r).unwrap_or_else(|m| add(e, m));
        }
    }
    if let Some(x) = save.get_stat(13).filter(|&x| (1..=99).contains(&level) && level_for_exp(x as u32) as i64 != level) {
        add(w, format!("experience {x} does not match level {level}"));
    }
    for (i, &b) in save.skills.iter().skip(2).enumerate() {
        crate::limits::check(format!("skill {i} level"), b.into(), crate::limits::skill_level()).unwrap_or_else(|m| add(e, m));
    }
    for it in save.merc.iter().flatten().filter(|i| i.view.location == 1) {
        if !crate::edit::merc_slot_ok(sch, save.merc_type() as u32, it.view.code, it.view.body) {
            add(w, format!("merc item {} in slot {} breaks the hireling's equipment rules", String::from_utf8_lossy(&it.view.code).trim(), it.view.body));
        }
    }
    let mut used = HashSet::new();
    for it in &save.items {
        let v = &it.view;
        if v.sv == 71 {
            continue; // raw records: page codes and socket fields are not decoded
        }
        let code = String::from_utf8_lossy(&v.code).into_owned();
        if v.location == 1 && !crate::edit::slot_ok(sch, v.code, v.body) {
            add(e, format!("item {} is equipped in body slot {} which it cannot occupy", code.trim(), v.body));
        }
        if !save.status(EXPANSION) && sch.base(v.code).is_some_and(|b| b.version >= 100) {
            add(e, format!("item {} is expansion-only but the character is classic", code.trim()));
        }
        let Some((cols, rows)) = (if v.location == 0 { crate::edit::grid_size(v.store, save.status(EXPANSION)) } else { None }) else { continue };
        let Some(b) = sch.base(v.code) else { continue };
        if v.x + b.w > cols || v.y + b.h > rows {
            add(w, format!("item {code} at ({},{}) is outside its {cols}x{rows} grid", v.x, v.y));
        }
        for (x, y) in (v.x..v.x + b.w).flat_map(|x| (v.y..v.y + b.h).map(move |y| (x, y))) {
            if !used.insert((v.store, x, y)) {
                add(e, format!("item {code} overlaps another at ({x},{y}) of store {}", v.store));
                break;
            }
        }
    }
    let t = crate::tables::ItemTables::get();
    for it in save.all_items().into_iter().filter(|i| i.view.sv != 71) {
        let errs = crate::legit::load_errors(t, sch, it);
        for m in &errs {
            add(e, format!("item {}: {m}", String::from_utf8_lossy(&it.view.code).trim()));
        }
        for m in crate::legit::check(t, sch, it).into_iter().filter(|m| !errs.contains(m)) {
            add(w, format!("item {}: legit: {m}", String::from_utf8_lossy(&it.view.code).trim()));
        }
        if it.view.sv != 71 && it.view.n_filled > it.view.sockets || it.children.len() != it.view.n_filled as usize {
            add(e, format!("item {} has {} filled sockets of {} with {} children", String::from_utf8_lossy(&it.view.code), it.view.n_filled, it.view.sockets, it.children.len()));
        }
    }
    for (o, n, what) in [(0x38, 16, "hotkey"), (0x78, 4, "mouse skill")].into_iter().filter(|_| !legacy) {
        for i in 0..n {
            let r = u16at(&save.header, o + 4 * i + 2);
            if r > save.items.len() {
                add(e, format!("{what} {i} refers to item {r} but there are {} items", save.items.len()));
            }
        }
    }
    out
}

/// Encodes only when no Error finding exists (plan invariant I10).
pub fn encode_checked(save: &Save, sch: &Schema, file_stem: &str) -> Result<Vec<u8>, Vec<Finding>> {
    let f = validate(save, sch, file_stem);
    if f.iter().any(|x| x.level == Level::Error) {
        return Err(f);
    }
    save.to_bytes(sch).map_err(|x| vec![Finding { level: Level::Error, msg: x.msg }])
}
