//! Game 1.14d linear congruential RNG, shared by item and map code.

const MULT: u64 = 0x6AC6_90C5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seed {
    pub low: u32,
    pub high: u32,
}

impl Seed {
    pub fn init(v: u32) -> Seed {
        Seed { low: v, high: 666 }
    }

    pub fn step(&mut self) -> u32 {
        let t = u64::from(self.low) * MULT + u64::from(self.high);
        self.low = t as u32;
        self.high = (t >> 32) as u32;
        self.low
    }

    /// `n < 1` returns 0 without drawing.
    pub fn range(&mut self, n: i32) -> u32 {
        if n < 1 {
            return 0;
        }
        self.step() % n as u32
    }

    pub fn range_pow2(&mut self, n: u32) -> u32 {
        self.step() & n.wrapping_sub(1)
    }
}
