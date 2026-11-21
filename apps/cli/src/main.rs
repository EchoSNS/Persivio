//! `millipede` — headless debug/dev client that runs the engine in-process.
//!
//! Subcommands (`add`, `list`, `pause`, `resume`, `cancel`, `rm`, `watch`,
//! `settings`) arrive in Phase 5 (with `anyhow` for error reporting); for now this
//! only parses arguments.

use clap::Parser;

/// Command-line interface definition.
#[derive(Debug, Parser)]
#[command(
    name = "millipede",
    version,
    about = "Millipede download manager (headless client)"
)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!(
        "millipede {} (engine: {}) — no commands implemented yet",
        env!("CARGO_PKG_VERSION"),
        mp_engine::CRATE_NAME
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Catches clap definition mistakes (duplicate flags, bad defaults) at test time
    /// instead of at runtime.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
