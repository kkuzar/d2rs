//! Seed tree: map seed to act, level and room seeds (specs/rng/seeds.md).
use crate::Seed;

pub struct SeedTree {
    pub map_seed: u32,
}

/// Vanilla act (1..=5) of a level id; phase 3 replaces this with the Levels table.
pub const fn act_of(level_id: u32) -> u8 {
    match level_id {
        0..=39 => 1,
        40..=74 => 2,
        75..=102 => 3,
        103..=108 => 4,
        _ => 5,
    }
}

impl SeedTree {
    /// Seeds step 3a-3c: `act` is the 0-based act index; the returned seed is after the act's own draws.
    pub fn act_seed(&self, act: u8) -> Seed {
        let mut s = Seed::init(self.map_seed);
        s.step();
        match act {
            1 => drop(tomb_pair(&mut s)),
            2 => drop(s.step()),
            _ => {}
        }
        s
    }

    /// Seeds step 3b: the first act draw, the same for every act.
    pub fn level_seed_base(&self, _act: u8) -> u32 {
        let mut s = Seed::init(self.map_seed);
        s.step()
    }

    /// Seeds step 4: `Init(base + id)`, a wrapping sum.
    pub fn level_seed(&self, act: u8, level_id: u32) -> Seed {
        Seed::init(self.level_seed_base(act).wrapping_add(level_id))
    }

    /// Seeds step 3c: Act II true tomb and Kaa's tomb level ids (66..=72).
    pub fn tombs(&self) -> (u32, u32) {
        let mut s = Seed::init(self.map_seed);
        s.step();
        tomb_pair(&mut s)
    }

    /// Seeds step 3c: Act III bit, one draw after the base.
    pub fn act3_bit(&self) -> bool {
        self.act_seed(2).low & 1 == 1
    }
}

/// Seeds step 5: one level draw, then `Init(level.low)` and one room draw.
pub fn room_seed(level: &mut Seed) -> Seed {
    level.step();
    let mut r = Seed::init(level.low);
    r.step();
    r
}

/// Seeds step 3c: pairs of draws mod 7 until they differ.
fn tomb_pair(s: &mut Seed) -> (u32, u32) {
    loop {
        let (a, b) = (s.step() % 7, s.step() % 7);
        if a != b {
            return (a + 66, b + 66);
        }
    }
}
