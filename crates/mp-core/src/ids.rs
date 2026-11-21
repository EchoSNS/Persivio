//! Identifiers and timestamps.

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier of a download task.
///
/// Backed by a UUID **v7**, whose leading bits are a millisecond timestamp, so ids sort
/// by creation time — handy for "oldest first" queues and readable database dumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Uuid);

impl TaskId {
    /// Generates a new, time-ordered id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wraps an existing UUID (e.g. one loaded from the database).
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// A wall-clock instant as milliseconds since the Unix epoch (UTC).
///
/// A plain integer keeps the type trivially serializable, storable in SQLite and
/// comparable without pulling in a date-time library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixMillis(pub i64);

impl UnixMillis {
    /// The current wall-clock time. Clamps to 0 if the system clock is before 1970.
    #[must_use]
    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        Self(millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_ids_are_unique_and_time_ordered() {
        let a = TaskId::new();
        let b = TaskId::new();
        assert_ne!(a, b);
        assert!(a < b, "UUID v7 ids generated later must sort later");
        assert_eq!(a.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn task_id_display_parse_round_trip() {
        let id = TaskId::default();
        let parsed: TaskId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
        assert_eq!(TaskId::from_uuid(*id.as_uuid()), id);
        assert!("not-a-uuid".parse::<TaskId>().is_err());
    }

    #[test]
    fn task_id_serializes_as_plain_string() {
        let id = TaskId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<TaskId>(&json).unwrap(), id);
    }

    #[test]
    fn unix_millis_now_is_recent() {
        // 2020-01-01T00:00:00Z in millis; any sane clock is past this.
        assert!(UnixMillis::now().0 > 1_577_836_800_000);
        assert_eq!(serde_json::to_string(&UnixMillis(42)).unwrap(), "42");
    }
}
