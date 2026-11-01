//! Still-picture encoder.
//!
//! One picture, 8-bit 4:2:0, frame size a multiple of 64. Each 64x64
//! superblock is split into square blocks from 64x64 down to 8x8, chosen by
//! rate-distortion cost. Every block is predicted with DC_PRED in all three
//! planes and its residual coded with the largest DCT that fits. The picture
//! the encoder reconstructs must equal what any AV2 decoder makes of the
//! stream.
//!
//! AV2 partitions are a binary tree. A square block is split in four as the
//! decoder reads it: the square splits horizontally (H) into two halves, and
//! each half splits vertically (V) into two squares.

use crate::coef::{
    CoefContext, NO_COEF_CTX, Quant, quant_step_pixels, quantize, reconstruct, write_coefs,
};
use crate::ec::{BitCounter, RangeEncoder, SymbolWriter};
use crate::frame::Frame;
use crate::headers::{StillParams, still_picture_stream};
use crate::shared::cdf::{CdfContext, cdf_thread_init_static_qcat};
use crate::shared::dq_lookup;
use crate::shared::tables::BLOCK_DIMENSIONS;

const SB: usize = 64;
const MIN_BLOCK: usize = 8;

/// Quantizer rounding offset: below one half, so that coefficients just
/// above a quantizer step are dropped rather than paid for.
const ROUNDING: f32 = 0.36;

/// Rate-distortion multiplier relative to the squared quantizer step in
/// pixels. Tuned on the first frame of bus_cif (320x256 crop, qidx 60-200):
/// 0.1 saves 4.8% BD-rate against ignoring rate; the textbook 0.34 saves
/// nothing (see zettel/lambda-tuned-on-bus.md).
const LAMBDA_PER_STEP2: f64 = 0.1;

/// rav2d `BlockSize` index of a square block.
fn square_bs(size: usize) -> usize {
    match size {
        64 => 6,
        32 => 12,
        16 => 18,
        8 => 24,
        _ => unreachable!("no square block of {size}"),
    }
}

/// rav2d `BlockSize` index of the `size` x `size / 2` half of a square.
fn half_bs(size: usize) -> usize {
    match size {
        64 => 7,
        32 => 13,
        16 => 19,
        _ => unreachable!("no half block of {size}"),
    }
}

/// `PARTITION_SUBB[bs].ctx` in rav2d's `decode.rs`: context offsets of the
/// split and direction flags.
fn partition_ctx_offsets(bs: usize) -> [usize; 2] {
    match bs {
        6 => [3, 6],
        7 => [3, 5],
        12 => [2, 3],
        13 => [2, 2],
        18 => [1, 0],
        19 => [1, 2],
        24 => [0, 0],
        _ => unreachable!("partition offsets for block size {bs}"),
    }
}

/// Largest transforms of a square block: luma (`TX_PART_TBL[bs][0]`) and
/// 4:2:0 chroma (`MAX_TXFM_SIZE_FOR_BS[bs][I444 - I420]`).
fn transforms(size: usize) -> (usize, usize) {
    match size {
        64 => (4, 3),
        32 => (3, 2),
        16 => (2, 1),
        8 => (1, 0),
        _ => unreachable!("no transforms for {size}"),
    }
}

pub struct Encoded {
    pub bitstream: Vec<u8>,
    pub recon: Frame,
}

#[derive(Debug)]
pub enum EncodeError {
    /// Both dimensions must be non-zero multiples of 64.
    UnsupportedSize { width: usize, height: usize },
    /// 0 would signal a lossless frame, which rav2e does not code.
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

/// Above and left entropy contexts the decoder keeps per 4x4 unit. The left
/// ones cover the current superblock row only.
#[derive(Clone)]
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

/// Reconstruction and contexts: everything a trial coding changes.
struct State {
    recon: Frame,
    ctx: Contexts,
}

/// What a trial changed in a square region, to put back afterwards.
struct Snapshot {
    planes: [Vec<u8>; 3],
    ctx: Contexts,
}

impl State {
    fn region(pl: usize, x: usize, y: usize, size: usize) -> (usize, usize, usize) {
        if pl == 0 {
            (x, y, size)
        } else {
            (x / 2, y / 2, size / 2)
        }
    }

