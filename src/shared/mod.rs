//! Code the encoder shares with the decoder.
//!
//! The encoder has to reconstruct exactly what a decoder will, so it uses the
//! decoder's own tables and formulas. These are copied from
//! [rav2d](https://github.com/stukenov/rav2d) at commit aab9d2d (same author,
//! same BSD-2-Clause license), each file naming its source. Change them only
//! together with rav2d; the closed-loop test catches any drift.

pub mod cdf;
pub mod ctx;
pub mod msac_tables;
pub mod tables;

use tables::{TX_SHIFT, TXFM_DIMENSIONS};

/// Dequantizer for a quantizer index (`quantizer.rs::dq_lookup`).
pub fn dq_lookup(qidx: i32) -> i32 {
    let qidx = qidx.clamp(0, 255);
    if qidx == 0 {
        return 64;
    }
    let qidx = qidx - 1;
    let shift = qidx / 24;
    let qidx = (qidx % 24) as usize;
    static DQ_LOOKUP_TBL: [u8; 24] = [
        40, 41, 43, 44, 45, 47, 48, 49, 51, 52, 54, 55, 57, 59, 60, 62, 64, 66, 68, 70, 72, 74, 76,
        78,
    ];
    (DQ_LOOKUP_TBL[qidx] as i32) << shift
}

/// DC_PRED of a square, power-of-two block into `dst` (stride `size`), from
/// the row above and the column to the left; `None` when that edge is not
/// available. The four cases are `ipred_dc`, `ipred_dc_top`, `ipred_dc_left`
/// and `ipred_dc_128` in `ipred.rs`, 8-bit, without IBP.
pub fn predict_dc(dst: &mut [u8], size: usize, top: Option<&[u8]>, left: Option<&[u8]>) {
    debug_assert!(size.is_power_of_two());
    let sum = |e: &[u8]| e[..size].iter().map(|&v| v as u32).sum::<u32>();
    let dc = match (top, left) {
        // dc_gen: (sum + width) >> log2(width + height) for power-of-two totals.
        (Some(t), Some(l)) => (sum(t) + sum(l) + size as u32) >> (2 * size).trailing_zeros(),
        // dc_gen_top / dc_gen_left: (sum + size / 2) >> log2(size).
        (Some(t), None) => (sum(t) + (size >> 1) as u32) >> size.trailing_zeros(),
        (None, Some(l)) => (sum(l) + (size >> 1) as u32) >> size.trailing_zeros(),
        // ipred_dc_128: (bitdepth_max + 1) >> 1.
        (None, None) => 128,
    };
    dst[..size * size].fill(dc as u8);
}

/// Adds the inverse transform of a block whose only non-zero coefficient is
/// `dc` (`inv_txfm_add` with `eob + txtp == 0`, the DCT_DCT DC-only path in
/// `itx.rs`), clipping to 8 bits.
pub fn add_dc_only_inverse(dst: &mut [u8], stride: usize, dc: i32, tx: usize) {
    let t_dim = &TXFM_DIMENSIONS[tx];
    let tx_sh = &TX_SHIFT[tx];
    let (w, h) = (4 * t_dim.w as usize, 4 * t_dim.h as usize);
    let is_rect2 = ((t_dim.lw + t_dim.lh) & 1) != 0;
    let shift_p1 = tx_sh[0] as i32;
    let shift = shift_p1 + tx_sh[1] as i32 - 12;
    let rnd = (1 << (shift - 1)) + shift_p1 - 6;
    let mut dc = dc;
    if is_rect2 {
        dc = (dc * 181 + 128) >> 8;
    }
    dc = (dc + rnd) >> shift;
    for y in 0..h {
        for p in &mut dst[y * stride..y * stride + w] {
            *p = (*p as i32 + dc).clamp(0, 255) as u8;
        }
    }
}
