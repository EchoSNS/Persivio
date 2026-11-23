//! The error every command returns, as the UI sees it.

use pv_engine::EngineError;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Broad error category, for the UI to pick a message style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The input was rejected (bad URL, bad setting, ...).
    InvalidInput,
    /// No such task.
    NotFound,
    /// Not possible in the task's current state.
    InvalidState,
    /// The engine is not running (e.g. another Persivio process holds the data folder).
    Unavailable,
    /// Anything else (database, file system).
    Internal,
}

/// A failed command: a category and a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommandError {
    /// Category.
    pub kind: ErrorKind,
    /// Message to show.
    pub message: String,
}

impl CommandError {
    /// An [`ErrorKind::InvalidInput`] error.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::InvalidInput,
            message: message.into(),
        }
    }

    /// An [`ErrorKind::Unavailable`] error.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Unavailable,
            message: message.into(),
        }
    }
}

impl From<EngineError> for CommandError {
    fn from(e: EngineError) -> Self {
        let kind = match &e {
            EngineError::Stopped => ErrorKind::Unavailable,
            EngineError::NotFound(_) => ErrorKind::NotFound,
            EngineError::InvalidTransition(_) | EngineError::NotATorrent(_) => {
                ErrorKind::InvalidState
            }
            EngineError::Spec(_) | EngineError::Settings(_) | EngineError::NoProtocol => {
                ErrorKind::InvalidInput
            }
            EngineError::Store(_) | EngineError::Io(_) => ErrorKind::Internal,
        };
        Self {
            kind,
            message: e.to_string(),
        }
    }
}
