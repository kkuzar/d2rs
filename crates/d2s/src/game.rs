//! Embedded game knowledge for the UI and validation: classes, skills, experience, quests, waypoints.
//! Quest facts: bit s of word q is bit s of the little-endian u16 at block offset 2q (d2s findings 0002).
use crate::Save;
use std::sync::OnceLock;

pub const CLASSES: [&str; 7] = ["Amazon", "Sorceress", "Necromancer", "Paladin", "Barbarian", "Druid", "Assassin"];
const CLASS_CODES: [&str; 7] = ["ama", "sor", "nec", "pal", "bar", "dru", "ass"];

/// The 30 skills of a class in the order the `if` section stores them: (name, required level).
pub fn class_skills(class: usize) -> Vec<(&'static str, u32)> {
    let code = CLASS_CODES.get(class).copied().unwrap_or("-");
    let rows = include_str!("../data/skills.tsv").lines().skip(1).map(|l| l.split('\t').collect::<Vec<_>>());
    rows.filter(|c| c.get(2) == Some(&code)).map(|c| (c[0], c.get(3).and_then(|v| v.parse().ok()).unwrap_or(1))).collect()
}

fn exp_table() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        let rows = include_str!("../data/experience.tsv").lines().skip(2);
        // Row n holds the minimum experience of level n + 1 (row 0 = level 1 = 0).
        rows.filter_map(|l| l.split('\t').nth(1)?.trim().parse().ok()).collect()
    })
}

/// Minimum experience of `level` (1..=99).
pub fn exp_for_level(level: u32) -> u32 {
    exp_table().get((level.clamp(1, 99) - 1) as usize).copied().unwrap_or(0)
}

/// The level (1..=99) an experience total corresponds to.
pub fn level_for_exp(exp: u32) -> u32 {
    exp_table().iter().take(99).take_while(|&&t| t <= exp).count().max(1) as u32
}

/// (act 1..=5, name, quest word) for every quest whose bit 0 means "completed".
pub const QUESTS: [(u8, &str, usize); 25] = [
    (1, "Den of Evil", 1), (1, "Sisters' Burial Grounds", 2), (1, "Tools of the Trade", 3),
    (1, "The Search for Cain", 4), (1, "The Forgotten Tower", 5), (1, "Sisters to the Slaughter", 6),
    (2, "Radament's Lair", 9), (2, "The Horadric Staff", 10), (2, "Tainted Sun", 11),
    (2, "Arcane Sanctuary", 12), (2, "The Summoner", 13), (2, "The Seven Tombs", 14),
    (3, "Lam Esen's Tome", 17), (3, "Khalim's Will", 18), (3, "Blade of the Old Religion", 19),
    (3, "The Golden Bird", 20), (3, "The Blackened Temple", 21), (3, "The Guardian", 22),
    (4, "The Fallen Angel", 25), (4, "Terror's End", 26), (4, "Hell's Forge", 27),
    (5, "Siege on Harrogath", 35), (5, "Rescue on Mount Arreat", 36), (5, "Prison of Ice", 37),
    (5, "Betrayal of Harrogath", 38),
];

const QUEST_BASE: usize = 10; // "Woo!" + u32 6 + u16 size
const WP_BASE: usize = 298 + 8; // after the quest section: "WS" + u32 1 + u16 0x50

fn word_at(diff: usize, word: usize) -> usize {
    QUEST_BASE + 96 * diff + 2 * word
}

/// Quest word `word` of difficulty `diff` (0..=2); 0 when out of range.
pub fn quest_word(s: &Save, diff: usize, word: usize) -> u16 {
    let o = word_at(diff.min(2), word.min(47));
    s.blocks.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
}

/// Sets or clears the bits of `mask` in one quest word; every other bit and word stays as it was.
pub fn set_quest_bits(s: &mut Save, diff: usize, word: usize, mask: u16, on: bool) {
    let (w, o) = (quest_word(s, diff, word), word_at(diff.min(2), word.min(47)));
    if let Some(b) = s.blocks.get_mut(o..o + 2) {
        b.copy_from_slice(&(if on { w | mask } else { w & !mask }).to_le_bytes());
    }
}

/// Bit 0 of the quest's word: completed.
pub fn quest_done(s: &Save, diff: usize, q: usize) -> bool {
    QUESTS.get(q).is_some_and(|q| quest_word(s, diff, q.2) & 1 == 1)
}

