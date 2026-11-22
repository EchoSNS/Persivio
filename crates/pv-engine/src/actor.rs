//! The engine actor: one task owns all download state (ADR-002).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pv_core::{
    EngineEvent, FailReason, Outcome, Progress, ProgressSink, Protocol, ProtocolError,
    ResolvedMetadata, ResumeState, RunContext, RunEvents, RunPhase, Settings, SpeedMeter, TaskId,
    TaskKind, TaskSnapshot, TaskState, TaskStateKind, TorrentResume, UnixMillis, transition,
};
use pv_http::filename::{contained_path, part_name};
use pv_store::Store;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::EngineError;
use crate::handle::{Command, MoveDirection, Reply};
use crate::limiter::{GlobalLimiter, TaskLimiter};

/// How often progress is sampled and broadcast (ADR-010: ~4 Hz).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// How long a graceful shutdown waits for running tasks to checkpoint.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

/// Messages from running protocol tasks back to the actor.
pub(crate) enum Internal {
    Phase(TaskId, RunPhase),
    Metadata(TaskId, ResolvedMetadata),
    Warning(TaskId, String),
    Finished(TaskId, Result<Outcome, ProtocolError>),
}

/// Why the engine asked a running task to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Pause,
    Cancel,
    Remove {
        delete_files: bool,
    },
    /// Re-run with changed parameters (e.g. a new torrent file selection).
    Restart,
    Shutdown,
}

struct Running {
    cancel: CancellationToken,
    progress: ProgressSink,
    meter: SpeedMeter,
    last_sample: Instant,
    stop: Option<Stop>,
}

/// Forwards protocol notifications to the actor. Sync + non-blocking (unbounded send).
struct ActorEvents {
    id: TaskId,
    tx: mpsc::UnboundedSender<Internal>,
}

impl RunEvents for ActorEvents {
    fn phase(&self, phase: RunPhase) {
        let _ = self.tx.send(Internal::Phase(self.id, phase));
    }
    fn metadata(&self, meta: ResolvedMetadata) {
        let _ = self.tx.send(Internal::Metadata(self.id, meta));
    }
    fn warning(&self, message: String) {
        let _ = self.tx.send(Internal::Warning(self.id, message));
    }
}

pub(crate) struct Actor {
    pub store: Store,
    pub settings: Arc<Settings>,
    /// All tasks, by id.
    pub tasks: HashMap<TaskId, TaskSnapshot>,
    /// Display order (creation order).
    pub order: Vec<TaskId>,
    /// Waiting tasks, front runs first.
    pub queue: VecDeque<TaskId>,
    running: HashMap<TaskId, Running>,
    pub global: Arc<GlobalLimiter>,
    pub events: broadcast::Sender<EngineEvent>,
    internal_tx: mpsc::UnboundedSender<Internal>,
    pub protocols: Vec<Arc<dyn Protocol>>,
    pub autostart: bool,
}

impl Actor {
    pub fn new(
        store: Store,
        settings: Settings,
        events: broadcast::Sender<EngineEvent>,
        internal_tx: mpsc::UnboundedSender<Internal>,
        protocols: Vec<Arc<dyn Protocol>>,
        autostart: bool,
    ) -> Self {
        let global = Arc::new(GlobalLimiter::default());
        global.set(settings.global_speed_limit_bps);
        Self {
            store,
            settings: Arc::new(settings),
            tasks: HashMap::new(),
            order: Vec::new(),
            queue: VecDeque::new(),
            running: HashMap::new(),
            global,
            events,
            internal_tx,
            protocols,
            autostart,
        }
    }

    fn emit(&self, event: EngineEvent) {
        // No subscribers is fine.
        let _ = self.events.send(event);
    }

