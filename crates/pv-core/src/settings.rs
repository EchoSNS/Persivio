//! User-tunable settings.
//!
//! Every field has a default and the structs use `#[serde(default)]`, so settings saved
//! by an older version (missing new fields) still load, and unknown fields from a newer
//! version are ignored. That keeps upgrades and downgrades from wiping user settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::task::MAX_CONNECTIONS;

/// 1 MiB.
const MIB: u64 = 1024 * 1024;

/// Application-wide settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Default destination; `None` = the OS downloads directory (resolved by the engine).
    pub download_dir: Option<PathBuf>,
    /// Maximum tasks running at once.
    pub max_concurrent_tasks: u16,
    /// Connections for a new HTTP task (1..=32).
    pub default_connections: u8,
    /// Never split segments smaller than this (bytes).
    pub min_segment_size: u64,
    /// Global download limit in bytes/second; `None` = unlimited.
    pub global_speed_limit_bps: Option<u64>,
    /// Automatic retries per failure before a task is marked failed.
    pub max_retries: u32,
    /// Abort a connection that receives nothing for this many seconds.
    pub idle_timeout_secs: u32,
    /// TCP/TLS connect timeout in seconds.
    pub connect_timeout_secs: u32,
    /// `User-Agent` header for HTTP requests.
    pub user_agent: String,
    /// Resume tasks that were running when the app last exited (else they stay paused).
    pub auto_resume_on_startup: bool,
    /// Category labels offered in the UI (sidebar filters, add dialog).
    pub categories: Vec<String>,
    /// BitTorrent settings.
    pub torrent: TorrentSettings,
    /// Desktop app integration (ignored by the CLI).
    pub desktop: DesktopSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: None,
            max_concurrent_tasks: 3,
            default_connections: 8,
            min_segment_size: 2 * MIB,
            global_speed_limit_bps: None,
            max_retries: 5,
            idle_timeout_secs: 30,
            connect_timeout_secs: 15,
            user_agent: format!("Persivio/{}", env!("CARGO_PKG_VERSION")),
            auto_resume_on_startup: false,
            categories: ["Video", "Music", "Documents", "Software", "Archives"]
                .map(String::from)
                .to_vec(),
            torrent: TorrentSettings::default(),
            desktop: DesktopSettings::default(),
        }
    }
}

/// Default loopback port of the browser bridge.
pub const DEFAULT_BRIDGE_PORT: u16 = 47_321;
/// Shortest accepted bridge token (hex characters = 128 bits).
pub const MIN_BRIDGE_TOKEN_LEN: usize = 32;

/// How the desktop app integrates with the operating system (Phase 9, ADR-020).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off switches shown as toggles in the UI"
)]
pub struct DesktopSettings {
    /// Closing the window keeps Persivio running in the system tray.
    pub close_to_tray: bool,
    /// Notify when a download completes.
    pub notify_on_complete: bool,
    /// Notify when a download fails.
    pub notify_on_failure: bool,
    /// Offer to add http(s) and magnet links copied to the clipboard (opt-in).
    pub watch_clipboard: bool,
    /// Start Persivio (minimized to the tray) when the user logs in.
    pub autostart: bool,
    /// Keep the computer from sleeping while downloads run.
    pub prevent_sleep: bool,
    /// Open `magnet:` links with Persivio (Windows and Linux; macOS asks the OS).
    pub handle_magnet_links: bool,
    /// Accept downloads from the browser extension on 127.0.0.1 (Phase 10, ADR-021).
    pub browser_bridge: bool,
    /// Loopback port of the browser bridge.
    pub bridge_port: u16,
    /// Pairing token the extension must send (hex). Generated when the bridge is enabled.
    pub bridge_token: String,
    /// Check for app updates at startup.
    pub check_for_updates: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            notify_on_complete: true,
            notify_on_failure: true,
            watch_clipboard: false,
            autostart: false,
            prevent_sleep: true,
            // Opt-in: never take over another torrent client's links silently.
            handle_magnet_links: false,
            browser_bridge: false,
            bridge_port: DEFAULT_BRIDGE_PORT,
            bridge_token: String::new(),
            check_for_updates: true,
        }
    }
}

