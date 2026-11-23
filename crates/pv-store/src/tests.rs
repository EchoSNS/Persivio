//! Store tests against a real temp-file database (not `:memory:`), so WAL, reopening
//! and multi-connection behavior match production.

use std::path::PathBuf;

use pv_core::{
    CheckpointSink, Checksum, ChecksumAlgorithm, FailReason, HttpResumeInfo, Segment, Settings,
    Source, TaskId, TaskSnapshot, TaskSpec, TaskState, TaskStateKind, UnixMillis,
};
use tempfile::TempDir;
use url::Url;

use crate::{Store, StoreError};

async fn open_temp() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("persivio.db")).await.unwrap();
    (dir, store)
}

fn dest() -> PathBuf {
    std::env::temp_dir().join("downloads")
}

/// An HTTP task with every optional field populated.
fn full_http_task() -> TaskSnapshot {
    let mut spec = TaskSpec::new(
        Source::Http {
            url: Url::parse("https://example.com/files/big file.iso?x=1").unwrap(),
            headers: vec![("X-Custom".into(), "1".into())],
            cookies: Some("session=abc".into()),
            referer: Some(Url::parse("https://example.com/").unwrap()),
        },
        dest(),
    );
    spec.file_name = Some("big file.iso".into());
    spec.connections = Some(16);
    spec.speed_limit_bps = Some(1_000_000);
    spec.checksum = Some(Checksum::new(ChecksumAlgorithm::Sha256, &"ab".repeat(32)).unwrap());
    spec.category = Some("ISO".into());
    let mut task = TaskSnapshot::new(spec);
    task.total_size = Some(10 * 1024 * 1024 * 1024); // 10 GiB: > u32
    task
}

fn segments() -> Vec<Segment> {
    vec![
        Segment {
            index: 0,
            start: 0,
            end: 99,
            downloaded: 10,
        },
        Segment {
            index: 1,
            start: 100,
            end: 199,
            downloaded: 0,
        },
        Segment {
            index: 2,
            start: 200,
            end: 249,
            downloaded: 50,
        },
    ]
}

#[tokio::test]
async fn open_enables_wal_and_foreign_keys_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.pragma("PRAGMA journal_mode").await.unwrap(), "wal");
    assert_eq!(store.pragma("PRAGMA foreign_keys").await.unwrap(), "1");
    // NORMAL = 1
    assert_eq!(store.pragma("PRAGMA synchronous").await.unwrap(), "1");
    store.close().await;
    // Re-opening re-runs the migrator, which must be a no-op.
    let again = Store::open(&path).await.unwrap();
    assert_eq!(again.load_all().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn task_round_trip_preserves_every_field() {
    let (_dir, store) = open_temp().await;
    let http = full_http_task();
    let magnet = TaskSnapshot::new(TaskSpec::new(
        Source::magnet("magnet:?xt=urn:btih:0123456789abcdef").unwrap(),
        dest(),
    ));
    let torrent = TaskSnapshot::new(TaskSpec::new(
        Source::TorrentFile {
            bytes: vec![b'd', 0, 0xff, b'e'],
        },
        dest(),
    ));
    for t in [&http, &magnet, &torrent] {
        store.insert_task(t).await.unwrap();
    }

    let loaded = store.load_all().await.unwrap();
    let snapshots: Vec<_> = loaded.iter().map(|t| t.snapshot.clone()).collect();
    assert_eq!(
        snapshots,
        vec![http.clone(), magnet, torrent],
        "oldest first, all fields equal"
    );
    assert_eq!(loaded[0].http, HttpResumeInfo::default());

    let one = store.get_task(http.id).await.unwrap().unwrap();
    assert_eq!(one.snapshot, http);
    assert!(store.get_task(TaskId::new()).await.unwrap().is_none());
}

#[tokio::test]
async fn every_state_round_trips() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    for kind in TaskStateKind::ALL {
        let state = match kind {
            TaskStateKind::Failed => TaskState::Failed {
                reason: FailReason::DiskFull {
                    needed: 5,
                    available: 1,
                },
                retryable: true,
            },
            TaskStateKind::Queued => TaskState::Queued,
            TaskStateKind::Probing => TaskState::Probing,
            TaskStateKind::FetchingMetadata => TaskState::FetchingMetadata,
            TaskStateKind::Downloading => TaskState::Downloading,
            TaskStateKind::Paused => TaskState::Paused,
            TaskStateKind::Verifying => TaskState::Verifying,
            TaskStateKind::Seeding => TaskState::Seeding,
            TaskStateKind::Completed => TaskState::Completed,
            TaskStateKind::Cancelled => TaskState::Cancelled,
        };
        store
            .update_state(task.id, &state, UnixMillis(42))
            .await
            .unwrap();
        let loaded = store.get_task(task.id).await.unwrap().unwrap().snapshot;
        assert_eq!(loaded.state, state, "{kind}");
        assert_eq!(loaded.updated_at, UnixMillis(42));
    }
}

