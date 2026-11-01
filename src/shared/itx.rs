//! Inverse transform, copied from rav2d (`crates/rav2d/src/itx.rs` and
//! `itx_1d.rs` at 4440fd2): the DCT_DCT path only, 8-bit, scalar. The SIMD
//! column pass in rav2d computes the same values as this scalar one.

// Kept as written in rav2d, which mirrors the C reference term by term.
#![allow(clippy::erasing_op, clippy::identity_op)]

use super::scan::LAST_EOB_PER_COL;
use super::tables::{TX_SHIFT, TXFM_DIMENSIONS};

static DCT8_KERNEL: [i8; 16] = [
    89, 75, 50, 18, 75, -18, -89, -50, 50, -89, 18, 75, 18, -50, 75, -89,
];

static DCT16_KERNEL: [i8; 64] = [
    90, 87, 80, 70, 57, 43, 26, 9, 87, 57, 9, -43, -80, -90, -70, -26, 80, 9, -70, -87, -26, 57,
    90, 43, 70, -43, -87, 9, 90, 26, -80, -57, 57, -80, -26, 90, -9, -87, 43, 70, 43, -90, 57, 26,
    -87, 70, 9, -80, 26, -70, 90, -80, 43, 9, -57, 87, 9, -26, 43, -57, 70, -80, 87, -90,
];

static DCT32_KERNEL: [i8; 256] = [
    90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 47, 39, 30, 22, 13, 4, 90, 82, 67, 47, 22, -4, -30,
    -54, -73, -85, -90, -88, -78, -61, -39, -13, 88, 67, 30, -13, -54, -82, -90, -78, -47, -4, 39,
    73, 90, 85, 61, 22, 85, 47, -13, -67, -90, -73, -22, 39, 82, 88, 54, -4, -61, -90, -78, -30,
    82, 22, -54, -90, -61, 13, 78, 85, 30, -47, -90, -67, 4, 73, 88, 39, 78, -4, -82, -73, 13, 85,
    67, -22, -88, -61, 30, 90, 54, -39, -90, -47, 73, -30, -90, -22, 78, 67, -39, -90, -13, 82, 61,
    -47, -88, -4, 85, 54, 67, -54, -78, 39, 85, -22, -90, 4, 90, 13, -88, -30, 82, 47, -73, -61,
    61, -73, -47, 82, 30, -88, -13, 90, -4, -90, 22, 85, -39, -78, 54, 67, 54, -85, -4, 88, -47,
    -61, 82, 13, -90, 39, 67, -78, -22, 90, -30, -73, 47, -90, 39, 54, -90, 30, 61, -88, 22, 67,
    -85, 13, 73, -82, 4, 78, 39, -88, 73, -4, -67, 90, -47, -30, 85, -78, 13, 61, -90, 54, 22, -82,
    30, -78, 90, -61, 4, 54, -88, 82, -39, -22, 73, -90, 67, -13, -47, 85, 22, -61, 85, -90, 73,
    -39, -4, 47, -78, 90, -82, 54, -13, -30, 67, -88, 13, -39, 61, -78, 88, -90, 85, -73, 54, -30,
    4, 22, -47, 67, -82, 90, 4, -13, 22, -30, 39, -47, 54, -61, 67, -73, 78, -82, 85, -88, 90, -90,
];

fn inv_dct_1d(c: &mut [i32], stride: usize, mat: &[i8], n: usize) {
    let mut a = [0i32; 16];
    let mut b = [0i32; 16];
    let k = n * 2 - 1;
    let mut mi = 0;

    for i in 0..n {
        let mut sum = 0i32;
        let mut j = 1;
        while j <= k {
            sum += mat[mi] as i32 * c[j * stride];
            mi += 1;
            j += 2;
        }
        a[i] = c[i * 2 * stride];
        b[i] = sum;
    }

    for i in 0..n {
        c[i * stride] = a[i] + b[i];
        c[(k - i) * stride] = a[i] - b[i];
    }
}

fn inv_dct4_1d(c: &mut [i32], stride: usize) {
    let a0 = c[0 * stride] * 64 + c[2 * stride] * 64;
    let a1 = c[0 * stride] * 64 - c[2 * stride] * 64;
    let b0 = c[stride] * 83 + c[3 * stride] * 35;
    let b1 = c[stride] * 35 - c[3 * stride] * 83;

    c[0 * stride] = a0 + b0;
    c[stride] = a1 + b1;
    c[2 * stride] = a1 - b1;
    c[3 * stride] = a0 - b0;
}

