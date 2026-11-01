//! rav2d's symbol decoder (`crates/rav2d/src/msac.rs` and the golomb helpers
//! of `recon.rs` at aab9d2d), copied unchanged except for imports. rav2d keeps
//! it crate-private, so the range-coder test carries its own copy as the
//! reference reader.

#![allow(dead_code, clippy::needless_range_loop, clippy::unnecessary_cast)]

use rav2e::shared::msac_tables::{MSAC_MIN_PROB, MSAC_RATE};

fn clz(v: u32) -> u32 {
    v.leading_zeros()
}

pub struct MsacContext<'a> {
    buf_pos: usize,
    buf: &'a [u8],
    dif: u64,
    rng: u32,
    cnt: i32,
    allow_update_cdf: bool,
}

impl<'a> MsacContext<'a> {
    pub fn new(data: &'a [u8], disable_cdf_update_flag: bool) -> Self {
        let mut s = Self {
            buf_pos: 0,
            buf: data,
            dif: !0u64 >> 1,
            rng: 0x8000,
            cnt: -15,
            allow_update_cdf: !disable_cdf_update_flag,
        };
        s.ctx_refill();
        s
    }

    #[inline]
    pub fn dbg_rng(&self) -> u32 {
        self.rng
    }

    #[inline]
    pub fn dbg_dif(&self) -> u64 {
        self.dif
    }

    #[inline]
    fn ctx_refill(&mut self) {
        let mut c = 40 - self.cnt;
        let mut dif = self.dif;
        loop {
            if self.buf_pos >= self.buf.len() {
                break;
            }
            dif ^= (self.buf[self.buf_pos] as u64) << c;
            self.buf_pos += 1;
            c -= 8;
            if c < 0 {
                break;
            }
        }
        self.dif = dif;
        self.cnt = 40 - c;
    }

    #[inline]
    fn ctx_norm(&mut self, dif: u64, rng: u32) {
        let d = 15 ^ (31 ^ clz(rng));
        let cnt = self.cnt;
        debug_assert!(rng <= 65535);
        self.dif = ((dif + 1) << d) - 1;
        self.rng = rng << d;
        self.cnt = cnt - d as i32;
        if (cnt as u32) < d {
            self.ctx_refill();
        }
    }

    pub fn decode_bools_bypass(&mut self, n_bits: u32) -> u32 {
        debug_assert!(n_bits > 0 && n_bits <= 32);
        if (self.cnt as u32) < n_bits {
            self.ctx_refill();
        }

        let r = self.rng as u64;
        let mut dif = self.dif;
        debug_assert!(r & 1 == 0);
        debug_assert!((dif >> 48) < r);
        let mut vw = r << 47;
        let mut ret: u32 = 0;
        for _ in 0..n_bits {
            ret <<= 1;
            if dif >= vw {
                dif -= vw;
            } else {
                ret |= 1;
            }
            vw >>= 1;
        }
        self.dif = ((dif + 1) << n_bits) - 1;
        self.cnt -= n_bits as i32;
        ret
    }

    #[inline]
    pub fn decode_bool_bypass(&mut self) -> u32 {
        self.decode_bools_bypass(1)
    }

    pub fn decode_unary_bypass(&mut self, max_bits: u32) -> u32 {
        debug_assert!(max_bits == 5 || max_bits == 6 || max_bits == 21);
        if (self.cnt as u32) < max_bits {
            self.ctx_refill();
        }

        let r = self.rng as u64;
        let mut dif = self.dif;
        debug_assert!(r & 1 == 0);
        debug_assert!((dif >> 48) < r);
        let mut vw = r << 47;
        let mut ret: u32 = 0;
        let mut bit: u32 = 0;
        while bit < max_bits {
            if dif >= vw {
                dif -= vw;
                vw >>= 1;
                ret += 1;
                bit += 1;
            } else {
                bit += 1;
                break;
            }
        }
        self.dif = ((dif + 1) << bit) - 1;
        self.cnt -= bit as i32;
        ret
    }

    fn decode_bool_raw(&mut self, f: u32) -> u32 {
        let r = self.rng;
        let dif = self.dif;
        debug_assert!((dif >> 48) < r as u64);
        let p = ((f >> 7) << 4) + 8;
        let mut v = (((r >> 8) * p) >> 7) << 3;
        let vw = (v as u64) << 48;
        let ret = if dif >= vw { 1 } else { 0 };
        let new_dif = dif - ret as u64 * vw;
        if ret != 0 {
            v = r - v;
        }
        self.ctx_norm(new_dif, v);
        (ret == 0) as u32
    }

