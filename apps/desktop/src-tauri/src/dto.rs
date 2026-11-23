//! The types the UI sees (ADR-018). TypeScript types are generated from these.
//!
//! They are separate from `pv-core`'s types on purpose:
//! - **Secrets stay in Rust.** A task's cookies, custom headers, URL password and
//!   `.torrent` bytes never cross into the webview; [`TaskView`] carries a display
//!   string instead of the `Source`.
//! - **A stable, JavaScript-friendly shape** (`camelCase`, ids as strings) that can
//!   evolve independently of the engine's persistence format.
//! - `pv-core` stays free of the pre-release `specta` dependency.
//!
//! Sizes and speeds are `u64` and exported as TypeScript `number`. That is exact up to
//! 2^53 bytes (8 PiB), far above any file size or speed.

use std::path::PathBuf;

use pv_core::{
    EngineEvent, FileEntry, Progress, Settings, Source, TaskKind, TaskSnapshot, TaskState,
    TorrentSettings,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use url::Url;

/// Download protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskKindView {
    /// HTTP(S).
    Http,
    /// Magnet link or `.torrent` file.
    Torrent,
}

impl From<TaskKind> for TaskKindView {
    fn from(kind: TaskKind) -> Self {
        match kind {
            TaskKind::Http => Self::Http,
            TaskKind::Torrent => Self::Torrent,
        }
    }
}

/// A task's state. `type` is the discriminant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskStateView {
    /// Waiting for a slot.
    Queued,
    /// HTTP: checking size and resume support.
    Probing,
    /// Torrent: getting metadata from peers.
    FetchingMetadata,
    /// Transferring data.
    Downloading,
    /// Stopped by the user; can be resumed.
    Paused,
    /// Checking the checksum.
    Verifying,
    /// Torrent: finished, uploading to peers.
    Seeding,
    /// Finished.
    Completed,
    /// Stopped by an error.
    Failed {
        /// Human-readable reason.
        message: String,
        /// Whether retrying (resume) could succeed.
        retryable: bool,
    },
    /// Cancelled by the user.
    Cancelled,
}

impl From<&TaskState> for TaskStateView {
    fn from(state: &TaskState) -> Self {
        match state {
            TaskState::Queued => Self::Queued,
            TaskState::Probing => Self::Probing,
            TaskState::FetchingMetadata => Self::FetchingMetadata,
            TaskState::Downloading => Self::Downloading,
            TaskState::Paused => Self::Paused,
            TaskState::Verifying => Self::Verifying,
            TaskState::Seeding => Self::Seeding,
            TaskState::Completed => Self::Completed,
            TaskState::Failed { reason, retryable } => Self::Failed {
                message: reason.to_string(),
                retryable: *retryable,
            },
            TaskState::Cancelled => Self::Cancelled,
        }
    }
}

/// One task as shown in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    /// Task id.
    pub id: String,
    /// Protocol.
    pub kind: TaskKindView,
    /// Display name: the file name once known, else a best guess from the source.
    pub name: String,
    /// Where it comes from, safe to display: URL without password, or the magnet link.
    pub source: String,
    /// Destination directory.
    pub dest_dir: String,
    /// State.
    pub state: TaskStateView,
    /// Total bytes, once known.
    pub total: Option<u64>,
    /// Bytes downloaded.
    pub downloaded: u64,
    /// Creation time (Unix milliseconds).
    pub created_at: i64,
    /// Completion time (Unix milliseconds).
    pub completed_at: Option<i64>,
    /// Category label.
    pub category: Option<String>,
}

impl From<&TaskSnapshot> for TaskView {
    fn from(task: &TaskSnapshot) -> Self {
        Self {
            id: task.id.to_string(),
            kind: task.kind().into(),
            name: task
                .spec
                .file_name
                .clone()
                .unwrap_or_else(|| guess_name(&task.spec.source)),
            source: display_source(&task.spec.source),
            dest_dir: task.spec.dest_dir.display().to_string(),
            state: (&task.state).into(),
            total: task.total_size,
            downloaded: task.downloaded,
            created_at: task.created_at.0,
            completed_at: task.completed_at.map(|t| t.0),
            category: task.spec.category.clone(),
        }
    }
}

