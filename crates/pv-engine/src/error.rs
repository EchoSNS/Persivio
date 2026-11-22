//! Engine errors returned to front ends.

use pv_core::{InvalidTransition, SettingsError, SpecError, TaskId};
use pv_store::StoreError;

/// Why an engine command failed.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine has shut down (or crashed); no more commands are accepted.
    #[error("the engine is not running")]
    Stopped,
    /// No task with this id.
    #[error("task {0} not found")]
    NotFound(TaskId),
    /// The command is not valid in the task's current state (e.g. pausing a finished task).
    #[error(transparent)]
    InvalidTransition(#[from] InvalidTransition),
    /// The task specification is invalid.
    #[error("invalid download: {0}")]
    Spec(#[from] SpecError),
    /// The settings are invalid.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// The operation only applies to torrents.
    #[error("task {0} is not a torrent")]
    NotATorrent(TaskId),
    /// No registered protocol handles this source.
    #[error("no protocol can download this source")]
    NoProtocol,
    /// Persistence failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// File-system error (e.g. deleting files on remove).
    #[error("file system error: {0}")]
    Io(#[from] std::io::Error),
}
