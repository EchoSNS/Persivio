//! Phase 11: what happens to secrets, and to tasks when the database or disk is full.
//!
//! The disk tests need a small file system and are ignored by default. CI mounts a tmpfs
//! on Linux (see `.github/workflows/ci.yml`); to run them elsewhere, point
//! `PERSIVIO_SMALL_DISK` at an empty directory on a 24 MiB file system:
//! `cargo nextest run -p pv-engine --test hardening --run-ignored only`

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pv_core::{
    FailReason, Settings, Source, TaskId, TaskSnapshot, TaskSpec, TaskState, TaskStateKind,
    TorrentSettings,
};
use pv_engine::{Engine, EngineConfig, EngineHandle};
use pv_http::HttpProtocol;
use pv_store::Store;
use pv_testkit::{Fixture, TestServer, sha256_file};
use pv_torrent::TorrentProtocol;
use pv_torrent::testing::LocalSeeder;
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(60);

async fn task(e: &EngineHandle, id: TaskId) -> TaskSnapshot {
    let list = e.list().await.unwrap();
    list.into_iter().find(|t| t.id == id).unwrap()
}

async fn wait_for(e: &EngineHandle, id: TaskId, kinds: &[TaskStateKind]) -> TaskSnapshot {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let t = task(e, id).await;
        if kinds.contains(&t.state.kind()) {
            return t;
        }
        assert!(Instant::now() < deadline, "timed out: {:?}", t.state);
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

fn http(url: &str) -> Source {
    Source::http(Url::parse(url).unwrap()).unwrap()
}

/// Whether `needle` appears anywhere in the database files.
fn on_disk(db: &Path, needle: &str) -> bool {
    let dir = db.parent().unwrap();
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .any(|p| {
            let data = std::fs::read(&p).unwrap();
            data.windows(needle.len()).any(|w| w == needle.as_bytes())
        })
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_are_erased_from_disk_when_a_download_ends() {
    const SECRET: &str = "sEcReT-cookie-7f3a";
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let db = data.join("persivio.db");
    let f = Fixture::random("a.bin", 300_000, 1);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let source = |url: String| Source::Http {
        url: Url::parse(&url).unwrap(),
        headers: vec![("X-Token".into(), SECRET.into())],
        cookies: Some(format!("session={SECRET}")),
        referer: None,
    };
    let e = Engine::start(EngineConfig::new(&db)).await.unwrap();
    e.set_settings(Settings {
        max_retries: 0,
        ..Settings::default()
    })
    .await
    .unwrap();
    let done = e
        .add(TaskSpec::new(source(server.url("/file/a.bin")), dir.path()))
        .await
        .unwrap();
    // A retryable failure must keep them: a retry needs them.
    let missing = e
        .add(TaskSpec::new(
            source(server.url("/flaky/a.bin?fail_rate=1.0")),
            dir.path(),
        ))
        .await
        .unwrap();
    wait_for(&e, done, &[TaskStateKind::Completed]).await;
    wait_for(&e, missing, &[TaskStateKind::Failed]).await;
    let has_secret = |t: &TaskSnapshot| matches!(&t.spec.source, Source::Http { cookies, .. } if cookies.is_some());
    assert!(!has_secret(&task(&e, done).await), "gone from memory");
    assert!(has_secret(&task(&e, missing).await), "kept for a retry");
    assert!(on_disk(&db, SECRET), "the test can find what is stored");
    e.cancel(missing).await.unwrap();
    wait_for(&e, missing, &[TaskStateKind::Cancelled]).await;
    e.shutdown().await;

    // Secure delete: not even in free pages or the write-ahead log.
    assert!(!on_disk(&db, SECRET), "erased from the database files");
    let e = Engine::start(EngineConfig::new(&db)).await.unwrap();
    assert!(!has_secret(&task(&e, done).await));
    assert_eq!(task(&e, done).await.state, TaskState::Completed);
    e.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_stored_by_older_versions_are_erased_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("persivio.db");
    let store = Store::open(&db).await.unwrap();
    let mut old = TaskSnapshot::new(TaskSpec::new(
        http("https://user:pw@example.com/f"),
        dir.path(),
    ));
    old.state = TaskState::Completed;
    store.insert_task(&old).await.unwrap();
    store.close().await;
    let e = Engine::start(EngineConfig::new(&db)).await.unwrap();
    assert_eq!(
        task(&e, old.id).await.spec.source,
        http("https://example.com/f")
    );
    e.shutdown().await;
}

/// When the end of a run cannot be saved, the task must not look as if it still ran.
#[tokio::test(flavor = "multi_thread")]
async fn a_task_whose_end_cannot_be_saved_is_shown_as_failed_and_can_be_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("persivio.db");
    let f = Fixture::random("a.bin", 600_000, 2);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let e = Engine::start(EngineConfig::new(&db)).await.unwrap();
    let url = server.url("/throttle/a.bin?bps=400000");
    let mut spec = TaskSpec::new(http(&url), dir.path());
    spec.connections = Some(1);
    let id = e.add(spec).await.unwrap();
    while task(&e, id).await.downloaded < 100_000 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let store = Store::open(&db).await.unwrap();
    store.fail_task_updates(true).await.unwrap();
    let failed = wait_for(&e, id, &[TaskStateKind::Failed, TaskStateKind::Completed]).await;
    let TaskState::Failed { reason, retryable } = &failed.state else {
        panic!("{:?}", failed.state);
    };
    assert!(*retryable);
    assert!(
        reason.to_string().contains("could not be saved"),
        "{reason}"
    );

    store.fail_task_updates(false).await.unwrap();
    assert!(store.is_intact().await.unwrap());
    store.close().await;
    // The finished file may be there already; a retry downloads it again beside it.
    e.resume(id).await.unwrap();
    let done = wait_for(&e, id, &[TaskStateKind::Completed, TaskStateKind::Failed]).await;
    assert_eq!(done.state, TaskState::Completed);
    let path = dir.path().join(done.spec.file_name.unwrap());
    assert_eq!(sha256_file(path).await.unwrap(), f.sha256);
    e.shutdown().await;
}

// --- Full disk --------------------------------------------------------------

fn small_disk() -> PathBuf {
    let dir = std::env::var_os("PERSIVIO_SMALL_DISK").expect("PERSIVIO_SMALL_DISK is not set");
    PathBuf::from(dir)
}

fn assert_disk_full(t: &TaskSnapshot) {
    match &t.state {
        TaskState::Failed {
            reason: reason @ FailReason::DiskFull { .. },
            retryable: true,
        } => println!("failed clearly: {reason}"),
        other => panic!("expected a clear, retryable disk error, got {other:?}"),
    }
}

/// The disk fills up *while* downloading (the size is unknown, so nothing can be refused
/// up front), with the database on the same disk. Then space is freed and a retry works.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a small file system (PERSIVIO_SMALL_DISK)"]
async fn disk_full_while_downloading_fails_cleanly_and_the_database_survives() {
    let disk = tempfile::tempdir_in(small_disk()).unwrap();
    let db = disk.path().join("data/persivio.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let big = Fixture::random("big.bin", 64 * 1024 * 1024, 3);
    let small = Fixture::random("small.bin", 2 * 1024 * 1024, 4);
    let server = TestServer::start([big, small.clone()]).await.unwrap();
    let e = Engine::start(EngineConfig::new(&db)).await.unwrap();
    let dest = disk.path().join("dl");

    // Known size: refused before anything is written.
    let known = e
        .add(TaskSpec::new(http(&server.url("/file/big.bin")), &dest))
        .await
        .unwrap();
    assert_disk_full(
        &wait_for(
            &e,
            known,
            &[TaskStateKind::Failed, TaskStateKind::Completed],
        )
        .await,
    );

    // A disk that fills up underneath the download.
    let ok = e
        .add({
            let mut s = TaskSpec::new(http(&server.url("/throttle/small.bin?bps=1000000")), &dest);
            s.connections = Some(1);
            s
        })
        .await
        .unwrap();
    while task(&e, ok).await.downloaded < 200_000 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let filler = disk.path().join("filler");
    fill(&filler);
    let t = wait_for(
        &e,
        ok,
        &[
            TaskStateKind::Failed,
            TaskStateKind::Completed,
            TaskStateKind::Paused,
        ],
    )
    .await;
    println!("disk filled underneath: {:?}", t.state);
    assert!(
        matches!(
            t.state,
            TaskState::Failed {
                retryable: true,
                ..
            }
        ),
        "must fail, retryable: {:?}",
        t.state
    );

    // Still responsive, and everything recovers once there is room again.
    assert_eq!(e.list().await.unwrap().len(), 2);
    std::fs::remove_file(&filler).unwrap();
    let store = Store::open(&db).await.unwrap();
    assert!(store.is_intact().await.unwrap(), "the database is intact");
    store.close().await;
    e.resume(ok).await.unwrap();
    let done = wait_for(&e, ok, &[TaskStateKind::Completed, TaskStateKind::Failed]).await;
    assert_eq!(done.state, TaskState::Completed);
    assert_eq!(
        sha256_file(dest.join("small.bin")).await.unwrap(),
        small.sha256
    );
    e.shutdown().await;

    let store = Store::open(&db).await.unwrap();
    assert!(store.is_intact().await.unwrap());
    store.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a small file system (PERSIVIO_SMALL_DISK)"]
async fn a_torrent_that_cannot_fit_is_refused_with_a_clear_reason() {
    let disk = tempfile::tempdir_in(small_disk()).unwrap();
    let seed = tempfile::tempdir().unwrap();
    let f = Fixture::random("t.bin", 64 * 1024 * 1024, 5);
    std::fs::write(seed.path().join("t.bin"), &f.bytes).unwrap();
    let seeder = LocalSeeder::start(&seed.path().join("t.bin"), "t.bin").await;
    let db = disk.path().join("persivio.db");
    let mut config = EngineConfig::new(&db);
    config.protocols = vec![
        Arc::new(HttpProtocol),
        Arc::new(
            TorrentProtocol::with_work_dir(disk.path().join("work"))
                .with_initial_peers(vec![seeder.addr]),
        ),
    ];
    let e = Engine::start(config).await.unwrap();
    e.set_settings(Settings {
        torrent: TorrentSettings {
            dht: false,
            upnp: false,
            ..TorrentSettings::default()
        },
        ..Settings::default()
    })
    .await
    .unwrap();
    let dest = disk.path().join("dl");
    let id = e
        .add(TaskSpec::new(
            Source::TorrentFile {
                bytes: seeder.torrent.clone(),
            },
            &dest,
        ))
        .await
        .unwrap();
    assert_disk_full(&wait_for(&e, id, &[TaskStateKind::Failed, TaskStateKind::Completed]).await);
    assert!(!dest.join("t.bin").exists(), "nothing was written");
    e.shutdown().await;
    seeder.stop().await;
}

/// Fills the disk completely.
fn fill(path: &Path) {
    use std::io::Write;
    let mut file = std::fs::File::create(path).unwrap();
    for size in [1024 * 1024, 4096, 512, 1] {
        let block = vec![1u8; size];
        while file.write_all(&block).is_ok() {}
    }
    let _ = file.sync_all();
}

/// A server that accepts connections and never answers.
async fn silent_server() -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/x.bin", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            open.push(socket);
        }
    });
    (Url::parse(&url).unwrap(), handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_answers_can_be_paused_and_times_out() {
    let dir = tempfile::tempdir().unwrap();
    let (url, server) = silent_server().await;
    let e = Engine::start(EngineConfig::new(dir.path().join("db")))
        .await
        .unwrap();
    let id = e
        .add(TaskSpec::new(Source::http(url).unwrap(), dir.path()))
        .await
        .unwrap();
    wait_for(&e, id, &[TaskStateKind::Probing]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let start = Instant::now();
    e.pause(id).await.unwrap();
    wait_for(&e, id, &[TaskStateKind::Paused]).await;
    assert!(start.elapsed() < Duration::from_secs(2), "pause is prompt");

    e.set_settings(Settings {
        idle_timeout_secs: 1,
        max_retries: 0,
        ..Settings::default()
    })
    .await
    .unwrap();
    e.resume(id).await.unwrap();
    let t = wait_for(&e, id, &[TaskStateKind::Failed]).await;
    assert!(
        matches!(&t.state, TaskState::Failed { reason: FailReason::Network { message }, retryable: true } if message.contains("timed out")),
        "{:?}",
        t.state
    );
    e.shutdown().await;
    server.abort();
}

/// Temporary failures are retried everywhere: in the probe, and when a connection that
/// cannot resume breaks.
#[tokio::test(flavor = "multi_thread")]
async fn temporary_failures_are_retried_in_every_phase() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("r.bin", 200_000, 6);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let e = Engine::start(EngineConfig::new(dir.path().join("db")))
        .await
        .unwrap();
    e.set_settings(Settings {
        max_retries: 20,
        max_concurrent_tasks: 16,
        ..Settings::default()
    })
    .await
    .unwrap();
    for i in 0..4 {
        for path in [
            format!("/flaky/r.bin?fail_rate=0.35&seed={i}"),
            format!("/flaky/r.bin?ranges=0&drop_after=50000&drop_rate=0.35&seed={i}"),
        ] {
            let spec = TaskSpec::new(http(&server.url(&path)), dir.path().join("dl"));
            let id = e.add(spec).await.unwrap();
            let t = wait_for(&e, id, &[TaskStateKind::Completed, TaskStateKind::Failed]).await;
            assert_eq!(t.state, TaskState::Completed, "{path}");
            let file = dir.path().join("dl").join(t.spec.file_name.unwrap());
            assert_eq!(sha256_file(file).await.unwrap(), f.sha256, "{path}");
        }
    }
    e.shutdown().await;
}