/// The source as safe display text: no cookies, headers or URL password.
#[must_use]
pub fn display_source(source: &Source) -> String {
    match source {
        Source::Http { url, .. } => {
            let mut url = url.clone();
            // Both only fail for URLs that cannot carry credentials: nothing to remove.
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.to_string()
        }
        Source::Magnet { uri } => uri.clone(),
        Source::TorrentFile { .. } => "torrent file".to_owned(),
    }
}

/// A name to show before the real one is known.
fn guess_name(source: &Source) -> String {
    match source {
        Source::Http { url, .. } => url
            .path_segments()
            .and_then(|mut s| s.rfind(|seg| !seg.is_empty()))
            .map_or_else(
                || url.host_str().unwrap_or("download").to_owned(),
                percent_decode,
            ),
        Source::Magnet { uri } => Url::parse(uri)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "dn")
                    .map(|(_, v)| v.into_owned())
            })
            .unwrap_or_else(|| "magnet link".to_owned()),
        Source::TorrentFile { .. } => "torrent".to_owned(),
    }
}

fn percent_decode(segment: &str) -> String {
    // `Url` keeps path segments percent-encoded; parse a dummy query to decode.
    Url::parse(&format!("x:?v={segment}"))
        .ok()
        .and_then(|u| u.query_pairs().next().map(|(_, v)| v.into_owned()))
        .unwrap_or_else(|| segment.to_owned())
}

/// Live progress of one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProgressView {
    /// Task id.
    pub id: String,
    /// Bytes downloaded.
    pub downloaded: u64,
    /// Total bytes, if known.
    pub total: Option<u64>,
    /// Download speed, bytes/second.
    pub speed_bps: u64,
    /// Seconds remaining, if known.
    pub eta_secs: Option<u64>,
    /// Open connections (HTTP) or connected peers (torrent).
    pub connections: u16,
    /// Torrent: connected peers.
    pub peers: Option<u32>,
    /// Torrent: upload speed, bytes/second.
    pub upload_bps: Option<u64>,
}

impl From<&Progress> for ProgressView {
    fn from(p: &Progress) -> Self {
        Self {
            id: p.task_id.to_string(),
            downloaded: p.downloaded,
            total: p.total,
            speed_bps: p.speed_bps,
            eta_secs: p.eta_secs,
            connections: p.connections_active,
            peers: p.peers,
            upload_bps: p.upload_bps,
        }
    }
}

/// A file inside a task (one for HTTP, many for torrents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FileView {
    /// Index in the torrent.
    pub index: u32,
    /// Relative path.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Whether it is downloaded.
    pub selected: bool,
}

impl From<&FileEntry> for FileView {
    fn from(f: &FileEntry) -> Self {
        Self {
            index: f.index,
            path: f.path.clone(),
            size: f.size,
            selected: f.selected,
        }
    }
}

/// Engine events for the UI (progress goes through its own channel). `type` is the
/// discriminant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineUiEvent {
    /// A task was created.
    TaskAdded {
        /// The new task.
        task: TaskView,
    },
    /// A task changed state.
    StateChanged {
        /// Task id.
        id: String,
        /// New state.
        state: TaskStateView,
    },
    /// Name, size and files became known.
    MetadataResolved {
        /// Task id.
        id: String,
        /// File (or torrent) name.
        name: String,
        /// Total size, if known.
        size: Option<u64>,
        /// Files.
        files: Vec<FileView>,
    },
    /// A task was removed.
    TaskRemoved {
        /// Task id.
        id: String,
    },
    /// A non-fatal problem to show.
    Warning {
        /// Related task.
        id: Option<String>,
        /// Message.
        message: String,
    },
    /// Events were missed: reload the task list.
    Resync,
}

