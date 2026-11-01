# rav2e takes rav2d from the encoder-internals branch

rav2d's modules are crate-private. The `encoder-internals` feature makes
them public (doc-hidden, no semver promise), and rav2e calls the decoder's
own CDF tables, context helpers, DC prediction and inverse transform, so its
reconstruction cannot disagree with the decoder by construction.

The feature lives on a branch, not on rav2d's main, because main's working
tree had another session's work in progress when it was added. Cargo.lock
pins the commit. Merge the branch into main and switch the dependency to a
release once that work lands.
