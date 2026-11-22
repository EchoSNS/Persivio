//! The [`Store`] repository.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use pv_core::{
    CheckpointError, CheckpointSink, Checksum, FailReason, HttpResumeInfo, Segment, Settings,
    Source, TaskId, TaskKind, TaskSnapshot, TaskSpec, TaskState, TaskStateKind, UnixMillis,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteRow,
    SqliteSynchronous,
};
use url::Url;

use crate::StoreError;
use crate::convert::{opt_to_i64, opt_to_u64, path_to_str, to_i64, to_small, to_u64};

/// Migrations embedded at compile time from `crates/pv-store/migrations/`.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Settings-table key under which the application [`Settings`] are stored.
pub const SETTINGS_KEY: &str = "app";

/// JSON stored in `tasks.fail_reason` for failed tasks.
#[derive(Serialize, Deserialize)]
struct FailJson {
    reason: FailReason,
    retryable: bool,
}

/// Everything persisted about one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTask {
    /// The task as front ends see it. For HTTP tasks `downloaded` is the sum of the
    /// segments' durable progress.
    pub snapshot: TaskSnapshot,
    /// HTTP validators for resume (empty for torrents).
    pub http: HttpResumeInfo,
    /// Segment table, ordered by index (empty until planned, and for torrents).
    pub segments: Vec<Segment>,
    /// Manual queue position (lower runs first).
    pub queue_order: i64,
}

