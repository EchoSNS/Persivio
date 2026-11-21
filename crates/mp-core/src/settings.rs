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
    /// BitTorrent settings.
    pub torrent: TorrentSettings,
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
            user_agent: format!("Millipede/{}", env!("CARGO_PKG_VERSION")),
            auto_resume_on_startup: false,
            torrent: TorrentSettings::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_sensible() {
        let s = Settings::default();
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(s.default_connections, 8);
        assert_eq!(s.min_segment_size, 2 * MIB);
        assert!(s.user_agent.starts_with("Millipede/"));
        assert!(!s.torrent.use_extra_trackers, "extra trackers are opt-in");
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        type Mutate = fn(&mut Settings);
        let cases: [(&str, Mutate); 9] = [
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
