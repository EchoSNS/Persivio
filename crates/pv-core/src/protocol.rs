//! The seam between the engine and download protocols (HTTP, BitTorrent).
//!
//! The engine owns scheduling, persistence and events; a [`Protocol`] only moves bytes.
//! Everything a protocol needs from the outside world arrives through [`RunContext`]
//! as traits, so protocols can be tested with in-memory fakes and the engine never
//! depends on protocol internals (ADR-006).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::ids::TaskId;
use crate::progress::ProgressSink;
use crate::settings::Settings;
use crate::source::Source;
use crate::state::{FailReason, TaskState};
use crate::task::{FileEntry, HttpResumeInfo, Segment, TaskSnapshot};

/// A download protocol implementation.
///
/// Uses `async_trait` (boxed futures) rather than native `async fn` in traits because
/// the engine stores protocols as `Arc<dyn Protocol>`, which needs dyn-compatibility.
#[async_trait]
pub trait Protocol: Send + Sync {
    /// Whether this protocol can download `source`.
    fn handles(&self, source: &Source) -> bool;

    /// Runs the task until it completes, fails, or `ctx.cancel` fires.
    ///
    /// Must return promptly after cancellation, having flushed and checkpointed what it
    /// wrote (ADR-005). Pause vs. cancel is decided by the engine, not the protocol.
    async fn run(&self, ctx: RunContext) -> Result<Outcome, ProtocolError>;

    /// Releases shared resources (sockets, sessions) at engine shutdown. Default: nothing.
    async fn shutdown(&self) {}

    /// The user changed the settings while this protocol may be running. Must not block.
    /// Default: nothing (new runs get the new settings anyway).
    fn settings_changed(&self, _settings: &Settings) {}
}

/// How a run ended without error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// All data is downloaded (and verified, if requested) at `path`.
    Completed {
        /// Final file (or torrent root directory).
        path: PathBuf,
    },
    /// The cancellation token fired; progress is checkpointed.
    Stopped,
}

/// Running sub-phases a protocol reports, mapped onto [`TaskState`] by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    /// HTTP probe in flight.
    Probing,
    /// Torrent metadata being resolved.
    FetchingMetadata,
    /// Torrent: re-checking pieces already on disk.
    Checking,
    /// Data transfer.
    Downloading,
    /// Checksum verification.
    Verifying,
    /// Torrent finished downloading, uploading to peers.
    Seeding,
}

impl RunPhase {
    /// The task state this phase corresponds to.
    #[must_use]
    pub const fn to_state(self) -> TaskState {
        match self {
            Self::Probing => TaskState::Probing,
            Self::FetchingMetadata => TaskState::FetchingMetadata,
            Self::Checking => TaskState::Checking,
            Self::Downloading => TaskState::Downloading,
            Self::Verifying => TaskState::Verifying,
            Self::Seeding => TaskState::Seeding,
        }
    }
}

/// Facts a protocol learned about the download (probe result / torrent metadata).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedMetadata {
    /// Sanitized file (or torrent) name.
    pub name: Option<String>,
    /// Total size in bytes.
    pub total_size: Option<u64>,
    /// File list (torrents).
    pub files: Vec<FileEntry>,
    /// HTTP validators for safe resume.
    pub http: Option<HttpResumeInfo>,
}

/// Non-persistent notifications from a running protocol to the engine.
///
/// Methods are synchronous and must not block: implementations forward to the actor's
/// channel. They take `&self` so one `Arc<dyn RunEvents>` can be shared by workers.
pub trait RunEvents: Send + Sync {
    /// The run entered a new phase.
    fn phase(&self, phase: RunPhase);
    /// Metadata became known (the engine persists it and emits `MetadataResolved`).
    fn metadata(&self, meta: ResolvedMetadata);
    /// A non-fatal warning for the user.
    fn warning(&self, message: String);
}

/// A checkpoint could not be persisted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("checkpoint failed: {0}")]
pub struct CheckpointError(pub String);

/// Durable progress storage used by protocols.
///
/// **Contract (ADR-005):** callers must only pass positions whose bytes have already been
/// written *and* `sync_data`'d. Implementations persist atomically (one transaction).
#[async_trait]
pub trait CheckpointSink: Send + Sync {
    /// Replaces the task's whole segment table (new plan, or after a dynamic split).
    async fn save_segments(&self, id: TaskId, segments: &[Segment]) -> Result<(), CheckpointError>;

    /// Updates `downloaded` for existing segments: `(index, downloaded)` pairs.
    async fn save_progress(
        &self,
        id: TaskId,
        progress: &[(u32, u64)],
    ) -> Result<(), CheckpointError>;

    /// Stores resolved torrent metadata. The default does nothing (HTTP-only sinks).
    async fn save_torrent(
        &self,
        _id: TaskId,
        _info_hash: &str,
        _torrent_bytes: &[u8],
    ) -> Result<(), CheckpointError> {
        Ok(())
    }

