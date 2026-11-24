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
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

use crate::api::EngineApi;
use crate::bridge::BridgeStatus;
use crate::changelog::ReleaseNotesView;
use crate::desktop::Desktop;
use crate::diagnostics;
use crate::dto::{
    AddRequest, MoveDirectionView, ProgressView, SettingsView, TaskDetailView, TaskView,
    TorrentInfoView,
};
use crate::error::CommandError;
use crate::external::ExternalAddView;
use crate::forward::{PROGRESS_INTERVAL, forward_progress};
use crate::ops;
use crate::updates::{UpdateProgress, UpdateView, Updates};

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

    /// Why the engine is not running, if it is not.
    #[must_use]
    pub fn engine_error(&self) -> Option<&str> {
        self.engine.as_ref().err().map(String::as_str)
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
    let mut settings = settings;
    // Turning the bridge on for the first time creates its pairing token.
    if settings.desktop.browser_bridge && settings.desktop.bridge_token.is_empty() {
        settings.desktop.bridge_token = crate::bridge::new_token()
            .map_err(|e| CommandError::invalid(format!("cannot create a token: {e}")))?;
    }
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

/// Whether the browser bridge listens (and why not).
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn bridge_status(desktop: State<'_, Desktop>) -> BridgeStatus {
    desktop.bridge_status()
}

/// A fresh pairing token for the browser extension (saved with the settings).
#[tauri::command]
#[specta::specta]
pub fn new_bridge_token() -> Result<String, CommandError> {
    crate::bridge::new_token()
        .map_err(|e| CommandError::invalid(format!("cannot create a token: {e}")))
}

/// Release notes of every version, newest first (for "What's new").
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn release_notes() -> Vec<ReleaseNotesView> {
    crate::changelog::all()
}

/// The running app's version.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Checks for a newer version.
#[tauri::command]
#[specta::specta]
pub async fn check_for_update(
    app: AppHandle,
    updates: State<'_, Updates>,
) -> Result<Option<UpdateView>, CommandError> {
    updates.check(&app).await.map_err(CommandError::unavailable)
}

/// Downloads and verifies the update found by `check_for_update`, stops the engine
/// gracefully (downloads checkpoint), installs it and restarts the app.
#[tauri::command]
#[specta::specta]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    updates: State<'_, Updates>,
    on_progress: Channel<UpdateProgress>,
) -> Result<(), CommandError> {
    let update = updates
        .take()
        .ok_or_else(|| CommandError::invalid("no update to install: check again"))?;
    let mut downloaded = 0u64;
    let mut last_percent = None;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                let percent = total.map(|t| downloaded * 100 / t.max(1));
                if percent != last_percent || total.is_none() {
                    last_percent = percent;
                    let _ = on_progress.send(UpdateProgress::Downloading { downloaded, total });
                }
            },
            || {},
        )
        .await
        .map_err(|e| CommandError::unavailable(format!("update download failed: {e}")))?;
    // Only after a verified download: running downloads stop and checkpoint.
    let _ = on_progress.send(UpdateProgress::Installing);
    if let Some(engine) = state.running_engine() {
        engine.shutdown().await;
    }
    update
        .install(bytes)
        .map_err(|e| CommandError::unavailable(format!("update install failed: {e}")))?;
    app.restart()
}

/// The diagnostics report, for the user to read before saving it.
#[tauri::command]
#[specta::specta]
pub async fn diagnostics_report(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let (settings, tasks) = match state.engine() {
        Ok(engine) => (
            engine.settings().await.unwrap_or_default(),
            engine.list().await.unwrap_or_default(),
        ),
        Err(_) => Default::default(),
    };
    let dir = log_dir(&app)?;
    let log = tauri::async_runtime::spawn_blocking(move || diagnostics::read_logs(&dir))
        .await
        .unwrap_or_default();
    Ok(diagnostics::report(&diagnostics::Input {
        version: env!("CARGO_PKG_VERSION"),
        settings: &settings,
        tasks: &tasks,
        engine_error: state.engine_error(),
        log: &log,
    }))
}

/// Asks where to save the report and writes it. `false`: the user cancelled. The path
/// comes from the system's dialog, never from the webview.
#[tauri::command]
#[specta::specta]
pub async fn save_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, CommandError> {
    let text = diagnostics_report(app.clone(), state).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name("persivio-diagnostics.txt")
        .add_filter("Text", &["txt"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.ok().flatten() else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|e| CommandError::invalid(format!("cannot save there: {e}")))?;
    tauri::async_runtime::spawn_blocking(move || std::fs::write(path, text))
        .await
        .map_err(|e| CommandError::invalid(e.to_string()))?
        .map_err(|e| CommandError::invalid(format!("cannot save the report: {e}")))?;
    Ok(true)
}

/// Shows the log folder in the system file manager.
#[tauri::command]
#[specta::specta]
pub fn open_logs_folder(app: AppHandle) -> Result<(), CommandError> {
    let dir = log_dir(&app)?;
    let file = dir.join("Persivio.log");
    let target = if file.exists() { file } else { dir };
    app.opener()
        .reveal_item_in_dir(target)
        .map_err(|e| CommandError::invalid(format!("cannot show the folder: {e}")))
}

fn log_dir(app: &AppHandle) -> Result<std::path::PathBuf, CommandError> {
    app.path()
        .app_log_dir()
        .map_err(|e| CommandError::invalid(format!("no log folder: {e}")))
}
