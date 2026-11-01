//! 8-bit 4:2:0 frames and a minimal y4m reader/writer.

use std::io::{self, BufRead, Write};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// Planes Y, U, V, each tightly packed (stride = plane width).
    pub planes: [Vec<u8>; 3],
}

impl Frame {
    pub fn new(width: usize, height: usize) -> Self {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        Self {
            width,
            height,
            planes: [vec![0; width * height], vec![0; cw * ch], vec![0; cw * ch]],
        }
    }

    pub fn plane_width(&self, pl: usize) -> usize {
        if pl == 0 {
            self.width
        } else {
            self.width.div_ceil(2)
        }
    }

    pub fn plane_height(&self, pl: usize) -> usize {
        if pl == 0 {
            self.height
        } else {
            self.height.div_ceil(2)
        }
    }
}

/// Reads the first frame of an 8-bit 4:2:0 y4m stream.
pub fn read_y4m_frame(r: &mut impl BufRead) -> io::Result<Frame> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut header = String::new();
    r.read_line(&mut header)?;
    let mut fields = header.split_whitespace();
    if fields.next() != Some("YUV4MPEG2") {
        return Err(bad("not a y4m stream"));
    }
    let (mut width, mut height) = (0usize, 0usize);
    for f in fields {
        match f.split_at(1) {
            ("W", v) => width = v.parse().map_err(|_| bad("bad width"))?,
            ("H", v) => height = v.parse().map_err(|_| bad("bad height"))?,
            ("C", v) if !v.starts_with("420") => {
                return Err(bad("only 8-bit 4:2:0 y4m is supported"));
            }
            _ => {}
        }
    }
    if width == 0 || height == 0 {
        return Err(bad("missing frame size"));
    }
    let mut frame_line = String::new();
    r.read_line(&mut frame_line)?;
    if !frame_line.starts_with("FRAME") {
        return Err(bad("missing FRAME marker"));
    }
    let mut f = Frame::new(width, height);
    for pl in &mut f.planes {
        r.read_exact(pl)?;
    }
    Ok(f)
}

pub fn write_y4m(w: &mut impl Write, frames: &[&Frame]) -> io::Result<()> {
    let first = frames.first().expect("at least one frame");
    writeln!(
        w,
        "YUV4MPEG2 W{} H{} F25:1 Ip A0:0 C420jpeg",
        first.width, first.height
    )?;
    for f in frames {
        w.write_all(b"FRAME\n")?;
        for pl in &f.planes {
            w.write_all(pl)?;
        }
    }
    Ok(())
}
