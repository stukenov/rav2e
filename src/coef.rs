//! Transform coefficients: forward transform, quantization, the decoder's
//! dequantization, and the coefficient syntax of rav2d's `decode_coefs` for
//! DCT_DCT (transform class 2-D).
//!
//! The forward transform does not have to be bit-exact; only the inverse
//! does. So instead of a hand-scaled forward DCT, each transform size is
//! calibrated against the decoder's own inverse: the basis function of every
//! coefficient is measured by inverse-transforming an impulse, and a residual
//! is projected onto those measured bases. Scaling, the 64-point
//! "reconstruct at 32 and upsample" rule and rounding all come out of the
//! decoder's arithmetic.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::ec::SymbolWriter;
use crate::shared::cdf::CdfContext;
use crate::shared::ctx::{get_dc_sign_ctx, get_lo_ctx, get_skip_ctx};
use crate::shared::itx::inv_txfm_add_dct;
use crate::shared::scan::SCANS;
use crate::shared::tables::{TXFM_DIMENSIONS, TxfmInfo};

/// Context value of a 4x4 unit with no coded coefficients.
pub const NO_COEF_CTX: u8 = 0x40;

/// Coded transform-block size in each dimension (64-point transforms keep 32).
fn coded_dims(t_dim: &TxfmInfo) -> (usize, usize) {
    (
        (4 * t_dim.w as usize).min(32),
        (4 * t_dim.h as usize).min(32),
    )
}

/// Measured basis functions of one transform size: `basis[rc]` is the
/// residual the decoder produces for a coefficient of value `AMPLITUDE` at
/// position `rc` (rav2d layout), together with its squared norm.
struct Basis {
    w: usize,
    h: usize,
    vectors: Vec<Vec<f32>>,
    norms: Vec<f32>,
}

const AMPLITUDE: i32 = 1024;

fn basis(tx: usize) -> &'static Basis {
    static CACHE: OnceLock<Mutex<HashMap<usize, &'static Basis>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(b) = cache.lock().unwrap().get(&tx) {
        return b;
    }
    let t_dim = &TXFM_DIMENSIONS[tx];
    let (w, h) = (4 * t_dim.w as usize, 4 * t_dim.h as usize);
    let (sw, sh) = coded_dims(t_dim);
    let scan = SCANS[tx];
    let mut vectors = vec![Vec::new(); sw * sh];
    let mut norms = vec![0f32; sw * sh];
    for (i, &rc) in scan.iter().enumerate() {
        let rc = rc as usize;
        let mut coeff = vec![0i32; sw * sh];
        coeff[rc] = AMPLITUDE;
        let mut block = vec![128u8; w * h];
        // eob only bounds the columns the inverse visits; the scan index of
        // the impulse is what the decoder would pass.
        inv_txfm_add_dct(&mut block, w, &mut coeff, i as i32, tx);
        let v: Vec<f32> = block.iter().map(|&p| p as f32 - 128.0).collect();
        norms[rc] = v.iter().map(|x| x * x).sum();
        vectors[rc] = v;
    }
    let b: &'static Basis = Box::leak(Box::new(Basis {
        w,
        h,
        vectors,
        norms,
    }));
    cache.lock().unwrap().insert(tx, b);
    b
}

/// Dequantizer parameters for one transform block.
#[derive(Clone, Copy)]
pub struct Quant {
    /// Dequantizer from `dq_lookup`, DC and AC (M1 streams set them equal).
    pub dq_dc: u32,
    pub dq_ac: u32,
}

fn dq_shift(t_dim: &TxfmInfo) -> u32 {
    3 + (t_dim.ctx as i32 - 2).max(0) as u32
}

/// Largest token coded without a Golomb remainder at scan position `i`.
fn max_base_range(i: usize, chroma: bool) -> u32 {
    let hi_to_low = if chroma { 1 } else { 10 };
    if i == 0 || i < hi_to_low {
        if chroma { 5 } else { 8 }
    } else {
        6
    }
}