/// Touches only bit 0 of the quest's word (never bit 1, the reward-pending bit, or other words).
pub fn set_quest_done(s: &mut Save, diff: usize, q: usize, on: bool) {
    if let Some(q) = QUESTS.get(q) {
        set_quest_bits(s, diff, q.2, 1, on);
    }
}

/// Pending-reward states that a blind "complete" would destroy: (name, word, bits, bit values while pending).
/// Charsi's imbue (word 3 bit 1, bit 0 clear), Larzuk's socket (word 35 bits 1 and 3, bit 0 clear),
/// Cow King credit (word 4 bit 10 clear = cow level still open), Anya's personalisation (word 38 bit 1, bit 0 clear).
pub const PENDING: [(&str, usize, u16, u16); 4] = [
    ("Charsi imbue", 3, 0b11, 0b10),
    ("Larzuk socket", 35, 0b1011, 0b1010),
    ("Cow King (cow level open)", 4, 1 << 10, 0),
    ("Anya personalisation", 38, 0b11, 0b10),
];

pub fn is_pending(s: &Save, diff: usize, i: usize) -> bool {
    PENDING.get(i).is_some_and(|p| quest_word(s, diff, p.1) & p.2 == p.3)
}

/// Pending: the game's pending bit pattern. Not pending: the pattern's complement (the game's "done" state).
pub fn set_pending(s: &mut Save, diff: usize, i: usize, on: bool) {
    if let Some(&(_, w, bits, want)) = PENDING.get(i) {
        let v = if on { want } else { bits ^ want };
        set_quest_bits(s, diff, w, bits & !v, false);
        set_quest_bits(s, diff, w, v, true);
    }
}

pub const WAYPOINTS: [&str; 39] = [
    "Rogue Encampment", "Cold Plains", "Stony Field", "Dark Wood", "Black Marsh", "Outer Cloister", "Jail Level 1",
    "Inner Cloister", "Catacombs Level 2", "Lut Gholein", "Sewers Level 2", "Dry Hills", "Halls of the Dead Level 2",
    "Far Oasis", "Lost City", "Palace Cellar Level 1", "Arcane Sanctuary", "Canyon of the Magi", "Kurast Docktown",
    "Spider Forest", "Great Marsh", "Flayer Jungle", "Lower Kurast", "Kurast Bazaar", "Upper Kurast", "Travincal",
    "Durance of Hate Level 2", "The Pandemonium Fortress", "City of the Damned", "River of Flame", "Harrogath",
    "Rigid Highlands", "Arreat Plateau", "Crystalized Cavern Level 1", "Crystalized Cavern Level 2",
    "Halls of Death's Calling", "Tundra Wastelands", "Glacial Caves Level 1", "Worldstone Keep Level 2",
];
/// First waypoint index of each act (act 1..=5), plus the end.
pub const ACT_WP: [usize; 6] = [0, 9, 18, 27, 30, 39];

fn wp_at(diff: usize, i: usize) -> (usize, u8) {
    (WP_BASE + 24 * diff.min(2) + 2 + i / 8, 1 << (i % 8))
}

pub fn waypoint(s: &Save, diff: usize, i: usize) -> bool {
    let (o, m) = wp_at(diff, i);
    i < WAYPOINTS.len() && s.blocks.get(o).is_some_and(|b| b & m != 0)
}

/// Waypoint 0 (Rogue Encampment) cannot be switched off: the game forces it on.
pub fn set_waypoint(s: &mut Save, diff: usize, i: usize, on: bool) {
    let (o, m) = wp_at(diff, i);
    if let (true, Some(b)) = (i < WAYPOINTS.len() && (i > 0 || on), s.blocks.get_mut(o)) {
        *b = if on { *b | m } else { *b & !m };
    }
}

/// Opens the act tab of act 2..=5 (bit 0 of word 7, 15, 23; 26 and 28 for act 5) and every waypoint of
/// acts up to it. Act 5 also needs the expansion bit and progression; word 26 may mark the Act 4 boss dead.
pub fn unlock_act(s: &mut Save, diff: usize, act: usize) {
    for w in match act { 2 => &[7][..], 3 => &[15], 4 => &[23], 5 => &[26, 28], _ => &[] } {
        set_quest_bits(s, diff, *w, 1, true);
    }
    for i in ACT_WP[act.clamp(1, 5) - 1]..ACT_WP[act.clamp(1, 5)] {
        set_waypoint(s, diff, i, true);
    }
}
