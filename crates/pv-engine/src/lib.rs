//! The download engine: an actor that owns all task state, schedules work under
//! concurrency and speed limits, persists through `pv-store` and recovers after crashes.
//!
//! Front ends talk to it only through [`EngineHandle`] commands (mpsc) and
//! `EngineEvent`s (broadcast), keeping it UI-agnostic (ADR-002).
//!
//! ```no_run
//! # async fn demo() -> Result<(), pv_engine::EngineError> {
//! use pv_engine::{Engine, EngineConfig};
//! let engine = Engine::start(EngineConfig::new("persivio.db")).await?;
//! let mut events = engine.subscribe();
//! // engine.add(spec).await?; while let Ok(e) = events.recv().await { ... }
//! engine.shutdown().await;
//! # Ok(()) }
//! ```

mod actor;
mod data_dir;
mod error;
mod handle;
pub mod limiter;

use std::path::PathBuf;
use std::sync::Arc;

use pv_core::{FailReason, Protocol, TaskState, TaskStateKind, UnixMillis};
use pv_http::HttpProtocol;
use pv_http::filename::{contained_path, part_name};
use pv_store::{Store, StoredTask};
use pv_torrent::TorrentProtocol;
use tokio::sync::{broadcast, mpsc};

pub use actor::PROGRESS_INTERVAL;
pub use data_dir::{DataDir, DataDirError, os_download_dir};
pub use error::EngineError;
pub use handle::{EngineHandle, MoveDirection, TaskDetail};
pub use pv_torrent::{MAX_TORRENT_FILE_SIZE, TorrentInfo, inspect_torrent};

/// How to start the engine.
#[derive(Clone)]
pub struct EngineConfig {
    /// SQLite database file (created if missing).
    pub db_path: PathBuf,
    /// Download protocols, tried in order. Defaults to HTTP and BitTorrent.
    pub protocols: Vec<Arc<dyn Protocol>>,
    /// Start queued tasks automatically. `false` lets a front end inspect or edit the
    /// queue (e.g. a CLI `list`) without downloading anything.
    pub autostart: bool,
}

impl EngineConfig {
    /// Defaults: HTTP and BitTorrent protocols, autostart on.
    #[must_use]
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        let db_path = db_path.into();
        let torrents = TorrentProtocol::with_work_dir(db_path.with_file_name("torrent-session"));
        Self {
            db_path,
            protocols: vec![Arc::new(HttpProtocol), Arc::new(torrents)],
            autostart: true,
        }
    }
}

/// Entry point.
pub struct Engine;

impl Engine {
    /// Opens the database, recovers interrupted tasks, and starts the actor.
    ///
    /// Recovery: tasks found in a running state (the app crashed or was shut down) become
    /// `Paused`, or are re-queued when `auto_resume_on_startup` is set. Their `.part`
    /// files are checked first; a missing or truncated one marks the task `Failed` with
    /// a clear reason instead of resuming into a corrupt file.
    ///
    /// # Errors
    /// Database open/migration/load failures.
    pub async fn start(config: EngineConfig) -> Result<EngineHandle, EngineError> {
        let store = Store::open(&config.db_path).await?;
        let settings = store.load_settings().await?;
        let (commands_tx, commands_rx) = mpsc::channel(64);
        let (internal_tx, internal_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(1024);

        let auto_resume = settings.auto_resume_on_startup;
        let mut actor = actor::Actor::new(
            store.clone(),
            settings,
            events.clone(),
            internal_tx,
            config.protocols,
            config.autostart,
        );

        let mut stored = store.load_all().await?;
        let mut queued: Vec<(i64, UnixMillis, pv_core::TaskId)> = Vec::new();
        stored.sort_by_key(|t| (t.snapshot.created_at, t.snapshot.id));
        for task in &stored {
            actor.tasks.insert(task.snapshot.id, task.snapshot.clone());
            actor.order.push(task.snapshot.id);
        }
        for task in &stored {
            let id = task.snapshot.id;
            let kind = task.snapshot.state.kind();
            if kind.is_running() {
                let next = match part_problem(task).await {
                    Some(reason) => TaskState::Failed {
                        reason,
                        retryable: true,
                    },
                    None => TaskState::Paused,
                };
                actor.set_state(id, next.clone()).await?;
                if auto_resume && next == TaskState::Paused {
                    actor.set_state(id, TaskState::Queued).await?;
                    queued.push((i64::MIN, task.snapshot.created_at, id)); // resumed first
                }
            } else if kind == TaskStateKind::Queued {
                queued.push((task.queue_order, task.snapshot.created_at, id));
            }
        }
        // Ended downloads need no credentials (also from versions before ADR-025).
        for task in &stored {
            if task.snapshot.state.kind().is_terminal() {
                actor.forget_secrets(task.snapshot.id).await;
            }
        }
        queued.sort();
        actor.queue = queued.into_iter().map(|(_, _, id)| id).collect();

        tokio::spawn(actor.run(commands_rx, internal_rx));
        Ok(EngineHandle {
            commands: commands_tx,
            events,
        })
    }
}

/// Checks that an interrupted task's `.part` file can back its persisted progress.
async fn part_problem(task: &StoredTask) -> Option<FailReason> {
    let needed = task
        .segments
        .iter()
        .map(|s| s.start + s.downloaded.min(s.len()))
        .max()
        .filter(|&n| n > 0)?;
    let name = task.snapshot.spec.file_name.as_deref()?;
    let part = contained_path(&task.snapshot.spec.dest_dir, &part_name(name)).ok()?;
    match tokio::fs::metadata(&part).await {
        Err(_) => Some(FailReason::PartFileMissing),
        Ok(meta) if meta.len() < needed => Some(FailReason::PartFileInconsistent {
            expected: needed,
            actual: meta.len(),
        }),
        Ok(_) => None,
    }
}
