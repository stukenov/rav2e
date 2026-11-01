//! Coefficient contexts, copied from rav2d (`crates/rav2d/src/recon.rs`;
//! `get_skip_ctx` and `get_dc_sign_ctx` at aab9d2d, `get_lo_ctx` at 4440fd2).
//! The encoder derives the same contexts the decoder will.

use super::tables::{BLOCK_DIMENSIONS, N_BS_SIZES, TxfmInfo};

fn read_u16_ne(a: &[u8]) -> u16 {
    u16::from_ne_bytes(a[..2].try_into().unwrap())
}

fn read_u32_ne(a: &[u8]) -> u32 {
    u32::from_ne_bytes(a[..4].try_into().unwrap())
}

fn read_u64_ne(a: &[u8]) -> u64 {
    u64::from_ne_bytes(a[..8].try_into().unwrap())
}

#[allow(clippy::too_many_arguments)]
pub fn get_skip_ctx(
    t_dim: &TxfmInfo,
    bs: usize,
    a: &[u8],
    l: &[u8],
    plane: i32,
    u_has_cf: i32,
    ss_hor: bool,
    ss_ver: bool,
) -> u32 {
    debug_assert!(bs < N_BS_SIZES);
    let b_dim = &BLOCK_DIMENSIONS[bs];

    if plane != 0 {
        let not_one_blk = (b_dim[2] - (b_dim[2] != 0 && ss_hor) as u8 > t_dim.lw)
            || (b_dim[3] - (b_dim[3] != 0 && ss_ver) as u8 > t_dim.lh);

        let ca: bool = match t_dim.lw {
            0 => a[0] != 0x40,
            1 => read_u16_ne(a) != 0x4040,
            2 => read_u32_ne(a) != 0x40404040,
            3 => read_u64_ne(a) != 0x4040404040404040,
            4 => (read_u64_ne(a) | read_u64_ne(&a[8..])) != 0x4040404040404040,
            _ => unreachable!(),
        };
        let cl: bool = match t_dim.lh {
            0 => l[0] != 0x40,
            1 => read_u16_ne(l) != 0x4040,
            2 => read_u32_ne(l) != 0x40404040,
            3 => read_u64_ne(l) != 0x4040404040404040,
            4 => (read_u64_ne(l) | read_u64_ne(&l[8..])) != 0x4040404040404040,
            _ => unreachable!(),
        };

        let offset = if plane == 1 {
            6
        } else {
            6 * u_has_cf + not_one_blk as i32 * 3
        } as u32;
        offset + ca as u32 + cl as u32
    } else if b_dim[2] == t_dim.lw && b_dim[3] == t_dim.lh {
        0
    } else {
        let merge = |dir: &[u8], tx: u8| -> u32 {
            let mut v: u32;
            if tx == 4 {
                let tmp = read_u64_ne(dir) | read_u64_ne(&dir[8..]);
                v = (tmp >> 32) as u32 | tmp as u32;
            } else {
                v = match tx {
                    0 => dir[0] as u32,
                    1 => read_u16_ne(dir) as u32,
                    2 | 3 => read_u32_ne(dir),
                    _ => unreachable!(),
                };
            }
            if tx == 3 {
                v |= read_u32_ne(&dir[4..]);
            }
            if tx >= 2 {
                v |= v >> 16;
            }
            if tx >= 1 {
                v |= v >> 8;
            }
            v
        };
        let la = merge(a, t_dim.lw);
        let ll = merge(l, t_dim.lh);
        ((la & 0x3F).min(4) + (ll & 0x3F).min(4) + 3) >> 1
    }
}

pub fn get_dc_sign_ctx(t_dim: &TxfmInfo, a: &[u8], l: &[u8]) -> u32 {
    let mask: u64 = 0xC0C0C0C0C0C0C0C0;
    let mul: u64 = 0x0101010101010101;
    let mut t: u64 = 0;

    for &(edge, len) in &[(a, t_dim.lw), (l, t_dim.lh)] {
        match len {
            0 => t += (edge[0] >> 6) as u64,
            1 => t += (read_u16_ne(edge) as u64 & mask) >> 6,
            2 => t += (read_u32_ne(edge) as u64 & mask) >> 6,
            3 => t += (read_u64_ne(edge) & mask) >> 6,
            4 => {
                t += (read_u64_ne(&edge[8..]) & mask) >> 6;
                t += (read_u64_ne(edge) & mask) >> 6;
            }
            _ => unreachable!(),
        }
    }

    t = t.wrapping_mul(mul);
    let s = (t >> 56) as i32 - t_dim.w as i32 - t_dim.h as i32;
    (s != 0) as u32 + (s > 0) as u32
}

pub fn get_lo_ctx(
    levels: &[i8],
    off: usize,
    tx_class: u8,
    hi_mag: &mut u32,
    xy: u32,
    plane: i32,
    stride: usize,
) -> u32 {
    let chroma = plane != 0;
    let lo_freq = xy
        < if chroma {
            1
        } else if tx_class == 0 {
            4
        } else {
            2
        };
    let mut lim: u32 = if lo_freq { 5 } else { 3 };
    let mut lo_mag: u32 = 0;
    let mut hi: u32 = 0;

    macro_rules! add {
        ($v:expr) => {{
            let val = $v as u32;
            lo_mag += val.min(lim);
            hi += val.min(5);
        }};
    }

    add!(levels[off + 1]);
    add!(levels[off + stride]);

    let offset: u32;
    if tx_class == 0 {
        add!(levels[off + stride + 1]);
        if !chroma {
            lo_mag +=
                (levels[off + 2] as u32).min(lim) + (levels[off + 2 * stride] as u32).min(lim);
            if lo_freq {
                offset = if xy == 0 {
                    0
                } else if xy < 2 {
                    9
                } else {
                    16
                };
                lim = if xy == 0 {
                    8
                } else if xy < 2 {
                    6
                } else {
                    4
                };
            } else {
                offset = if xy < 6 {
                    0
                } else if xy < 8 {
                    5
                } else {
                    10
                };
                lim = 4;
            }
        } else {
            lim = 3;
            offset = if plane == 1 { 0 } else { 4 };
        }
    } else {
        if !chroma {
            lim = 3;
            add!(levels[off + 2]);
            lo_mag += (levels[off + 3] as u32).min(3) + (levels[off + 4] as u32).min(3);
            if lo_freq {
                offset = if xy == 0 { 21 } else { 28 };
                lim = if xy == 0 { 6 } else { 4 };
            } else {
                offset = 15;
                lim = 4;
            }
        } else {
            offset = 8;
            lim = 3;
        }
    }

    *hi_mag = (if !chroma && lo_freq && (xy > 0 || tx_class != 0) {
        7
    } else {
        0
    }) + ((hi + 1) >> 1).min(if chroma { 3 } else { 6 });
    offset + ((lo_mag + 1) >> 1).min(lim)
}