/// The value `decode_coefs` writes into `cf` for a level at scan position `i`.
fn dequantize(level: i32, i: usize, chroma: bool, t_dim: &TxfmInfo, q: Quant) -> i32 {
    if level == 0 {
        return 0;
    }
    let tok = level.unsigned_abs();
    let negative = level < 0;
    let dq = if i == 0 { q.dq_dc } else { q.dq_ac };
    let shift = dq_shift(t_dim);
    let cf_max = !((!127u32) << 8) as i32;
    let val = if tok >= max_base_range(i, chroma) {
        let v = tok & 0xfffff;
        ((((v.wrapping_mul(dq)) & 0xffffff).wrapping_add(4) >> shift) as i32)
            .min(cf_max + negative as i32)
    } else {
        (tok.wrapping_mul(dq).wrapping_add(4) >> shift) as i32
    };
    if negative { -val } else { val }
}

/// Quantized levels of one transform block, in rav2d's coefficient layout.
pub struct Levels {
    pub tx: usize,
    pub chroma: bool,
    pub q: Vec<i32>,
    /// Scan index of the last non-zero level, or -1 when all are zero.
    pub eob: i32,
}

/// Forward-transforms and quantizes `residual` (`w * h`, row-major).
/// `rounding` is the quantizer's rounding offset (0.5 rounds to nearest;
/// smaller values zero out more small coefficients).
pub fn quantize(residual: &[i32], tx: usize, chroma: bool, q: Quant, rounding: f32) -> Levels {
    let t_dim = &TXFM_DIMENSIONS[tx];
    let b = basis(tx);
    debug_assert_eq!(residual.len(), b.w * b.h);
    let scan = SCANS[tx];
    let shift = dq_shift(t_dim) as f32;
    let mut levels = vec![0i32; b.vectors.len()];
    let mut eob = -1;
    for (i, &rc) in scan.iter().enumerate() {
        let rc = rc as usize;
        if b.norms[rc] == 0.0 {
            continue;
        }
        let dot: f32 = b.vectors[rc]
            .iter()
            .zip(residual)
            .map(|(&v, &r)| v * r as f32)
            .sum();
        // Coefficient value the decoder must see, then the level that
        // dequantizes closest to it.
        let target = dot / b.norms[rc] * AMPLITUDE as f32;
        let dq = if i == 0 { q.dq_dc } else { q.dq_ac } as f32;
        let x = target * (1u32 << shift as u32) as f32 / dq;
        let level = (x.abs() + rounding).floor() as i32 * x.signum() as i32;
        if level != 0 {
            levels[rc] = level;
            eob = i as i32;
        }
    }
    Levels {
        tx,
        chroma,
        q: levels,
        eob,
    }
}

/// Adds the decoder's reconstruction of `levels` to `dst`.
pub fn reconstruct(levels: &Levels, quant: Quant, dst: &mut [u8], stride: usize) {
    if levels.eob < 0 {
        return;
    }
    let t_dim = &TXFM_DIMENSIONS[levels.tx];
    let scan = SCANS[levels.tx];
    let mut cf = vec![0i32; levels.q.len()];
    for (i, &rc) in scan.iter().enumerate().take(levels.eob as usize + 1) {
        let rc = rc as usize;
        cf[rc] = dequantize(levels.q[rc], i, levels.chroma, t_dim, quant);
    }
    inv_txfm_add_dct(dst, stride, &mut cf, levels.eob, levels.tx);
}

/// Where the block sits for the skip context.
pub struct CoefContext<'a> {
    pub plane: usize,
    /// rav2d block size index, for the skip context.
    pub bs: usize,
    pub above: &'a [u8],
    pub left: &'a [u8],
    pub u_has_cf: i32,
}