impl EngineUiEvent {
    /// The UI event for an engine event; `None` for progress (sent on its channel).
    #[must_use]
    pub fn from_engine(event: &EngineEvent) -> Option<Self> {
        Some(match event {
            EngineEvent::TaskAdded(task) => Self::TaskAdded {
                task: task.as_ref().into(),
            },
            EngineEvent::StateChanged { id, state } => Self::StateChanged {
                id: id.to_string(),
                state: state.into(),
            },
            EngineEvent::MetadataResolved {
                id,
                name,
                size,
                files,
            } => Self::MetadataResolved {
                id: id.to_string(),
                name: name.clone(),
                size: *size,
                files: files.iter().map(Into::into).collect(),
            },
            EngineEvent::TaskRemoved(id) => Self::TaskRemoved { id: id.to_string() },
            EngineEvent::Warning { id, message } => Self::Warning {
                id: id.map(|i| i.to_string()),
                message: message.clone(),
            },
            EngineEvent::ProgressBatch(_) => return None,
        })
    }
}

/// Where a new download comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AddSource {
    /// An `http(s)://` URL or a `magnet:?` link.
    Link {
        /// The URL or magnet link.
        url: String,
    },
    /// The content of a `.torrent` file, read by the UI (no path crosses IPC).
    TorrentFile {
        /// Base64 of the file's bytes.
        data: String,
    },
}

/// One extra HTTP request header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HeaderView {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
}

/// Checksum algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithmView {
    /// SHA-256.
    Sha256,
    /// MD5 (only to match published checksums).
    Md5,
}

impl From<ChecksumAlgorithmView> for pv_core::ChecksumAlgorithm {
    fn from(a: ChecksumAlgorithmView) -> Self {
        match a {
            ChecksumAlgorithmView::Sha256 => Self::Sha256,
            ChecksumAlgorithmView::Md5 => Self::Md5,
        }
    }
}

/// An expected checksum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ChecksumView {
    /// Algorithm.
    pub algorithm: ChecksumAlgorithmView,
    /// Expected digest in hex.
    pub hex: String,
}

/// What the user asked to download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddRequest {
    /// Link or `.torrent` file.
    pub source: AddSource,
    /// Destination directory; default: the settings' or the OS download folder.
    pub dest_dir: Option<String>,
    /// File name override (HTTP).
    pub file_name: Option<String>,
    /// Connections (1–32) for HTTP.
    pub connections: Option<u8>,
    /// Speed limit in bytes/second.
    pub speed_limit_bps: Option<u64>,
    /// Category label.
    pub category: Option<String>,
    /// HTTP: extra request headers.
    pub headers: Vec<HeaderView>,
    /// HTTP: `Cookie` header value.
    pub cookies: Option<String>,
    /// HTTP: `Referer` URL.
    pub referer: Option<String>,
    /// Checksum to verify after download.
    pub checksum: Option<ChecksumView>,
    /// Torrents: file indexes to download; `null` = all.
    pub selected_files: Option<Vec<u32>>,
}

/// A `.torrent` file's contents, shown before adding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TorrentInfoView {
    /// Torrent name.
    pub name: String,
    /// Info hash (hex).
    pub info_hash: String,
    /// Sum of all file sizes.
    pub total_size: u64,
    /// Files.
    pub files: Vec<FileView>,
}

impl From<&pv_engine::TorrentInfo> for TorrentInfoView {
    fn from(t: &pv_engine::TorrentInfo) -> Self {
        Self {
            name: t.name.clone(),
            info_hash: t.info_hash.clone(),
            total_size: t.files.iter().map(|f| f.size).sum(),
            files: t.files.iter().map(Into::into).collect(),
        }
    }
}

/// One HTTP segment, for the segment bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SegmentView {
    /// First byte.
    pub start: u64,
    /// Last byte (inclusive).
    pub end: u64,
    /// Bytes written.
    pub downloaded: u64,
}