    fn save(&self, x: usize, y: usize, size: usize) -> Snapshot {
        let planes = std::array::from_fn(|pl| {
            let (px, py, ps) = Self::region(pl, x, y, size);
            let stride = self.recon.plane_width(pl);
            (0..ps)
                .flat_map(|r| {
                    let row = (py + r) * stride + px;
                    self.recon.planes[pl][row..row + ps].iter().copied()
                })
                .collect()
        });
        Snapshot {
            planes,
            ctx: self.ctx.clone(),
        }
    }

    fn restore(&mut self, s: &Snapshot, x: usize, y: usize, size: usize) {
        for pl in 0..3 {
            let (px, py, ps) = Self::region(pl, x, y, size);
            let stride = self.recon.plane_width(pl);
            for r in 0..ps {
                let row = (py + r) * stride + px;
                self.recon.planes[pl][row..row + ps]
                    .copy_from_slice(&s.planes[pl][r * ps..(r + 1) * ps]);
            }
        }
        self.ctx.clone_from(&s.ctx);
    }
}

/// The chosen partition of a square block.
enum Plan {
    Leaf,
    Split(Box<[Plan; 4]>),
}

struct Coder<'a> {
    src: &'a Frame,
    cdf: CdfContext,
    quant: Quant,
    lambda: f64,
}

pub fn encode_still(src: &Frame, qidx: u8) -> Result<Encoded, EncodeError> {
    let (width, height) = (src.width, src.height);
    if width == 0 || height == 0 || width % SB != 0 || height % SB != 0 {
        return Err(EncodeError::UnsupportedSize { width, height });
    }
    if qidx == 0 {
        return Err(EncodeError::UnsupportedQidx(qidx));
    }

    let dq = dq_lookup(qidx as i32) as u32;
    let step = quant_step_pixels(1, dq);
    let qcat = cdf_thread_init_static_qcat(qidx as u32) as usize;
    let mut coder = Coder {
        src,
        cdf: CdfContext::init_from_defaults(qcat, false),
        quant: Quant {
            dq_dc: dq,
            dq_ac: dq,
        },
        lambda: LAMBDA_PER_STEP2 * step * step,
    };
    let mut ec = RangeEncoder::new(true);
    let mut st = State {
        recon: Frame::new(width, height),
        ctx: Contexts::new(width),
    };

    for sby in 0..height / SB {
        st.ctx.reset_left();
        for sbx in 0..width / SB {
            let (x, y) = (sbx * SB, sby * SB);
            // Decide the partition with trial codings, then write it for real
            // from the state the superblock started in.
            let start = st.save(x, y, SB);
            let (_, plan) = coder.search(&mut st, x, y, SB);
            st.restore(&start, x, y, SB);
            coder.code(&mut ec, &mut st, x, y, SB, &plan);
        }
    }

    let params = StillParams {
        width: width as u32,
        height: height as u32,
        qidx,
    };
    let bitstream = still_picture_stream(&params, &ec.finish());
    Ok(Encoded {
        bitstream,
        recon: st.recon,
    })
}

