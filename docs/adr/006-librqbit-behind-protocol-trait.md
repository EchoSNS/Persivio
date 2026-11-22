# ADR-006: librqbit behind the Protocol trait

- **Status:** Accepted
- **Date:** 2026-11-22

## Context
Writing a production BitTorrent client is a project of its own. `librqbit` is a capable Rust
implementation, but its API evolves between versions and we may want to swap or upgrade it.

## Decision
BitTorrent support lives in `pv-torrent`, an adapter that implements `pv-core`'s `Protocol`
trait. `pv-torrent` is the **only** crate allowed to depend on `librqbit`; no librqbit type
crosses its public API.

## Consequences
- librqbit upgrades or a replacement touch one crate.
- The engine treats HTTP and torrent tasks uniformly.
- Some librqbit features may be awkward to surface through a generic trait; extensions go through
  `pv-core` types, never leaked librqbit types.
