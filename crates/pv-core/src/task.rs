//! Task requests ([`TaskSpec`]), engine-reported views ([`TaskSnapshot`]) and the pieces
//! a task is made of ([`Segment`], [`FileEntry`]).

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::checksum::Checksum;
use crate::ids::{TaskId, UnixMillis};
use crate::source::{Source, SourceError};
use crate::state::TaskState;

/// Upper bound on connections per HTTP task.
pub const MAX_CONNECTIONS: u8 = 32;

/// Protocol family of a task (stored in the `tasks.kind` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// HTTP or HTTPS.
    Http,
    /// BitTorrent (magnet or `.torrent`).
    Torrent,
}

impl TaskKind {
    /// Stable lower-case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Torrent => "torrent",
        }
    }
}

impl fmt::Display for TaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "http" => Ok(Self::Http),
            "torrent" => Ok(Self::Torrent),
            other => Err(format!("unknown task kind `{other}`")),
        }
    }
}

/// What the user asked to download. Immutable intent; the engine's view is
/// [`TaskSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Where to download from.
    pub source: Source,
    /// Directory the finished file(s) go into.
    pub dest_dir: PathBuf,
    /// File name override; `None` = resolve from the server / torrent.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Connection count override (1..=[`MAX_CONNECTIONS`]); `None` = settings default.
    #[serde(default)]
    pub connections: Option<u8>,
    /// Per-task speed limit in bytes/second; `None` = unlimited (global limit still applies).
    #[serde(default)]
    pub speed_limit_bps: Option<u64>,
    /// Expected checksum to verify after download.
    #[serde(default)]
    pub checksum: Option<Checksum>,
    /// Category label (e.g. "Video") used for filtering and destination rules.
    #[serde(default)]
    pub category: Option<String>,
    /// Torrents: indexes of the files to download; `None` = all files.
    #[serde(default)]
    pub selected_files: Option<Vec<u32>>,
}

/// Why a [`TaskSpec`] is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// The source is invalid.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// `connections` outside 1..=32.
    #[error("connections must be between 1 and {MAX_CONNECTIONS}, got {0}")]
    Connections(u8),
    /// A speed limit of zero would stall the task forever.
    #[error("speed limit must be greater than zero")]
    ZeroSpeedLimit,
    /// `dest_dir` must be an absolute path (relative paths depend on the process cwd).
    #[error("destination directory must be an absolute path")]
    RelativeDestination,
    /// A file name override must be a plain name, not a path.
    #[error("file name must not be empty or contain path separators")]
    InvalidFileName,
    /// A category label that is empty, too long or has control characters.
    #[error("category must be 1 to 40 printable characters")]
    InvalidCategory,
}

impl TaskSpec {
    /// A spec with defaults for every optional field.
    #[must_use]
    pub fn new(source: Source, dest_dir: impl Into<PathBuf>) -> Self {
        Self {
            source,
            dest_dir: dest_dir.into(),
            file_name: None,
            connections: None,
            speed_limit_bps: None,
            checksum: None,
            category: None,
            selected_files: None,
        }
    }

    /// Validates the spec. Full filename sanitization happens in `pv-http`; this only
    /// rejects overrides that are obviously paths.
    ///
    /// # Errors
    /// The first [`SpecError`] found.
    pub fn validate(&self) -> Result<(), SpecError> {
        self.source.validate()?;
        if let Some(n) = self.connections
            && !(1..=MAX_CONNECTIONS).contains(&n)
        {
            return Err(SpecError::Connections(n));
        }
        if self.speed_limit_bps == Some(0) {
            return Err(SpecError::ZeroSpeedLimit);
        }
        if !self.dest_dir.is_absolute() {
            return Err(SpecError::RelativeDestination);
        }
        if let Some(name) = &self.file_name
            && (name.trim().is_empty() || name.contains(['/', '\\']) || name == "." || name == "..")
        {
            return Err(SpecError::InvalidFileName);
        }
        if let Some(c) = &self.category
            && (c.trim().is_empty()
                || c.chars().count() > crate::settings::MAX_CATEGORY_LEN
                || c.chars().any(char::is_control))
        {
            return Err(SpecError::InvalidCategory);
        }
        Ok(())
    }
}

/// A contiguous byte range of an HTTP download handled by one worker.
///
/// `end` is **inclusive**, matching HTTP `Range: bytes=start-end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Position in the segment table.
    pub index: u32,
    /// First byte offset.
    pub start: u64,
    /// Last byte offset (inclusive).
    pub end: u64,
    /// Bytes of this segment durably written so far.
    pub downloaded: u64,
}

impl Segment {
    /// Total bytes in the range.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Always false: a segment covers at least one byte. Provided for clippy's
    /// `len_without_is_empty` convention.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Bytes still to fetch.
    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.len().saturating_sub(self.downloaded)
    }

    /// Offset of the next byte to fetch.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.start + self.downloaded
    }

    /// All bytes written.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.downloaded >= self.len()
    }
}

/// One file inside a torrent (or the single file of an HTTP task).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Index within the torrent's file list.
    pub index: u32,
    /// Relative path, `/`-separated (sanitized before touching disk).
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Whether the user chose to download it.
    pub selected: bool,
}

/// What an HTTP resume needs to know about the remote file (ADR-007).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResumeInfo {
    /// URL after redirects.
    pub final_url: Option<Url>,
    /// Strong `ETag`, if the server sent one.
    pub etag: Option<String>,
    /// `Last-Modified` header value.
    pub last_modified: Option<String>,
}

