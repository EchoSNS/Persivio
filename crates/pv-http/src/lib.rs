//! HTTP/HTTPS download protocol: probing, filename resolution, segment planning,
//! segment workers, the single positional writer and retry policy.
//!
//! **Rules:** segment clients are HTTP/1.1-only and send `Accept-Encoding: identity`
//! (ADR-003, ADR-011); data is synced before checkpoints are persisted (ADR-005).

/// The crate's package name, used by logging and diagnostics to identify the component.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(CRATE_NAME, "pv-http");
    }
}
