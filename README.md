# Millipede

A desktop download manager: a Rust engine with multi-connection segmented HTTP downloads,
resume, queueing, speed limits and BitTorrent/magnet support (via `librqbit`), with a
Tauri v2 + Svelte 5 desktop UI.

> **Status:** Phase 1 — domain model (`mp-core`) and SQLite persistence (`mp-store`). No downloads yet.

## Layout

| Path | Purpose |
|---|---|
| `crates/mp-core` | Domain types, traits, events, errors (no I/O) |
| `crates/mp-http` | HTTP protocol: probe, segments, writer, retry |
| `crates/mp-torrent` | BitTorrent adapter over librqbit |
| `crates/mp-store` | SQLite persistence (sqlx) |
| `crates/mp-engine` | Engine actor: queue, scheduler, limits, recovery |
| `crates/mp-testkit` | Test servers and fixtures (dev-only) |
| `apps/cli` | Headless `millipede` CLI |
| `docs/` | [Architecture](docs/architecture.md), [ADRs](docs/adr/) |

## Prerequisites

- Rust via [rustup](https://rustup.rs) (the `rust-toolchain.toml` selects stable + rustfmt + clippy)
- Windows: Visual Studio C++ Build Tools
- `cargo install --locked cargo-nextest cargo-deny`

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
cargo run -p millipede-cli -- --help
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.
