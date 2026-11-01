# Changelog

## Unreleased

- M1a: full DCT_DCT coefficient coding (end of block, base and high-range
  tokens with their contexts, signs, Golomb remainders) with the decoder's
  dequantization and inverse transform. Luma PSNR on the test pictures rises
  from 17–18 dB to 25.8 dB.

## 0.1.0 — 2026-11-01

Milestone M0: the worst legal AV2 encoder.

- Encodes the first frame of an 8-bit 4:2:0 y4m as an AV2 still picture
  (reduced still-picture sequence header, one key frame, one tile). Width and
  height must be multiples of 64.
- Each 64x64 superblock is one DC_PRED block with a single DC coefficient
  (64x64 luma and 32x32 chroma transforms); the level is chosen by bisection
  on the reconstructed block.
- Range encoder mirroring rav2d's symbol decoder and AVM's `entenc.c`, with
  CDF adaptation (M0 streams keep CDFs frozen).
- `rav2e` command-line encoder with `--qidx` and `--recon`.
- Closed-loop test: rav2d and `avmdec` decode every test stream to the
  encoder's reconstruction, byte for byte.

## 0.0.1 — 2026-11-01

Name reservation; no API.
