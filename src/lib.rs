//! rav2e — an AV2 video encoder in Rust.
//!
//! Early development. [`encode_still`] writes a legal AV2 still picture
//! (8-bit 4:2:0, width and height multiples of 64) in which every 64x64
//! block is a DC prediction plus a fully coded DCT residual.
//! The reconstruction it returns is exactly what an AV2 decoder outputs for
//! the bitstream; the test suite checks that against rav2d and `avmdec`.
//!
//! ```
//! use rav2e::{Frame, encode_still};
//!
//! let mut frame = Frame::new(128, 64);
//! frame.planes[0].fill(100);
//! frame.planes[1].fill(128);
//! frame.planes[2].fill(128);
//!
//! let encoded = encode_still(&frame, 128).unwrap();
//! assert!(!encoded.bitstream.is_empty());
//! assert_eq!(encoded.recon.planes[0].len(), 128 * 64);
//! ```
//!
//! The plan and the milestones are in `STRATEGY.md` in the repository.

pub mod bitwriter;
pub mod coef;
pub mod ec;
pub mod encoder;
pub mod frame;
pub mod headers;
#[doc(hidden)]
pub mod shared;

pub use encoder::{EncodeError, Encoded, encode_still};
pub use frame::Frame;

/// Crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
