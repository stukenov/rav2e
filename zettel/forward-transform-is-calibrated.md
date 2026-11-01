# The forward transform is measured from the decoder's inverse

Only the inverse transform has to be bit-exact; the forward one just has to
put the coefficients near the right values. Instead of a hand-scaled forward
DCT, rav2e measures each transform size once: it inverse-transforms an
impulse at every coefficient position with the decoder's own code and
projects residuals onto those measured basis functions.

That way the scale factors, the per-size shifts and AV2's rule for 64-point
transforms come out of the decoder's arithmetic instead of being re-derived.
The 64-point rule was a surprise: AV2 codes 32x32 coefficients of a 64x64
transform, reconstructs at 32x32 and upsamples by 2, so one 64x64 transform
cannot carry detail finer than 2x2 pixels. That set the 25.8 dB ceiling of
M1a and is why partitioning comes next. See [[closed-loop-is-the-oracle]].