/// Everything the detail drawer shows. Still no secrets: only whether some were sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetailView {
    /// The task.
    pub task: TaskView,
    /// Full path of the file (or torrent folder) once its name is known.
    pub path: Option<String>,
    /// HTTP: URL after redirects (without credentials).
    pub final_url: Option<String>,
    /// HTTP: segments with durable progress.
    pub segments: Vec<SegmentView>,
    /// Files (torrents: with selection).
    pub files: Vec<FileView>,
    /// Expected checksum, e.g. `SHA-256 3a7b…`.
    pub checksum: Option<String>,
    /// Connections override.
    pub connections: Option<u8>,
    /// Per-task speed limit.
    pub speed_limit_bps: Option<u64>,
    /// Whether cookies or custom headers are sent (their values stay in Rust).
    pub sends_credentials: bool,
}

impl From<&pv_engine::TaskDetail> for TaskDetailView {
    fn from(d: &pv_engine::TaskDetail) -> Self {
        let spec = &d.task.spec;
        let sends_credentials = matches!(
            &spec.source,
            Source::Http { headers, cookies, .. } if !headers.is_empty() || cookies.is_some()
        );
        Self {
            task: (&d.task).into(),
            path: spec.file_name.as_ref().map(|name| {
                pv_core::filename::contained_path(&spec.dest_dir, name)
                    .unwrap_or_else(|_| spec.dest_dir.clone())
                    .display()
                    .to_string()
            }),
            final_url: d.http.final_url.as_ref().map(|u| {
                display_source(&Source::Http {
                    url: u.clone(),
                    headers: Vec::new(),
                    cookies: None,
                    referer: None,
                })
            }),
            segments: d
                .segments
                .iter()
                .map(|s| SegmentView {
                    start: s.start,
                    end: s.end,
                    downloaded: s.downloaded,
                })
                .collect(),
            files: d.files.iter().map(Into::into).collect(),
            checksum: spec
                .checksum
                .as_ref()
                .map(|c| format!("{} {}", c.algorithm(), c.expected())),
            connections: spec.connections,
            speed_limit_bps: spec.speed_limit_bps,
            sends_credentials,
        }
    }
}

/// Where to move a queued task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MoveDirectionView {
    /// One place earlier.
    Up,
    /// One place later.
    Down,
    /// First.
    Top,
    /// Last.
    Bottom,
}

impl From<MoveDirectionView> for pv_engine::MoveDirection {
    fn from(d: MoveDirectionView) -> Self {
        match d {
            MoveDirectionView::Up => Self::Up,
            MoveDirectionView::Down => Self::Down,
            MoveDirectionView::Top => Self::Top,
            MoveDirectionView::Bottom => Self::Bottom,
        }
    }
}

/// Application settings (mirror of `pv_core::Settings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// Default destination; `None` = the OS download folder.
    pub download_dir: Option<String>,
    /// Tasks running at once.
    pub max_concurrent_tasks: u16,
    /// Connections for new HTTP tasks.
    pub default_connections: u8,
    /// Smallest segment worth splitting, bytes.
    pub min_segment_size: u64,
    /// Global download limit, bytes/second.
    pub global_speed_limit_bps: Option<u64>,
    /// Automatic retries.
    pub max_retries: u32,
    /// Idle connection timeout, seconds.
    pub idle_timeout_secs: u32,
    /// Connect timeout, seconds.
    pub connect_timeout_secs: u32,
    /// HTTP `User-Agent`.
    pub user_agent: String,
    /// Resume interrupted tasks at startup.
    pub auto_resume_on_startup: bool,
    /// Category labels.
    pub categories: Vec<String>,
    /// BitTorrent settings.
    pub torrent: TorrentSettingsView,
    /// Desktop integration.
    pub desktop: DesktopSettingsView,
}

/// Desktop integration settings (mirror of `pv_core::DesktopSettings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off switches shown as toggles"
)]
pub struct DesktopSettingsView {
    /// Closing the window keeps Persivio in the tray.
    pub close_to_tray: bool,
    /// Notify on completion.
    pub notify_on_complete: bool,
    /// Notify on failure.
    pub notify_on_failure: bool,
    /// Offer copied links.
    pub watch_clipboard: bool,
    /// Start with the system (in the tray).
    pub autostart: bool,
    /// Keep the computer awake while downloading.
    pub prevent_sleep: bool,
    /// Open `magnet:` links with Persivio.
    pub handle_magnet_links: bool,
    /// Accept downloads from the browser extension.
    pub browser_bridge: bool,
    /// Bridge port on 127.0.0.1.
    pub bridge_port: u16,
    /// Pairing token to paste into the extension (empty until the bridge is enabled).
    pub bridge_token: String,
    /// Check for updates at startup.
    pub check_for_updates: bool,
}

