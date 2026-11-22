//! BitTorrent / magnet support as an adapter over `librqbit` implementing the
//! `pv-core` `Protocol` trait (ADR-006).
//!
//! **Boundary:** this is the only crate allowed to depend on `librqbit`.

/// The crate's package name, used by logging and diagnostics to identify the component.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(CRATE_NAME, "pv-torrent");
    }
}