/// The engine's view of a task, sent to front ends and persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    /// Task id.
    pub id: TaskId,
    /// What was requested. `spec.file_name` holds the resolved name once known.
    pub spec: TaskSpec,
    /// Current state.
    pub state: TaskState,
    /// Total size in bytes, once known.
    pub total_size: Option<u64>,
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Creation time.
    pub created_at: UnixMillis,
    /// Last change.
    pub updated_at: UnixMillis,
    /// When the task completed.
    pub completed_at: Option<UnixMillis>,
}

impl TaskSnapshot {
    /// A fresh `Queued` snapshot for a new spec.
    #[must_use]
    pub fn new(spec: TaskSpec) -> Self {
        let now = UnixMillis::now();
        Self {
            id: TaskId::new(),
            spec,
            state: TaskState::Queued,
            total_size: None,
            downloaded: 0,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    /// Protocol family, derived from the source.
    #[must_use]
    pub const fn kind(&self) -> TaskKind {
        self.spec.source.kind()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::ChecksumAlgorithm;

    fn abs_dir() -> PathBuf {
        std::env::temp_dir()
    }

    fn http_spec() -> TaskSpec {
        TaskSpec::new(
            Source::http(Url::parse("https://example.com/f.bin").unwrap()).unwrap(),
            abs_dir(),
        )
    }

    #[test]
    fn spec_validation() {
        assert_eq!(http_spec().validate(), Ok(()));

        let mut s = http_spec();
        s.connections = Some(0);
        assert_eq!(s.validate(), Err(SpecError::Connections(0)));
        s.connections = Some(33);
        assert_eq!(s.validate(), Err(SpecError::Connections(33)));
        s.connections = Some(32);
        assert_eq!(s.validate(), Ok(()));

        let mut s = http_spec();
        s.speed_limit_bps = Some(0);
        assert_eq!(s.validate(), Err(SpecError::ZeroSpeedLimit));

        let mut s = http_spec();
        s.category = Some(
            "a
b"
            .into(),
        );
        assert_eq!(s.validate(), Err(SpecError::InvalidCategory));
        s.category = Some("Video".into());
        assert_eq!(s.validate(), Ok(()));

        let mut s = http_spec();
        s.dest_dir = PathBuf::from("relative/dir");
        assert_eq!(s.validate(), Err(SpecError::RelativeDestination));

        for bad in ["", "  ", "a/b", "a\\b", ".", ".."] {
            let mut s = http_spec();
            s.file_name = Some(bad.into());
            assert_eq!(s.validate(), Err(SpecError::InvalidFileName), "{bad:?}");
        }

        let s = TaskSpec::new(Source::Magnet { uri: "nope".into() }, abs_dir());
        assert!(matches!(s.validate(), Err(SpecError::Source(_))));
    }

    #[test]
    fn segment_arithmetic_uses_inclusive_end() {
        let seg = Segment {
            index: 0,
            start: 100,
            end: 199,
            downloaded: 40,
        };
        assert_eq!(seg.len(), 100);
        assert!(!seg.is_empty());
        assert_eq!(seg.remaining(), 60);
        assert_eq!(seg.position(), 140);
        assert!(!seg.is_complete());
        let done = Segment {
            downloaded: 100,
            ..seg
        };
        assert!(done.is_complete());
        assert_eq!(done.remaining(), 0);
        let single = Segment {
            index: 1,
            start: 5,
            end: 5,
            downloaded: 0,
        };
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn task_kind_names() {
        for kind in [TaskKind::Http, TaskKind::Torrent] {
            assert_eq!(kind.as_str().parse::<TaskKind>().unwrap(), kind);
            assert_eq!(kind.to_string(), kind.as_str());
        }
        assert!("ftp".parse::<TaskKind>().is_err());
    }

    #[test]
    fn snapshot_new_and_round_trip() {
        let mut spec = http_spec();
        spec.checksum = Some(Checksum::new(ChecksumAlgorithm::Md5, &"a".repeat(32)).unwrap());
        spec.category = Some("Video".into());
        spec.file_name = Some("f.bin".into());
        spec.connections = Some(8);
        spec.speed_limit_bps = Some(1024);
        let snap = TaskSnapshot::new(spec);
        assert_eq!(snap.state, TaskState::Queued);
        assert_eq!(snap.kind(), TaskKind::Http);
        assert_eq!(snap.created_at, snap.updated_at);

        let json = serde_json::to_string(&snap).unwrap();
        assert_eq!(serde_json::from_str::<TaskSnapshot>(&json).unwrap(), snap);

        for value in [
            serde_json::to_string(&Segment {
                index: 2,
                start: 0,
                end: 9,
                downloaded: 3,
            })
            .unwrap(),
            serde_json::to_string(&FileEntry {
                index: 0,
                path: "dir/a.txt".into(),
                size: 9,
                selected: true,
            })
            .unwrap(),
            serde_json::to_string(&HttpResumeInfo {
                final_url: Some(Url::parse("https://cdn.example/f").unwrap()),
                etag: Some("\"abc\"".into()),
                last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".into()),
            })
            .unwrap(),
        ] {
            assert_ne!(value, "");
        }
        let info: HttpResumeInfo =
            serde_json::from_str(r#"{"final_url":null,"etag":"\"e\"","last_modified":null}"#)
                .unwrap();
        assert_eq!(info.etag.as_deref(), Some("\"e\""));
    }

    #[test]
    fn spec_optional_fields_default_when_missing() {
        let json = format!(
            r#"{{"source":{{"type":"magnet","uri":"magnet:?xt=1"}},"dest_dir":{}}}"#,
            serde_json::to_string(&abs_dir()).unwrap()
        );
        let spec: TaskSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec.connections, None);
        assert_eq!(spec.validate(), Ok(()));
    }
}
