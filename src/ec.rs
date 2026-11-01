//! Range encoder: the mirror of rav2d's `MsacContext`.
//!
//! The byte output and the end-of-stream flush follow AVM's `entenc.c`
//! (`od_ec_enc_normalize`, `avm_od_ec_enc_done`). The split points of each
//! symbol are computed with the same formulas and tables rav2d's decoder uses,
//! so whatever is written here is read back as the same symbol. CDF
//! adaptation is the decoder's, step for step.

use crate::shared::msac_tables::{MSAC_MIN_PROB, MSAC_RATE};

pub struct RangeEncoder {
    /// 16-bit digits before carry propagation (AVM `precarry_buf`).
    precarry: Vec<u16>,
    low: u64,
    rng: u32,
    cnt: i32,
    allow_update_cdf: bool,
}

impl RangeEncoder {
    pub fn new(disable_cdf_update: bool) -> Self {
        Self {
            precarry: Vec::new(),
            low: 0,
            rng: 0x8000,
            // Crosses zero after one byte plus a carry bit (AVM enc_reset).
            cnt: -9,
            allow_update_cdf: !disable_cdf_update,
        }
    }

    fn normalize(&mut self, mut low: u64, rng: u32, n_bypass: i32) {
        debug_assert!(rng <= 65535);
        let (c, d) = if n_bypass > 0 {
            (self.cnt + n_bypass, 0)
        } else {
            (self.cnt, rng.leading_zeros() as i32 - 16)
        };
        let mut s = c + d;
        if s >= 0 {
            let mut c = c + 16;
            let mut m: u64 = (1u64 << c) - 1;
            if s >= 8 {
                self.precarry.push((low >> c) as u16);
                low &= m;
                c -= 8;
                m >>= 8;
            }
            self.precarry.push((low >> c) as u16);
            s = c + d - 24;
            low &= m;
        }
        self.low = low << d;
        self.rng = rng << d;
        self.cnt = s;
    }

    /// Flushes and returns the coded bytes (`avm_od_ec_enc_done`). The last
    /// bits form the "1 then zeros" pattern the decoder's trailing check
    /// expects.
    pub fn finish(mut self) -> Vec<u8> {
        let l = self.low;
        let mut c = self.cnt;
        let mut s = 10 + c;
        let m: u64 = 0x3FFF;
        let mut e = ((l + m) & !m) | (m + 1);
        if s > 0 {
            let mut n: u64 = (1u64 << (c + 16)) - 1;
            loop {
                self.precarry.push((e >> (c + 16)) as u16);
                e &= n;
                s -= 8;
                c -= 8;
                n >>= 8;
                if s <= 0 {
                    break;
                }
            }
        }
        let mut out = vec![0u8; self.precarry.len()];
        let mut carry = 0u32;
        for i in (0..self.precarry.len()).rev() {
            carry += self.precarry[i] as u32;
            out[i] = carry as u8;
            carry >>= 8;
        }
        out
    }
}

impl SymbolWriter for RangeEncoder {
    /// Writes one bit with probability `cdf[0]` and adapts the CDF the way
    /// `decode_bool_adapt` does.
    fn bool_adapt(&mut self, bit: bool, cdf: &mut [u16]) {
        let r = self.rng;
        let f = cdf[0] as u32;
        let p = ((f >> 7) << 4) + 8;
        let v = (((r >> 8) * p) >> 7) << 3;
        // The decoder returns 1 when the code value is below `v`.
        let (low, rng) = if bit {
            (self.low + (r - v) as u64, v)
        } else {
            (self.low, r - v)
        };
        self.normalize(low, rng, 0);

        if self.allow_update_cdf {
            let pc = cdf[1];
            let count = (pc & 0xFF) as u8;
            let rate = MSAC_RATE[(pc >> 8) as usize][(count >> 4) as usize];
            if bit {
                cdf[0] += (32768 - cdf[0]) >> rate;
            } else {
                cdf[0] -= cdf[0] >> rate;
            }
            cdf[1] = pc + if count < 32 { 1 } else { 0 };
        }
    }

    /// Writes `val` from an alphabet of `n_symbols + 1` symbols (rav2d's
    /// convention: `n_symbols` is the largest symbol, and `cdf[n_symbols]` holds
    /// the adaptation counter).
    fn symbol_adapt(&mut self, val: u32, cdf: &mut [u16], n_symbols: usize) {
        debug_assert!(n_symbols <= 7);
        debug_assert!(val as usize <= n_symbols);
        let r8 = self.rng >> 8;
        let min_prob = &MSAC_MIN_PROB[n_symbols - 1];
        let split = |s: usize| -> u32 {
            if s >= n_symbols {
                return 0;
            }
            let p = ((cdf[s] | 127) as i32 - min_prob[s] as i32).max(0) as u32;
            ((r8 * p) >> 10) << 3
        };
        let s = val as usize;
        let u = if s == 0 { self.rng } else { split(s - 1) };
        let v = split(s);
        debug_assert!(u > v);
        let low = self.low + (self.rng - u) as u64;
        self.normalize(low, u - v, 0);

        if self.allow_update_cdf {
            let pc = cdf[n_symbols];
            let count = (pc & 0xFF) as u8;
            let rate = MSAC_RATE[(pc >> 8) as usize][(count >> 4) as usize]
                + if n_symbols > 2 { 1 } else { 0 };
            for c in cdf.iter_mut().take(s) {
                *c += (32768 - *c) >> rate;
            }
            for c in cdf.iter_mut().take(n_symbols).skip(s) {
                *c -= *c >> rate;
            }
            cdf[n_symbols] = pc + if count < 32 { 1 } else { 0 };
        }
    }

