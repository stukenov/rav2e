//! rav2e — an AV2 video encoder in Rust.
//!
//! Milestone M0 (see `STRATEGY.md`): it writes a legal AV2 still picture whose
//! reconstruction matches what a decoder produces, and nothing more. Each
//! 64x64 block is a flat DC prediction plus one DC coefficient.

pub mod bitwriter;
pub mod ec;
pub mod encoder;
pub mod frame;
pub mod headers;

pub use encoder::{EncodeError, Encoded, encode_still};
pub use frame::Frame;

/// Crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