/// Writes the block's coefficients the way `decode_coefs` reads them and
/// returns the context value the decoder stores for its 4x4 units.
pub fn write_coefs<W: SymbolWriter>(
    ec: &mut W,
    cdf: &mut CdfContext,
    levels: &Levels,
    c: &CoefContext,
) -> u8 {
    let t_dim = &TXFM_DIMENSIONS[levels.tx];
    let chroma = c.plane != 0;
    let sctx = get_skip_ctx(
        t_dim,
        c.bs,
        c.above,
        c.left,
        c.plane as i32,
        c.u_has_cf,
        true,
        true,
    ) as usize;
    let all_skip = levels.eob < 0;
    if c.plane == 2 {
        ec.encode_bool_adapt(all_skip, cdf.coef.skip_v(sctx));
    } else {
        ec.encode_bool_adapt(all_skip, cdf.coef.skip(0, t_dim.ctx as usize, sctx));
    }
    if all_skip {
        return NO_COEF_CTX;
    }

    let eob = levels.eob as u32;
    let slw = t_dim.lw.min(3) as usize;
    let slh = t_dim.lh.min(3) as usize;
    let tx2dszctx = slw + slh;
    write_eob(ec, cdf, eob, tx2dszctx, if chroma { 2 } else { 0 });

    let scan = SCANS[levels.tx];
    let abs = |i: usize| levels.q[scan[i] as usize].unsigned_abs();
    let mut cul_level = 0u32;
    let mut hr_avg = 0i32;

    let dc_abs;
    if eob == 0 {
        dc_abs = abs(0);
        if chroma {
            ec.encode_symbol_adapt(dc_abs.min(5) - 1, cdf.coef.eob_base_uv_tok_lf(0), 4);
        } else {
            let t = t_dim.ctx as usize;
            ec.encode_symbol_adapt(dc_abs.min(5) - 1, cdf.coef.eob_base_y_tok_lf(t, 0), 4);
            if dc_abs >= 5 {
                ec.encode_symbol_adapt((dc_abs - 5).min(3), cdf.coef.br_y_tok_lf(0), 3);
            }
        }
    } else {
        let stride = 4usize << slh;
        let shift = slh + 2;
        let mask = (4usize << slh) - 1;
        let hi_to_low: u32 = if chroma { 1 } else { 10 };
        let mut lv = vec![0i8; 1089];

        // CDF banks in `coef.data` (rav2d decode_coefs): lo token base and
        // stride, lo symbol count, hi (base-range) bank, and the token limit.
        let t = t_dim.ctx as usize;
        let lf = |chroma: bool| -> (usize, usize, usize, Option<usize>, u32) {
            if chroma {
                (4560, 8, 5, None, 5)
            } else {
                (1440 + t * 528, 8, 5, Some(4080), 5)
            }
        };
        let hf = |chroma: bool| -> (usize, usize, usize, Option<usize>, u32) {
            if chroma {
                (4460, 4, 3, Some(4508), 3)
            } else {
                (452 + t * 160, 4, 3, Some(1252), 3)
            }
        };

        // Last coefficient.
        let ctx_init = 1 + (eob > 2 << tx2dszctx) as usize + (eob > 4 << tx2dszctx) as usize;
        let (mut lo_base, mut lo_stride, mut lo_nsym, mut hi_base, mut lim) = if eob >= hi_to_low {
            hf(chroma)
        } else {
            lf(chroma)
        };
        let a = abs(eob as usize);
        let tok = if eob >= hi_to_low {
            let s = a.min(3) - 1;
            if chroma {
                ec.encode_symbol_adapt(s, cdf.coef.eob_base_uv_tok_hf(ctx_init), 2);
            } else {
                ec.encode_symbol_adapt(s, cdf.coef.eob_base_y_tok_hf(t, ctx_init), 2);
            }
            s + 1
        } else {
            let s = a.min(5) - 1;
            if chroma {
                ec.encode_symbol_adapt(s, cdf.coef.eob_base_uv_tok_lf(ctx_init), 4);
            } else {
                ec.encode_symbol_adapt(s, cdf.coef.eob_base_y_tok_lf(t, ctx_init), 4);
            }
            s + 1
        };
        let mut tok = tok;
        if tok == lim
            && let Some(hb) = hi_base
        {
            let hi_idx = if lim == 5 { 7 } else { 0 };
            let o = hb + hi_idx * 4;
            let br = (a - lim).min(3);
            ec.encode_symbol_adapt(br, &mut cdf.coef.data[o..o + 4], 3);
            tok += br;
        }
        lv[scan[eob as usize] as usize] = tok as i8;

        // AC tokens, last-but-one down to 1, then DC.
        let mut i = eob as i64 - 1;
        loop {
            if i == hi_to_low as i64 - 1 {
                (lo_base, lo_stride, lo_nsym, hi_base, lim) = lf(chroma);
            }
            let rc = if i == 0 { 0 } else { scan[i as usize] as usize };
            let (x, y) = (rc >> shift, rc & mask);
            let mut hr_ctx = 0u32;
            let xy = if i == 0 { 0 } else { (x + y) as u32 };
            let ctx = get_lo_ctx(&lv, rc, 0, &mut hr_ctx, xy, c.plane as i32, stride);
            let o = lo_base + (ctx * (2 - chroma as u32)) as usize * lo_stride;
            let a = abs(i as usize);
            let mut tok = a.min(lim);
            ec.encode_symbol_adapt(tok, &mut cdf.coef.data[o..o + lo_stride], lo_nsym);
            if tok == lim
                && let Some(hb) = hi_base
            {
                let o2 = hb + hr_ctx as usize * 4;
                let br = (a - lim).min(3);
                ec.encode_symbol_adapt(br, &mut cdf.coef.data[o2..o2 + 4], 3);
                tok += br;
            }
            lv[rc] = tok as i8;
            if i == 0 {
                break;
            }
            i -= 1;
        }
        dc_abs = abs(0);

        // AC signs and remainders, last down to 1.
        for i in (1..=eob as usize).rev() {
            let a = abs(i);
            if a == 0 {
                continue;
            }
            ec.encode_bool_bypass(levels.q[scan[i] as usize] < 0);
            let max_br = max_base_range(i, chroma);
            if a >= max_br {
                let hr = a - max_br;
                ec.encode_hr(hr, hr_avg);
                hr_avg = (hr_avg + hr as i32) >> 1;
            }
            cul_level += a;
        }
    }

    // DC sign and remainder.
    if dc_abs == 0 {
        return (cul_level.min(63) as u8) | 0x40;
    }
    let negative = levels.q[0] < 0;
    if chroma {
        ec.encode_bool_bypass(negative);
    } else {
        let ctx = get_dc_sign_ctx(t_dim, c.above, c.left) as usize;
        ec.encode_bool_adapt(negative, cdf.coef.dc_sign(0, 0, ctx));
    }
    let max_br = max_base_range(0, chroma);
    if dc_abs >= max_br {
        ec.encode_hr(dc_abs - max_br, hr_avg);
    }
    cul_level += dc_abs;
    (cul_level.min(63) as u8) | if negative { 0 } else { 0x80 }
}

