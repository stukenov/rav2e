//! M0: the worst legal encoder.
//!
//! One still picture, 8-bit 4:2:0, frame size a multiple of 64. Every 64x64
//! superblock is one block (partition NONE), predicted with DC_PRED in all
//! three planes, and its residual is coded as a single DC coefficient of the
//! largest transform (64x64 luma, 32x32 chroma). The picture the encoder
//! reconstructs must equal what any AV2 decoder makes of the stream.

use rav2d::cdf::{CdfContext, cdf_thread_init_static_qcat};
use rav2d::decode::PARTITION_SUBB;
use rav2d::ipred::{ipred_dc_8bpc, ipred_dc_128_8bpc, ipred_dc_left_8bpc, ipred_dc_top_8bpc};
use rav2d::itx::inv_txfm_add_8bpc;
use rav2d::levels::BlockSize;
use rav2d::quantizer::dq_lookup;
use rav2d::recon::{get_dc_sign_ctx, get_skip_ctx};
use rav2d::tables::{MAX_TXFM_SIZE_FOR_BS, TX_PART_TBL, TXFM_DIMENSIONS, TxfmInfo};

use crate::ec::RangeEncoder;
use crate::frame::Frame;
use crate::headers::{StillParams, still_picture_stream};

const SB: usize = 64;
/// Context value of a 4x4 unit with no coded coefficients.
const NO_COEF_CTX: u8 = 0x40;
/// What `decode_sb` leaves in the partition context after a NONE block of
/// 64x64: `!(bw4 - 1)` with bw4 = 16.
const PARTITION_CTX_64X64: u8 = !(16u8 - 1);

pub struct Encoded {
    pub bitstream: Vec<u8>,
    pub recon: Frame,
}