#[tokio::test]
async fn completed_at_is_set_once_and_kept() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    store
        .update_state(task.id, &TaskState::Completed, UnixMillis(100))
        .await
        .unwrap();
    // A later unrelated update must not clear it.
    store
        .update_state(task.id, &TaskState::Completed, UnixMillis(200))
        .await
        .unwrap();
    let t = store.get_task(task.id).await.unwrap().unwrap().snapshot;
    assert_eq!(t.completed_at, Some(UnixMillis(200)));
    store
        .update_state(task.id, &TaskState::Paused, UnixMillis(300))
        .await
        .unwrap();
    let t = store.get_task(task.id).await.unwrap().unwrap().snapshot;
    assert_eq!(t.completed_at, Some(UnixMillis(200)));
}

#[tokio::test]
async fn updates_on_missing_task_report_not_found() {
    let (_dir, store) = open_temp().await;
    let id = TaskId::new();
    assert!(matches!(
        store.update_state(id, &TaskState::Paused, UnixMillis(1)).await,
        Err(StoreError::NotFound(x)) if x == id
    ));
    assert!(matches!(
        store
            .update_metadata(id, None, None, &HttpResumeInfo::default(), UnixMillis(1))
            .await,
        Err(StoreError::NotFound(_))
    ));
    assert!(!store.delete_task(id).await.unwrap());
}

#[tokio::test]
async fn metadata_round_trip_and_name_kept_when_none() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    let info = HttpResumeInfo {
        final_url: Some(Url::parse("https://cdn.example/real.iso").unwrap()),
        etag: Some("\"v1\"".into()),
        last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".into()),
    };
    store
        .update_metadata(
            task.id,
            Some("real.iso"),
            Some(12_345),
            &info,
            UnixMillis(5),
        )
        .await
        .unwrap();
    let t = store.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(t.http, info);
    assert_eq!(t.snapshot.spec.file_name.as_deref(), Some("real.iso"));
    assert_eq!(t.snapshot.total_size, Some(12_345));

    store
        .update_metadata(
            task.id,
            None,
            None,
            &HttpResumeInfo::default(),
            UnixMillis(6),
        )
        .await
        .unwrap();
    let t = store.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(
        t.snapshot.spec.file_name.as_deref(),
        Some("real.iso"),
        "kept"
    );
    assert_eq!(t.snapshot.total_size, None);
    assert_eq!(t.http, HttpResumeInfo::default());
}

#[tokio::test]
async fn segments_upsert_replaces_table_and_sums_downloaded() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    store.upsert_segments(task.id, &segments()).await.unwrap();
    let t = store.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(t.segments, segments());
    assert_eq!(t.snapshot.downloaded, 60);

    // Re-plan with fewer segments: stale rows must disappear.
    let replan = vec![Segment {
        index: 0,
        start: 0,
        end: 249,
        downloaded: 0,
    }];
    store.upsert_segments(task.id, &replan).await.unwrap();
    assert_eq!(store.load_segments(task.id).await.unwrap(), replan);
}

#[tokio::test]
async fn invalid_segment_plan_rolls_back_entirely() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    store.upsert_segments(task.id, &segments()).await.unwrap();

    let bad = vec![
        Segment {
            index: 0,
            start: 0,
            end: 9,
            downloaded: 0,
        },
        Segment {
            index: 1,
            start: 20,
            end: 10,
            downloaded: 0,
        }, // end < start
    ];
    assert!(store.upsert_segments(task.id, &bad).await.is_err());
    assert_eq!(
        store.load_segments(task.id).await.unwrap(),
        segments(),
        "old plan intact"
    );

    // Unknown task: foreign key violation.
    assert!(
        store
            .upsert_segments(TaskId::new(), &segments())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn batched_progress_is_one_all_or_nothing_transaction() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    store.upsert_segments(task.id, &segments()).await.unwrap();

    store
        .update_segment_progress(task.id, &[(0, 100), (1, 40), (2, 50)])
        .await
        .unwrap();
    let got: Vec<u64> = store
        .load_segments(task.id)
        .await
        .unwrap()
        .iter()
        .map(|s| s.downloaded)
        .collect();
    assert_eq!(got, vec![100, 40, 50]);

    // Index 9 does not exist: the earlier updates in the same batch must roll back.
    let err = store
        .update_segment_progress(task.id, &[(0, 1), (1, 1), (9, 1)])
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::SegmentNotFound { index: 9, .. }));
    // Progress beyond the segment length violates a CHECK: also all-or-nothing.
    assert!(
        store
            .update_segment_progress(task.id, &[(0, 2), (1, 101)])
            .await
            .is_err()
    );
    let after: Vec<u64> = store
        .load_segments(task.id)
        .await
        .unwrap()
        .iter()
        .map(|s| s.downloaded)
        .collect();
    assert_eq!(after, vec![100, 40, 50], "no partial batch was applied");
}

