//! Store errors.

use mp_core::TaskId;

/// Anything that can go wrong in [`crate::Store`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Database driver error (I/O, constraint violation, locked DB, ...).
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Schema migration failed.
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A JSON column could not be encoded/decoded.
    #[error("JSON column error: {0}")]
    Json(#[from] serde_json::Error),
    /// No task with this id.
    #[error("task {0} not found")]
    NotFound(TaskId),
    /// No segment with this index for the task.
    #[error("segment {index} of task {task} not found")]
    SegmentNotFound {
        /// Task id.
        task: TaskId,
        /// Segment index.
        index: u32,
    },
    /// A value does not fit SQLite's signed 64-bit integers.
    #[error("value {0} is too large to store")]
    Overflow(u64),
    /// A path is not valid UTF-8 and cannot be stored as TEXT.
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(String),
    /// A stored row violates an invariant (manual edit, bug, or corruption).
    #[error("corrupt row: {0}")]
    Corrupt(String),
}
