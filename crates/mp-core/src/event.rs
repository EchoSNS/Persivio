//! Events the engine broadcasts to every front end (ADR-002).

use serde::{Deserialize, Serialize};

use crate::ids::TaskId;
use crate::progress::Progress;
use crate::state::TaskState;
use crate::task::{FileEntry, TaskSnapshot};

/// Something that happened in the engine.
///
/// Front ends apply these to their local view. Because broadcast receivers can lag and
/// drop events, a front end that sees a gap must resync with the engine's `list()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EngineEvent {
    /// A task was created. Boxed: a snapshot is ~500 bytes and would bloat every event.
    TaskAdded(Box<TaskSnapshot>),
    /// A task changed state.
    StateChanged {
        /// Which task.
        id: TaskId,
        /// Its new state.
        state: TaskState,
    },
    /// Progress for all active tasks, emitted every ~250 ms (ADR-010).
    ProgressBatch(Vec<Progress>),
    /// Name/size/files became known (HTTP probe or torrent metadata).
    MetadataResolved {
        /// Which task.
        id: TaskId,
        /// Resolved display/file name.
        name: String,
        /// Total size, if known.
        size: Option<u64>,
        /// Files (one entry for HTTP; many for torrents).
        files: Vec<FileEntry>,
    },
    /// A task was removed.
    TaskRemoved(TaskId),
    /// A non-fatal problem worth showing to the user.
    Warning {
        /// Related task, if any.
        id: Option<TaskId>,
        /// Human-readable message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use crate::task::TaskSpec;

    #[test]
    fn every_event_round_trips_through_json() {
        let id = TaskId::new();
        let snap = TaskSnapshot::new(TaskSpec::new(
            Source::magnet("magnet:?xt=urn:btih:abc").unwrap(),
            std::env::temp_dir(),
        ));
        let events = [
            EngineEvent::TaskAdded(Box::new(snap)),
            EngineEvent::StateChanged {
                id,
                state: TaskState::Downloading,
            },
            EngineEvent::ProgressBatch(vec![Progress {
                task_id: id,
                downloaded: 1,
                total: Some(2),
                speed_bps: 3,
                eta_secs: Some(1),
                connections_active: 1,
                peers: Some(4),
                upload_bps: Some(5),
            }]),
            EngineEvent::MetadataResolved {
                id,
                name: "ubuntu.iso".into(),
                size: Some(9),
                files: vec![FileEntry {
                    index: 0,
                    path: "ubuntu.iso".into(),
                    size: 9,
                    selected: true,
                }],
            },
            EngineEvent::TaskRemoved(id),
            EngineEvent::Warning {
                id: None,
                message: "disk almost full".into(),
            },
        ];
        for event in events {
            let json = serde_json::to_value(&event).unwrap();
            assert!(json["type"].is_string());
            assert_eq!(serde_json::from_value::<EngineEvent>(json).unwrap(), event);
        }
    }
}
