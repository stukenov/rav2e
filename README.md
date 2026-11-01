# rav2e

**rav2e** is an AV2 video encoder in Rust.

> Pre-alpha: it does not encode anything yet. The plan is in
> [STRATEGY.md](STRATEGY.md).

rav2e is written from scratch, the way [rav1e](https://github.com/xiph/rav1e)
was for AV1, and shares its reconstruction with
[rav2d](https://github.com/stukenov/rav2d), the Rust AV2 decoder. Every
frame it produces is checked in a closed loop: the encoder's own
reconstruction must match, byte for byte, what `avmdec` and rav2d decode
from its bitstream.

## License

BSD 2-Clause.
