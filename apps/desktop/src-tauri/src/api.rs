//! The engine operations the commands need, as a trait so tests can use a mock engine.

use async_trait::async_trait;
use pv_core::{EngineEvent, Settings, TaskId, TaskSnapshot, TaskSpec};
use pv_engine::{EngineError, EngineHandle, MoveDirection};
use tokio::sync::broadcast;

/// What the desktop app needs from the engine. Implemented by [`EngineHandle`].
#[async_trait]
pub trait EngineApi: Send + Sync {
    /// See [`EngineHandle::add`].
    async fn add(&self, spec: TaskSpec) -> Result<TaskId, EngineError>;
    /// See [`EngineHandle::pause`].
    async fn pause(&self, id: TaskId) -> Result<(), EngineError>;
    /// See [`EngineHandle::resume`].
    async fn resume(&self, id: TaskId) -> Result<(), EngineError>;
    /// See [`EngineHandle::cancel`].
    async fn cancel(&self, id: TaskId) -> Result<(), EngineError>;
    /// See [`EngineHandle::remove`].
    async fn remove(&self, id: TaskId, delete_files: bool) -> Result<(), EngineError>;
    /// See [`EngineHandle::pause_all`].
    async fn pause_all(&self) -> Result<(), EngineError>;
    /// See [`EngineHandle::resume_all`].
    async fn resume_all(&self) -> Result<(), EngineError>;
    /// See [`EngineHandle::move_task`].
    async fn move_task(&self, id: TaskId, direction: MoveDirection) -> Result<(), EngineError>;
    /// See [`EngineHandle::select_files`].
    async fn select_files(&self, id: TaskId, files: Option<Vec<u32>>) -> Result<(), EngineError>;
    /// See [`EngineHandle::settings`].
    async fn settings(&self) -> Result<Settings, EngineError>;
    /// See [`EngineHandle::set_settings`].
    async fn set_settings(&self, settings: Settings) -> Result<(), EngineError>;
    /// See [`EngineHandle::list`].
    async fn list(&self) -> Result<Vec<TaskSnapshot>, EngineError>;
    /// See [`EngineHandle::subscribe`].
    fn subscribe(&self) -> broadcast::Receiver<EngineEvent>;
    /// See [`EngineHandle::shutdown`].
    async fn shutdown(&self);
}

#[async_trait]
impl EngineApi for EngineHandle {
    async fn add(&self, spec: TaskSpec) -> Result<TaskId, EngineError> {
        Self::add(self, spec).await
    }
    async fn pause(&self, id: TaskId) -> Result<(), EngineError> {
        Self::pause(self, id).await
    }
    async fn resume(&self, id: TaskId) -> Result<(), EngineError> {
        Self::resume(self, id).await
    }
    async fn cancel(&self, id: TaskId) -> Result<(), EngineError> {
        Self::cancel(self, id).await
    }
    async fn remove(&self, id: TaskId, delete_files: bool) -> Result<(), EngineError> {
        Self::remove(self, id, delete_files).await
    }
    async fn pause_all(&self) -> Result<(), EngineError> {
        Self::pause_all(self).await
    }
    async fn resume_all(&self) -> Result<(), EngineError> {
        Self::resume_all(self).await
    }
    async fn move_task(&self, id: TaskId, direction: MoveDirection) -> Result<(), EngineError> {
        Self::move_task(self, id, direction).await
    }
    async fn select_files(&self, id: TaskId, files: Option<Vec<u32>>) -> Result<(), EngineError> {
        Self::select_files(self, id, files).await
    }
    async fn settings(&self) -> Result<Settings, EngineError> {
        Self::settings(self).await
    }
    async fn set_settings(&self, settings: Settings) -> Result<(), EngineError> {
        Self::set_settings(self, settings).await
    }
    async fn list(&self) -> Result<Vec<TaskSnapshot>, EngineError> {
        Self::list(self).await
    }
    fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        Self::subscribe(self)
    }
    async fn shutdown(&self) {
        Self::shutdown(self).await;
    }
}