    /// Stores how much of a torrent is on disk, and (only after its data was synced) the
    /// protocol's opaque record of *which* parts (`None` clears it). The default does
    /// nothing (HTTP-only sinks).
    async fn save_torrent_progress(
        &self,
        _id: TaskId,
        _downloaded: u64,
        _have: Option<&[u8]>,
    ) -> Result<(), CheckpointError> {
        Ok(())
    }
}

/// A byte-rate limiter shared by all workers of a task (and globally).
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Waits until `bytes` may be consumed. Implementations must accept amounts larger
    /// than their burst size (by waiting proportionally), never error.
    async fn acquire(&self, bytes: u64);
}

/// A limiter that never waits.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unlimited;

#[async_trait]
impl RateLimiter for Unlimited {
    async fn acquire(&self, _bytes: u64) {}
}

/// Persisted torrent metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentResume {
    /// Hex info hash.
    pub info_hash: String,
    /// The bencoded `.torrent` (metainfo) bytes.
    pub torrent_bytes: Vec<u8>,
    /// Which parts are on disk, as saved by [`CheckpointSink::save_torrent_progress`]
    /// when the last run stopped cleanly.
    pub have: Option<Vec<u8>>,
}

/// Where a resumed run picks up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeState {
    /// HTTP validators from the previous probe.
    pub http: Option<HttpResumeInfo>,
    /// Persisted segment table (empty for a fresh start).
    pub segments: Vec<Segment>,
    /// Torrent metadata from a previous run (so magnets need no second metadata fetch).
    pub torrent: Option<TorrentResume>,
}

/// Everything a protocol run receives from the engine.
#[derive(Clone)]
pub struct RunContext {
    /// The task being run.
    pub task: TaskSnapshot,
    /// Prior progress, if resuming.
    pub resume: ResumeState,
    /// Settings at the moment the run started.
    pub settings: Arc<Settings>,
    /// Fires on pause/cancel/shutdown.
    pub cancel: CancellationToken,
    /// Live counters read by the engine ticker.
    pub progress: ProgressSink,
    /// Combined global + per-task limiter.
    pub limiter: Arc<dyn RateLimiter>,
    /// Durable checkpoint storage.
    pub checkpoint: Arc<dyn CheckpointSink>,
    /// Phase/metadata/warning notifications.
    pub events: Arc<dyn RunEvents>,
}