impl Coder<'_> {
    /// Rate-distortion search over the partitions of a square block. Leaves
    /// `st` as the best choice coded it and returns its cost.
    fn search(&mut self, st: &mut State, x: usize, y: usize, size: usize) -> (f64, Plan) {
        let before = st.save(x, y, size);
        let mut bits = BitCounter::default();
        let dist = self.code(&mut bits, st, x, y, size, &Plan::Leaf);
        let leaf_cost = dist as f64 + self.lambda * bits.bits;
        if size == MIN_BLOCK {
            return (leaf_cost, Plan::Leaf);
        }
        let leaf = st.save(x, y, size);
        st.restore(&before, x, y, size);

        let h = size / 2;
        let mut bits = BitCounter::default();
        self.write_partition(&mut bits, st, x, y, square_bs(size), Some(false));
        let mut split_cost = 0.0;
        let mut children: Vec<Plan> = Vec::with_capacity(4);
        for hy in [y, y + h] {
            self.write_partition(&mut bits, st, x, hy, half_bs(size), Some(true));
            for cx in [x, x + h] {
                let (cost, plan) = self.search(st, cx, hy, h);
                split_cost += cost;
                children.push(plan);
            }
        }
        split_cost += self.lambda * bits.bits;

        if split_cost < leaf_cost {
            let plan: [Plan; 4] = children.try_into().unwrap_or_else(|_| unreachable!());
            (split_cost, Plan::Split(Box::new(plan)))
        } else {
            st.restore(&leaf, x, y, size);
            (leaf_cost, Plan::Leaf)
        }
    }

    /// Codes a square block as `plan` says and returns the distortion (sum of
    /// squared errors over the three planes).
    fn code<W: SymbolWriter>(
        &mut self,
        w: &mut W,
        st: &mut State,
        x: usize,
        y: usize,
        size: usize,
        plan: &Plan,
    ) -> u64 {
        match plan {
            Plan::Leaf => self.code_leaf(w, st, x, y, size),
            Plan::Split(children) => {
                let h = size / 2;
                self.write_partition(w, st, x, y, square_bs(size), Some(false));
                let mut dist = 0;
                for (i, hy) in [y, y + h].into_iter().enumerate() {
                    self.write_partition(w, st, x, hy, half_bs(size), Some(true));
                    dist += self.code(w, st, x, hy, h, &children[2 * i]);
                    dist += self.code(w, st, x + h, hy, h, &children[2 * i + 1]);
                }
                dist
            }
        }
    }

    /// The partition flags of one node of `decode_sb`: whether it splits
    /// and, if so, the direction (`Some(false)` H, `Some(true)` V).
    fn write_partition<W: SymbolWriter>(
        &mut self,
        w: &mut W,
        st: &State,
        x: usize,
        y: usize,
        bs: usize,
        split_dir: Option<bool>,
    ) {
        let b_dim = &BLOCK_DIMENSIONS[bs];
        let (x4, ly4) = (x / 4, (y % SB) / 4);
        let shift_a = (b_dim[2] as i32 - 1).max(0);
        let shift_l = (b_dim[3] as i32 - 1).max(0);
        let ctx1 = ((st.ctx.above_partition[x4] >> shift_a) & 1) as usize
            + ((st.ctx.left_partition[ly4] >> shift_l) & 1) as usize * 2;
        let [split_off, dir_off] = partition_ctx_offsets(bs);
        w.encode_bool_adapt(
            split_dir.is_some(),
            self.cdf.m.part_split(0, ctx1 + split_off * 4),
        );
        if let Some(vertical) = split_dir {
            w.encode_bool_adapt(vertical, self.cdf.m.part_dir(0, ctx1 + dir_off * 4));
        }
    }

    /// One leaf block: partition NONE, DC_PRED for luma and chroma, then the
    /// luma, U and V residuals.
    fn code_leaf<W: SymbolWriter>(
        &mut self,
        w: &mut W,
        st: &mut State,
        x: usize,
        y: usize,
        size: usize,
    ) -> u64 {
        let bs = square_bs(size);
        self.write_partition(w, st, x, y, bs, None);
        // DC_PRED: first set, first index; with DC everywhere no neighbour
        // carries a directional mode, so both contexts are 0.
        w.encode_symbol_adapt(0, self.cdf.m.intra_y_set(), 3);
        w.encode_symbol_adapt(0, self.cdf.m.intra_y_idx0(0), 7);
        w.encode_symbol_adapt(0, self.cdf.m.intra_uv_mode(0), 7);

        let (luma_tx, chroma_tx) = transforms(size);
        let (x4, ly4) = (x / 4, (y % SB) / 4);
        let n4 = size / 4;
        let mut dist = 0;

        let (res, _, d) = self.code_plane(w, st, 0, x, y, size, luma_tx, bs, 0);
        dist += d;
        st.ctx.above_luma[x4..x4 + n4].fill(res);
        st.ctx.left_luma[ly4..ly4 + n4].fill(res);

        let (cx4, cly4, cn4) = (x4 / 2, ly4 / 2, (n4 / 2).max(1));
        let mut u_has_cf = 0;
        for pl in 1..3 {
            let (res, coded, d) =
                self.code_plane(w, st, pl, x / 2, y / 2, size / 2, chroma_tx, bs, u_has_cf);
            dist += d;
            if pl == 1 {
                u_has_cf = coded as i32;
            }
            st.ctx.above_chroma[pl - 1][cx4..cx4 + cn4].fill(res);
            st.ctx.left_chroma[pl - 1][cly4..cly4 + cn4].fill(res);
        }

        let part = !(n4 as u8 - 1);
        st.ctx.above_partition[x4..x4 + n4].fill(part);
        st.ctx.left_partition[ly4..ly4 + n4].fill(part);
        dist
    }

    /// Predicts one plane of a block with DC_PRED, codes its residual and
    /// stores the decoder's reconstruction. Returns the coefficient context
    /// for its 4x4 units, whether anything was coded, and the distortion.
    #[allow(clippy::too_many_arguments)]
    fn code_plane<W: SymbolWriter>(
        &mut self,
        w: &mut W,
        st: &mut State,
        pl: usize,
        x: usize,
        y: usize,
        size: usize,
        tx: usize,
        bs: usize,
        u_has_cf: i32,
    ) -> (u8, bool, u64) {
        let stride = st.recon.plane_width(pl);
        let recon = &st.recon.planes[pl];
        let top =
            (y > 0).then(|| recon[(y - 1) * stride + x..(y - 1) * stride + x + size].to_vec());
        let left = (x > 0).then(|| {
            (0..size)
                .map(|i| recon[(y + i) * stride + x - 1])
                .collect::<Vec<_>>()
        });
        let mut out = vec![0u8; size * size];
        crate::shared::predict_dc(&mut out, size, top.as_deref(), left.as_deref());

        let src = &self.src.planes[pl];
        let residual: Vec<i32> = (0..size * size)
            .map(|k| src[(y + k / size) * stride + x + k % size] as i32 - out[k] as i32)
            .collect();
        let levels = quantize(&residual, tx, pl != 0, self.quant, ROUNDING);

        let (x4, ly4) = (x / 4, (y % (SB >> (pl != 0) as usize)) / 4);
        let ctx = if pl == 0 {
            CoefContext {
                plane: 0,
                bs,
                above: &st.ctx.above_luma[x4..],
                left: &st.ctx.left_luma[ly4..],
                u_has_cf,
            }
        } else {
            CoefContext {
                plane: pl,
                bs,
                above: &st.ctx.above_chroma[pl - 1][x4..],
                left: &st.ctx.left_chroma[pl - 1][ly4..],
                u_has_cf,
            }
        };
        let res = write_coefs(w, &mut self.cdf, &levels, &ctx);
        reconstruct(&levels, self.quant, &mut out, size);

        let mut dist = 0u64;
        let recon = &mut st.recon.planes[pl];
        for r in 0..size {
            let row = (y + r) * stride + x;
            for c in 0..size {
                let d = out[r * size + c] as i64 - src[row + c] as i64;
                dist += (d * d) as u64;
            }
            recon[row..row + size].copy_from_slice(&out[r * size..(r + 1) * size]);
        }
        (res, levels.eob >= 0, dist)
    }
}