impl From<&pv_core::DesktopSettings> for DesktopSettingsView {
    fn from(d: &pv_core::DesktopSettings) -> Self {
        Self {
            close_to_tray: d.close_to_tray,
            notify_on_complete: d.notify_on_complete,
            notify_on_failure: d.notify_on_failure,
            watch_clipboard: d.watch_clipboard,
            autostart: d.autostart,
            prevent_sleep: d.prevent_sleep,
            handle_magnet_links: d.handle_magnet_links,
            browser_bridge: d.browser_bridge,
            bridge_port: d.bridge_port,
            bridge_token: d.bridge_token.clone(),
            check_for_updates: d.check_for_updates,
        }
    }
}

impl From<DesktopSettingsView> for pv_core::DesktopSettings {
    fn from(d: DesktopSettingsView) -> Self {
        Self {
            close_to_tray: d.close_to_tray,
            notify_on_complete: d.notify_on_complete,
            notify_on_failure: d.notify_on_failure,
            watch_clipboard: d.watch_clipboard,
            autostart: d.autostart,
            prevent_sleep: d.prevent_sleep,
            handle_magnet_links: d.handle_magnet_links,
            browser_bridge: d.browser_bridge,
            bridge_port: d.bridge_port,
            bridge_token: d.bridge_token,
            check_for_updates: d.check_for_updates,
        }
    }
}

/// Signal: external add requests are waiting (`take_external_requests`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct ExternalRequestsEvent;

/// Links found on the clipboard, offered to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct ClipboardLinksEvent {
    /// http(s) and magnet links.
    pub links: Vec<String>,
}

/// BitTorrent settings (mirror of `pv_core::TorrentSettings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off switches shown as toggles"
)]
pub struct TorrentSettingsView {
    /// Incoming peer port; `None` = automatic.
    pub listen_port: Option<u16>,
    /// Use the DHT.
    pub dht: bool,
    /// Ask the router to forward the port.
    pub upnp: bool,
    /// Stop when the download finishes.
    pub never_seed: bool,
    /// Stop seeding at this ratio, in percent.
    pub seed_ratio_limit_percent: Option<u32>,
    /// Stop seeding after this many minutes.
    pub seed_time_limit_mins: Option<u32>,
    /// Add `extraTrackers` to every torrent.
    pub use_extra_trackers: bool,
    /// Additional tracker URLs.
    pub extra_trackers: Vec<String>,
}

impl From<&Settings> for SettingsView {
    fn from(s: &Settings) -> Self {
        let t = &s.torrent;
        Self {
            download_dir: s.download_dir.as_ref().map(|d| d.display().to_string()),
            max_concurrent_tasks: s.max_concurrent_tasks,
            default_connections: s.default_connections,
            min_segment_size: s.min_segment_size,
            global_speed_limit_bps: s.global_speed_limit_bps,
            max_retries: s.max_retries,
            idle_timeout_secs: s.idle_timeout_secs,
            connect_timeout_secs: s.connect_timeout_secs,
            user_agent: s.user_agent.clone(),
            auto_resume_on_startup: s.auto_resume_on_startup,
            categories: s.categories.clone(),
            torrent: TorrentSettingsView {
                listen_port: t.listen_port,
                dht: t.dht,
                upnp: t.upnp,
                never_seed: t.never_seed,
                seed_ratio_limit_percent: t.seed_ratio_limit_percent,
                seed_time_limit_mins: t.seed_time_limit_mins,
                use_extra_trackers: t.use_extra_trackers,
                extra_trackers: t.extra_trackers.clone(),
            },
            desktop: (&s.desktop).into(),
        }
    }
}

