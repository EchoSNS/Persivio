//! Domain types, traits, events and errors shared by every Persivio crate.
//!
//! **Boundary:** this crate is pure. It performs no I/O and never spawns tasks;
//! everything here must be testable with plain `#[test]` functions.
//!
//! Map of the crate:
//! - [`ids`]: [`TaskId`] and [`UnixMillis`] timestamps.
//! - [`source`]: where a download comes from ([`Source`]), with log-safe `Debug`.
//! - [`checksum`]: expected hashes ([`Checksum`]).
//! - [`state`]: the task state machine ([`TaskState`], [`transition`]).
//! - [`task`]: what the user asked for ([`TaskSpec`]) and what the engine reports
//!   ([`TaskSnapshot`]), plus [`Segment`] and [`FileEntry`].
//! - [`progress`]: lock-free progress counters ([`ProgressSink`]) and [`Progress`] samples.
//! - [`event`]: [`EngineEvent`]s broadcast to front ends.
//! - [`settings`]: user-tunable [`Settings`] with defaults and validation.
//! - [`protocol`]: the [`Protocol`] trait implemented by `pv-http` and `pv-torrent`.
//! - [`motw`]: Mark-of-the-Web text for downloaded files (Windows).

pub mod checksum;
pub mod event;
pub mod filename;
pub mod ids;
pub mod motw;
pub mod progress;
pub mod protocol;
pub mod settings;
pub mod source;
pub mod state;
pub mod task;

pub use checksum::{Checksum, ChecksumAlgorithm, ChecksumError};
pub use event::EngineEvent;
pub use ids::{TaskId, UnixMillis};
pub use progress::{Progress, ProgressSample, ProgressSink, SpeedMeter, eta_secs};
pub use protocol::{
    CheckpointError, CheckpointSink, Outcome, Protocol, ProtocolError, RateLimiter,
    ResolvedMetadata, ResumeState, RunContext, RunEvents, RunPhase, TorrentResume, Unlimited,
};
pub use settings::{DesktopSettings, Settings, SettingsError, TorrentSettings};
pub use source::{Source, SourceError};
pub use state::{FailReason, InvalidTransition, TaskState, TaskStateKind, transition};
pub use task::{
    FileEntry, HttpResumeInfo, MAX_CONNECTIONS, Segment, SpecError, TaskKind, TaskSnapshot,
    TaskSpec,
};
