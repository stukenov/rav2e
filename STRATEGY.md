# Strategy

rav2e is an AV2 encoder written from scratch in Rust. It is not a port of
`avmenc`. It follows the path rav1e took for AV1, with one advantage rav1e
did not have: a Rust decoder for the same codec,
[rav2d](https://github.com/stukenov/rav2d), already bit-exact against the
reference.

## How rav1e was built

Read from the rav1e git history (`xiph/rav1e`, 4288 commits):

| When | What |
|---|---|
| 2017-09-24 | First commit by Thomas Daede. A small Rust driver plus libaom C files copied in (entropy coder, forward and inverse transforms, intra prediction), built by `build.rs`; the same day libaom became a submodule. README: *"The fastest and worstest AV1 compressor"*, input must be a multiple of 64. Output checked by decoding it with `aomdec`. |
| 2018-02 | C pieces replaced by Rust one at a time (V_PRED, H_PRED, ...). Lambda and quantizers tuned against AWCY (arewecompressedyet.com, BD-rate on a fixed clip set). |
| 2018-05 | Inter frames and motion search. |
| 2018-07 | Automated encode→decode tests across many frame sizes. Rust forward transforms. |
| 2018-09/10 | C API. dav1d's motion-compensation assembly reused. |
| 2019-01 | Rate control API; weekly release tags begin. |
| 2019-09 | Fuzzing. |
| 2019–2020 | Last libaom code removed from the encoder. |

The lessons:

1. **Valid before good.** The first encoder wrote the simplest legal
   bitstream and nothing else. Every tool after that was added only when the
   encoder already worked end to end.
2. **The decoder is the test.** Every output was decoded by the reference
   decoder from the first day.
3. **Borrow, then replace.** Reference code was used as scaffolding and
   swapped out piece by piece, each swap checked against the same decoder.
4. **Measure every tool.** A tool goes in when it lowers BD-rate on a fixed
   clip set, at a known speed cost.
5. **Reuse the decoder's assembly.** The inverse side of the encoder is the
   decoder; its SIMD serves both.

## How rav2e applies them

| rav1e | rav2e |
|---|---|
| libaom C via FFI as scaffolding | rav2d's Rust reconstruction: intra and inter prediction, inverse transforms, in-loop filters, CDF and quantizer tables, MV prediction — already bit-exact against dav2d and avmdec |
| `aomdec` decodes every output | `avmdec` and rav2d decode every output |
| AWCY | a local BD-rate harness: rav2e vs `avmenc` (and vs rav1e for AV1 context) on a fixed clip set |
| dav1d asm | rav2d's vendored NEON (MC, intra) |

### The oracle

An encoder is not normative: there is no "correct" output to match, so the
rav2d method (byte-compare against the C reference) does not carry over.
What does carry over is the **closed loop**: the picture the encoder
reconstructs internally must be byte-identical to what `avmdec` and rav2d
decode from its bitstream, for every frame of every test. If they differ,
the encoder's reference frames drift and the stream is broken, even if it
decodes. This check is automatic and is the gate for every change.

Quality is a separate, measured property: BD-rate (PSNR, SSIM) against
`avmenc` on a fixed clip set, recorded per milestone.

### What has to be written

rav2d gives the reconstruction half. The encoder half is new:

- range encoder (the mirror of rav2d's `msac.rs`);
- OBU, sequence and frame header writers;
- the symbol writer for every syntax element (the mirror of rav2d's
  `decode.rs` and coefficient reading in `recon.rs`);
- forward transforms, including the secondary transform and CCTX;
- quantization, with parity hiding done on the encoder side;
- mode decision and RDO, partition search;
- motion estimation;
- parameter search for deblocking, CDEF, CCSO, loop restoration and GDF;
- rate control, lookahead, GOP structure;
- threading (tiles first).

`avmenc` (BSD-3-Clause) may be read for algorithms. It is not ported
wholesale: about 160k lines of research encoder plus 97k of common code,
with ML models, and its speed is not a target.

## Milestones

Each milestone ends with an exit check that runs in CI.

**M0 — the worst legal encoder.**
y4m in, OBU/IVF out. 8-bit 4:2:0, one tile, frame size a multiple of the
superblock. Keyframes only, one fixed partition, DC prediction, DCT only,
fixed quantizer, every in-loop filter off.
*Exit:* `avmdec` and rav2d decode every frame; their output equals the
encoder's reconstruction byte for byte.

**M1 — intra.**
All intra modes chosen by RDO, partition search, transform types and sizes,
lambda from the quantizer, parity hiding, 10-bit. The BD-rate harness
exists.
*Exit:* closed loop green on the clip set; intra BD-rate vs `avmenc`
recorded.

**M2 — inter.**
Single-reference P frames with motion search (integer, then subpel through
rav2d's MC), a fixed GOP, then compound and B frames, MV prediction through
rav2d's `refmvs`.
*Exit:* closed loop green on multi-frame clips; BD-rate recorded.

**M3 — in-loop filters.**
Parameter search for deblocking, CDEF, CCSO, loop restoration and GDF.
*Exit:* closed loop green with filters on; BD-rate gain per filter recorded.

**M4 — usable.**
Rate control (constant quality, then target bitrate), speed presets,
lookahead, tile threading, a stable Rust API, a CLI, a C API (`cargo-c`),
fuzzing of the API and the y4m reader, release cadence.
*Exit:* encodes a 1080p clip at a stated speed; CLI and API published.

**M5 — fast.**
Forward-side SIMD (SAD, SATD, forward transforms) of our own; reuse
rav2d's NEON for reconstruction; x86.

## Decisions to take at M0

- **How rav2e reaches rav2d's internals.** All rav2d modules are
  `pub(crate)` today. Recommended: a feature in rav2d (say
  `encoder-internals`) that makes the needed modules public, with rav2e
  depending on it by git or path; split out a shared crate only when the
  interface settles. The alternative, a separate `rav2-common` crate now, is
  cleaner but costs a refactor of rav2d before any encoder code exists.
- **Where `avmdec` comes from in CI.** Built from the AVM repository at a
  pinned commit and cached, as rav2d does with the vector generator.

## Not goals

- Matching `avmenc` output.
- Competing with SVT-AV2 on speed before M4.
- A safety argument as strong as rav2d's: an encoder reads raw frames, not
  hostile bitstreams. rav2e's case is a Rust-native AV2 encoder with no C
  toolchain, sharing its reconstruction with rav2d.

## Risks

- AV2 has many more coding tools than AV1; M1–M3 are each large.
- SVT-AV2 is reported to be in development at Intel and will likely lead on
  speed.
- rav2d's internals were shaped for decoding; reusing them may need
  refactoring there first.
