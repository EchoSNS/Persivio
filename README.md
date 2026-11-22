# Persivio

A desktop download manager: a Rust engine with multi-connection segmented HTTP downloads,
resume, queueing, speed limits and BitTorrent/magnet support (via `librqbit`), with a
Tauri v2 + Svelte 5 desktop UI.

> **Status:** Phase 2 — domain model, SQLite persistence and deterministic HTTP test servers. No downloads yet.

## Layout

| Path | Purpose |
|---|---|
| `crates/pv-core` | Domain types, traits, events, errors (no I/O) |
| `crates/pv-http` | HTTP protocol: probe, segments, writer, retry |
| `crates/pv-torrent` | BitTorrent adapter over librqbit |
| `crates/pv-store` | SQLite persistence (sqlx) |
| `crates/pv-engine` | Engine actor: queue, scheduler, limits, recovery |
| `crates/pv-testkit` | Test servers and fixtures (dev-only) |
| `apps/cli` | Headless `persivio` CLI |
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
cargo run -p persivio-cli -- --help
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.
