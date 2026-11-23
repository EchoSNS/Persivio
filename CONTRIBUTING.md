# Contributing to Persivio

Read [docs/architecture.md](docs/architecture.md) and [docs/adr/](docs/adr/) before making
structural changes.

## Workspace

| Crate | Role and boundary |
|---|---|
| `crates/pv-core` | Pure domain types, traits and errors. **No I/O, no `tokio::spawn`.** |
| `crates/pv-http` | HTTP protocol implementation. |
| `crates/pv-torrent` | librqbit adapter. **Only crate allowed to depend on librqbit.** |
| `crates/pv-store` | sqlx SQLite. **Only crate that touches the database.** |
| `crates/pv-engine` | Actor and scheduler. Depends on core, http, torrent, store. |
| `crates/pv-testkit` | Test servers and fixtures; dev-dependency only. |
| `apps/cli`, `apps/desktop` | Thin adapters over `pv-engine`. |

## Rules

- Libraries use `thiserror`; applications use `anyhow`.
- No `unwrap`/`expect` outside tests unless justified with a comment.
- Never block in async code; never hold `std` mutex guards across `.await`.
- The segment HTTP client is HTTP/1.1-only and sends `Accept-Encoding: identity`.
- Durability: write data, `sync_data`, **then** persist the checkpoint.
- Every file name from a server or torrent goes through `sanitize_filename` and a path
  containment check.
- Progress to the UI is batched, at most 10 Hz.
- Every bug fix starts with a failing test.
- Add or update an ADR for any non-obvious design decision.
- Secrets (cookies, headers, URL passwords, torrent bytes) never cross into the webview;
  the UI gets its own types (ADR-018). A new desktop command is added in three places:
  the command, `build.rs` and `capabilities/main.json`.

## Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
cargo +nightly fuzz run content_range   # optional, needs cargo-fuzz (runs in CI)
```

Desktop app (`apps/desktop`): `npm run check`, `npm test`, `npm run tauri dev`. Build the
frontend once (`npm run build`) before compiling the workspace.

## Definition of done

fmt, clippy and tests pass; new behavior has tests; docs and ADRs are updated if needed.