fn inv_dct8_1d(c: &mut [i32], stride: usize) {
    inv_dct4_1d(c, 2 * stride);
    inv_dct_1d(c, stride, &DCT8_KERNEL, 4);
}

fn inv_dct16_1d(c: &mut [i32], stride: usize) {
    inv_dct8_1d(c, 2 * stride);
    inv_dct_1d(c, stride, &DCT16_KERNEL, 8);
}

fn inv_dct32_1d(c: &mut [i32], stride: usize) {
    inv_dct16_1d(c, 2 * stride);
    inv_dct_1d(c, stride, &DCT32_KERNEL, 16);
}

fn inv_dct_1d_for(size_log2: u8) -> fn(&mut [i32], usize) {
    match size_log2 {
        0 => inv_dct4_1d,
        1 => inv_dct8_1d,
        2 => inv_dct16_1d,
        // 32 and 64 (64-point transforms are coded as 32-point, see below).
        _ => inv_dct32_1d,
    }
}

/// `inv_txfm_add` for `txtp` DCT_DCT: inverse-transforms `coeff` (rav2d's
/// layout, `coeff[u * min(h, 32) + v]` for horizontal frequency `u` and
/// vertical frequency `v`) and adds it to the 8-bit block at `dst`.
/// `coeff` is cleared, as in the decoder.
pub fn inv_txfm_add_dct(dst: &mut [u8], stride: usize, coeff: &mut [i32], eob: i32, tx: usize) {
    let t_dim = &TXFM_DIMENSIONS[tx];
    let tx_sh = &TX_SHIFT[tx];
    let w = 4 * t_dim.w as usize;
    let h = 4 * t_dim.h as usize;
    let is_rect2 = ((t_dim.lw + t_dim.lh) & 1) != 0;

    if eob == 0 {
        // DC-only path.
        let shift_p1 = tx_sh[0] as i32;
        let shift = shift_p1 + tx_sh[1] as i32 - 12;
        let rnd = (1 << (shift - 1)) + shift_p1 - 6;
        let mut dc = coeff[0];
        coeff[0] = 0;
        if is_rect2 {
            dc = (dc * 181 + 128) >> 8;
        }
        dc = (dc + rnd) >> shift;
        for y in 0..h {
            for p in &mut dst[y * stride..y * stride + w] {
                *p = (*p as i32 + dc).clamp(0, 255) as u8;
            }
        }
        return;
    }

    let first_1d_fn = inv_dct_1d_for(t_dim.lw);
    let second_1d_fn = inv_dct_1d_for(t_dim.lh);
    let sh = h.min(32);
    let sw = w.min(32);
    let (row_clip_min, row_clip_max) = (i16::MIN as i32, i16::MAX as i32);

    let mut tmp = [0i32; 32 * 32];
    let mut col = 0usize;
    let off = LAST_EOB_PER_COL.offset[tx] as usize;
    let last_eob = &LAST_EOB_PER_COL.table[off..];
    let mut ei = 0usize;
    loop {
        for x in 0..sw {
            let v = coeff[col + x * sh];
            tmp[col * sw + x] = if is_rect2 { (v * 181 + 128) >> 8 } else { v };
        }
        first_1d_fn(&mut tmp[col * sw..], 1);
        col += 1;
        if col & 3 == 0 {
            if eob > last_eob[ei] as i32 {
                ei += 1;
            } else {
                break;
            }
        }
    }
    if col < sh {
        tmp[col * sw..sh * sw].fill(0);
    }
    coeff[..sw * sh].fill(0);

    let shift0 = tx_sh[0] as i32;
    let rnd0 = (1 << shift0) >> 1;
    for v in &mut tmp[..sw * sh] {
        *v = ((*v + rnd0) >> shift0).clamp(row_clip_min, row_clip_max);
    }
    for x in 0..sw {
        second_1d_fn(&mut tmp[x..], sw);
    }

    let shift1 = tx_sh[1] as i32;
    let rnd1 = (1 << shift1) >> 1;
    // 64-point dimensions are reconstructed at 32 points and upsampled by 2.
    let (xs, ys) = (w / sw, h / sh);
    for y in 0..sh {
        for x in 0..sw {
            let r = (tmp[y * sw + x] + rnd1) >> shift1;
            for dy in 0..ys {
                for dx in 0..xs {
                    let d = (y * ys + dy) * stride + x * xs + dx;
                    dst[d] = (dst[d] as i32 + r).clamp(0, 255) as u8;
                }
            }
        }
    }
}
