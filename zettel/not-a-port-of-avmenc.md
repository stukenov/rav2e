# rav2e is written from scratch, not ported from avmenc

rav2d was a port of dav2d, about 54k lines of portable C. The AV2 reference
encoder in AVM is about 160k lines, plus 97k of common code, with ML models,
and it is slow by design. Porting it would cost several times rav2d and
deliver a slow encoder. rav1e faced the same choice with libaom and wrote its
own encoder, using libaom code only as temporary scaffolding.

avmenc stays a reference for algorithms and the baseline for BD-rate.
See [[closed-loop-is-the-oracle]].
