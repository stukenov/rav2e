# rav2e

**rav2e** is an AV2 video encoder in Rust.

> Pre-alpha. Milestone M0 is done: rav2e writes a legal AV2 still picture
> (8-bit 4:2:0, size a multiple of 64), one flat DC-predicted 64x64 block at
> a time. It is the worst legal encoder on purpose. The plan is in
> [STRATEGY.md](STRATEGY.md).

```sh
rav2e input.y4m -o output.obu --qidx 128 --recon recon.y4m
```

rav2e is written from scratch, the way [rav1e](https://github.com/xiph/rav1e)
was for AV1, and shares its reconstruction with
[rav2d](https://github.com/stukenov/rav2d), the Rust AV2 decoder. Every
frame it produces is checked in a closed loop: the encoder's own
reconstruction must match, byte for byte, what `avmdec` and rav2d decode
from its bitstream. CI runs that check with both decoders on every push.

## License

BSD 2-Clause.
