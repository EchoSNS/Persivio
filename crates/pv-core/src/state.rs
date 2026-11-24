//! The task lifecycle state machine.
//!
//! ```text
//!            ┌──────────── resume ────────────┐
//!            ▼                                │
//!  ──► Queued ──► Probing ──────────┐      Paused ◄── (any running state)
//!        │                          ▼         │
//!        ├──► FetchingMetadata ──► Downloading ──► Verifying ──► Completed
//!        ├──► Checking ────────────► │               ▲
//!        │    (torrent resume)       │               │
//!        └──────────────────────────┤                │ (HTTP, no checksum: direct)
//!                                   └──► Seeding ────┘ (torrents)
//!
//!  Failed ──► Queued (retry)          Cancelled / Completed: terminal
//! ```
//!
//! [`transition`] is a pure function, so the engine, the store and the UI can all ask
//! "is this move legal?" without sharing any state.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Why a task failed. Serializable so it can be persisted and shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailReason {
    /// Connection, DNS, TLS or timeout problems.
    #[error("network error: {message}")]
    Network {
        /// Human-readable detail.
        message: String,
    },
    /// The server answered with an unexpected HTTP status.
    #[error("server responded with HTTP {status}")]
    HttpStatus {
        /// The status code.
        status: u16,
    },
    /// A signed/expiring URL stopped working mid-download (e.g. 403 after progress).
    #[error("download link expired")]
    LinkExpired,
    /// Not enough free space at the destination.
    #[error("{}", disk_full(*needed, *available))]
    DiskFull {
        /// Bytes required; 0 if unknown (the disk filled up while writing).
        needed: u64,
        /// Bytes available.
        available: u64,
    },
    /// Local filesystem error.
    #[error("file system error: {message}")]
    Io {
        /// Human-readable detail.
        message: String,
    },
    /// The finished file does not match the expected checksum.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected hex digest.
        expected: String,
        /// Computed hex digest.
        actual: String,
    },
    /// The source failed validation.
    #[error("invalid source: {message}")]
    InvalidSource {
        /// Human-readable detail.
        message: String,
    },
    /// On recovery, the `.part` file was missing.
    #[error("partial file is missing")]
    PartFileMissing,
    /// On recovery, the `.part` file size did not match the recorded progress.
    #[error("partial file is inconsistent: expected at least {expected} bytes, found {actual}")]
    PartFileInconsistent {
        /// Bytes the checkpoints claim.
        expected: u64,
        /// Bytes found on disk.
        actual: u64,
    },
    /// Error reported by the BitTorrent engine.
    #[error("torrent error: {message}")]
    Torrent {
        /// Human-readable detail.
        message: String,
    },
    /// A bug or unexpected condition inside Persivio.
    #[error("internal error: {message}")]
    Internal {
        /// Human-readable detail.
        message: String,
    },
}

/// The text for a full disk, with the numbers if they are known.
pub(crate) fn disk_full(needed: u64, available: u64) -> String {
    if needed == 0 {
        "the disk is full".to_owned()
    } else {
        format!("not enough disk space: {needed} bytes needed, {available} available")
    }
}

/// The lifecycle state of a task, with data where a state needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting for a concurrency slot.
    Queued,
    /// HTTP: discovering size, range support and validators.
    Probing,
    /// Torrent: resolving metadata from a magnet link.
    FetchingMetadata,
    /// Torrent: re-checking the pieces already on disk before continuing.
    Checking,
    /// Transferring data.
    Downloading,
    /// Stopped by the user (or by crash recovery); can be resumed.
    Paused,
    /// Checking the finished file against a checksum.
    Verifying,
    /// Torrent: download complete, uploading to peers.
    Seeding,
    /// Finished successfully.
    Completed,
    /// Stopped by an error.
    Failed {
        /// What went wrong.
        reason: FailReason,
        /// Whether retrying could plausibly succeed (e.g. network errors).
        retryable: bool,
    },
    /// Cancelled by the user.
    Cancelled,
}

/// [`TaskState`] without data: used for the transition table, DB columns and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStateKind {
    /// See [`TaskState::Queued`].
    Queued,
    /// See [`TaskState::Probing`].
    Probing,
    /// See [`TaskState::FetchingMetadata`].
    FetchingMetadata,
    /// See [`TaskState::Checking`].
    Checking,
    /// See [`TaskState::Downloading`].
    Downloading,
    /// See [`TaskState::Paused`].
    Paused,
    /// See [`TaskState::Verifying`].
    Verifying,
    /// See [`TaskState::Seeding`].
    Seeding,
    /// See [`TaskState::Completed`].
    Completed,
    /// See [`TaskState::Failed`].
    Failed,
    /// See [`TaskState::Cancelled`].
    Cancelled,
}

