//! rav2e — an AV2 video encoder in Rust.
//!
//! Pre-alpha: nothing is encoded yet. See `STRATEGY.md` for the plan.

/// Crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_manifest() {
        assert_eq!(super::VERSION, "0.0.0");
    }
}
