# The encoder's reconstruction must equal the decoder's output

An encoder is not normative, so there is no output to byte-compare against,
as rav2d did with dav2d. The check that replaces it: for every frame, the
picture the encoder reconstructs internally must equal what avmdec and rav2d
decode from the bitstream. A mismatch means the encoder predicts from
references the decoder never sees; the stream drifts while still decoding
without errors, so "it decodes" alone proves nothing.

Quality is measured separately, by BD-rate against avmenc.
See [[not-a-port-of-avmenc]].
