# Lambda is tuned on real video, not taken from the textbook

The partition search weighs distortion against bits with lambda = c * step²,
where step is the quantizer step in pixels. The high-rate textbook value
c = 0.34 gave no gain at all over ignoring rate (BD-rate 0.0%) on the first
frame of bus_cif, cropped to 320x256, at qidx 60–200. c = 0.05 gave -4.5%,
0.1 gave -4.8%, 0.2 gave -3.5%. rav2e uses 0.1.

The synthetic noise pictures of the closed-loop test cannot tune this: at
coarse quantizers the right answer there is to code nothing, so any lambda
looks plausible. Tuning needs real video and a BD-rate number; the clip lives
on the runner (`~/build/clips/bus_320x256.y4m`), not in the repository yet.
Revisit c whenever rate-distortion decisions are added (modes, RDOQ).
