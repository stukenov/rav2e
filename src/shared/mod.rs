//! Code the encoder shares with the decoder.
//!
//! The encoder has to reconstruct exactly what a decoder will, so it uses the
//! decoder's own tables and formulas. These are copied from
//! [rav2d](https://github.com/stukenov/rav2d) (same author, same BSD-2-Clause
//! license), each file naming its source file and commit. Change them only
//! together with rav2d; the closed-loop test catches any drift.

pub mod cdf;
pub mod ctx;
pub mod itx;
pub mod msac_tables;
pub mod scan;
pub mod tables;

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
