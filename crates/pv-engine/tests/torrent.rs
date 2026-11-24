//! Torrents through the whole engine: local seeder, persistence, restart, selection.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pv_core::{
    EngineEvent, Settings, Source, TaskId, TaskSnapshot, TaskSpec, TaskState, TaskStateKind,
    TorrentSettings,
};
use pv_engine::{Engine, EngineConfig, EngineError, EngineHandle};
use pv_http::HttpProtocol;
use pv_testkit::{Fixture, sha256_file};
use pv_torrent::TorrentProtocol;
use pv_torrent::testing::LocalSeeder;

fn settings() -> Settings {
    Settings {
        torrent: TorrentSettings {
            dht: false,
            upnp: false,
            never_seed: true,
            ..TorrentSettings::default()
        },
        ..Settings::default()
    }
}

async fn engine(db: &Path, seeder: &LocalSeeder) -> EngineHandle {
    let mut config = EngineConfig::new(db);
    config.protocols = vec![
        Arc::new(HttpProtocol),
        Arc::new(
            TorrentProtocol::with_work_dir(db.with_file_name("torrent-session"))
                .with_initial_peers(vec![seeder.addr]),
        ),
    ];
    let e = Engine::start(config).await.unwrap();
    e.set_settings(settings()).await.unwrap();
    e
}

async fn task(e: &EngineHandle, id: TaskId) -> TaskSnapshot {
    e.list()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap()
}

async fn wait_for(e: &EngineHandle, id: TaskId, kind: TaskStateKind) -> TaskSnapshot {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let t = task(e, id).await;
        if t.state.kind() == kind {
            return t;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {kind}: {:?}",
            t.state
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn seed_file(dir: &Path, f: &Fixture) -> PathBuf {
    let seed = dir.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    let path = seed.join(&f.name);
    std::fs::write(&path, &f.bytes).unwrap();
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_file_downloads_through_the_engine_and_can_be_removed_with_files() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("e.bin", 500_000, 1);
    let seeder = LocalSeeder::start(&seed_file(dir.path(), &f), "e.bin").await;
    let e = engine(&dir.path().join("db.sqlite"), &seeder).await;
    let dest = dir.path().join("dl");

    let id = e
        .add(TaskSpec::new(
            Source::TorrentFile {
                bytes: seeder.torrent.clone(),
            },
            &dest,
        ))
        .await
        .unwrap();
    let done = wait_for(&e, id, TaskStateKind::Completed).await;
    assert_eq!(done.spec.file_name.as_deref(), Some("e.bin"));
    assert_eq!((done.downloaded, done.total_size), (500_000, Some(500_000)));
    assert_eq!(sha256_file(dest.join("e.bin")).await.unwrap(), f.sha256);
    let detail = e.detail(id).await.unwrap();
    assert_eq!(detail.segments, [], "torrents have no segment table");
    assert_eq!(detail.files.len(), 1);
    assert_eq!(
        (detail.files[0].path.as_str(), detail.files[0].size),
        ("e.bin", 500_000)
    );
    e.select_files(id, Some(vec![])).await.unwrap();
    assert!(
        !e.detail(id).await.unwrap().files[0].selected,
        "the stored list reflects the current selection"
    );
    assert!(matches!(
        e.detail(TaskId::new()).await,
        Err(EngineError::NotFound(_))
    ));

    assert!(matches!(
        e.select_files(TaskId::new(), None).await,
        Err(EngineError::NotFound(_))
    ));
    e.remove(id, true).await.unwrap();
    assert!(
        !dest.join("e.bin").exists(),
        "delete_files removes the torrent's data"
    );
    e.shutdown().await;
    seeder.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn magnet_survives_an_engine_restart_without_refetching_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("m.bin", 2_000_000, 2);
    let seeder = LocalSeeder::start(&seed_file(dir.path(), &f), "m.bin").await;
    let db = dir.path().join("db.sqlite");
    let dest = dir.path().join("dl");

    let e = engine(&db, &seeder).await;
    let mut spec = TaskSpec::new(Source::magnet(seeder.magnet.clone()).unwrap(), &dest);
    spec.speed_limit_bps = Some(800_000);
    let id = e.add(spec).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while task(&e, id).await.downloaded < 600_000 {
        assert!(Instant::now() < deadline, "no progress");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    e.shutdown().await;

    let e2 = engine(&db, &seeder).await;
    let t = task(&e2, id).await;
    assert_eq!(t.state, TaskState::Paused, "recovered as paused");
    assert!(
        t.downloaded >= 600_000,
        "progress is known before the torrent runs again: {}",
        t.downloaded
    );
    let mut events = e2.subscribe();
    e2.resume(id).await.unwrap();
    wait_for(&e2, id, TaskStateKind::Completed).await;
    assert_eq!(sha256_file(dest.join("m.bin")).await.unwrap(), f.sha256);
    let mut phases = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let EngineEvent::StateChanged { state, .. } = ev {
            phases.push(state.kind());
        }
    }
    assert!(
        !phases.contains(&TaskStateKind::FetchingMetadata),
        "metadata came from the database: {phases:?}"
    );
    let checking = phases.iter().position(|k| *k == TaskStateKind::Checking);
    let downloading = phases.iter().position(|k| *k == TaskStateKind::Downloading);
    assert!(
        checking.is_some() && checking < downloading,
        "a resume checks before it downloads: {phases:?}"
    );
    e2.shutdown().await;
    seeder.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_torrent_deletes_its_partial_data_and_http_only_tasks_reject_selection() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("c.bin", 2_000_000, 3);
    let seeder = LocalSeeder::start(&seed_file(dir.path(), &f), "c.bin").await;
    let e = engine(&dir.path().join("db.sqlite"), &seeder).await;
    let dest = dir.path().join("dl");
    let mut spec = TaskSpec::new(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dest,
    );
    spec.speed_limit_bps = Some(500_000);
    let id = e.add(spec).await.unwrap();
    while task(&e, id).await.downloaded < 200_000 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    e.select_files(id, Some(vec![0])).await.unwrap(); // restart with a selection
    e.cancel(id).await.unwrap();
    wait_for(&e, id, TaskStateKind::Cancelled).await;
    assert!(
        !dest.join("c.bin").exists(),
        "partial torrent data deleted on cancel"
    );

    let http = e
        .add(TaskSpec::new(
            Source::http(url::Url::parse("https://example.invalid/x").unwrap()).unwrap(),
            &dest,
        ))
        .await
        .unwrap();
    e.pause(http).await.ok();
    assert!(matches!(
        e.select_files(http, None).await,
        Err(EngineError::NotATorrent(_))
    ));
    e.shutdown().await;
    seeder.stop().await;
}