impl TaskStateKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 11] = [
        Self::Queued,
        Self::Probing,
        Self::FetchingMetadata,
        Self::Checking,
        Self::Downloading,
        Self::Paused,
        Self::Verifying,
        Self::Seeding,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
    ];

    /// Stable `snake_case` name (used as the DB column value).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Probing => "probing",
            Self::FetchingMetadata => "fetching_metadata",
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Verifying => "verifying",
            Self::Seeding => "seeding",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// No transitions leave this state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }

    /// The task is actively running and holds a scheduler slot.
    #[must_use]
    pub const fn is_running(self) -> bool {
        matches!(
            self,
            Self::Probing
                | Self::FetchingMetadata
                | Self::Checking
                | Self::Downloading
                | Self::Verifying
                | Self::Seeding
        )
    }

    /// Whether moving from `self` to `to` is legal. Self-transitions are never legal:
    /// the engine must not emit no-op state changes.
    #[must_use]
    pub const fn can_transition_to(self, to: Self) -> bool {
        use TaskStateKind as K;
        match self {
            // Leaving the queue: start (HTTP probe / torrent metadata), skip straight to
            // Downloading when metadata is already known (resume), or stop.
            K::Queued => matches!(
                to,
                K::Probing
                    | K::FetchingMetadata
                    | K::Checking
                    | K::Downloading
                    | K::Paused
                    | K::Failed
                    | K::Cancelled
            ),
            // Preparing: proceed, get preempted back to the queue, or stop.
            K::Probing | K::Checking => matches!(
                to,
                K::Downloading | K::Queued | K::Paused | K::Failed | K::Cancelled
            ),
            K::FetchingMetadata => matches!(
                to,
                K::Checking | K::Downloading | K::Queued | K::Paused | K::Failed | K::Cancelled
            ),
            // Downloading: finish via verify / directly / seeding, preempt, or stop.
            K::Downloading => matches!(
                to,
                K::Verifying
                    | K::Completed
                    | K::Seeding
                    | K::Queued
                    | K::Paused
                    | K::Failed
                    | K::Cancelled
            ),
            // Verifying: result, or interrupted (crash recovery pauses it).
            // Seeding: limits reached, or stop.
            K::Verifying | K::Seeding => {
                matches!(to, K::Completed | K::Paused | K::Failed | K::Cancelled)
            }
            // Paused: resume goes through the queue; recovery may fail it.
            K::Paused => matches!(to, K::Queued | K::Failed | K::Cancelled),
            // Failed: retry or give up.
            K::Failed => matches!(to, K::Queued | K::Cancelled),
            K::Completed | K::Cancelled => false,
        }
    }
}

impl fmt::Display for TaskStateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error for an unknown state name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown task state `{0}`")]
pub struct UnknownState(pub String);

impl FromStr for TaskStateKind {
    type Err = UnknownState;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| UnknownState(s.to_owned()))
    }
}

impl TaskState {
    /// The data-less kind of this state.
    #[must_use]
    pub const fn kind(&self) -> TaskStateKind {
        match self {
            Self::Queued => TaskStateKind::Queued,
            Self::Probing => TaskStateKind::Probing,
            Self::FetchingMetadata => TaskStateKind::FetchingMetadata,
            Self::Checking => TaskStateKind::Checking,
            Self::Downloading => TaskStateKind::Downloading,
            Self::Paused => TaskStateKind::Paused,
            Self::Verifying => TaskStateKind::Verifying,
            Self::Seeding => TaskStateKind::Seeding,
            Self::Completed => TaskStateKind::Completed,
            Self::Failed { .. } => TaskStateKind::Failed,
            Self::Cancelled => TaskStateKind::Cancelled,
        }
    }
}

/// An illegal state change was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid state transition from {from} to {to}")]
pub struct InvalidTransition {
    /// Current state.
    pub from: TaskStateKind,
    /// Requested state.
    pub to: TaskStateKind,
}