/// A protocol run failed.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Connection/DNS/TLS/timeout failure.
    #[error("network error: {0}")]
    Network(String),
    /// Unexpected HTTP status.
    #[error("server responded with HTTP {status}")]
    HttpStatus {
        /// The status code.
        status: u16,
    },
    /// A signed URL expired mid-download.
    #[error("download link expired")]
    LinkExpired,
    /// Not enough space for the file.
    #[error("not enough disk space: {needed} bytes needed, {available} available")]
    DiskFull {
        /// Bytes required.
        needed: u64,
        /// Bytes available.
        available: u64,
    },
    /// Local I/O failure.
    #[error("file system error: {0}")]
    Io(#[from] std::io::Error),
    /// Verification failed.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },
    /// The source cannot be handled.
    #[error("invalid source: {0}")]
    InvalidSource(String),
    /// BitTorrent engine error.
    #[error("torrent error: {0}")]
    Torrent(String),
    /// Progress could not be persisted.
    #[error(transparent)]
    Checkpoint(#[from] CheckpointError),
}

impl ProtocolError {
    /// Whether an automatic retry could plausibly succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            // 408 Request Timeout, 429 Too Many Requests, and server errors.
            Self::HttpStatus { status } => matches!(*status, 408 | 429 | 500..=599),
            _ => false,
        }
    }

    /// The persisted reason for this error.
    #[must_use]
    pub fn fail_reason(&self) -> FailReason {
        match self {
            Self::Network(message) => FailReason::Network {
                message: message.clone(),
            },
            Self::HttpStatus { status } => FailReason::HttpStatus { status: *status },
            Self::LinkExpired => FailReason::LinkExpired,
            Self::DiskFull { needed, available } => FailReason::DiskFull {
                needed: *needed,
                available: *available,
            },
            Self::Io(e) => FailReason::Io {
                message: e.to_string(),
            },
            Self::ChecksumMismatch { expected, actual } => FailReason::ChecksumMismatch {
                expected: expected.clone(),
                actual: actual.clone(),
            },
            Self::InvalidSource(message) => FailReason::InvalidSource {
                message: message.clone(),
            },
            Self::Torrent(message) => FailReason::Torrent {
                message: message.clone(),
            },
            Self::Checkpoint(e) => FailReason::Internal {
                message: e.to_string(),
            },
        }
    }

    /// The `Failed` state for this error.
    #[must_use]
    pub fn to_failed_state(&self) -> TaskState {
        TaskState::Failed {
            reason: self.fail_reason(),
            retryable: self.is_retryable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::task::TaskSpec;

    #[tokio::test]
    async fn unlimited_never_waits() {
        let limiter: Arc<dyn RateLimiter> = Arc::new(Unlimited);
        limiter.acquire(u64::MAX).await;
    }

    #[test]
    fn phases_map_to_running_states() {
        for phase in [
            RunPhase::Probing,
            RunPhase::FetchingMetadata,
            RunPhase::Checking,
            RunPhase::Downloading,
            RunPhase::Verifying,
            RunPhase::Seeding,
        ] {
            assert!(phase.to_state().kind().is_running(), "{phase:?}");
        }
    }

    #[test]
    fn errors_map_to_reasons_and_retryability() {
        let cases: Vec<(ProtocolError, bool)> = vec![
            (ProtocolError::Network("reset".into()), true),
            (ProtocolError::HttpStatus { status: 503 }, true),
            (ProtocolError::HttpStatus { status: 429 }, true),
            (ProtocolError::HttpStatus { status: 408 }, true),
            (ProtocolError::HttpStatus { status: 404 }, false),
            (ProtocolError::LinkExpired, false),
            (
                ProtocolError::DiskFull {
                    needed: 2,
                    available: 1,
                },
                false,
            ),
            (std::io::Error::other("denied").into(), false),
            (
                ProtocolError::ChecksumMismatch {
                    expected: "a".into(),
                    actual: "b".into(),
                },
                false,
            ),
            (ProtocolError::InvalidSource("x".into()), false),
            (ProtocolError::Torrent("x".into()), false),
            (CheckpointError("db locked".into()).into(), false),
        ];
        for (err, retryable) in cases {
            assert_eq!(err.is_retryable(), retryable, "{err}");
            let TaskState::Failed {
                reason,
                retryable: r,
            } = err.to_failed_state()
            else {
                panic!("not a failed state");
            };
            assert_eq!(r, retryable);
            // The persisted reason reads the same as the error for the user.
            assert_eq!(
                reason.to_string(),
                err.to_string()
                    .replace("checkpoint failed", "internal error: checkpoint failed")
            );
        }
    }

    /// Demonstrates the fakes protocol tests will use, and that `RunContext` is buildable
    /// from trait objects alone.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl RunEvents for Recorder {
        fn phase(&self, phase: RunPhase) {
            self.0.lock().unwrap().push(format!("{phase:?}"));
        }
        fn metadata(&self, meta: ResolvedMetadata) {
            self.0.lock().unwrap().push(format!("meta:{:?}", meta.name));
        }
        fn warning(&self, message: String) {
            self.0.lock().unwrap().push(message);
        }
    }

    #[async_trait]
    impl CheckpointSink for Recorder {
        async fn save_segments(&self, _id: TaskId, s: &[Segment]) -> Result<(), CheckpointError> {
            self.0.lock().unwrap().push(format!("segments:{}", s.len()));
            Ok(())
        }
        async fn save_progress(
            &self,
            _id: TaskId,
            p: &[(u32, u64)],
        ) -> Result<(), CheckpointError> {
            self.0.lock().unwrap().push(format!("progress:{}", p.len()));
            Ok(())
        }
    }

    struct Fake;

    #[async_trait]
    impl Protocol for Fake {
        fn handles(&self, source: &Source) -> bool {
            matches!(source, Source::Magnet { .. })
        }
        async fn run(&self, ctx: RunContext) -> Result<Outcome, ProtocolError> {
            ctx.events.phase(RunPhase::Downloading);
            ctx.events.metadata(ResolvedMetadata {
                name: Some("n".into()),
                ..Default::default()
            });
            ctx.events.warning("w".into());
            ctx.checkpoint.save_segments(ctx.task.id, &[]).await?;
            ctx.checkpoint.save_progress(ctx.task.id, &[(0, 1)]).await?;
            ctx.limiter.acquire(10).await;
            ctx.progress.add_downloaded(10);
            if ctx.cancel.is_cancelled() {
                return Ok(Outcome::Stopped);
            }
            Ok(Outcome::Completed {
                path: ctx.task.spec.dest_dir.clone(),
            })
        }
    }

    #[tokio::test]
    async fn protocol_runs_against_fakes() {
        let recorder = Arc::new(Recorder::default());
        let task = TaskSnapshot::new(TaskSpec::new(
            Source::magnet("magnet:?xt=1").unwrap(),
            std::env::temp_dir(),
        ));
        let ctx = RunContext {
            task,
            resume: ResumeState::default(),
            settings: Arc::new(Settings::default()),
            cancel: CancellationToken::new(),
            progress: ProgressSink::new(),
            limiter: Arc::new(Unlimited),
            checkpoint: recorder.clone(),
            events: recorder.clone(),
        };
        let protocol: Arc<dyn Protocol> = Arc::new(Fake);
        assert!(protocol.handles(&ctx.task.spec.source));
        assert!(!protocol.handles(&Source::TorrentFile { bytes: vec![1] }));

        let progress = ctx.progress.clone();
        let outcome = protocol.run(ctx.clone()).await.unwrap();
        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(progress.sample().downloaded, 10);

        ctx.cancel.cancel();
        assert_eq!(protocol.run(ctx).await.unwrap(), Outcome::Stopped);

        let log = recorder.0.lock().unwrap().clone();
        assert_eq!(
            &log[..5],
            [
                "Downloading",
                "meta:Some(\"n\")",
                "w",
                "segments:0",
                "progress:1"
            ]
        );
    }
}