/// BitTorrent settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off switches shown as toggles in the UI"
)]
pub struct TorrentSettings {
    /// Incoming peer port; `None` = let the engine choose.
    pub listen_port: Option<u16>,
    /// Use the DHT for peer discovery.
    pub dht: bool,
    /// Ask the router to forward the listen port.
    pub upnp: bool,
    /// Stop as soon as the download finishes (no seeding).
    pub never_seed: bool,
    /// Stop seeding at this upload/download ratio (e.g. 1.0 = 100). Stored ×100 to stay `Eq`.
    pub seed_ratio_limit_percent: Option<u32>,
    /// Stop seeding after this many minutes.
    pub seed_time_limit_mins: Option<u32>,
    /// Add `extra_trackers` to every torrent (off by default; see risks in the plan).
    pub use_extra_trackers: bool,
    /// Additional tracker URLs.
    pub extra_trackers: Vec<String>,
}

impl Default for TorrentSettings {
    fn default() -> Self {
        Self {
            listen_port: None,
            dht: true,
            upnp: true,
            never_seed: false,
            seed_ratio_limit_percent: Some(100),
            seed_time_limit_mins: None,
            use_extra_trackers: false,
            extra_trackers: Vec::new(),
        }
    }
}

/// An out-of-range setting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid setting `{field}`: {message}")]
pub struct SettingsError {
    /// Field name.
    pub field: &'static str,
    /// What is wrong.
    pub message: String,
}

impl SettingsError {
    fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl Settings {
    /// Validates ranges that the type system cannot express.
    ///
    /// # Errors
    /// The first [`SettingsError`] found.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !(1..=256).contains(&self.max_concurrent_tasks) {
            return Err(SettingsError::new(
                "max_concurrent_tasks",
                "must be 1..=256",
            ));
        }
        if !(1..=MAX_CONNECTIONS).contains(&self.default_connections) {
            return Err(SettingsError::new(
                "default_connections",
                format!("must be 1..={MAX_CONNECTIONS}"),
            ));
        }
        if self.min_segment_size < 64 * 1024 {
            return Err(SettingsError::new(
                "min_segment_size",
                "must be at least 64 KiB",
            ));
        }
        if self.global_speed_limit_bps == Some(0) {
            return Err(SettingsError::new(
                "global_speed_limit_bps",
                "must be > 0 or unset",
            ));
        }
        if self.idle_timeout_secs == 0 || self.connect_timeout_secs == 0 {
            return Err(SettingsError::new("timeouts", "must be > 0"));
        }
        if self.user_agent.trim().is_empty()
            || self.user_agent.bytes().any(|b| b.is_ascii_control())
        {
            return Err(SettingsError::new(
                "user_agent",
                "must be non-empty printable text",
            ));
        }
        if let Some(message) = category_problem(&self.categories) {
            return Err(SettingsError::new("categories", message));
        }
        let d = &self.desktop;
        if d.bridge_port < 1024 {
            return Err(SettingsError::new(
                "desktop.bridge_port",
                "must be 1024..=65535",
            ));
        }
        let token_ok = d.bridge_token.is_empty()
            || (d.bridge_token.len() >= MIN_BRIDGE_TOKEN_LEN
                && d.bridge_token.bytes().all(|b| b.is_ascii_hexdigit()));
        if !token_ok || (d.browser_bridge && d.bridge_token.is_empty()) {
            return Err(SettingsError::new(
                "desktop.bridge_token",
                format!("must be at least {MIN_BRIDGE_TOKEN_LEN} hex characters"),
            ));
        }
        if self.torrent.listen_port == Some(0) {
            return Err(SettingsError::new(
                "torrent.listen_port",
                "must be 1..=65535 or unset",
            ));
        }
        if let Some(bad) = self.torrent.extra_trackers.iter().find(|t| {
            !(t.starts_with("http://") || t.starts_with("https://") || t.starts_with("udp://"))
        }) {
            return Err(SettingsError::new(
                "torrent.extra_trackers",
                format!("unsupported tracker URL `{bad}`"),
            ));
        }
        Ok(())
    }
}

