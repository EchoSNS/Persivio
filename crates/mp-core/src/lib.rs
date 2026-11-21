//! Domain types, traits, events and errors shared by every Millipede crate.
//!
//! **Boundary:** this crate is pure. It performs no I/O and never spawns tasks;
//! everything here must be testable with plain `#[test]` functions.
//! The task state machine (`transition`) will live here (Phase 1).

/// The crate's package name, used by logging and diagnostics to identify the component.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(CRATE_NAME, "mp-core");
    }
}
