//! Deterministic test infrastructure: in-process HTTP servers (ranges, throttling,
//! failures, redirects, ...) and fixture helpers with known SHA-256 hashes.
//!
//! **Boundary:** used only as a `dev-dependency`; never linked into shipped binaries.

/// The crate's package name, used by logging and diagnostics to identify the component.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(CRATE_NAME, "mp-testkit");
    }
}