/// Longest category label, in characters.
pub const MAX_CATEGORY_LEN: usize = 40;

fn category_problem(categories: &[String]) -> Option<String> {
    if categories.len() > 100 {
        return Some("at most 100 categories".to_owned());
    }
    let mut seen = std::collections::HashSet::new();
    for c in categories {
        if c.trim().is_empty() || c.trim() != c {
            return Some("names must be non-empty and have no surrounding spaces".to_owned());
        }
        if c.chars().count() > MAX_CATEGORY_LEN || c.chars().any(char::is_control) {
            return Some(format!("`{c}` is too long or has control characters"));
        }
        if !seen.insert(c.to_lowercase()) {
            return Some(format!("`{c}` is listed twice"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_sensible() {
        let s = Settings::default();
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.default_connections, 8);
        assert_eq!(s.min_segment_size, 2 * MIB);
        assert!(s.user_agent.starts_with("Persivio/"));
        assert!(!s.torrent.use_extra_trackers, "extra trackers are opt-in");
        assert!(!s.desktop.watch_clipboard, "clipboard watching is opt-in");
        assert!(!s.desktop.autostart && !s.desktop.handle_magnet_links);
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        type Mutate = fn(&mut Settings);
        let cases: [(&str, Mutate); 15] = [
            ("desktop.bridge_port", |s| s.desktop.bridge_port = 80),
            ("desktop.bridge_token", |s| {
                s.desktop.bridge_token = "abc".into();
            }),
            ("desktop.bridge_token", |s| s.desktop.browser_bridge = true),
            ("categories", |s| s.categories = vec![" Video".into()]),
            ("categories", |s| {
                s.categories = vec!["A".into(), "a".into()];
            }),
            ("categories", |s| s.categories = vec!["x".repeat(41)]),
            ("max_concurrent_tasks", |s| s.max_concurrent_tasks = 0),
            ("default_connections", |s| s.default_connections = 33),
            ("min_segment_size", |s| s.min_segment_size = 1),
            ("global_speed_limit_bps", |s| {
                s.global_speed_limit_bps = Some(0);
            }),
            ("timeouts", |s| s.idle_timeout_secs = 0),
            ("user_agent", |s| s.user_agent = " ".into()),
            ("user_agent", |s| s.user_agent = "a\nb".into()),
            ("torrent.listen_port", |s| s.torrent.listen_port = Some(0)),
            ("torrent.extra_trackers", |s| {
                s.torrent.extra_trackers = vec!["file:///x".into()];
            }),
        ];
        for (field, mutate) in cases {
            let mut s = Settings::default();
            mutate(&mut s);
            let err = s.validate().unwrap_err();
            assert_eq!(err.field, field);
            assert!(err.to_string().contains(field));
        }
        let mut ok = Settings::default();
        ok.torrent.extra_trackers = vec!["udp://tracker.example:1337/announce".into()];
        assert_eq!(ok.validate(), Ok(()));
    }

    #[test]
    fn partial_and_future_json_loads_with_defaults() {
        // Old version: only one field. New version: an unknown field.
        let s: Settings = serde_json::from_str(
            r#"{"max_concurrent_tasks": 7, "torrent": {"dht": false}, "some_future_field": 1}"#,
        )
        .unwrap();
        assert_eq!(s.max_concurrent_tasks, 7);
        assert!(!s.torrent.dht);
        assert!(s.torrent.upnp, "missing nested fields take defaults");
        assert_eq!(s.default_connections, 8);

        let full = serde_json::to_string(&Settings::default()).unwrap();
        assert_eq!(
            serde_json::from_str::<Settings>(&full).unwrap(),
            Settings::default()
        );
    }
}