    pub fn decode_symbol_adapt(&mut self, cdf: &mut [u16], n_symbols: usize) -> u32 {
        let c = (self.dif >> 48) as u32;
        let r = self.rng >> 8;
        let mut u: u32;
        let mut v = self.rng;
        let mut val: u32 = 0;
        let min_prob = &MSAC_MIN_PROB[n_symbols - 1];

        debug_assert!(n_symbols <= 7);

        loop {
            u = v;
            let p_raw = (cdf[val as usize] | 127) as i32 - min_prob[val as usize] as i32;
            let p = p_raw.max(0) as u32;
            v = ((r * p) >> 10) << 3;
            if c >= v {
                break;
            }
            val += 1;
        }

        debug_assert!(u <= self.rng);

        self.ctx_norm(self.dif - ((v as u64) << 48), u - v);

        if self.allow_update_cdf {
            let pc = cdf[n_symbols];
            let count = (pc & 0xFF) as u8;
            debug_assert!(count <= 32);
            let rate = MSAC_RATE[(pc >> 8) as usize][(count >> 4) as usize]
                + if n_symbols > 2 { 1 } else { 0 };
            for i in 0..val as usize {
                cdf[i] += (32768 - cdf[i]) >> rate;
            }
            for i in val as usize..n_symbols {
                cdf[i] -= cdf[i] >> rate;
            }
            cdf[n_symbols] = pc + if count < 32 { 1 } else { 0 };
        }

        val
    }

    pub fn decode_bool_adapt(&mut self, cdf: &mut [u16]) -> u32 {
        let bit = self.decode_bool_raw(cdf[0] as u32);

        if self.allow_update_cdf {
            let pc = cdf[1];
            let count = (pc & 0xFF) as u8;
            let rate = MSAC_RATE[(pc >> 8) as usize][(count >> 4) as usize];
            if bit != 0 {
                cdf[0] += (32768 - cdf[0]) >> rate;
            } else {
                cdf[0] -= cdf[0] >> rate;
            }
            cdf[1] = pc + if count < 32 { 1 } else { 0 };
        }

        bit
    }

    pub fn decode_uniform(&mut self, n: u32) -> u32 {
        debug_assert!(n > 0);
        let l = 31 - n.leading_zeros() + 1;
        debug_assert!(l > 1);
        let m = (1u32 << l) - n;
        let v = self.decode_bools_bypass((l - 1) as u32);
        if v < m {
            v
        } else {
            (v << 1) - m + self.decode_bool_bypass()
        }
    }

    /// Current internal bit count. Used to detect symbol-decoder overread
    /// (`cnt <= -15` after decoding a tile superblock row).
    pub fn cnt(&self) -> i32 {
        self.cnt
    }

    pub fn check_trailing_bits(&self) -> bool {
        let n_bits = -(self.cnt + 14);
        debug_assert!(n_bits <= 0);
        let n_bytes = (n_bits + 7) >> 3;
        let p = (self.buf_pos as i32 + n_bytes) as usize;
        let pattern: i32 = 128 >> ((n_bits - 1) & 7);
        let mask = (2 * pattern - 1) as u8;
        if (self.buf[p - 1] & mask) != pattern as u8 {
            return true;
        }
        for i in p..self.buf.len() {
            if self.buf[i] != 0 {
                return true;
            }
        }
        false
    }
}

pub fn decode_exp_golomb(msac: &mut MsacContext, k: u32) -> u32 {
    let length = msac.decode_unary_bypass(21) + k;
    let x = (1u32 << length) + msac.decode_bools_bypass(length);
    x - (1 << k)
}

pub fn decode_hr(msac: &mut MsacContext, hr_avg: i32) -> i32 {
    let m = 31 - (hr_avg.clamp(2, 64) as u32).leading_zeros();
    let cmax = (m + 4).min(6);
    let q = msac.decode_unary_bypass(cmax);
    let rem = if q == cmax {
        decode_exp_golomb(msac, m + 1)
    } else {
        msac.decode_bools_bypass(m)
    };
    (rem + (q << m)) as i32
}
