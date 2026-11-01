//! MSB-first bit writer for OBU headers: the mirror of rav2d's `GetBits`.

#[derive(Default)]
pub struct BitWriter {
    buf: Vec<u8>,
    /// Bits used in the last byte of `buf` (0 when byte-aligned).
    bit: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_bit(&mut self, b: bool) {
        if self.bit == 0 {
            self.buf.push(0);
        }
        if b {
            *self.buf.last_mut().unwrap() |= 0x80 >> self.bit;
        }
        self.bit = (self.bit + 1) & 7;
    }

    /// Writes the `n` low bits of `v`, most significant first (`get_bits`).
    pub fn put_bits(&mut self, v: u32, n: u32) {
        debug_assert!(n <= 32);
        debug_assert!(n == 32 || v >> n == 0, "{v} does not fit in {n} bits");
        for i in (0..n).rev() {
            self.put_bit((v >> i) & 1 != 0);
        }
    }

    /// Exp-Golomb code read by `get_vlc`.
    pub fn put_vlc(&mut self, v: u32) {
        if v == 0 {
            self.put_bit(true);
            return;
        }
        // Smallest n with v < 2^(n+1) - 1.
        let n = 32 - (v + 1).leading_zeros() - 1;
        for _ in 0..n {
            self.put_bit(false);
        }
        self.put_bit(true);
        self.put_bits(v - ((1 << n) - 1), n);
    }

    /// Value in `0..max` as read by `get_uniform(max)`.
    pub fn put_uniform(&mut self, v: u32, max: u32) {
        debug_assert!(v < max);
        if max <= 1 {
            return;
        }
        let l = 32 - max.leading_zeros();
        let m = (1 << l) - max;
        if v < m {
            self.put_bits(v, l - 1);
        } else {
            self.put_bits((v + m) >> 1, l - 1);
            self.put_bit((v + m) & 1 != 0);
        }
    }

    /// Pads with zero bits to the next byte boundary.
    pub fn byte_align(&mut self) {
        self.bit = 0;
    }

    /// A one bit, then zero bits to the byte boundary.
    pub fn trailing_bits(&mut self) {
        self.put_bit(true);
        self.byte_align();
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

pub fn put_uleb128(out: &mut Vec<u8>, mut v: usize) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rav2d::getbits::GetBits;

    #[test]
    fn vlc_and_uniform_read_back() {
        let mut w = BitWriter::new();
        let vlcs = [0u32, 1, 2, 3, 6, 7, 100, 65535];
        let uniforms = [(0u32, 3u32), (2, 3), (4, 5), (0, 2), (9, 10), (17, 31)];
        for &v in &vlcs {
            w.put_vlc(v);
        }
        for &(v, max) in &uniforms {
            w.put_uniform(v, max);
        }
        w.put_bits(0x5a, 7);
        w.trailing_bits();
        let bytes = w.into_bytes();

        let mut gb = GetBits::new(&bytes);
        for &v in &vlcs {
            assert_eq!(gb.get_vlc(), v);
        }
        for &(v, max) in &uniforms {
            assert_eq!(gb.get_uniform(max), v);
        }
        assert_eq!(gb.get_bits(7), 0x5a);
        assert_eq!(gb.get_bit(), 1);
        assert!(!gb.has_error());
    }
}