#[derive(Debug)]
pub enum EncodeError {
    /// M0 needs both dimensions to be non-zero multiples of 64.
    UnsupportedSize { width: usize, height: usize },
    /// 0 would signal a lossless frame, which M0 does not code.
    UnsupportedQidx(u8),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSize { width, height } => write!(
                f,
                "{width}x{height}: width and height must be non-zero multiples of 64"
            ),
            Self::UnsupportedQidx(q) => write!(f, "qidx {q}: must be 1..=255"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Above and left entropy contexts the decoder keeps per 4x4 unit.
struct Contexts {
    above_partition: Vec<u8>,
    left_partition: Vec<u8>,
    above_luma: Vec<u8>,
    left_luma: Vec<u8>,
    above_chroma: [Vec<u8>; 2],
    left_chroma: [Vec<u8>; 2],
}

impl Contexts {
    fn new(width: usize) -> Self {
        let w4 = width / 4;
        Self {
            above_partition: vec![0; w4],
            left_partition: vec![0; SB / 4],
            above_luma: vec![NO_COEF_CTX; w4],
            left_luma: vec![NO_COEF_CTX; SB / 4],
            above_chroma: [vec![NO_COEF_CTX; w4 / 2], vec![NO_COEF_CTX; w4 / 2]],
            left_chroma: [vec![NO_COEF_CTX; SB / 8], vec![NO_COEF_CTX; SB / 8]],
        }
    }

    /// The decoder resets its left context at the start of every superblock
    /// row of a tile.
    fn reset_left(&mut self) {
        self.left_partition.fill(0);
        self.left_luma.fill(NO_COEF_CTX);
        for l in &mut self.left_chroma {
            l.fill(NO_COEF_CTX);
        }
    }
}

/// One transform block's worth of everything `decode_coefs` needs.
struct TxBlock<'a> {
    plane: usize,
    tx: usize,
    /// Square block size in pixels in this plane.
    size: usize,
    x: usize,
    y: usize,
    stride: usize,
    src: &'a [u8],
    have_top: bool,
    have_left: bool,
    dq_dc: u32,
}

pub fn encode_still(src: &Frame, qidx: u8) -> Result<Encoded, EncodeError> {
    let (width, height) = (src.width, src.height);
    if width == 0 || height == 0 || width % SB != 0 || height % SB != 0 {
        return Err(EncodeError::UnsupportedSize { width, height });
    }
    if qidx == 0 {
        return Err(EncodeError::UnsupportedQidx(qidx));
    }

    let bs = BlockSize::Bs64x64 as usize;
    let luma_tx = TX_PART_TBL[bs][0] as usize;
    // dav2d_max_txfm_size_for_bs[bs][I444 - I420]
    let chroma_tx = MAX_TXFM_SIZE_FOR_BS[bs][2] as usize;
    let dq_dc = dq_lookup(qidx as i32) as u32;

    let qcat = cdf_thread_init_static_qcat(qidx as u32) as usize;
    let mut cdf = CdfContext::init_from_defaults(qcat);
    let mut ec = RangeEncoder::new(true);
    let mut ctx = Contexts::new(width);
    let mut recon = Frame::new(width, height);

    for sby in 0..height / SB {
        ctx.reset_left();
        for sbx in 0..width / SB {
            let (bx4, by4) = (sbx * SB / 4, 0usize);

            // Partition: one NONE block for the whole superblock.
            let ctx1 = ((ctx.above_partition[bx4] >> 3) & 1) as usize
                + ((ctx.left_partition[by4] >> 3) & 1) as usize * 2;
            let ctx2 = ctx1 + PARTITION_SUBB[bs].ctx[0] as usize * 4;
            ec.encode_bool_adapt(false, cdf.m.part_split(0, ctx2));

            // Luma mode DC_PRED: first set, first index. With DC everywhere the
            // neighbours carry no directional mode, so the context is 0.
            ec.encode_symbol_adapt(0, cdf.m.intra_y_set(), 3);
            ec.encode_symbol_adapt(0, cdf.m.intra_y_idx0(0), 7);
            // Chroma mode DC_PRED (context 0: the luma mode is not directional).
            ec.encode_symbol_adapt(0, cdf.m.intra_uv_mode(0), 7);

            let have_top = sby > 0;
            let have_left = sbx > 0;

            // Luma coefficients, then U, then V, as the decoder reads them.
            let luma = TxBlock {
                plane: 0,
                tx: luma_tx,
                size: SB,
                x: sbx * SB,
                y: sby * SB,
                stride: width,
                src: &src.planes[0],
                have_top,
                have_left,
                dq_dc,
            };
            let (res_ctx, _) = code_dc_block(
                &mut ec,
                &mut cdf,
                &luma,
                &mut recon.planes[0],
                &ctx.above_luma[bx4..],
                &ctx.left_luma[by4..],
                0,
            );
            ctx.above_luma[bx4..bx4 + 16].fill(res_ctx);
            ctx.left_luma[by4..by4 + 16].fill(res_ctx);

            let mut u_has_cf = 0;
            for pl in 1..3 {
                let c = pl - 1;
                let cw = width / 2;
                let blk = TxBlock {
                    plane: pl,
                    tx: chroma_tx,
                    size: SB / 2,
                    x: sbx * SB / 2,
                    y: sby * SB / 2,
                    stride: cw,
                    src: &src.planes[pl],
                    have_top,
                    have_left,
                    dq_dc,
                };
                let cbx4 = bx4 / 2;
                let (res_ctx, coded) = code_dc_block(
                    &mut ec,
                    &mut cdf,
                    &blk,
                    &mut recon.planes[pl],
                    &ctx.above_chroma[c][cbx4..],
                    &ctx.left_chroma[c][..],
                    u_has_cf,
                );
                if pl == 1 {
                    u_has_cf = coded as i32;
                }
                ctx.above_chroma[c][cbx4..cbx4 + 8].fill(res_ctx);
                ctx.left_chroma[c][..8].fill(res_ctx);
            }

            ctx.above_partition[bx4..bx4 + 16].fill(PARTITION_CTX_64X64);
            ctx.left_partition[by4..by4 + 16].fill(PARTITION_CTX_64X64);
        }
    }

    let params = StillParams {
        width: width as u32,
        height: height as u32,
        qidx,
    };
    let bitstream = still_picture_stream(&params, &ec.finish());
    Ok(Encoded { bitstream, recon })
}

/// Predicts the block with DC_PRED from the reconstruction so far, picks the
/// DC level that brings the block closest to the source, writes it, and
/// stores the reconstruction. Returns the coefficient context the decoder
/// keeps for the block's 4x4 units and whether any coefficient was coded.
fn code_dc_block(
    ec: &mut RangeEncoder,
    cdf: &mut CdfContext,
    b: &TxBlock,
    recon: &mut [u8],
    above: &[u8],
    left: &[u8],
    u_has_cf: i32,
) -> (u8, bool) {
    let t_dim = &TXFM_DIMENSIONS[b.tx];
    let pred = predict_dc(recon, b);

    // Level search. The reconstruction's sum grows monotonically with the
    // level, so binary-search the first level whose block is not darker than
    // the source, then let the error decide between it and its neighbour.
    // (Probing with level 1 does not work: at fine quantizers one step of DC
    // rounds to nothing.)
    const MAX_LEVEL: i32 = 1 << 14;
    let src_sum: i64 = (0..b.size)
        .flat_map(|y| (0..b.size).map(move |x| (y, x)))
        .map(|(y, x)| b.src[(b.y + y) * b.stride + b.x + x] as i64)
        .sum();
    let excess = |level: i32| -> i64 {
        reconstruct(&pred, b, t_dim, level)
            .iter()
            .map(|&v| v as i64)
            .sum::<i64>()
            - src_sum
    };
    let (mut lo, mut hi) = (-MAX_LEVEL, MAX_LEVEL);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if excess(mid) >= 0 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let mut best = (u64::MAX, 0i32);
    for level in [0, lo - 1, lo] {
        let level = level.clamp(-MAX_LEVEL, MAX_LEVEL);
        let sse = sse(&reconstruct(&pred, b, t_dim, level), b);
        if sse < best.0 || (sse == best.0 && level.abs() < best.1.abs()) {
            best = (sse, level);
        }
    }
    let level = best.1;

    write_dc_coef(ec, cdf, b, t_dim, above, left, u_has_cf, level);

    let out = reconstruct(&pred, b, t_dim, level);
    for y in 0..b.size {
        let row = (b.y + y) * b.stride + b.x;
        recon[row..row + b.size].copy_from_slice(&out[y * b.size..(y + 1) * b.size]);
    }

    if level == 0 {
        return (NO_COEF_CTX, false);
    }
    let tok = level.unsigned_abs();
    let sign_bits = if level < 0 { 0 } else { 0x80 };
    ((tok.min(63) as u8) | sign_bits, true)
}

/// `decode_coefs` for a block whose only coefficient is DC: the skip flag, an
/// end-of-block at position 0, the DC token, its sign and remainder.
#[allow(clippy::too_many_arguments)]
fn write_dc_coef(
    ec: &mut RangeEncoder,
    cdf: &mut CdfContext,
    b: &TxBlock,
    t_dim: &TxfmInfo,
    above: &[u8],
    left: &[u8],
    u_has_cf: i32,
    level: i32,
) {
    let chroma = b.plane != 0;
    let bs = BlockSize::Bs64x64 as usize;
    let sctx = get_skip_ctx(t_dim, bs, above, left, b.plane as i32, u_has_cf, true, true) as usize;
    let all_skip = level == 0;
    match b.plane {
        2 => ec.encode_bool_adapt(all_skip, cdf.coef.skip_v(sctx)),
        _ => ec.encode_bool_adapt(all_skip, cdf.coef.skip(0, t_dim.ctx as usize, sctx)),
    }
    if all_skip {
        return;
    }

    // End of block at scan position 0 (only DC is non-zero). The largest
    // transforms code their eob with the 1024-entry class.
    let tx2dszctx = t_dim.lw.min(3) as usize + t_dim.lh.min(3) as usize;
    assert_eq!(tx2dszctx, 6, "M0 codes DC-only 32x32 and 64x64 transforms");
    let eob_ctx = if chroma { 2 } else { 0 };
    ec.encode_symbol_adapt(0, cdf.coef.eob_bin_1024(eob_ctx), 7);

    let tok = level.unsigned_abs();
    let negative = level < 0;
    if chroma {
        ec.encode_symbol_adapt(tok.min(5) - 1, cdf.coef.eob_base_uv_tok_lf(0), 4);
        ec.encode_bool_bypass(negative);
        if tok >= 5 {
            ec.encode_hr(tok - 5, 0);
        }
    } else {
        ec.encode_symbol_adapt(
            tok.min(5) - 1,
            cdf.coef.eob_base_y_tok_lf(t_dim.ctx as usize, 0),
            4,
        );
        if tok >= 5 {
            // hi_idx 0: the 2-D transform class.
            ec.encode_symbol_adapt((tok - 5).min(3), cdf.coef.br_y_tok_lf(0), 3);
        }
        let dc_sign_ctx = get_dc_sign_ctx(t_dim, above, left) as usize;
        ec.encode_bool_adapt(negative, cdf.coef.dc_sign(0, 0, dc_sign_ctx));
        if tok >= 8 {
            ec.encode_hr(tok - 8, 0);
        }
    }
}

/// DC_PRED from the reconstructed neighbours, choosing the variant the
/// decoder picks for the available edges.
fn predict_dc(recon: &[u8], b: &TxBlock) -> Vec<u8> {
    let size = b.size;
    let mut dst = vec![0u8; size * size];
    let o = size;
    let mut edge = vec![0u8; 2 * size + 1];
    if b.have_top {
        let row = (b.y - 1) * b.stride + b.x;
        edge[o + 1..o + 1 + size].copy_from_slice(&recon[row..row + size]);
    }
    if b.have_left {
        for i in 0..size {
            edge[o - 1 - i] = recon[(b.y + i) * b.stride + b.x - 1];
        }
    }
    match (b.have_left, b.have_top) {
        (true, true) => ipred_dc_8bpc(&mut dst, size, &edge, o, size, size, 0),
        (false, true) => ipred_dc_top_8bpc(&mut dst, size, &edge, o, size, size, 0),
        (true, false) => ipred_dc_left_8bpc(&mut dst, size, &edge, o, size, size, 0),
        (false, false) => ipred_dc_128_8bpc(&mut dst, size, size, size),
    }
    dst
}

/// The decoder's dequantized DC coefficient for a signed level.
fn dc_coefficient(b: &TxBlock, t_dim: &TxfmInfo, level: i32) -> i32 {
    let tok = level.unsigned_abs();
    let negative = level < 0;
    let max_br = if b.plane != 0 { 5 } else { 8 };
    let shift = 3 + (t_dim.ctx as i32 - 2).max(0);
    // cf_max for 8-bit: !((!127) << 8)
    let cf_max = !((!127u32) << 8) as i32;
    let val = if tok >= max_br {
        let v = tok & 0xfffff;
        let raw = (((v.wrapping_mul(b.dq_dc)) & 0xffffff).wrapping_add(4) >> shift) as i32;
        raw.min(cf_max + negative as i32)
    } else {
        (tok.wrapping_mul(b.dq_dc).wrapping_add(4) >> shift) as i32
    };
    if negative { -val } else { val }
}

fn reconstruct(pred: &[u8], b: &TxBlock, t_dim: &TxfmInfo, level: i32) -> Vec<u8> {
    let mut out = pred.to_vec();
    if level != 0 {
        let mut coeff = vec![0i32; (t_dim.w as usize * 4) * (t_dim.h as usize * 4)];
        coeff[0] = dc_coefficient(b, t_dim, level);
        // txtp DCT_DCT (0) with eob 0 takes the decoder's DC-only path.
        inv_txfm_add_8bpc(&mut out, 0, b.size, &mut coeff, 0, 0, b.tx);
    }
    out
}

fn sse(block: &[u8], b: &TxBlock) -> u64 {
    let mut s = 0u64;
    for y in 0..b.size {
        for x in 0..b.size {
            let d = block[y * b.size + x] as i64 - b.src[(b.y + y) * b.stride + b.x + x] as i64;
            s += (d * d) as u64;
        }
    }
    s
}
