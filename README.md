# Persivio

A desktop download manager: a Rust engine with multi-connection segmented HTTP downloads,
resume, queueing, speed limits and BitTorrent/magnet support (via `librqbit`), with a
Tauri v2 + Svelte 5 desktop UI.

> **Status:** Phase 5 — a working headless download manager: queue, speed limits, crash recovery and a CLI.

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

See [CONTRIBUTING.md](CONTRIBUTING.md) for project rules and the definition of done.

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

## Using the CLI

```sh
persivio add https://example.com/file.iso            # add and download, with progress bars
persivio add URL1 URL2 --dir D:/ISOs -c 8 --no-wait  # only queue
persivio list                                          # all downloads and their state
persivio pause <id|all>     persivio resume <id|all>
persivio cancel <id>        persivio rm <id> [--delete-files]
persivio watch                                         # run the queue until it is empty
persivio settings show
persivio settings set global_speed_limit_bps 2000000   # or `none`
```

Ids can be shortened to any unique fragment shown by `list`. Ctrl+C stops gracefully;
interrupted downloads resume on the next `watch` (or automatically with
`settings set auto_resume_on_startup true`).

## License

Dual-licensed under MIT or Apache-2.0, at your option.