impl From<SettingsView> for Settings {
    fn from(s: SettingsView) -> Self {
        let t = s.torrent;
        Self {
            download_dir: s.download_dir.filter(|d| !d.is_empty()).map(PathBuf::from),
            max_concurrent_tasks: s.max_concurrent_tasks,
            default_connections: s.default_connections,
            min_segment_size: s.min_segment_size,
            global_speed_limit_bps: s.global_speed_limit_bps,
            max_retries: s.max_retries,
            idle_timeout_secs: s.idle_timeout_secs,
            connect_timeout_secs: s.connect_timeout_secs,
            user_agent: s.user_agent,
            auto_resume_on_startup: s.auto_resume_on_startup,
            categories: s.categories,
            torrent: TorrentSettings {
                listen_port: t.listen_port,
                dht: t.dht,
                upnp: t.upnp,
                never_seed: t.never_seed,
                seed_ratio_limit_percent: t.seed_ratio_limit_percent,
                seed_time_limit_mins: t.seed_time_limit_mins,
                use_extra_trackers: t.use_extra_trackers,
                extra_trackers: t.extra_trackers,
            },
            desktop: s.desktop.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pv_core::{FailReason, TaskId, TaskSpec};

    use super::*;

    fn snapshot(source: Source) -> TaskSnapshot {
        TaskSnapshot::new(TaskSpec::new(source, std::env::temp_dir()))
    }

    #[test]
    fn task_view_never_contains_secrets() {
        let source = Source::Http {
            url: Url::parse("https://user:hunter2@example.com/files/a%20b.iso?sig=1").unwrap(),
            headers: vec![("Authorization".into(), "Bearer tok-SECRET".into())],
            cookies: Some("session=cookie-SECRET".into()),
            referer: None,
        };
        let view = TaskView::from(&snapshot(source));
        let json = serde_json::to_string(&view).unwrap();
        for secret in ["hunter2", "tok-SECRET", "cookie-SECRET", "Authorization"] {
            assert!(!json.contains(secret), "{secret} leaked: {json}");
        }
        assert_eq!(view.source, "https://example.com/files/a%20b.iso?sig=1");
        assert_eq!(view.name, "a b.iso", "decoded last path segment");
        assert_eq!(view.kind, TaskKindView::Http);
    }

    #[test]
    fn torrent_bytes_are_not_sent() {
        let view = TaskView::from(&snapshot(Source::TorrentFile {
            bytes: vec![0xAB; 64],
        }));
        assert_eq!(view.source, "torrent file");
        assert!(!serde_json::to_string(&view).unwrap().contains("abab"));
    }

    #[test]
    fn magnet_names_come_from_dn() {
        let view = TaskView::from(&snapshot(
            Source::magnet("magnet:?xt=urn:btih:abc&dn=Debian%2013.iso").unwrap(),
        ));
        assert_eq!(view.name, "Debian 13.iso");
        assert_eq!(view.kind, TaskKindView::Torrent);
    }

    #[test]
    fn json_shapes_match_the_generated_typescript() {
        let failed = TaskStateView::from(&TaskState::Failed {
            reason: FailReason::LinkExpired,
            retryable: false,
        });
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            serde_json::json!({"type": "failed", "message": "download link expired", "retryable": false})
        );
        let event = EngineUiEvent::from_engine(&EngineEvent::TaskRemoved(TaskId::new())).unwrap();
        assert_eq!(
            serde_json::to_value(&event).unwrap()["type"],
            "task_removed"
        );
        assert!(EngineUiEvent::from_engine(&EngineEvent::ProgressBatch(vec![])).is_none());
    }

    #[test]
    fn settings_round_trip() {
        let mut settings = Settings {
            download_dir: Some(std::env::temp_dir()),
            ..Settings::default()
        };
        settings.torrent.extra_trackers = vec!["udp://t.example:1337".into()];
        let back = Settings::from(SettingsView::from(&settings));
        assert_eq!(back, settings);
        // An empty folder field means "default".
        let mut view = SettingsView::from(&settings);
        view.download_dir = Some(String::new());
        assert_eq!(Settings::from(view).download_dir, None);
    }
}