/// Checks whether a task may move from `from` to `to`.
///
/// # Errors
/// [`InvalidTransition`] if the move is not in the transition table.
pub fn transition(from: &TaskState, to: &TaskState) -> Result<(), InvalidTransition> {
    let (from, to) = (from.kind(), to.kind());
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(InvalidTransition { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full expected table, written independently of `can_transition_to`.
    /// Rows = from, columns = to, in `TaskStateKind::ALL` order:
    /// Que Pro Fet Che Dow Pau Ver See Com Fai Can
    const EXPECTED: [[u8; 11]; 11] = [
        /* Queued           */ [0, 1, 1, 1, 1, 1, 0, 0, 0, 1, 1],
        /* Probing          */ [1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1],
        /* FetchingMetadata */ [1, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1],
        /* Checking         */ [1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1],
        /* Downloading      */ [1, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1],
        /* Paused           */ [1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1],
        /* Verifying        */ [0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1],
        /* Seeding          */ [0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1],
        /* Completed        */ [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        /* Failed           */ [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        /* Cancelled        */ [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ];

    fn sample(kind: TaskStateKind) -> TaskState {
        match kind {
            TaskStateKind::Queued => TaskState::Queued,
            TaskStateKind::Probing => TaskState::Probing,
            TaskStateKind::FetchingMetadata => TaskState::FetchingMetadata,
            TaskStateKind::Checking => TaskState::Checking,
            TaskStateKind::Downloading => TaskState::Downloading,
            TaskStateKind::Paused => TaskState::Paused,
            TaskStateKind::Verifying => TaskState::Verifying,
            TaskStateKind::Seeding => TaskState::Seeding,
            TaskStateKind::Completed => TaskState::Completed,
            TaskStateKind::Failed => TaskState::Failed {
                reason: FailReason::HttpStatus { status: 404 },
                retryable: false,
            },
            TaskStateKind::Cancelled => TaskState::Cancelled,
        }
    }

    #[test]
    fn transition_table_covers_every_pair() {
        let mut checked = 0;
        for (i, from) in TaskStateKind::ALL.into_iter().enumerate() {
            for (j, to) in TaskStateKind::ALL.into_iter().enumerate() {
                let expected = EXPECTED[i][j] == 1;
                let actual = transition(&sample(from), &sample(to));
                assert_eq!(
                    actual.is_ok(),
                    expected,
                    "{from} -> {to}: expected {}",
                    if expected { "allowed" } else { "rejected" }
                );
                if let Err(e) = actual {
                    assert_eq!(e, InvalidTransition { from, to });
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 121);
    }

    #[test]
    fn terminal_states_have_no_exits_and_no_self_loops_exist() {
        for from in TaskStateKind::ALL {
            assert!(!from.can_transition_to(from), "{from} self-loop");
            if from.is_terminal() {
                assert!(
                    TaskStateKind::ALL
                        .iter()
                        .all(|&to| !from.can_transition_to(to))
                );
            }
        }
    }

    #[test]
    fn every_running_state_can_be_paused_cancelled_and_failed() {
        for from in TaskStateKind::ALL.into_iter().filter(|k| k.is_running()) {
            for to in [
                TaskStateKind::Paused,
                TaskStateKind::Cancelled,
                TaskStateKind::Failed,
            ] {
                assert!(from.can_transition_to(to), "{from} -> {to}");
            }
        }
    }

    #[test]
    fn kind_names_round_trip_and_match_serde() {
        for kind in TaskStateKind::ALL {
            assert_eq!(kind.as_str().parse::<TaskStateKind>().unwrap(), kind);
            assert_eq!(kind.to_string(), kind.as_str());
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{kind}\""));
            assert_eq!(sample(kind).kind(), kind);
            // TaskState's serde tag uses the same names.
            let json = serde_json::to_value(sample(kind)).unwrap();
            assert_eq!(json["state"], kind.as_str());
        }
        assert_eq!(
            "bogus".parse::<TaskStateKind>(),
            Err(UnknownState("bogus".into()))
        );
    }

    #[test]
    fn states_and_reasons_round_trip_through_json() {
        let reasons = [
            FailReason::Network {
                message: "reset".into(),
            },
            FailReason::HttpStatus { status: 503 },
            FailReason::LinkExpired,
            FailReason::DiskFull {
                needed: 10,
                available: 1,
            },
            FailReason::Io {
                message: "denied".into(),
            },
            FailReason::ChecksumMismatch {
                expected: "aa".into(),
                actual: "bb".into(),
            },
            FailReason::InvalidSource {
                message: "scheme".into(),
            },
            FailReason::PartFileMissing,
            FailReason::PartFileInconsistent {
                expected: 5,
                actual: 3,
            },
            FailReason::Torrent {
                message: "tracker".into(),
            },
            FailReason::Internal {
                message: "bug".into(),
            },
        ];
        for reason in reasons {
            assert_ne!(reason.to_string(), "");
            let state = TaskState::Failed {
                reason,
                retryable: true,
            };
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<TaskState>(&json).unwrap(), state);
        }
        for kind in TaskStateKind::ALL {
            let state = sample(kind);
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<TaskState>(&json).unwrap(), state);
        }
    }

    #[test]
    fn invalid_transition_message() {
        let err = transition(&TaskState::Completed, &TaskState::Queued).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid state transition from completed to queued"
        );
    }
}
