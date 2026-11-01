//! Whatever the range encoder writes, rav2d's symbol decoder must read back
//! as the same symbols, with CDF adaptation on and off.

mod support;

use rav2e::ec::{RangeEncoder, SymbolWriter};
use rav2e::shared::cdf::CdfContext;
use support::msac::{MsacContext, decode_exp_golomb, decode_hr};

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Bool(bool),
    Symbol3(u32),
    Symbol7(u32),
    Symbol7Skewed(u32),
    Bypass(u32, u32),
    Unary(u32, u32),
    Hr(u32, i32),
    ExpGolomb(u32, u32),
}

fn random_ops(rng: &mut Rng, n: usize) -> Vec<Op> {
    (0..n)
        .map(|_| match rng.below(8) {
            0 => Op::Bool(rng.below(5) == 0),
            1 => Op::Symbol3(rng.below(4) as u32),
            2 => Op::Symbol7(rng.below(8) as u32),
            // Mostly the first symbol, the way real streams look.
            3 => Op::Symbol7Skewed(if rng.below(10) < 8 {
                0
            } else {
                rng.below(8) as u32
            }),
            4 => {
                let bits = 1 + rng.below(24) as u32;
                Op::Bypass(rng.next() as u32 & ((1u32 << bits) - 1), bits)
            }
            5 => {
                let max = [5u32, 6, 21][rng.below(3) as usize];
                Op::Unary(rng.below(max as u64 + 1) as u32, max)
            }
            6 => Op::Hr(rng.below(3000) as u32, rng.below(80) as i32),
            _ => Op::ExpGolomb(rng.below(100_000) as u32, rng.below(4) as u32),
        })
        .collect()
}

fn roundtrip(seed: u64, disable_cdf_update: bool) {
    let mut rng = Rng(seed);
    let ops = random_ops(&mut rng, 4000);

    let mut enc_cdf = CdfContext::init_from_defaults(1, false);
    let mut enc = RangeEncoder::new(disable_cdf_update);
    for &op in &ops {
        match op {
            Op::Bool(b) => enc.encode_bool_adapt(b, enc_cdf.m.part_split(0, 3)),
            Op::Symbol3(v) => enc.encode_symbol_adapt(v, enc_cdf.m.intra_y_set(), 3),
            Op::Symbol7(v) => enc.encode_symbol_adapt(v, enc_cdf.m.intra_uv_mode(1), 7),
            Op::Symbol7Skewed(v) => enc.encode_symbol_adapt(v, enc_cdf.coef.eob_bin_1024(0), 7),
            Op::Bypass(v, n) => enc.encode_bools_bypass(v, n),
            Op::Unary(q, max) => enc.encode_unary_bypass(q, max),
            Op::Hr(v, avg) => enc.encode_hr(v, avg),
            Op::ExpGolomb(v, k) => enc.encode_exp_golomb(v, k),
        }
    }
    let bytes = enc.finish();

    let mut dec_cdf = CdfContext::init_from_defaults(1, false);
    let mut dec = MsacContext::new(&bytes, disable_cdf_update);
    for (i, &op) in ops.iter().enumerate() {
        let ok = match op {
            Op::Bool(b) => dec.decode_bool_adapt(dec_cdf.m.part_split(0, 3)) == b as u32,
            Op::Symbol3(v) => dec.decode_symbol_adapt(dec_cdf.m.intra_y_set(), 3) == v,
            Op::Symbol7(v) => dec.decode_symbol_adapt(dec_cdf.m.intra_uv_mode(1), 7) == v,
            Op::Symbol7Skewed(v) => dec.decode_symbol_adapt(dec_cdf.coef.eob_bin_1024(0), 7) == v,
            Op::Bypass(v, n) => dec.decode_bools_bypass(n) == v,
            Op::Unary(q, max) => dec.decode_unary_bypass(max) == q,
            Op::Hr(v, avg) => decode_hr(&mut dec, avg) == v as i32,
            Op::ExpGolomb(v, k) => decode_exp_golomb(&mut dec, k) == v,
        };
        assert!(ok, "seed {seed}: op {i} ({op:?}) read back differently");
    }
}

#[test]
fn adaptive_cdfs_read_back() {
    for seed in 1u64..=8 {
        roundtrip(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), false);
    }
}

#[test]
fn frozen_cdfs_read_back() {
    for seed in 1u64..=8 {
        roundtrip(seed.wrapping_mul(0x0123_4567_89ab_cdef), true);
    }
}
