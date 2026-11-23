//! Tauri commands: thin adapters from IPC to [`crate::ops`]. Every command here must also
//! be listed in `build.rs` and granted in `capabilities/main.json`.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments (State, Channel, DTOs) by value"
)]
#![allow(
    clippy::missing_errors_doc,
    reason = "every command fails only with `CommandError`, documented there"
)]

use std::sync::{Arc, Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

use crate::api::EngineApi;
use crate::desktop::Desktop;
use crate::dto::{
    AddRequest, MoveDirectionView, ProgressView, SettingsView, TaskDetailView, TaskView,
    TorrentInfoView,
};
use crate::error::CommandError;
use crate::external::ExternalAddView;
use crate::forward::{PROGRESS_INTERVAL, forward_progress};
use crate::ops;

/// Shared state of the running app.
pub struct AppState {
    /// The engine, or why it could not start (shown by the UI).
    engine: Result<Arc<dyn EngineApi>, String>,
    /// Cancels the current progress subscription when the UI subscribes again (reload).
    progress: Mutex<Option<CancellationToken>>,
}

impl AppState {
    /// State around a running engine, or the reason it is unavailable.
    #[must_use]
    pub fn new(engine: Result<Arc<dyn EngineApi>, String>) -> Self {
        Self {
            engine,
            progress: Mutex::new(None),
        }
    }

    /// The engine, or an `unavailable` error carrying the startup failure.
    ///
    /// # Errors
    /// [`CommandError::unavailable`] when the engine did not start.
    pub fn engine(&self) -> Result<&dyn EngineApi, CommandError> {
        self.engine
            .as_deref()
            .map_err(|reason| CommandError::unavailable(reason.clone()))
    }

    /// The engine for shutdown, if it is running.
    #[must_use]
    pub fn running_engine(&self) -> Option<Arc<dyn EngineApi>> {
        self.engine.as_ref().ok().cloned()
    }
}

/// Every task, in queue order.
#[tauri::command]
#[specta::specta]
pub async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskView>, CommandError> {
    ops::list_tasks(state.engine()?).await
}

/// Adds a download (URL or magnet link) and returns its id.
#[tauri::command]
#[specta::specta]
pub async fn add_task(
    state: State<'_, AppState>,
    request: AddRequest,
) -> Result<String, CommandError> {
    ops::add_task(state.engine()?, request, pv_engine::os_download_dir()).await
}

/// Pauses a task.
#[tauri::command]
#[specta::specta]
pub async fn pause_task(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.engine()?.pause(ops::parse_id(&id)?).await?)
}

/// Resumes (or retries) a task.
#[tauri::command]
#[specta::specta]
pub async fn resume_task(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.engine()?.resume(ops::parse_id(&id)?).await?)
}

/// Cancels a task and deletes its partial data.
#[tauri::command]
#[specta::specta]
pub async fn cancel_task(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.engine()?.cancel(ops::parse_id(&id)?).await?)
}

/// Removes a task from the list, optionally deleting its files.
#[tauri::command]
#[specta::specta]
pub async fn remove_task(
    state: State<'_, AppState>,
    id: String,
    delete_files: bool,
) -> Result<(), CommandError> {
    Ok(state
        .engine()?
        .remove(ops::parse_id(&id)?, delete_files)
        .await?)
}

/// Pauses every task.
#[tauri::command]
#[specta::specta]
pub async fn pause_all(state: State<'_, AppState>) -> Result<(), CommandError> {
    Ok(state.engine()?.pause_all().await?)
}

/// Resumes every paused task.
#[tauri::command]
#[specta::specta]
pub async fn resume_all(state: State<'_, AppState>) -> Result<(), CommandError> {
    Ok(state.engine()?.resume_all().await?)
}

/// Moves a task in the queue.
#[tauri::command]
#[specta::specta]
pub async fn move_task(
    state: State<'_, AppState>,
    id: String,
    direction: MoveDirectionView,
) -> Result<(), CommandError> {
    Ok(state
        .engine()?
        .move_task(ops::parse_id(&id)?, direction.into())
        .await?)
}

/// Torrents: chooses which files to download (`null` = all).
#[tauri::command]
#[specta::specta]
pub async fn select_files(
    state: State<'_, AppState>,
    id: String,
    files: Option<Vec<u32>>,
) -> Result<(), CommandError> {
    Ok(state
        .engine()?
        .select_files(ops::parse_id(&id)?, files)
        .await?)
}

/// The current settings.
#[tauri::command]
#[specta::specta]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsView, CommandError> {
    Ok(SettingsView::from(&state.engine()?.settings().await?))
}

/// Replaces the settings and applies the desktop ones (tray, autostart, magnet handler,
/// clipboard, sleep). Returns problems that did not prevent saving.
#[tauri::command]
#[specta::specta]
pub async fn set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    desktop: State<'_, Desktop>,
    settings: SettingsView,
) -> Result<Vec<String>, CommandError> {
    let wanted: pv_core::DesktopSettings = settings.desktop.clone().into();
    ops::set_settings(state.engine()?, settings).await?;
    desktop.settings.send_replace(wanted.clone());
    Ok(crate::desktop::apply_os_settings(&app, &wanted))
}

/// External add requests (magnet clicks, opened .torrent files) waiting for the UI.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn take_external_requests(desktop: State<'_, Desktop>) -> Vec<ExternalAddView> {
    desktop.take_pending()
}

/// The folder new downloads go to.
#[tauri::command]
#[specta::specta]
pub async fn default_download_dir(state: State<'_, AppState>) -> Result<String, CommandError> {
    let dir = ops::default_download_dir(state.engine()?, pv_engine::os_download_dir()).await?;
    Ok(ops::display_path(&dir))
}

/// One task with segments, files and safe metadata, for the detail drawer.
#[tauri::command]
#[specta::specta]
pub async fn get_task_detail(
    state: State<'_, AppState>,
    id: String,
) -> Result<TaskDetailView, CommandError> {
    ops::task_detail(state.engine()?, ops::parse_id(&id)?).await
}

/// Reads a `.torrent` file (base64) without adding it, to choose files first.
#[tauri::command]
#[specta::specta]
pub fn inspect_torrent(data: String) -> Result<TorrentInfoView, CommandError> {
    ops::inspect_torrent(&data)
}

/// Shows a task's file (or folder) in the system file manager. Never opens or runs it.
#[tauri::command]
#[specta::specta]
pub async fn reveal_task(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    let path = ops::task_path(state.engine()?, ops::parse_id(&id)?).await?;
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| CommandError::invalid(format!("cannot show the file: {e}")))
}

/// Streams progress to `on_progress` (≤ 4 Hz) until the UI subscribes again.
#[tauri::command]
#[specta::specta]
pub fn subscribe_progress(
    state: State<'_, AppState>,
    on_progress: Channel<Vec<ProgressView>>,
) -> Result<(), CommandError> {
    let events = state.engine()?.subscribe();
    let cancel = CancellationToken::new();
    let previous = state
        .progress
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(cancel.clone());
    if let Some(previous) = previous {
        previous.cancel();
    }
    tauri::async_runtime::spawn(forward_progress(
        events,
        cancel,
        PROGRESS_INTERVAL,
        move |batch| on_progress.send(batch).is_ok(),
    ));
    Ok(())
}
