//! OBU, sequence header and frame header writers.
//!
//! M0 writes one shape of stream only: a reduced still-picture sequence, 8-bit
//! 4:2:0, 64x64 superblocks, one tile, every optional coding tool and in-loop
//! filter off. Each field below is the value rav2d's `parse_seq_hdr` /
//! `parse_frame_hdr` reads at that point for this configuration.

use crate::bitwriter::{BitWriter, put_uleb128};

pub const OBU_TEMPORAL_DELIMITER: u8 = 2;
pub const OBU_SEQUENCE_HEADER: u8 = 1;
pub const OBU_CLOSED_LOOP_KEY: u8 = 4;

/// Frames M0 can describe.
#[derive(Clone, Copy, Debug)]
pub struct StillParams {
    pub width: u32,
    pub height: u32,
    /// Base quantizer index, 1..=255 (0 would make the frame lossless).
    pub qidx: u8,
}

/// `obu_size` (uleb128) followed by the one-byte header and the payload.
pub fn obu(obu_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    put_uleb128(&mut out, payload.len() + 1);
    // has_extension = 0, obu_type (5 bits), tlayer_id = 0.
    out.push(obu_type << 2);
    out.extend_from_slice(payload);
    out
}

pub fn sequence_header(p: &StillParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.put_vlc(0); // seq_header_id
    w.put_bits(0, 5); // profile
    w.put_bit(true); // reduced_still_picture_header
    w.put_bits(0, 5); // level
    w.put_vlc(0); // chroma format: 4:2:0
    w.put_vlc(1); // bit depth: coded 1 means 8-bit
    let wbits = bits_for(p.width - 1);
    let hbits = bits_for(p.height - 1);
    w.put_bits(wbits - 1, 4);
    w.put_bits(hbits - 1, 4);
    w.put_bits(p.width - 1, wbits);
    w.put_bits(p.height - 1, hbits);
    w.put_bit(false); // cropping
    w.put_bit(false); // superblock 256
    w.put_bit(false); // superblock 128 (so 64x64)
    w.put_bit(false); // sdp
    w.put_bit(false); // ext_partitions
    w.put_bit(false); // max_pb_aspect_ratio signalled (no: 8:1)
    w.put_bit(false); // segmentation ext
    w.put_bit(false); // segmentation info present
    w.put_bit(false); // intra_dip
    w.put_bit(false); // intra_edge_filter
    w.put_bit(false); // mrls
    w.put_bit(false); // cfl
    w.put_bits(0, 2); // cfl_ds_filter_index
    w.put_bit(false); // mhccp
    w.put_bit(false); // ibp
    w.put_bit(false); // refmv_bank
    w.put_bit(true); // drl_reorder off
    w.put_uniform(0, 3); // def_max_bvp_drl_bits - 1
    w.put_bit(false); // allow_max_bvp_drl_bits
    w.put_bit(false); // bawp
    w.put_bit(false); // fsc
    w.put_bit(false); // idtx_intra
    w.put_bit(false); // ist intra
    w.put_bit(false); // ist inter
    w.put_bit(true); // chroma_dctonly
    w.put_bit(false); // reduced_tx_part_set
    w.put_bit(false); // cctx
    w.put_bit(false); // tcq
    w.put_bit(false); // parity_hiding
    w.put_bit(false); // separate_uv_delta_q
    w.put_bit(true); // equal_ac_dc_q
    w.put_bits(23, 5); // base_uvac_dq: coded value - 23 = 0
    w.put_bit(false); // uvac_dq_enabled
    w.put_bit(false); // disable_loopfilters_across_tiles
    w.put_bit(false); // cdef
    w.put_bit(false); // gdf
    w.put_bit(false); // restoration
    w.put_bit(false); // ccso
    w.put_bits(0, 2); // df_par_bits - 2
    w.put_bit(false); // tiling present
    w.put_bit(false); // film grain
    w.put_bit(false); // extension
    w.trailing_bits();
    w.into_bytes()
}

/// The frame-level bits of a closed-loop key frame OBU, up to and including
/// the byte alignment before the tile data.
pub fn frame_header(p: &StillParams) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.put_bit(true); // first tile group of the frame (implies a header)
    w.put_vlc(0); // multi-frame header id: none
    w.put_vlc(0); // sequence header id
    w.put_bit(false); // allow_screen_content_tools
    w.put_bit(false); // allow_intrabc
    w.put_bit(true); // disable_cdf_update
    // Tile info: uniform spacing, one tile.
    let sbw = p.width.div_ceil(64) as i32;
    let sbh = p.height.div_ceil(64) as i32;
    w.put_bit(true); // uniform
    let (min_log2_cols, max_log2_cols, min_log2_rows, max_log2_rows) = tile_log2_bounds(sbw, sbh);
    if min_log2_cols < max_log2_cols {
        w.put_bit(false); // no more columns
    }
    if min_log2_rows < max_log2_rows {
        w.put_bit(false); // no more rows
    }
    assert!(
        min_log2_cols == 0 && min_log2_rows == 0,
        "frame too large for one tile at level 0"
    );
    w.put_bits(p.qidx as u32, 8); // base_q_idx
    w.put_bit(false); // segmentation
    w.put_bit(false); // quantizer matrices
    w.put_bit(false); // delta_q present (base_q_idx > 0)
    w.put_bit(false); // deblocking luma vertical
    w.put_bit(false); // deblocking luma horizontal
    w.put_bit(false); // tx mode: largest
    w.put_bits(2, 2); // reduced_txtp_set
    w.byte_align();
    w.into_bytes()
}

/// A whole still-picture temporal unit around the entropy-coded tile data.
pub fn still_picture_stream(p: &StillParams, tile_data: &[u8]) -> Vec<u8> {
    let mut out = obu(OBU_TEMPORAL_DELIMITER, &[]);
    out.extend(obu(OBU_SEQUENCE_HEADER, &sequence_header(p)));
    let mut frame = frame_header(p);
    frame.extend_from_slice(tile_data);
    out.extend(obu(OBU_CLOSED_LOOP_KEY, &frame));
    out
}

fn bits_for(v: u32) -> u32 {
    (32 - v.leading_zeros()).max(1)
}

/// rav2d `tile_log2`: smallest k with `sz << k >= tgt`.
fn tile_log2(sz: i32, tgt: i32) -> i32 {
    let mut k = 0;
    while (sz << k) < tgt {
        k += 1;
    }
    k
}

/// Tile-count bounds for level 0, tier 0, 64x64 superblocks (rav2d
/// `parse_tile_info`).
fn tile_log2_bounds(sbw: i32, sbh: i32) -> (i32, i32, i32, i32) {
    const MAX_TILE_COLS: i32 = 64;
    const MAX_TILE_ROWS: i32 = 64;
    let sbsz_log2 = 6;
    let max_tile_width_sb = 4096 >> sbsz_log2;
    let max_tile_area_sb = (4096 * 2304) >> (2 * sbsz_log2);
    let min_log2_cols = tile_log2(max_tile_width_sb, sbw);
    let max_log2_cols = tile_log2(1, sbw.min(MAX_TILE_COLS));
    let max_log2_rows = tile_log2(1, sbh.min(MAX_TILE_ROWS));
    let min_log2_tiles = tile_log2(max_tile_area_sb, sbw * sbh).max(min_log2_cols);
    let min_log2_rows = (min_log2_tiles - min_log2_cols).max(0);
    (min_log2_cols, max_log2_cols, min_log2_rows, max_log2_rows)
}