/// End-of-block position: its class, then the high bit, then the rest.
fn write_eob<W: SymbolWriter>(
    ec: &mut W,
    cdf: &mut CdfContext,
    eob: u32,
    tx2dszctx: usize,
    ctx: usize,
) {
    let class = if eob <= 1 {
        eob
    } else {
        32 - eob.leading_zeros()
    };
    match tx2dszctx {
        0 => ec.encode_symbol_adapt(class, cdf.coef.eob_bin_16(ctx), 4),
        1 => ec.encode_symbol_adapt(class, cdf.coef.eob_bin_32(ctx), 5),
        2 => ec.encode_symbol_adapt(class, cdf.coef.eob_bin_64(ctx), 6),
        3 => ec.encode_symbol_adapt(class, cdf.coef.eob_bin_128(ctx), 7),
        4 => {
            ec.encode_symbol_adapt(class.min(7), cdf.coef.eob_bin_256(ctx), 7);
            if class >= 7 {
                ec.encode_bools_bypass(class - 7, 1);
            }
        }
        5 => {
            ec.encode_symbol_adapt(class.min(7), cdf.coef.eob_bin_512(ctx), 7);
            if class >= 7 {
                ec.encode_bools_bypass(class - 7, 2);
            }
        }
        _ => {
            ec.encode_symbol_adapt(class.min(7), cdf.coef.eob_bin_1024(ctx), 7);
            if class >= 7 {
                ec.encode_bools_bypass(class - 7, 2);
            }
        }
    }
    if class > 1 {
        ec.encode_bool_adapt((eob >> (class - 2)) & 1 != 0, cdf.coef.eob_hi_bit());
        if class > 2 {
            ec.encode_bools_bypass(eob & ((1 << (class - 2)) - 1), class - 2);
        }
    }
}

/// Pixel-domain size of one quantizer step for transform `tx`, from the
/// measured basis functions: what the rate-distortion trade-off is scaled by.
pub fn quant_step_pixels(tx: usize, dq: u32) -> f64 {
    let t_dim = &TXFM_DIMENSIONS[tx];
    let b = basis(tx);
    let mean_norm = b.norms.iter().map(|&n| n as f64).sum::<f64>() / b.norms.len() as f64;
    let coef_step = dq as f64 / (1u32 << dq_shift(t_dim)) as f64;
    mean_norm.sqrt() / AMPLITUDE as f64 * coef_step
}