#[tokio::test]
async fn delete_cascades_to_segments() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    store.upsert_segments(task.id, &segments()).await.unwrap();
    assert!(store.delete_task(task.id).await.unwrap());
    assert_eq!(store.load_segments(task.id).await.unwrap(), Vec::new());
    assert_eq!(store.load_all().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn settings_default_set_get_overwrite() {
    let (_dir, store) = open_temp().await;
    assert_eq!(store.load_settings().await.unwrap(), Settings::default());

    let mut s = Settings {
        max_concurrent_tasks: 9,
        ..Settings::default()
    };
    s.torrent.extra_trackers = vec!["udp://t.example:1/announce".into()];
    store.save_settings(&s).await.unwrap();
    assert_eq!(store.load_settings().await.unwrap(), s);

    s.max_concurrent_tasks = 2;
    store.save_settings(&s).await.unwrap();
    assert_eq!(store.load_settings().await.unwrap().max_concurrent_tasks, 2);

    store.set_setting("window", &(800, 600)).await.unwrap();
    assert_eq!(
        store.get_setting::<(u32, u32)>("window").await.unwrap(),
        Some((800, 600))
    );
    assert_eq!(store.get_setting::<u32>("missing").await.unwrap(), None);
    assert!(matches!(
        store.get_setting::<u32>("window").await,
        Err(StoreError::Json(_))
    ));
}

#[tokio::test]
async fn reopen_after_write_sees_everything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("durable.db");
    let task = full_http_task();
    let settings = Settings {
        default_connections: 4,
        ..Settings::default()
    };
    {
        let store = Store::open(&path).await.unwrap();
        store.insert_task(&task).await.unwrap();
        store.upsert_segments(task.id, &segments()).await.unwrap();
        store
            .update_segment_progress(task.id, &[(1, 77)])
            .await
            .unwrap();
        store
            .update_state(
                task.id,
                &TaskState::Failed {
                    reason: FailReason::Network {
                        message: "reset".into(),
                    },
                    retryable: true,
                },
                UnixMillis(9),
            )
            .await
            .unwrap();
        store.save_settings(&settings).await.unwrap();
        store.close().await;
    }
    let reopened = Store::open(&path).await.unwrap();
    let all = reopened.load_all().await.unwrap();
    assert_eq!(all.len(), 1);
    let t = &all[0];
    assert_eq!(t.segments[1].downloaded, 77);
    assert_eq!(t.snapshot.downloaded, 10 + 77 + 50);
    assert!(matches!(
        t.snapshot.state,
        TaskState::Failed {
            retryable: true,
            ..
        }
    ));
    assert_eq!(t.snapshot.spec, task.spec);
    assert_eq!(reopened.load_settings().await.unwrap(), settings);
}

