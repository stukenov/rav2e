# Search the DC level by bisection, not from a one-step probe

The first level search measured how much one level of DC moves the block and
divided the residual by it. At fine quantizers one level dequantizes and
shifts to zero (qidx 1: (1 * 4 + 4) >> 5 = 0), the measured step was 0, and
every block was coded as skip. The streams were still legal and decoded
bit-exactly, so the closed-loop gate stayed green while the picture was
just the prediction.

Now the level is bisected on the sum of the reconstructed block, which grows
monotonically with the level, and the closed-loop test also asserts that at
fine quantizers the result is within 0.5 dB of the block-mean picture.
Matching the decoder proves the stream is right, not that it is any good.
See [[closed-loop-is-the-oracle]].
