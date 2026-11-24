//! App updates (ADR-022): check, download, verify the signature, install, restart.
//!
//! Every update package is signed; the app embeds the public key and refuses anything
//! else (checked by `tauri-plugin-updater`). The endpoint is configured per build: the
//! default config points at GitHub Releases, `tauri.local.conf.json` at a local server.

use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

/// An available update, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// New version.
    pub version: String,
    /// Installed version.
    pub current_version: String,
    /// Release date, if published.
    pub date: Option<String>,
    /// Release notes from the update manifest.
    pub notes: Option<String>,
}

/// Install progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpdateProgress {
    /// Bytes downloaded so far.
    Downloading {
        /// Downloaded bytes.
        downloaded: u64,
        /// Total, if known.
        total: Option<u64>,
    },
    /// Download verified; stopping downloads and installing.
    Installing,
}

/// Notifies the UI that an update is available (startup check).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct UpdateAvailableEvent(pub UpdateView);

/// The update found by the last check, kept for `install`.
#[derive(Default)]
pub struct Updates {
    found: Mutex<Option<Update>>,
}

impl Updates {
    /// Checks the configured endpoint; remembers and returns a newer version.
    ///
    /// # Errors
    /// Network, manifest or configuration problems, as text for the UI.
    pub async fn check<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> Result<Option<UpdateView>, String> {
        let updater = app.updater().map_err(|e| e.to_string())?;
        self.check_with(&updater).await
    }

    /// [`check`](Self::check) with a given updater (tests use their own endpoint and key).
    ///
    /// # Errors
    /// As for [`check`](Self::check).
    pub async fn check_with(&self, updater: &Updater) -> Result<Option<UpdateView>, String> {
        let update = updater
            .check()
            .await
            .map_err(|e| format!("could not check for updates: {e}"))?;
        let view = update.as_ref().map(|u| UpdateView {
            version: u.version.clone(),
            current_version: u.current_version.clone(),
            date: u.date.map(|d| d.date().to_string()),
            notes: u.body.clone(),
        });
        *self.found.lock().unwrap_or_else(PoisonError::into_inner) = update;
        Ok(view)
    }

    /// The update from the last check.
    pub fn take(&self) -> Option<Update> {
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}
