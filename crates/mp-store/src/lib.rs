//! SQLite persistence (sqlx, WAL mode): tasks, segments, torrent state and settings.
//!
//! **Boundary:** this is the only crate that touches the database (ADR-012).

/// The crate's package name, used by logging and diagnostics to identify the component.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(CRATE_NAME, "mp-store");
    }
}
