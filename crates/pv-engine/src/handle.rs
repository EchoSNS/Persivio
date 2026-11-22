//! [`EngineHandle`]: the only way front ends talk to the engine (ADR-002).

use pv_core::{EngineEvent, Settings, TaskId, TaskSnapshot, TaskSpec};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::EngineError;

/// Reply channel for a command.
pub(crate) type Reply<T> = oneshot::Sender<Result<T, EngineError>>;

/// Where to move a queued task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    /// One position earlier.
    Up,
    /// One position later.
    Down,
    /// First in the queue.
    Top,
    /// Last in the queue.
    Bottom,
}

/// Commands processed by the engine actor.
pub(crate) enum Command {
    Add(Box<TaskSpec>, Reply<TaskId>),
    Pause(TaskId, Reply<()>),
    Resume(TaskId, Reply<()>),
    Cancel(TaskId, Reply<()>),
    Remove {
        id: TaskId,
        delete_files: bool,
        reply: Reply<()>,
    },
    PauseAll(Reply<()>),
    ResumeAll(Reply<()>),
    Move {
        id: TaskId,
        direction: MoveDirection,
        reply: Reply<()>,
    },
    SelectFiles {
        id: TaskId,
        files: Option<Vec<u32>>,
        reply: Reply<()>,
    },
    SetSettings(Box<Settings>, Reply<()>),
    GetSettings(Reply<Settings>),
    List(Reply<Vec<TaskSnapshot>>),
    Shutdown(oneshot::Sender<()>),
}

/// A cloneable handle to a running engine. All methods are cheap message sends; the
/// actor processes them one at a time, so there is no shared mutable state to lock.
#[derive(Clone)]
pub struct EngineHandle {
    pub(crate) commands: mpsc::Sender<Command>,
    pub(crate) events: broadcast::Sender<EngineEvent>,
}

impl EngineHandle {
    async fn call<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(make(reply))
            .await
            .map_err(|_| EngineError::Stopped)?;
        rx.await.map_err(|_| EngineError::Stopped)?
    }

    /// Adds a download. It is queued and starts when a slot is free.
    ///
    /// # Errors
    /// Invalid spec, unsupported source, persistence failure, or a stopped engine.
    pub async fn add(&self, spec: TaskSpec) -> Result<TaskId, EngineError> {
        self.call(|r| Command::Add(Box::new(spec), r)).await
    }

    /// Pauses a queued or running task (progress is kept).
    ///
    /// # Errors
    /// Unknown task, a state that cannot be paused, or a stopped engine.
    pub async fn pause(&self, id: TaskId) -> Result<(), EngineError> {
        self.call(|r| Command::Pause(id, r)).await
    }

    /// Puts a paused or failed task back in the queue.
    ///
    /// # Errors
    /// Unknown task, a state that cannot be resumed, or a stopped engine.
    pub async fn resume(&self, id: TaskId) -> Result<(), EngineError> {
        self.call(|r| Command::Resume(id, r)).await
    }

    /// Cancels a task for good and deletes its partial file.
    ///
    /// # Errors
    /// Unknown task, an already finished task, or a stopped engine.
    pub async fn cancel(&self, id: TaskId) -> Result<(), EngineError> {
        self.call(|r| Command::Cancel(id, r)).await
    }

    /// Removes a task from the list. Partial files are always deleted; the finished
    /// file only with `delete_files`.
    ///
    /// # Errors
    /// Unknown task, I/O or persistence failure, or a stopped engine.
    pub async fn remove(&self, id: TaskId, delete_files: bool) -> Result<(), EngineError> {
        self.call(|reply| Command::Remove {
            id,
            delete_files,
            reply,
        })
        .await
    }

    /// Pauses every queued and running task.
    ///
    /// # Errors
    /// A stopped engine.
    pub async fn pause_all(&self) -> Result<(), EngineError> {
        self.call(Command::PauseAll).await
    }

    /// Re-queues every paused and failed task.
    ///
    /// # Errors
    /// A stopped engine.
    pub async fn resume_all(&self) -> Result<(), EngineError> {
        self.call(Command::ResumeAll).await
    }

    /// Changes a queued task's position (manual priority).
    ///
    /// # Errors
    /// Unknown or not-queued task, or a stopped engine.
    pub async fn move_task(&self, id: TaskId, direction: MoveDirection) -> Result<(), EngineError> {
        self.call(|reply| Command::Move {
            id,
            direction,
            reply,
        })
        .await
    }

    /// Chooses which files of a torrent to download (`None` = all). A running torrent
    /// restarts with the new selection; pieces already on disk are kept.
    ///
    /// # Errors
    /// Unknown task, not a torrent, persistence failure, or a stopped engine.
    pub async fn select_files(
        &self,
        id: TaskId,
        files: Option<Vec<u32>>,
    ) -> Result<(), EngineError> {
        self.call(|reply| Command::SelectFiles { id, files, reply })
            .await
    }

    /// Validates, persists and applies new settings (limits apply immediately).
    ///
    /// # Errors
    /// Invalid settings, persistence failure, or a stopped engine.
    pub async fn set_settings(&self, settings: Settings) -> Result<(), EngineError> {
        self.call(|r| Command::SetSettings(Box::new(settings), r))
            .await
    }

    /// Current settings.
    ///
    /// # Errors
    /// A stopped engine.
    pub async fn settings(&self) -> Result<Settings, EngineError> {
        self.call(Command::GetSettings).await
    }

    /// All tasks in creation order, with live progress for running ones.
    ///
    /// # Errors
    /// A stopped engine.
    pub async fn list(&self) -> Result<Vec<TaskSnapshot>, EngineError> {
        self.call(Command::List).await
    }

    /// Subscribes to engine events. A slow subscriber may lag and miss events
    /// (`RecvError::Lagged`); it should then resync with [`EngineHandle::list`].
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    /// Graceful shutdown: stops running tasks (their state is kept for recovery),
    /// waits for final checkpoints and closes the database. Later calls fail with
    /// [`EngineError::Stopped`].
    pub async fn shutdown(&self) {
        let (done, rx) = oneshot::channel();
        if self.commands.send(Command::Shutdown(done)).await.is_ok() {
            let _ = rx.await;
        }
    }
}
