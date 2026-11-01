# M0 writes a reduced still picture with frozen CDFs

The first stream uses the smallest corner of AV2 syntax, so that every field
written can be checked against rav2d's parser line by line:

- `reduced_still_picture_header`: one key frame, no reference or motion
  fields in either header.
- Every optional tool and in-loop filter off in the sequence header, one
  tile, 64x64 superblocks, `txfm_mode` largest.
- `disable_cdf_update`: symbol probabilities stay at their defaults, so the
  encoder and the decoder cannot drift apart through adaptation.

Adaptation is implemented anyway and its round trip against rav2d's
`MsacContext` is tested, because M1 will need it.

Each block is DC_PRED plus a DC-only coefficient: the eob-zero path of
`decode_coefs`, which needs no scan, no transform-type symbol and no AC
context. That kept M0 to four mode symbols and a handful of coefficient
symbols per block. See [[closed-loop-is-the-oracle]].