    /// Writes the `n_bits` low bits of `val`, most significant first, at equal
    /// probability (`decode_bools_bypass`).
    fn bools_bypass(&mut self, val: u32, n_bits: u32) {
        debug_assert!(n_bits <= 32);
        let mut left = n_bits;
        while left > 0 {
            // AVM writes at most 16 bits at a time; 8 keeps the flush to the
            // two digits normalize() can emit.
            let n = left.min(8);
            left -= n;
            let chunk = (val >> left) & ((1u32 << n) - 1);
            let low = (self.low << n) + self.rng as u64 * chunk as u64;
            self.normalize(low, self.rng, n as i32);
        }
    }
}

/// What the encoder writes symbols to: the range encoder, or a counter that
/// only adds up their cost. The syntax code is written once, against this
/// trait. Method names follow rav2d's `MsacContext` reads.
pub trait SymbolWriter {
    /// One bit with probability `cdf[0]` (`decode_bool_adapt`).
    fn bool_adapt(&mut self, bit: bool, cdf: &mut [u16]);
    /// `val` of `0..=n_symbols` (`decode_symbol_adapt`).
    fn symbol_adapt(&mut self, val: u32, cdf: &mut [u16], n_symbols: usize);
    /// The `n_bits` low bits of `val`, most significant first, at equal
    /// probability (`decode_bools_bypass`).
    fn bools_bypass(&mut self, val: u32, n_bits: u32);

    fn encode_bool_adapt(&mut self, bit: bool, cdf: &mut [u16]) {
        self.bool_adapt(bit, cdf);
    }

    fn encode_symbol_adapt(&mut self, val: u32, cdf: &mut [u16], n_symbols: usize) {
        self.symbol_adapt(val, cdf, n_symbols);
    }

    fn encode_bools_bypass(&mut self, val: u32, n_bits: u32) {
        self.bools_bypass(val, n_bits);
    }

    fn encode_bool_bypass(&mut self, bit: bool) {
        self.bools_bypass(bit as u32, 1);
    }

    /// `decode_unary_bypass(max_bits)`: `q` zero bits, then a one unless `q`
    /// reached `max_bits`.
    fn encode_unary_bypass(&mut self, q: u32, max_bits: u32) {
        debug_assert!(q <= max_bits);
        for _ in 0..q {
            self.encode_bool_bypass(false);
        }
        if q < max_bits {
            self.encode_bool_bypass(true);
        }
    }

    /// `decode_exp_golomb(k)`.
    fn encode_exp_golomb(&mut self, v: u32, k: u32) {
        let x = v + (1 << k);
        let length = 31 - x.leading_zeros();
        self.encode_unary_bypass(length - k, 21);
        self.encode_bools_bypass(x - (1 << length), length);
    }

    /// `decode_hr(hr_avg)`: the remainder of a coefficient above the coded
    /// base range.
    fn encode_hr(&mut self, v: u32, hr_avg: i32) {
        let m = 31 - (hr_avg.clamp(2, 64) as u32).leading_zeros();
        let cmax = (m + 4).min(6);
        let q = v >> m;
        if q < cmax {
            self.encode_unary_bypass(q, cmax);
            self.encode_bools_bypass(v & ((1 << m) - 1), m);
        } else {
            self.encode_unary_bypass(cmax, cmax);
            self.encode_exp_golomb(v - (cmax << m), m + 1);
        }
    }
}

/// Adds up the cost in bits of what would be written, from the CDFs as they
/// stand. Exact for frozen CDFs (`disable_cdf_update`), which is what rav2e
/// codes today; it never adapts them.
#[derive(Default)]
pub struct BitCounter {
    pub bits: f64,
}

impl SymbolWriter for BitCounter {
    fn bool_adapt(&mut self, bit: bool, cdf: &mut [u16]) {
        // The decoder reads 1 below the split, which sits at cdf[0] / 32768.
        let p1 = (cdf[0] as f64 / 32768.0).clamp(1e-4, 1.0 - 1e-4);
        self.bits -= if bit { p1 } else { 1.0 - p1 }.log2();
    }

    fn symbol_adapt(&mut self, val: u32, cdf: &mut [u16], n_symbols: usize) {
        let s = val as usize;
        let hi = if s == 0 { 32768.0 } else { cdf[s - 1] as f64 };
        let lo = if s >= n_symbols { 0.0 } else { cdf[s] as f64 };
        self.bits -= ((hi - lo).max(1.0) / 32768.0).log2();
    }

    fn bools_bypass(&mut self, _val: u32, n_bits: u32) {
        self.bits += n_bits as f64;
    }
}
