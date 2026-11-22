//! SQLite persistence (sqlx, WAL mode): tasks, segments, torrent state and settings.
//!
//! **Boundary:** this is the only crate that touches the database (ADR-012). It stores
//! and loads; it does not decide. Business rules such as "is this state transition
//! legal?" live in `pv-core` and are enforced by the engine before calling the store.
//!
//! Every query is a `&'static str` literal with bound parameters. sqlx 0.9 enforces
//! this at compile time (`SqlSafeStr`), which rules out string-built SQL injection.

mod convert;
mod error;
mod store;

pub use error::StoreError;
pub use store::{SETTINGS_KEY, Store, StoredTask};

#[cfg(test)]
mod tests;
