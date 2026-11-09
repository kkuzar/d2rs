//! LSB-first bit reader/writer (the .d2s bit order, F§4).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub bit: usize,
    pub msg: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} at byte {} bit {}", self.msg, self.bit / 8, self.bit % 8)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err<T>(bit: usize, msg: impl Into<String>) -> Result<T> {
    Err(Error { bit, msg: msg.into() })
}

pub struct BitReader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8], byte: usize) -> Self {
        Self { buf, pos: byte * 8 }
    }

    /// Reads `n` (0..=32) bits, least significant first.
    pub fn read(&mut self, n: u32) -> Result<u32> {
        if self.pos + n as usize > self.buf.len() * 8 {
            return err(self.pos, format!("unexpected end of data reading {n} bits"));
        }
        let mut v = 0u64;
        for i in 0..n as usize {
            let p = self.pos + i;
            v |= (((self.buf[p / 8] >> (p % 8)) & 1) as u64) << i;
        }
        self.pos += n as usize;
        Ok(v as u32)
    }

    pub fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }

    /// Skips to the next byte boundary; the skipped bits must be zero.
    pub fn align(&mut self) -> Result<()> {
        let pad = (8 - self.pos % 8) % 8;
        let at = self.pos;
        if self.read(pad as u32)? != 0 {
            return err(at, "non-zero padding bits");
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct BitWriter {
    pub buf: Vec<u8>,
    pub pos: usize,
}

impl BitWriter {
    /// Writes the low `n` bits of `v`; refuses values that do not fit (never clamps).
    pub fn write(&mut self, v: u32, n: u32) -> Result<()> {
        if n < 32 && v >> n != 0 {
            return err(self.pos, format!("value {v} does not fit in {n} bits"));
        }
        for i in 0..n as usize {
            if self.pos.is_multiple_of(8) {
                self.buf.push(0);
            }
            self.buf[self.pos / 8] |= (((v >> i) & 1) as u8) << (self.pos % 8);
            self.pos += 1;
        }
        Ok(())
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.pos = self.buf.len() * 8;
        self.buf.extend_from_slice(b);
        self.pos += b.len() * 8;
    }

    pub fn align(&mut self) {
        self.pos = self.buf.len() * 8;
    }
}
