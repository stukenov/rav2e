//! Command-line front end: y4m in, AV2 OBU stream out.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::process::ExitCode;

const USAGE: &str = "usage: rav2e <input.y4m> -o <output.obu> [--qidx N] [--recon <recon.y4m>]

Encodes the first frame of an 8-bit 4:2:0 y4m file as an AV2 still picture.
Width and height must be multiples of 64. --qidx is the quantizer, 1..=255
(default 128).";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rav2e: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut input, mut output, mut recon_path, mut qidx) = (None, None, None, 128u8);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" | "--output" => output = args.next(),
            "--recon" => recon_path = args.next(),
            "--qidx" => qidx = args.next().ok_or(USAGE)?.parse()?,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if input.is_none() => input = Some(a),
            _ => return Err(format!("unexpected argument {a}\n\n{USAGE}").into()),
        }
    }
    let (input, output) = (input.ok_or(USAGE)?, output.ok_or(USAGE)?);

    let src = rav2e::frame::read_y4m_frame(&mut BufReader::new(File::open(&input)?))?;
    let enc = rav2e::encode_still(&src, qidx)?;
    File::create(&output)?.write_all(&enc.bitstream)?;
    if let Some(path) = recon_path {
        let mut w = BufWriter::new(File::create(path)?);
        rav2e::frame::write_y4m(&mut w, &[&enc.recon])?;
    }
    eprintln!(
        "{}x{} qidx {}: {} bytes",
        src.width,
        src.height,
        qidx,
        enc.bitstream.len()
    );
    Ok(())
}