    /// The main loop. Ends on `Shutdown` or when every handle is dropped.
    pub async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut internal: mpsc::UnboundedReceiver<Internal>,
    ) {
        self.schedule().await;
        let mut tick = tokio::time::interval(PROGRESS_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                Some(msg) = internal.recv() => self.on_internal(msg).await,
                cmd = commands.recv() => match cmd {
                    Some(Command::Shutdown(done)) => {
                        self.shutdown(&mut internal).await;
                        let _ = done.send(());
                        return;
                    }
                    Some(cmd) => self.on_command(cmd).await,
                    None => {
                        self.shutdown(&mut internal).await;
                        return;
                    }
                },
                _ = tick.tick() => self.publish_progress(),
            }
        }
    }

    async fn shutdown(&mut self, internal: &mut mpsc::UnboundedReceiver<Internal>) {
        self.autostart = false; // never start anything new
        for running in self.running.values_mut() {
            running.stop = Some(Stop::Shutdown);
            running.cancel.cancel();
        }
        let deadline = tokio::time::Instant::now() + SHUTDOWN_GRACE;
        while !self.running.is_empty() {
            match tokio::time::timeout_at(deadline, internal.recv()).await {
                Ok(Some(msg)) => self.on_internal(msg).await,
                Ok(None) | Err(_) => break,
            }
        }
        for protocol in &self.protocols {
            protocol.shutdown().await;
        }
        self.store.close().await;
    }

    async fn on_command(&mut self, cmd: Command) {
        match cmd {
            Command::Add(spec, r) => {
                let result = self.add(*spec).await;
                reply(r, result);
            }
            Command::Pause(id, r) => {
                let result = self.pause(id).await;
                reply(r, result);
            }
            Command::Resume(id, r) => {
                let result = self.resume(id).await;
                reply(r, result);
            }
            Command::Cancel(id, r) => {
                let result = self.cancel(id).await;
                reply(r, result);
            }
            Command::Remove {
                id,
                delete_files,
                reply: r,
            } => {
                let result = self.remove(id, delete_files).await;
                reply(r, result);
            }
            Command::PauseAll(r) => {
                let ids: Vec<TaskId> = self.order.clone();
                for id in ids {
                    let kind = self.kind(id);
                    if kind == Some(TaskStateKind::Queued)
                        || kind.is_some_and(TaskStateKind::is_running)
                    {
                        let _ = self.pause(id).await;
                    }
                }
                reply(r, Ok(()));
            }
            Command::ResumeAll(r) => {
                let ids: Vec<TaskId> = self.order.clone();
                for id in ids {
                    if matches!(
                        self.kind(id),
                        Some(TaskStateKind::Paused | TaskStateKind::Failed)
                    ) {
                        let _ = self.resume(id).await;
                    }
                }
                reply(r, Ok(()));
            }
            Command::Move {
                id,
                direction,
                reply: r,
            } => {
                let result = self.move_task(id, direction).await;
                reply(r, result);
            }
            Command::SetSettings(settings, r) => {
                let result = self.set_settings(*settings).await;
                reply(r, result);
            }
            Command::SelectFiles {
                id,
                files,
                reply: r,
            } => {
                let result = self.select_files(id, files).await;
                reply(r, result);
            }
            Command::GetSettings(r) => reply(r, Ok((*self.settings).clone())),
            Command::List(r) => {
                let list = self
                    .order
                    .iter()
                    .filter_map(|id| self.tasks.get(id).cloned())
                    .collect();
                reply(r, Ok(list));
            }
            Command::Shutdown(_) => unreachable_shutdown(),
        }
    }

    fn kind(&self, id: TaskId) -> Option<TaskStateKind> {
        self.tasks.get(&id).map(|t| t.state.kind())
    }

    /// Validates, persists, updates memory and broadcasts a state change.
    pub async fn set_state(&mut self, id: TaskId, state: TaskState) -> Result<(), EngineError> {
        let task = self.tasks.get(&id).ok_or(EngineError::NotFound(id))?;
        transition(&task.state, &state)?;
        let now = UnixMillis::now();
        self.store.update_state(id, &state, now).await?;
        if let Some(task) = self.tasks.get_mut(&id) {
            if state == TaskState::Completed {
                task.completed_at = Some(now);
            }
            task.state = state.clone();
            task.updated_at = now;
        }
        self.emit(EngineEvent::StateChanged { id, state });
        Ok(())
    }

    async fn add(&mut self, spec: pv_core::TaskSpec) -> Result<TaskId, EngineError> {
        spec.validate()?;
        if !self.protocols.iter().any(|p| p.handles(&spec.source)) {
            return Err(EngineError::NoProtocol);
        }
        let task = TaskSnapshot::new(spec);
        let id = task.id;
        self.store.insert_task(&task).await?;
        self.tasks.insert(id, task.clone());
        self.order.push(id);
        self.queue.push_back(id);
        self.persist_queue_order().await;
        self.emit(EngineEvent::TaskAdded(Box::new(task)));
        self.schedule().await;
        Ok(id)
    }

    async fn persist_queue_order(&self) {
        let order: Vec<(TaskId, i64)> = self
            .queue
            .iter()
            .zip(0i64..)
            .map(|(id, pos)| (*id, pos))
            .collect();
        if let Err(e) = self.store.set_queue_order(&order).await {
            tracing::warn!(%e, "could not persist queue order");
        }
    }

    /// Starts queued tasks while slots are free (ADR-015: the actor counts slots).
    pub async fn schedule(&mut self) {
        if !self.autostart {
            return;
        }
        while self.running.len() < usize::from(self.settings.max_concurrent_tasks) {
            let Some(id) = self.queue.pop_front() else {
                break;
            };
            if let Err(e) = self.start(id).await {
                tracing::warn!(%id, %e, "could not start task");
                let failed = TaskState::Failed {
                    reason: FailReason::Internal {
                        message: e.to_string(),
                    },
                    retryable: true,
                };
                let _ = self.set_state(id, failed).await;
            }
        }
        self.persist_queue_order().await;
    }

    async fn start(&mut self, id: TaskId) -> Result<(), EngineError> {
        let task = self
            .tasks
            .get(&id)
            .cloned()
            .ok_or(EngineError::NotFound(id))?;
        let protocol = self
            .protocols
            .iter()
            .find(|p| p.handles(&task.spec.source))
            .cloned()
            .ok_or(EngineError::NoProtocol)?;
        let stored = self
            .store
            .get_task(id)
            .await?
            .ok_or(EngineError::NotFound(id))?;
        let progress = ProgressSink::new();
        progress.set_downloaded(stored.snapshot.downloaded);
        progress.set_total(task.total_size);
        let cancel = CancellationToken::new();
        let ctx = RunContext {
            resume: ResumeState {
                http: (stored.http != pv_core::HttpResumeInfo::default()).then_some(stored.http),
                segments: stored.segments,
                torrent: self
                    .store
                    .torrent_state(id)
                    .await?
                    .map(|(info_hash, torrent_bytes)| TorrentResume {
                        info_hash,
                        torrent_bytes,
                    }),
            },
            settings: self.settings.clone(),
            cancel: cancel.clone(),
            progress: progress.clone(),
            limiter: Arc::new(TaskLimiter::new(
                self.global.clone(),
                task.spec.speed_limit_bps,
            )),
            checkpoint: Arc::new(self.store.clone()),
            events: Arc::new(ActorEvents {
                id,
                tx: self.internal_tx.clone(),
            }),
            task,
        };
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            let result = protocol.run(ctx).await;
            let _ = tx.send(Internal::Finished(id, result));
        });
        self.running.insert(
            id,
            Running {
                cancel,
                progress,
                meter: SpeedMeter::default(),
                last_sample: Instant::now(),
                stop: None,
            },
        );
        Ok(())
    }

    async fn pause(&mut self, id: TaskId) -> Result<(), EngineError> {
        if let Some(running) = self.running.get_mut(&id) {
            running.stop = Some(Stop::Pause);
            running.cancel.cancel(); // state becomes Paused when the run reports back
            return Ok(());
        }
        self.set_state(id, TaskState::Paused).await?;
        self.queue.retain(|q| *q != id);
        Ok(())
    }

    async fn resume(&mut self, id: TaskId) -> Result<(), EngineError> {
        self.set_state(id, TaskState::Queued).await?;
        self.queue.push_back(id);
        self.schedule().await;
        Ok(())
    }

    async fn cancel(&mut self, id: TaskId) -> Result<(), EngineError> {
        if let Some(running) = self.running.get_mut(&id) {
            running.stop = Some(Stop::Cancel);
            running.cancel.cancel();
            return Ok(());
        }
        self.set_state(id, TaskState::Cancelled).await?;
        self.queue.retain(|q| *q != id);
        self.delete_part(id).await;
        Ok(())
    }

    async fn remove(&mut self, id: TaskId, delete_files: bool) -> Result<(), EngineError> {
        if !self.tasks.contains_key(&id) {
            return Err(EngineError::NotFound(id));
        }
        if let Some(running) = self.running.get_mut(&id) {
            running.stop = Some(Stop::Remove { delete_files });
            running.cancel.cancel(); // removed when the run reports back
            return Ok(());
        }
        self.remove_now(id, delete_files).await
    }

    async fn remove_now(&mut self, id: TaskId, delete_files: bool) -> Result<(), EngineError> {
        self.delete_part(id).await;
        if delete_files
            && self.kind(id) == Some(TaskStateKind::Completed)
            && let Some(path) = self.final_path(id)
        {
            remove_path(&path).await?;
        }
        self.store.delete_task(id).await?;
        self.tasks.remove(&id);
        self.order.retain(|o| *o != id);
        self.queue.retain(|q| *q != id);
        self.emit(EngineEvent::TaskRemoved(id));
        Ok(())
    }

    fn final_path(&self, id: TaskId) -> Option<PathBuf> {
        let task = self.tasks.get(&id)?;
        let name = task.spec.file_name.as_deref()?;
        contained_path(&task.spec.dest_dir, name).ok()
    }

    /// Deletes an unfinished task's partial data: the `.part` file (HTTP) or the
    /// torrent's file/folder (whose name was made unique for this task at start).
    async fn delete_part(&self, id: TaskId) {
        let Some(task) = self.tasks.get(&id) else {
            return;
        };
        let Some(name) = task.spec.file_name.as_deref() else {
            return;
        };
        let partial = match task.kind() {
            TaskKind::Http => contained_path(&task.spec.dest_dir, &part_name(name)),
            TaskKind::Torrent if task.state != TaskState::Completed => {
                contained_path(&task.spec.dest_dir, name)
            }
            TaskKind::Torrent => return,
        };
        if let Ok(path) = partial
            && let Err(e) = remove_path(&path).await
        {
            tracing::warn!(%id, %e, "could not delete partial data");
        }
    }

    async fn select_files(
        &mut self,
        id: TaskId,
        files: Option<Vec<u32>>,
    ) -> Result<(), EngineError> {
        let task = self.tasks.get(&id).ok_or(EngineError::NotFound(id))?;
        if task.kind() != TaskKind::Torrent {
            return Err(EngineError::NotATorrent(id));
        }
        self.store.set_selected_files(id, files.as_deref()).await?;
        if let Some(task) = self.tasks.get_mut(&id) {
            task.spec.selected_files = files;
        }
        if let Some(running) = self.running.get_mut(&id) {
            running.stop = Some(Stop::Restart);
            running.cancel.cancel();
        }
        Ok(())
    }

    /// Puts a task back at the front of the queue (used after a restart request).
    async fn requeue_front(&mut self, id: TaskId) -> Result<(), EngineError> {
        if !self
            .kind(id)
            .is_some_and(|k| k.can_transition_to(TaskStateKind::Queued))
        {
            self.set_state(id, TaskState::Paused).await?;
        }
        self.set_state(id, TaskState::Queued).await?;
        self.queue.push_front(id);
        Ok(())
    }

    async fn move_task(&mut self, id: TaskId, direction: MoveDirection) -> Result<(), EngineError> {
        let pos = self
            .queue
            .iter()
            .position(|q| *q == id)
            .ok_or(EngineError::NotFound(id))?;
        let last = self.queue.len() - 1;
        let target = match direction {
            MoveDirection::Up => pos.saturating_sub(1),
            MoveDirection::Down => (pos + 1).min(last),
            MoveDirection::Top => 0,
            MoveDirection::Bottom => last,
        };
        if let Some(item) = self.queue.remove(pos) {
            self.queue.insert(target, item);
        }
        self.persist_queue_order().await;
        Ok(())
    }

    async fn set_settings(&mut self, settings: Settings) -> Result<(), EngineError> {
        settings.validate()?;
        self.store.save_settings(&settings).await?;
        self.global.set(settings.global_speed_limit_bps);
        self.settings = Arc::new(settings);
        self.schedule().await;
        Ok(())
    }

    async fn on_internal(&mut self, msg: Internal) {
        match msg {
            Internal::Phase(id, phase) => {
                let state = phase.to_state();
                if self.kind(id) != Some(state.kind())
                    && let Err(e) = self.set_state(id, state).await
                {
                    tracing::warn!(%id, %e, "ignoring phase change");
                }
            }
            Internal::Metadata(id, meta) => self.on_metadata(id, meta).await,
            Internal::Warning(id, message) => {
                self.emit(EngineEvent::Warning {
                    id: Some(id),
                    message,
                });
            }
            Internal::Finished(id, result) => {
                self.on_finished(id, result).await;
                self.schedule().await;
            }
        }
    }

    async fn on_metadata(&mut self, id: TaskId, meta: ResolvedMetadata) {
        let http = meta.http.clone().unwrap_or_default();
        if let Err(e) = self
            .store
            .update_metadata(
                id,
                meta.name.as_deref(),
                meta.total_size,
                &http,
                UnixMillis::now(),
            )
            .await
        {
            tracing::warn!(%id, %e, "could not persist metadata");
        }
        if let Some(task) = self.tasks.get_mut(&id) {
            if meta.name.is_some() {
                task.spec.file_name.clone_from(&meta.name);
            }
            task.total_size = meta.total_size;
        }
        self.emit(EngineEvent::MetadataResolved {
            id,
            name: meta.name.unwrap_or_default(),
            size: meta.total_size,
            files: meta.files,
        });
    }

    async fn on_finished(&mut self, id: TaskId, result: Result<Outcome, ProtocolError>) {
        let Some(running) = self.running.remove(&id) else {
            return;
        };
        let sample = running.progress.sample();
        if let Some(task) = self.tasks.get_mut(&id) {
            task.downloaded = sample.downloaded;
            if sample.total.is_some() {
                task.total_size = sample.total;
            }
        }
        let outcome = match (result, running.stop) {
            (_, Some(Stop::Shutdown)) => Ok(()), // state kept for recovery
            (_, Some(Stop::Remove { delete_files })) => self.remove_now(id, delete_files).await,
            (_, Some(Stop::Restart)) => self.requeue_front(id).await,
            (Ok(Outcome::Completed { path }), _) => self.complete(id, &path).await,
            (Ok(Outcome::Stopped) | Err(_), Some(Stop::Cancel)) => {
                let r = self.set_state(id, TaskState::Cancelled).await;
                self.delete_part(id).await;
                r
            }
            (Ok(Outcome::Stopped), _) | (Err(_), Some(Stop::Pause)) => {
                self.set_state(id, TaskState::Paused).await
            }
            (Err(e), None) => {
                tracing::info!(%id, %e, "task failed");
                self.set_state(id, e.to_failed_state()).await
            }
        };
        if let Err(e) = outcome {
            tracing::warn!(%id, %e, "could not record task end");
        }
    }

    async fn complete(&mut self, id: TaskId, path: &std::path::Path) -> Result<(), EngineError> {
        // A run that never reported a phase still has to pass through Downloading.
        if matches!(
            self.kind(id),
            Some(TaskStateKind::Queued | TaskStateKind::Probing | TaskStateKind::FetchingMetadata)
        ) {
            self.set_state(id, TaskState::Downloading).await?;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            self.store.update_file_name(id, name).await?;
            if let Some(task) = self.tasks.get_mut(&id) {
                task.spec.file_name = Some(name.to_owned());
                if let Some(total) = task.total_size {
                    task.downloaded = total;
                }
            }
        }
        self.set_state(id, TaskState::Completed).await
    }

    fn publish_progress(&mut self) {
        let mut batch = Vec::with_capacity(self.running.len());
        for (id, running) in &mut self.running {
            let now = Instant::now();
            let sample = running.progress.sample();
            let speed = running
                .meter
                .sample(sample.downloaded, now - running.last_sample);
            running.last_sample = now;
            batch.push(sample.to_progress(*id, speed));
            if let Some(task) = self.tasks.get_mut(id) {
                task.downloaded = sample.downloaded;
                if sample.total.is_some() {
                    task.total_size = sample.total;
                }
            }
        }
        if !batch.is_empty() {
            batch.sort_by_key(|p: &Progress| p.task_id);
            self.emit(EngineEvent::ProgressBatch(batch));
        }
    }
}

fn reply<T>(r: Reply<T>, result: Result<T, EngineError>) {
    // The caller may have given up waiting; that's fine.
    let _ = r.send(result);
}

/// `Shutdown` is handled in the main loop before dispatch.
const fn unreachable_shutdown() {}

/// Deletes a file, or a folder with its contents. Missing paths are fine.
async fn remove_path(path: &std::path::Path) -> std::io::Result<()> {
    let result = match tokio::fs::symlink_metadata(path).await {
        Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(path).await,
        Ok(_) => tokio::fs::remove_file(path).await,
        Err(e) => Err(e),
    };
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
