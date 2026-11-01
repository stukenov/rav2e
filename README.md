# rav2e

[![crates.io](https://img.shields.io/crates/v/rav2e.svg)](https://crates.io/crates/rav2e)
[![docs.rs](https://img.shields.io/docsrs/rav2e)](https://docs.rs/rav2e)
[![CI](https://github.com/stukenov/rav2e/actions/workflows/ci.yml/badge.svg)](https://github.com/stukenov/rav2e/actions/workflows/ci.yml)
[![License: BSD-2-Clause](https://img.shields.io/badge/license-BSD--2--Clause-blue.svg)](LICENSE)

**rav2e** is an AV2 video encoder in Rust, written from scratch the way
[rav1e](https://github.com/xiph/rav1e) was for AV1. It is the encoding
counterpart of [rav2d](https://github.com/stukenov/rav2d), the Rust AV2
decoder.

> **Status: early.** rav2e writes a legal AV2 still picture: square blocks
> from 64x64 to 8x8 chosen by rate-distortion cost, each a DC prediction
> plus a fully coded DCT residual (milestones M1a–M1b). The other intra
> modes come next. Each step builds on a
> stream that is already proven correct. See [STRATEGY.md](STRATEGY.md).

## What works today

| | |
|---|---|
| Input | 8-bit 4:2:0 y4m, first frame; width and height multiples of 64 |
| Output | AV2 low-overhead OBU stream: temporal delimiter, sequence header, one key frame |
| Coding | square blocks 64x64 to 8x8 by RD search, DC_PRED, DCT_DCT residual, one tile |
| Quantizer | `qidx` 1–255 |
| Verified with | rav2d and `avmdec` (the AOM reference decoder), byte for byte |

Not yet: rectangular blocks, other intra modes and transform types,
rate-distortion optimised quantization, inter frames, in-loop filters, 10-bit, rate control.

## Quick start

```sh
cargo install rav2e
rav2e input.y4m -o output.obu --qidx 128 --recon recon.y4m
```

`--recon` writes the picture the encoder reconstructed, which is what any
AV2 decoder will output for `output.obu`.

As a library:

```rust
use rav2e::{Frame, encode_still};

let mut frame = Frame::new(128, 64);
frame.planes[0].fill(100); // Y
frame.planes[1].fill(128); // U
frame.planes[2].fill(128); // V

let encoded = encode_still(&frame, 128).unwrap();
std::fs::write("out.obu", &encoded.bitstream).unwrap();
// encoded.recon is what a decoder will show
```

## How correctness is checked

An encoder has no single correct output, so rav2e cannot be compared byte by
byte against a reference encoder the way rav2d is against dav2d. What it can
be checked against is a **closed loop**: the picture the encoder
reconstructs internally must equal, byte for byte, what decoders produce from
its bitstream. If the two differ, the encoder predicts from pictures the
decoder never sees, and the stream drifts while still decoding without
errors.

`tests/closed_loop.rs` encodes six pictures (64x64 to 320x256, `qidx` 1 to
255) and decodes each with rav2d and with `avmdec`; both must match the
encoder's reconstruction in every plane. CI builds `avmdec` from the
[AVM](https://github.com/AOMediaCodec/avm) repository at a pinned commit and
fails if it is missing. The same test checks quality separately: decoding
bit-exactly proves the stream is right, not that it is any good.

`tests/ec_roundtrip.rs` checks the range coder on its own: 64,000 random
symbols, bypass bits and Golomb codes, written by rav2e and read back by
rav2d's symbol decoder, with probability adaptation on and off.

## How it is built

rav2e follows rav1e's path: the simplest legal stream first, then one coding
tool at a time, each checked by the decoder and measured by BD-rate.
[STRATEGY.md](STRATEGY.md) has the milestones (M0 legal stream → M1 intra →
M2 inter → M3 in-loop filters → M4 usable → M5 fast) and how rav1e did it.

The tables and formulas the encoder shares with the decoder (CDFs,
coefficient contexts, dequantizer, prediction, inverse transform) are copied
from rav2d into `src/shared/`, each file naming its source, so the encoder
reconstructs exactly what a decoder will. Non-obvious decisions are recorded
as short notes in [`zettel/`](zettel/).

| Path | |
|---|---|
| `src/encoder.rs` | partition search, prediction, residual, reconstruction |
| `src/coef.rs` | forward transform, quantization, coefficient syntax |
| `src/ec.rs` | range encoder (the mirror of rav2d's symbol decoder) and bit counter |
| `src/headers.rs`, `src/bitwriter.rs` | OBU, sequence and frame headers |
| `src/frame.rs` | frames and y4m input/output |
| `src/shared/` | code shared with the decoder, copied from rav2d |
| `src/bin/rav2e.rs` | command-line encoder |

## Building and testing

```sh
cargo build --release
cargo test                      # rav2d checks every stream
RAV2E_AVMDEC=/path/to/avmdec cargo test   # avmdec checks it too
```

To build `avmdec`:

```sh
git clone https://github.com/AOMediaCodec/avm.git
cmake -S avm -B avm/build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DENABLE_TESTS=0 -DENABLE_DOCS=0 -DCONFIG_TENSORFLOW_LITE=0
ninja -C avm/build avmdec
```

## Development

rav2e is written with heavy use of AI coding tools (Claude Code), as rav2d
was. Correctness does not rest on trusting them: every stream is decoded by
two independent decoders in CI, and the encoder's reconstruction must match
both.

## Related projects

- [rav2d](https://github.com/stukenov/rav2d) — AV2 decoder in Rust
- [rav1e](https://github.com/xiph/rav1e) — AV1 encoder in Rust
- [AVM](https://github.com/AOMediaCodec/avm) — the AV2 reference codec

## License

BSD 2-Clause, the same as rav2d.
