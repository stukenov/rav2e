//! The M0 gate: the picture rav2e reconstructs equals what decoders make of
//! its bitstream, byte for byte, in every plane.
//!
//! rav2d always runs. `avmdec`, the AOM reference decoder, runs when
//! `RAV2E_AVMDEC` names its binary; set `RAV2E_REQUIRE_AVMDEC=1` to make its
//! absence a failure instead of a skip (CI does).

use std::path::PathBuf;
use std::process::Command;

use rav2d::{Data, Decoder, Rav2dError, Settings};
use rav2e::{Frame, encode_still};

/// Deterministic test pictures: smooth gradients, hard edges and noise.
fn test_picture(width: usize, height: usize, seed: u32) -> Frame {
    let mut f = Frame::new(width, height);
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % 41) as i32 - 20
    };
    for pl in 0..3 {
        let (pw, ph) = (f.plane_width(pl), f.plane_height(pl));
        for y in 0..ph {
            for x in 0..pw {
                let gradient = (x * 255 / pw.max(1)) as i32 / 2 + (y * 255 / ph.max(1)) as i32 / 3;
                let edge = if (x / 23 + y / 17) % 3 == 0 { 60 } else { 0 };
                let v = gradient + edge + noise() + 16 * pl as i32;
                f.planes[pl][y * pw + x] = v.clamp(0, 255) as u8;
            }
        }
    }
    f
}

fn decode_with_rav2d(bitstream: &[u8]) -> Frame {
    let mut dec = Decoder::open(&Settings::default()).expect("open rav2d");
    dec.send_data(Some(Data::wrap(bitstream.to_vec())))
        .expect("send data");
    let mut pictures = Vec::new();
    let mut drained = false;
    for _ in 0..64 {
        match dec.get_picture() {
            Ok(p) => pictures.push(p),
            Err(Rav2dError::Again) if !drained => {
                dec.send_data(None).expect("drain");
                drained = true;
            }
            Err(Rav2dError::Again) | Err(Rav2dError::Eof) => break,
            Err(e) => panic!("rav2d failed to decode the stream: {e}"),
        }
    }
    assert_eq!(pictures.len(), 1, "expected exactly one picture");
    let p = &pictures[0];
    let (w, h) = p.plane_dimensions(0).expect("luma plane");
    let mut f = Frame::new(w, h);
    for pl in 0..3 {
        let pw = f.plane_width(pl);
        for (y, row) in p.plane_rows_u8(pl).enumerate() {
            f.planes[pl][y * pw..(y + 1) * pw].copy_from_slice(&row[..pw]);
        }
    }
    f
}

fn avmdec() -> Option<PathBuf> {
    match std::env::var_os("RAV2E_AVMDEC") {
        Some(p) => Some(PathBuf::from(p)),
        None => {
            assert!(
                std::env::var_os("RAV2E_REQUIRE_AVMDEC").is_none(),
                "RAV2E_REQUIRE_AVMDEC is set but RAV2E_AVMDEC is not"
            );
            None
        }
    }
}

fn decode_with_avmdec(avmdec: &PathBuf, bitstream: &[u8], name: &str) -> Vec<u8> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let obu = dir.join(format!("{name}.obu"));
    let yuv = dir.join(format!("{name}.yuv"));
    std::fs::write(&obu, bitstream).unwrap();
    let out = Command::new(avmdec)
        .arg("--rawvideo")
        .arg("-o")
        .arg(&yuv)
        .arg(&obu)
        .output()
        .expect("run avmdec");
    assert!(
        out.status.success(),
        "avmdec rejected {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read(&yuv).unwrap()
}

