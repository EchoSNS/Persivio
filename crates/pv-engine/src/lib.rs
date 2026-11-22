//! The download engine: an actor that owns all task state, schedules work under
//! concurrency and speed limits, persists through `pv-store` and recovers after crashes.
//!
//! Front ends talk to it only through `EngineHandle` commands (mpsc) and
//! `EngineEvent`s (broadcast), keeping it UI-agnostic (ADR-002).

/// The crate's package name, used by logging and diagnostics to identify the component.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_matches_package() {
        assert_eq!(CRATE_NAME, "pv-engine");
    }
}