#[tokio::test]
async fn store_is_a_checkpoint_sink() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    let sink: &dyn CheckpointSink = &store;
    sink.save_segments(task.id, &segments()).await.unwrap();
    sink.save_progress(task.id, &[(2, 50), (0, 100)])
        .await
        .unwrap();
    assert_eq!(
        store
            .get_task(task.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .downloaded,
        150
    );
    let err = sink.save_progress(task.id, &[(7, 1)]).await.unwrap_err();
    assert!(err.to_string().contains("segment 7"));
    assert!(
        sink.save_segments(TaskId::new(), &segments())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejects_duplicates_and_unstorable_values() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    assert!(matches!(
        store.insert_task(&task).await,
        Err(StoreError::Database(_))
    ));

    let mut huge = full_http_task();
    huge.total_size = Some(u64::MAX);
    assert!(matches!(
        store.insert_task(&huge).await,
        Err(StoreError::Overflow(u64::MAX))
    ));
}

#[tokio::test]
async fn corrupt_rows_are_reported_not_misread() {
    let (_dir, store) = open_temp().await;
    store.insert_task(&full_http_task()).await.unwrap();

    store
        .execute_raw("UPDATE tasks SET state = 'bogus'")
        .await
        .unwrap();
    assert!(matches!(
        store.load_all().await,
        Err(StoreError::Corrupt(_))
    ));

    store
        .execute_raw("UPDATE tasks SET state = 'queued', kind = 'torrent'")
        .await
        .unwrap();
    let err = store.load_all().await.unwrap_err();
    assert!(
        err.to_string().contains("does not match its source"),
        "{err}"
    );

    store
        .execute_raw("UPDATE tasks SET kind = 'http', id = 'not-a-uuid'")
        .await
        .unwrap();
    assert!(matches!(
        store.load_all().await,
        Err(StoreError::Corrupt(_))
    ));
}

#[tokio::test]
async fn queue_order_round_trips_and_defaults_to_zero() {
    let (_dir, store) = open_temp().await;
    let a = full_http_task();
    let b = full_http_task();
    store.insert_task(&a).await.unwrap();
    store.insert_task(&b).await.unwrap();
    assert!(
        store
            .load_all()
            .await
            .unwrap()
            .iter()
            .all(|t| t.queue_order == 0)
    );
    store
        .set_queue_order(&[(a.id, 5), (b.id, -1), (TaskId::new(), 9)])
        .await
        .unwrap();
    let all = store.load_all().await.unwrap();
    let order = |id| {
        all.iter()
            .find(|t| t.snapshot.id == id)
            .unwrap()
            .queue_order
    };
    assert_eq!((order(a.id), order(b.id)), (5, -1));
}

#[tokio::test]
async fn file_name_can_be_updated() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    store
        .update_file_name(task.id, "final (1).iso")
        .await
        .unwrap();
    let t = store.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(t.snapshot.spec.file_name.as_deref(), Some("final (1).iso"));
    assert!(matches!(
        store.update_file_name(TaskId::new(), "x").await,
        Err(StoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn torrent_state_and_file_selection_round_trip() {
    let (_dir, store) = open_temp().await;
    let mut spec = TaskSpec::new(Source::magnet("magnet:?xt=urn:btih:abc").unwrap(), dest());
    spec.selected_files = Some(vec![0, 2]);
    let task = TaskSnapshot::new(spec);
    store.insert_task(&task).await.unwrap();
    assert_eq!(
        store.get_task(task.id).await.unwrap().unwrap().snapshot,
        task
    );

    store.set_selected_files(task.id, Some(&[1])).await.unwrap();
    let loaded = store.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(loaded.snapshot.spec.selected_files, Some(vec![1]));
    store.set_selected_files(task.id, None).await.unwrap();
    assert_eq!(
        store
            .get_task(task.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .spec
            .selected_files,
        None
    );

    assert_eq!(store.torrent_state(task.id).await.unwrap(), None);
    let sink: &dyn CheckpointSink = &store;
    sink.save_torrent(task.id, "aa", b"d4:infod...e")
        .await
        .unwrap();
    store
        .save_torrent_state(task.id, "bb", b"v2")
        .await
        .unwrap();
    assert_eq!(
        store.torrent_state(task.id).await.unwrap(),
        Some(("bb".into(), b"v2".to_vec()))
    );
    assert!(
        store
            .save_torrent_state(TaskId::new(), "x", b"y")
            .await
            .is_err(),
        "foreign key"
    );
    store.delete_task(task.id).await.unwrap();
    assert_eq!(store.torrent_state(task.id).await.unwrap(), None, "cascade");
}

#[tokio::test]
async fn file_lists_are_stored_and_replaced() {
    let (_dir, store) = open_temp().await;
    let task = full_http_task();
    store.insert_task(&task).await.unwrap();
    assert_eq!(store.files(task.id).await.unwrap(), []);
    let files = vec![
        pv_core::FileEntry {
            index: 0,
            path: "a.bin".into(),
            size: 10,
            selected: true,
        },
        pv_core::FileEntry {
            index: 1,
            path: "sub/b.bin".into(),
            size: 20,
            selected: false,
        },
    ];
    store.set_files(task.id, &files).await.unwrap();
    assert_eq!(store.files(task.id).await.unwrap(), files);
    store.set_files(task.id, &files[..1]).await.unwrap();
    assert_eq!(store.files(task.id).await.unwrap(), files[..1]);
    assert_eq!(store.files(TaskId::new()).await.unwrap(), []);
    assert!(matches!(
        store.set_files(TaskId::new(), &files).await,
        Err(StoreError::NotFound(_))
    ));
}