fn assert_planes_equal(what: &str, expected: &Frame, actual: &Frame) {
    assert_eq!(
        (expected.width, expected.height),
        (actual.width, actual.height),
        "{what}: size"
    );
    for pl in 0..3 {
        let pw = expected.plane_width(pl);
        if let Some(i) =
            (0..expected.planes[pl].len()).find(|&i| expected.planes[pl][i] != actual.planes[pl][i])
        {
            panic!(
                "{what}: plane {pl} differs first at x={} y={}: encoder {} decoder {}",
                i % pw,
                i / pw,
                expected.planes[pl][i],
                actual.planes[pl][i]
            );
        }
    }
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mse = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| (x as f64 - y as f64).powi(2))
        .sum::<f64>()
        / a.len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

const CASES: &[(usize, usize, u8)] = &[
    (64, 64, 128),
    (128, 128, 1),
    (128, 128, 60),
    (192, 128, 128),
    (256, 192, 255),
    (320, 256, 100),
];

#[test]
fn rav2d_decodes_the_encoder_reconstruction() {
    for (i, &(w, h, qidx)) in CASES.iter().enumerate() {
        let src = test_picture(w, h, i as u32 + 1);
        let enc = encode_still(&src, qidx).unwrap();
        let decoded = decode_with_rav2d(&enc.bitstream);
        assert_planes_equal(&format!("{w}x{h} q{qidx} rav2d"), &enc.recon, &decoded);
        let got = psnr(&src.planes[0], &enc.recon.planes[0]);
        let ideal = psnr(&src.planes[0], &block_means(&src.planes[0], w, h, 64));
        eprintln!(
            "{w}x{h} qidx {qidx}: {} bytes, luma PSNR {got:.2} dB (block means: {ideal:.2} dB)",
            enc.bitstream.len()
        );
        // At fine quantizers one DC per 64x64 block should come close to the
        // best a flat block can do: the mean of the source.
        if qidx <= 60 {
            assert!(
                got >= ideal - 0.5,
                "{w}x{h} q{qidx}: {got:.2} dB, block means give {ideal:.2} dB"
            );
        }
    }
}

/// Each `n`x`n` block replaced by its rounded mean.
fn block_means(plane: &[u8], w: usize, h: usize, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; plane.len()];
    for by in (0..h).step_by(n) {
        for bx in (0..w).step_by(n) {
            let sum: u64 = (by..by + n)
                .flat_map(|y| (bx..bx + n).map(move |x| plane[y * w + x] as u64))
                .sum();
            let mean = ((sum + (n * n / 2) as u64) / (n * n) as u64) as u8;
            for y in by..by + n {
                out[y * w + bx..y * w + bx + n].fill(mean);
            }
        }
    }
    out
}

#[test]
fn avmdec_decodes_the_encoder_reconstruction() {
    let Some(avmdec) = avmdec() else {
        eprintln!("skipped: RAV2E_AVMDEC is not set");
        return;
    };
    for (i, &(w, h, qidx)) in CASES.iter().enumerate() {
        let src = test_picture(w, h, i as u32 + 1);
        let enc = encode_still(&src, qidx).unwrap();
        let raw = decode_with_avmdec(&avmdec, &enc.bitstream, &format!("m0-{w}x{h}-q{qidx}"));
        let expected: Vec<u8> = enc.recon.planes.concat();
        assert_eq!(
            raw.len(),
            expected.len(),
            "{w}x{h} q{qidx}: avmdec output size"
        );
        let decoded = {
            let mut f = Frame::new(w, h);
            let (ysz, csz) = (w * h, (w / 2) * (h / 2));
            f.planes[0].copy_from_slice(&raw[..ysz]);
            f.planes[1].copy_from_slice(&raw[ysz..ysz + csz]);
            f.planes[2].copy_from_slice(&raw[ysz + csz..]);
            f
        };
        assert_planes_equal(&format!("{w}x{h} q{qidx} avmdec"), &enc.recon, &decoded);
    }
}

#[test]
fn rejects_sizes_m0_cannot_code() {
    assert!(encode_still(&Frame::new(100, 64), 128).is_err());
    assert!(encode_still(&Frame::new(64, 64), 0).is_err());
}
