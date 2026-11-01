# The code shared with the decoder is copied from rav2d, not linked

rav2e has to reconstruct exactly what a decoder will, so it needs rav2d's
CDF tables, coefficient contexts, dequantizer, DC prediction and inverse
transform. rav2d keeps all of them crate-private.

A feature in rav2d that made its modules public was tried and dropped: it
tied every rav2e change to a rav2d release and kept rav2e off crates.io while
it pointed at a git branch. Instead `src/shared/` holds copies, each file
naming the rav2d file and commit (aab9d2d) it came from. rav2d stays a
dev-dependency through its public `Decoder` API only, as the test oracle.

The risk is drift: a rav2d fix that rav2e does not pick up. The closed-loop
test decodes every stream with rav2d and avmdec, so drift shows up as a
failing test, not as a silently broken stream. See
[[closed-loop-is-the-oracle]].