/// Handle to the Persivio database. Cheap to clone (it wraps a connection pool).
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and applies pending migrations.
    ///
    /// Connection settings:
    /// - `journal_mode=WAL`: readers never block the writer (ADR-012).
    /// - `synchronous=NORMAL`: in WAL mode a commit survives an app crash; only an OS
    ///   crash/power loss can drop the *latest* commits. That is safe for us: a lost
    ///   checkpoint makes progress lag behind the data on disk, never lead it (ADR-005).
    /// - `foreign_keys=ON`: deleting a task cascades to its segments/torrent state.
    /// - `busy_timeout=5s`: wait for a concurrent writer instead of failing.
    ///
    /// # Errors
    /// Database open or migration failures.
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    /// Closes all connections, waiting for in-flight queries (graceful shutdown).
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Inserts a new task.
    ///
    /// # Errors
    /// Duplicate id, constraint violations, or values that cannot be stored.
    pub async fn insert_task(&self, task: &TaskSnapshot) -> Result<(), StoreError> {
        let spec = &task.spec;
        sqlx::query(
            "INSERT INTO tasks (id, kind, source_json, dest_dir, file_name, state, fail_reason,
                                total_size, connections, speed_limit, checksum_json, category,
                                created_at, updated_at, completed_at, selected_files_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(task.id.to_string())
        .bind(task.kind().as_str())
        .bind(serde_json::to_string(&spec.source)?)
        .bind(path_to_str(&spec.dest_dir)?)
        .bind(spec.file_name.as_deref())
        .bind(task.state.kind().as_str())
        .bind(fail_json(&task.state)?)
        .bind(opt_to_i64(task.total_size)?)
        .bind(spec.connections.map(i64::from))
        .bind(opt_to_i64(spec.speed_limit_bps)?)
        .bind(
            spec.checksum
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        )
        .bind(spec.category.as_deref())
        .bind(task.created_at.0)
        .bind(task.updated_at.0)
        .bind(task.completed_at.map(|t| t.0))
        .bind(
            spec.selected_files
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Persists a state change. Sets `completed_at` when the state is `Completed`.
    /// Legality of the transition is the caller's job (`pv_core::transition`).
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if the task does not exist.
    pub async fn update_state(
        &self,
        id: TaskId,
        state: &TaskState,
        at: UnixMillis,
    ) -> Result<(), StoreError> {
        let completed_at = matches!(state, TaskState::Completed).then_some(at.0);
        let result = sqlx::query(
            "UPDATE tasks
             SET state = ?, fail_reason = ?, updated_at = ?,
                 completed_at = COALESCE(?, completed_at)
             WHERE id = ?",
        )
        .bind(state.kind().as_str())
        .bind(fail_json(state)?)
        .bind(at.0)
        .bind(completed_at)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        not_found_if_zero(result.rows_affected(), id)
    }

    /// Persists facts learned by probing / metadata resolution. A `None` file name keeps
    /// the current one; the other fields are replaced.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if the task does not exist.
    pub async fn update_metadata(
        &self,
        id: TaskId,
        file_name: Option<&str>,
        total_size: Option<u64>,
        http: &HttpResumeInfo,
        at: UnixMillis,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "UPDATE tasks
             SET file_name = COALESCE(?, file_name), total_size = ?, final_url = ?,
                 etag = ?, last_modified = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(file_name)
        .bind(opt_to_i64(total_size)?)
        .bind(http.final_url.as_ref().map(Url::as_str))
        .bind(http.etag.as_deref())
        .bind(http.last_modified.as_deref())
        .bind(at.0)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        not_found_if_zero(result.rows_affected(), id)
    }

    /// Atomically replaces the task's whole segment table (new plan, restart after the
    /// remote file changed, or a dynamic split). Stale segments are removed.
    ///
    /// # Errors
    /// Constraint violations (e.g. `end < start`, unknown task) roll back the whole call.
    pub async fn upsert_segments(
        &self,
        id: TaskId,
        segments: &[Segment],
    ) -> Result<(), StoreError> {
        let id_str = id.to_string();
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM segments WHERE task_id = ?")
            .bind(&id_str)
            .execute(&mut *tx)
            .await?;
        for seg in segments {
            sqlx::query(
                "INSERT INTO segments (task_id, idx, start, \"end\", downloaded)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&id_str)
            .bind(i64::from(seg.index))
            .bind(to_i64(seg.start)?)
            .bind(to_i64(seg.end)?)
            .bind(to_i64(seg.downloaded)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Updates `downloaded` for many segments in **one transaction** (the checkpoint
    /// hot path: one fsync per checkpoint instead of one per segment). All-or-nothing:
    /// if any segment is missing, nothing is written.
    ///
    /// # Errors
    /// [`StoreError::SegmentNotFound`] for an unknown `(task, index)`.
    pub async fn update_segment_progress(
        &self,
        id: TaskId,
        progress: &[(u32, u64)],
    ) -> Result<(), StoreError> {
        let id_str = id.to_string();
        let mut tx = self.pool.begin().await?;
        for &(index, downloaded) in progress {
            let result =
                sqlx::query("UPDATE segments SET downloaded = ? WHERE task_id = ? AND idx = ?")
                    .bind(to_i64(downloaded)?)
                    .bind(&id_str)
                    .bind(i64::from(index))
                    .execute(&mut *tx)
                    .await?;
            if result.rows_affected() == 0 {
                // Dropping `tx` without commit rolls back the updates made so far.
                return Err(StoreError::SegmentNotFound { task: id, index });
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Sets the task's current file name (e.g. the final name after de-duplication).
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if the task does not exist.
    pub async fn update_file_name(&self, id: TaskId, name: &str) -> Result<(), StoreError> {
        let result = sqlx::query("UPDATE tasks SET file_name = ? WHERE id = ?")
            .bind(name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        not_found_if_zero(result.rows_affected(), id)
    }

    /// Sets a torrent's file selection (`None` = all files).
    ///
    /// # Errors
    /// [`StoreError::NotFound`] if the task does not exist.
    pub async fn set_selected_files(
        &self,
        id: TaskId,
        files: Option<&[u32]>,
    ) -> Result<(), StoreError> {
        let json = files.map(serde_json::to_string).transpose()?;
        let result = sqlx::query("UPDATE tasks SET selected_files_json = ? WHERE id = ?")
            .bind(json)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        not_found_if_zero(result.rows_affected(), id)
    }

    /// Stores (or replaces) a torrent's resolved metadata: info hash and `.torrent` bytes.
    ///
    /// # Errors
    /// Database errors (including an unknown task: foreign key).
    pub async fn save_torrent_state(
        &self,
        id: TaskId,
        info_hash: &str,
        torrent_bytes: &[u8],
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO torrent_state (task_id, info_hash, resume_blob) VALUES (?, ?, ?)
             ON CONFLICT (task_id) DO UPDATE SET info_hash = excluded.info_hash,
                                                 resume_blob = excluded.resume_blob",
        )
        .bind(id.to_string())
        .bind(info_hash)
        .bind(torrent_bytes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Loads a torrent's metadata: `(info_hash, torrent_bytes)`.
    ///
    /// # Errors
    /// Database errors.
    pub async fn torrent_state(&self, id: TaskId) -> Result<Option<(String, Vec<u8>)>, StoreError> {
        let row = sqlx::query("SELECT info_hash, resume_blob FROM torrent_state WHERE task_id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| -> Result<_, StoreError> {
            Ok((
                r.try_get("info_hash")?,
                r.try_get::<Option<Vec<u8>>, _>("resume_blob")?
                    .unwrap_or_default(),
            ))
        })
        .transpose()
    }

    /// Persists manual queue positions for many tasks in one transaction.
    ///
    /// # Errors
    /// Database errors; unknown ids are ignored.
    pub async fn set_queue_order(&self, order: &[(TaskId, i64)]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for (id, position) in order {
            sqlx::query("UPDATE tasks SET queue_order = ? WHERE id = ?")
                .bind(*position)
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Loads every task with its segments, oldest first.
    ///
    /// # Errors
    /// Database errors or [`StoreError::Corrupt`] rows.
    pub async fn load_all(&self) -> Result<Vec<StoredTask>, StoreError> {
        let mut segments: HashMap<String, Vec<Segment>> = HashMap::new();
        let seg_rows = sqlx::query(
            "SELECT task_id, idx, start, \"end\", downloaded FROM segments ORDER BY task_id, idx",
        )
        .fetch_all(&self.pool)
        .await?;
        for row in &seg_rows {
            let task_id: String = row.try_get("task_id")?;
            segments
                .entry(task_id)
                .or_default()
                .push(segment_from_row(row)?);
        }

        let rows = sqlx::query("SELECT * FROM tasks ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                task_from_row(row, segments.remove(&id).unwrap_or_default())
            })
            .collect()
    }

    /// Loads one task.
    ///
    /// # Errors
    /// Database errors or [`StoreError::Corrupt`] rows.
    pub async fn get_task(&self, id: TaskId) -> Result<Option<StoredTask>, StoreError> {
        let Some(row) = sqlx::query("SELECT * FROM tasks WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };
        let segments = self.load_segments(id).await?;
        task_from_row(&row, segments).map(Some)
    }

    /// Loads a task's segments ordered by index.
    ///
    /// # Errors
    /// Database errors or [`StoreError::Corrupt`] rows.
    pub async fn load_segments(&self, id: TaskId) -> Result<Vec<Segment>, StoreError> {
        sqlx::query(
            "SELECT idx, start, \"end\", downloaded FROM segments WHERE task_id = ? ORDER BY idx",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(segment_from_row)
        .collect()
    }

    /// Deletes a task and (via `ON DELETE CASCADE`) its segments and torrent state.
    /// Returns whether a task was deleted.
    ///
    /// # Errors
    /// Database errors.
    pub async fn delete_task(&self, id: TaskId) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Reads a JSON setting.
    ///
    /// # Errors
    /// Database errors or invalid JSON for `T`.
    pub async fn get_setting<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, StoreError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(Into::into)
    }

    /// Writes (inserts or replaces) a JSON setting.
    ///
    /// # Errors
    /// Database errors or serialization failures.
    pub async fn set_setting<T: Serialize + ?Sized>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO settings (key, value_json) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json",
        )
        .bind(key)
        .bind(serde_json::to_string(value)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Loads the application settings, or defaults if none were saved yet.
    ///
    /// # Errors
    /// Database errors or invalid stored JSON.
    pub async fn load_settings(&self) -> Result<Settings, StoreError> {
        Ok(self.get_setting(SETTINGS_KEY).await?.unwrap_or_default())
    }

    /// Saves the application settings. Validation is the caller's job.
    ///
    /// # Errors
    /// Database errors.
    pub async fn save_settings(&self, settings: &Settings) -> Result<(), StoreError> {
        self.set_setting(SETTINGS_KEY, settings).await
    }

    /// Runs a read-only PRAGMA and returns its text value (tests and diagnostics).
    #[cfg(test)]
    pub(crate) async fn pragma(&self, name: &'static str) -> Result<String, StoreError> {
        let value: sqlx::sqlite::SqliteRow = sqlx::query(name).fetch_one(&self.pool).await?;
        Ok(value
            .try_get::<String, _>(0)
            .or_else(|_| value.try_get::<i64, _>(0).map(|v| v.to_string()))?)
    }

    /// Executes raw SQL to simulate corruption in tests.
    #[cfg(test)]
    pub(crate) async fn execute_raw(&self, sql: &'static str) -> Result<(), StoreError> {
        sqlx::raw_sql(sql).execute(&self.pool).await?;
        Ok(())
    }
}

/// The store is the engine's [`CheckpointSink`]: protocols call it after `sync_data`.
#[async_trait]
impl CheckpointSink for Store {
    async fn save_segments(&self, id: TaskId, segments: &[Segment]) -> Result<(), CheckpointError> {
        self.upsert_segments(id, segments)
            .await
            .map_err(|e| CheckpointError(e.to_string()))
    }

    async fn save_progress(
        &self,
        id: TaskId,
        progress: &[(u32, u64)],
    ) -> Result<(), CheckpointError> {
        self.update_segment_progress(id, progress)
            .await
            .map_err(|e| CheckpointError(e.to_string()))
    }

    async fn save_torrent(
        &self,
        id: TaskId,
        info_hash: &str,
        torrent_bytes: &[u8],
    ) -> Result<(), CheckpointError> {
        self.save_torrent_state(id, info_hash, torrent_bytes)
            .await
            .map_err(|e| CheckpointError(e.to_string()))
    }
}

fn not_found_if_zero(rows: u64, id: TaskId) -> Result<(), StoreError> {
    if rows == 0 {
        Err(StoreError::NotFound(id))
    } else {
        Ok(())
    }
}

fn fail_json(state: &TaskState) -> Result<Option<String>, StoreError> {
    match state {
        TaskState::Failed { reason, retryable } => Ok(Some(serde_json::to_string(&FailJson {
            reason: reason.clone(),
            retryable: *retryable,
        })?)),
        _ => Ok(None),
    }
}

fn corrupt(msg: impl Into<String>) -> StoreError {
    StoreError::Corrupt(msg.into())
}

fn state_from_columns(kind: TaskStateKind, fail: Option<&str>) -> Result<TaskState, StoreError> {
    use TaskStateKind as K;
    Ok(match kind {
        K::Queued => TaskState::Queued,
        K::Probing => TaskState::Probing,
        K::FetchingMetadata => TaskState::FetchingMetadata,
        K::Downloading => TaskState::Downloading,
        K::Paused => TaskState::Paused,
        K::Verifying => TaskState::Verifying,
        K::Seeding => TaskState::Seeding,
        K::Completed => TaskState::Completed,
        K::Cancelled => TaskState::Cancelled,
        K::Failed => {
            let f: FailJson =
                serde_json::from_str(fail.ok_or_else(|| corrupt("failed task without reason"))?)?;
            TaskState::Failed {
                reason: f.reason,
                retryable: f.retryable,
            }
        }
    })
}

fn segment_from_row(row: &SqliteRow) -> Result<Segment, StoreError> {
    let segment = Segment {
        index: to_small(row.try_get("idx")?, "segments.idx")?,
        start: to_u64(row.try_get("start")?, "segments.start")?,
        end: to_u64(row.try_get("end")?, "segments.end")?,
        downloaded: to_u64(row.try_get("downloaded")?, "segments.downloaded")?,
    };
    Ok(segment)
}

fn task_from_row(row: &SqliteRow, segments: Vec<Segment>) -> Result<StoredTask, StoreError> {
    let id_str: String = row.try_get("id")?;
    let id = TaskId::from_str(&id_str).map_err(|e| corrupt(format!("task id `{id_str}`: {e}")))?;

    let source: Source = serde_json::from_str(&row.try_get::<String, _>("source_json")?)?;
    let kind: String = row.try_get("kind")?;
    let kind = TaskKind::from_str(&kind).map_err(corrupt)?;
    if kind != source.kind() {
        return Err(corrupt(format!(
            "task {id}: kind `{kind}` does not match its source"
        )));
    }

    let state_name: String = row.try_get("state")?;
    let state_kind = TaskStateKind::from_str(&state_name).map_err(|e| corrupt(e.to_string()))?;
    let fail: Option<String> = row.try_get("fail_reason")?;
    let state = state_from_columns(state_kind, fail.as_deref())?;

    let checksum: Option<Checksum> = row
        .try_get::<Option<String>, _>("checksum_json")?
        .map(|j| serde_json::from_str(&j))
        .transpose()?;
    let final_url = row
        .try_get::<Option<String>, _>("final_url")?
        .map(|u| Url::parse(&u).map_err(|e| corrupt(format!("final_url `{u}`: {e}"))))
        .transpose()?;

    let spec = TaskSpec {
        source,
        dest_dir: PathBuf::from(row.try_get::<String, _>("dest_dir")?),
        file_name: row.try_get("file_name")?,
        connections: row
            .try_get::<Option<i64>, _>("connections")?
            .map(|c| to_small(c, "tasks.connections"))
            .transpose()?,
        speed_limit_bps: opt_to_u64(row.try_get("speed_limit")?, "tasks.speed_limit")?,
        checksum,
        category: row.try_get("category")?,
        selected_files: row
            .try_get::<Option<String>, _>("selected_files_json")?
            .map(|j| serde_json::from_str(&j))
            .transpose()?,
    };

    let snapshot = TaskSnapshot {
        id,
        spec,
        state,
        total_size: opt_to_u64(row.try_get("total_size")?, "tasks.total_size")?,
        downloaded: segments.iter().map(|s| s.downloaded).sum(),
        created_at: UnixMillis(row.try_get("created_at")?),
        updated_at: UnixMillis(row.try_get("updated_at")?),
        completed_at: row
            .try_get::<Option<i64>, _>("completed_at")?
            .map(UnixMillis),
    };

    Ok(StoredTask {
        snapshot,
        http: HttpResumeInfo {
            final_url,
            etag: row.try_get("etag")?,
            last_modified: row.try_get("last_modified")?,
        },
        segments,
        queue_order: row.try_get("queue_order")?,
    })
}
