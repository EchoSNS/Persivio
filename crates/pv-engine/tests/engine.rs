//! Phase 5 engine tests: an in-process engine against `pv-testkit` servers.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pv_core::{
    EngineEvent, FailReason, Settings, Source, TaskId, TaskSnapshot, TaskSpec, TaskState,
    TaskStateKind,
};
use pv_engine::{Engine, EngineConfig, EngineError, EngineHandle, MoveDirection};
use pv_store::Store;
use pv_testkit::{Fixture, TestServer, sha256_file};
use tempfile::TempDir;
use url::Url;

struct Env {
    dir: TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }
    fn db(&self) -> PathBuf {
        self.dir.path().join("persivio.db")
    }
    fn downloads(&self) -> PathBuf {
        self.dir.path().join("downloads")
    }
    async fn engine(&self) -> EngineHandle {
        Engine::start(EngineConfig::new(self.db())).await.unwrap()
    }
    async fn engine_with(&self, settings: Settings) -> EngineHandle {
        let e = self.engine().await;
        e.set_settings(settings).await.unwrap();
        e
    }
    fn spec(&self, url: &str, connections: u8) -> TaskSpec {
        let mut spec = TaskSpec::new(
            Source::http(Url::parse(url).unwrap()).unwrap(),
            self.downloads(),
        );
        spec.connections = Some(connections);
        spec
    }
}

fn small_segments() -> Settings {
    Settings {
        min_segment_size: 64 * 1024,
        ..Settings::default()
    }
}

async fn task(e: &EngineHandle, id: TaskId) -> TaskSnapshot {
    e.list()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == id)
        .unwrap()
}

async fn wait_for(
    e: &EngineHandle,
    id: TaskId,
    kind: TaskStateKind,
    timeout: Duration,
) -> TaskSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let t = task(e, id).await;
        if t.state.kind() == kind {
            return t;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {kind}; task is {:?}",
            t.state
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

async fn wait_progress(e: &EngineHandle, id: TaskId, at_least: u64) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while task(e, id).await.downloaded < at_least {
        assert!(Instant::now() < deadline, "no progress");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn final_path(t: &TaskSnapshot) -> PathBuf {
    t.spec.dest_dir.join(t.spec.file_name.as_ref().unwrap())
}

#[tokio::test]
async fn downloads_a_task_and_broadcasts_events() {
    let f = Fixture::random("a.bin", 300_000, 1);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let mut events = e.subscribe();
    let id = e.add(env.spec(&s.url("/file/a.bin"), 4)).await.unwrap();
    let done = wait_for(&e, id, TaskStateKind::Completed, Duration::from_secs(10)).await;

    assert_eq!(sha256_file(final_path(&done)).await.unwrap(), f.sha256);
    assert_eq!((done.downloaded, done.total_size), (300_000, Some(300_000)));
    assert!(done.completed_at.is_some());

    let mut states = Vec::new();
    let mut saw_added = false;
    let mut saw_metadata = false;
    while let Ok(ev) = events.try_recv() {
        match ev {
            EngineEvent::TaskAdded(t) => saw_added = t.id == id,
            EngineEvent::StateChanged { state, .. } => states.push(state.kind()),
            EngineEvent::MetadataResolved { name, size, .. } => {
                saw_metadata = name == "a.bin" && size == Some(300_000);
            }
            _ => {}
        }
    }
    assert!(saw_added && saw_metadata);
    assert_eq!(
        states,
        [
            TaskStateKind::Probing,
            TaskStateKind::Downloading,
            TaskStateKind::Completed
        ]
    );
    e.shutdown().await;
}

#[tokio::test]
async fn concurrency_limit_is_enforced() {
    let fixtures: Vec<Fixture> = (0..5)
        .map(|i| Fixture::random(format!("f{i}.bin"), 150_000, i))
        .collect();
    let s = TestServer::start(fixtures.clone()).await.unwrap();
    let env = Env::new();
    let e = env
        .engine_with(Settings {
            max_concurrent_tasks: 2,
            ..small_segments()
        })
        .await;
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            e.add(env.spec(&s.url(&format!("/throttle/f{i}.bin?bps=400000")), 1))
                .await
                .unwrap(),
        );
    }
    let mut max_running = 0;
    loop {
        let list = e.list().await.unwrap();
        let running = list.iter().filter(|t| t.state.kind().is_running()).count();
        max_running = max_running.max(running);
        if list
            .iter()
            .all(|t| t.state.kind() == TaskStateKind::Completed)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        max_running, 2,
        "never more than max_concurrent_tasks at once"
    );
    for (id, f) in ids.iter().zip(&fixtures) {
        assert_eq!(
            sha256_file(final_path(&task(&e, *id).await)).await.unwrap(),
            f.sha256
        );
    }
    e.shutdown().await;
}

#[tokio::test]
async fn pause_and_resume_keep_progress() {
    let f = Fixture::random("p.bin", 1_000_000, 2);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let id = e
        .add(env.spec(&s.url("/throttle/p.bin?bps=400000"), 2))
        .await
        .unwrap();
    wait_progress(&e, id, 300_000).await;
    e.pause(id).await.unwrap();
    let paused = wait_for(&e, id, TaskStateKind::Paused, Duration::from_secs(5)).await;
    assert!(paused.downloaded >= 300_000);

    // While paused nothing moves.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(task(&e, id).await.downloaded, paused.downloaded);

    s.clear_requests();
    e.resume(id).await.unwrap();
    let done = wait_for(&e, id, TaskStateKind::Completed, Duration::from_secs(15)).await;
    assert_eq!(sha256_file(final_path(&done)).await.unwrap(), f.sha256);
    let restarted_from_zero = s.requests().iter().any(|r| {
        r.header("range")
            .is_some_and(|h| h.starts_with("bytes=0-") && h != "bytes=0-0")
    });
    assert!(!restarted_from_zero, "resume continued from checkpoints");
    e.shutdown().await;
}

#[tokio::test]
async fn restart_recovers_interrupted_tasks_as_paused() {
    let f = Fixture::random("r.bin", 1_000_000, 3);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let id = e
        .add(env.spec(&s.url("/throttle/r.bin?bps=500000"), 2))
        .await
        .unwrap();
    wait_progress(&e, id, 300_000).await;
    e.shutdown().await;
    assert!(matches!(e.list().await, Err(EngineError::Stopped)));

    let e2 = env.engine().await;
    let t = task(&e2, id).await;
    assert_eq!(
        t.state,
        TaskState::Paused,
        "interrupted task recovered as paused"
    );
    assert!(
        t.downloaded >= 200_000,
        "progress survived: {}",
        t.downloaded
    );
    e2.resume(id).await.unwrap();
    let done = wait_for(&e2, id, TaskStateKind::Completed, Duration::from_secs(15)).await;
    assert_eq!(sha256_file(final_path(&done)).await.unwrap(), f.sha256);
    e2.shutdown().await;
}

#[tokio::test]
async fn auto_resume_requeues_interrupted_tasks() {
    let f = Fixture::random("ar.bin", 800_000, 4);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env
        .engine_with(Settings {
            auto_resume_on_startup: true,
            ..small_segments()
        })
        .await;
    let id = e
        .add(env.spec(&s.url("/throttle/ar.bin?bps=400000"), 2))
        .await
        .unwrap();
    wait_progress(&e, id, 200_000).await;
    e.shutdown().await;

    let e2 = env.engine().await;
    let done = wait_for(&e2, id, TaskStateKind::Completed, Duration::from_secs(15)).await;
    assert_eq!(sha256_file(final_path(&done)).await.unwrap(), f.sha256);
    e2.shutdown().await;
}

#[tokio::test]
async fn missing_part_file_is_reported_on_recovery_and_retry_starts_over() {
    let f = Fixture::random("m.bin", 800_000, 5);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let id = e
        .add(env.spec(&s.url("/throttle/m.bin?bps=400000"), 2))
        .await
        .unwrap();
    wait_progress(&e, id, 200_000).await;
    e.shutdown().await;
    std::fs::remove_file(env.downloads().join("m.bin.part")).unwrap();

    let e2 = env.engine().await;
    let t = task(&e2, id).await;
    assert_eq!(
        t.state,
        TaskState::Failed {
            reason: FailReason::PartFileMissing,
            retryable: true
        }
    );
    e2.resume(id).await.unwrap();
    let done = wait_for(&e2, id, TaskStateKind::Completed, Duration::from_secs(15)).await;
    assert_eq!(sha256_file(final_path(&done)).await.unwrap(), f.sha256);
    e2.shutdown().await;
}

#[tokio::test]
async fn global_speed_limit_is_accurate_within_ten_percent() {
    let f = Fixture::random("g.bin", 1_500_000, 6);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env
        .engine_with(Settings {
            global_speed_limit_bps: Some(500_000),
            ..small_segments()
        })
        .await;
    let mut events = e.subscribe();
    let started = Instant::now();
    let id = e.add(env.spec(&s.url("/file/g.bin"), 4)).await.unwrap();
    let rate = measured_rate(&mut events, id, 1_500_000).await;
    let done = wait_for(&e, id, TaskStateKind::Completed, Duration::from_secs(10)).await;
    println!(
        "transfer rate {rate:.0} B/s (limit 500000); end to end {:.2}s (ideal 3.0s + setup + final fsync)",
        started.elapsed().as_secs_f64()
    );
    assert!(
        (450_000.0..=550_000.0).contains(&rate),
        "{rate:.0} B/s is outside 500 KB/s ± 10%"
    );
    assert_eq!(sha256_file(final_path(&done)).await.unwrap(), f.sha256);
    e.shutdown().await;
}

#[tokio::test]
async fn per_task_limit_applies() {
    let f = Fixture::random("t.bin", 400_000, 7);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let mut spec = env.spec(&s.url("/file/t.bin"), 4);
    spec.speed_limit_bps = Some(400_000);
    let started = Instant::now();
    let id = e.add(spec).await.unwrap();
    wait_for(&e, id, TaskStateKind::Completed, Duration::from_secs(10)).await;
    let elapsed = started.elapsed().as_secs_f64();
    assert!(
        (0.85..=1.3).contains(&elapsed),
        "{elapsed:.2}s for 1 s worth of data"
    );
    e.shutdown().await;
}

#[tokio::test]
async fn shutdown_leaves_a_consistent_database() {
    let f = Fixture::random("c.bin", 1_000_000, 8);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let id = e
        .add(env.spec(&s.url("/throttle/c.bin?bps=300000"), 4))
        .await
        .unwrap();
    wait_progress(&e, id, 300_000).await;
    e.shutdown().await;

    // Every byte the database claims is really in the .part file (ADR-005).
    let store = Store::open(&env.db()).await.unwrap();
    let stored = store.get_task(id).await.unwrap().unwrap();
    let part = std::fs::read(env.downloads().join("c.bin.part")).unwrap();
    let mut claimed = 0;
    for seg in &stored.segments {
        let from = usize::try_from(seg.start).unwrap();
        let to = usize::try_from(seg.start + seg.downloaded).unwrap();
        assert_eq!(
            &part[from..to],
            &f.bytes[from..to],
            "segment {} claims unwritten bytes",
            seg.index
        );
        claimed += seg.downloaded;
    }
    assert!(claimed >= 200_000, "checkpoints were persisted: {claimed}");
    assert_eq!(stored.snapshot.downloaded, claimed);
    store.close().await;
}

#[tokio::test]
async fn cancel_deletes_the_part_file_and_remove_can_delete_downloads() {
    let f = Fixture::random("x.bin", 600_000, 9);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;

    let id = e
        .add(env.spec(&s.url("/throttle/x.bin?bps=300000"), 1))
        .await
        .unwrap();
    wait_progress(&e, id, 100_000).await;
    e.cancel(id).await.unwrap();
    wait_for(&e, id, TaskStateKind::Cancelled, Duration::from_secs(5)).await;
    assert!(!env.downloads().join("x.bin.part").exists());
    assert!(
        matches!(e.resume(id).await, Err(EngineError::InvalidTransition(_))),
        "cancelled is final"
    );
    e.remove(id, false).await.unwrap();

    let keep = e.add(env.spec(&s.url("/file/x.bin"), 2)).await.unwrap();
    let kept = wait_for(&e, keep, TaskStateKind::Completed, Duration::from_secs(10)).await;
    e.remove(keep, false).await.unwrap();
    assert!(
        final_path(&kept).exists(),
        "remove without delete keeps the file"
    );

    let del = e.add(env.spec(&s.url("/file/x.bin"), 2)).await.unwrap();
    let gone = wait_for(&e, del, TaskStateKind::Completed, Duration::from_secs(10)).await;
    assert_ne!(
        final_path(&gone),
        final_path(&kept),
        "no overwrite: de-duplicated name"
    );
    e.remove(del, true).await.unwrap();
    assert!(!final_path(&gone).exists());
    assert_eq!(e.list().await.unwrap(), Vec::new());
    e.shutdown().await;
}

#[tokio::test]
async fn queue_order_can_be_changed() {
    let fixtures: Vec<Fixture> = (0..4)
        .map(|i| Fixture::random(format!("q{i}.bin"), 100_000, 10 + i))
        .collect();
    let s = TestServer::start(fixtures).await.unwrap();
    let env = Env::new();
    let e = env
        .engine_with(Settings {
            max_concurrent_tasks: 1,
            ..small_segments()
        })
        .await;
    let mut events = e.subscribe();
    let mut ids = Vec::new();
    for i in 0..4 {
        ids.push(
            e.add(env.spec(&s.url(&format!("/throttle/q{i}.bin?bps=400000")), 1))
                .await
                .unwrap(),
        );
    }
    // q0 is running; queue is q1, q2, q3. Move q3 to the top and q1 to the bottom.
    e.move_task(ids[3], MoveDirection::Top).await.unwrap();
    e.move_task(ids[1], MoveDirection::Bottom).await.unwrap();
    assert!(
        matches!(
            e.move_task(ids[0], MoveDirection::Up).await,
            Err(EngineError::NotFound(_))
        ),
        "running tasks are not in the queue"
    );
    for id in &ids {
        wait_for(&e, *id, TaskStateKind::Completed, Duration::from_secs(15)).await;
    }
    let mut started = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let EngineEvent::StateChanged {
            id,
            state: TaskState::Probing,
        } = ev
        {
            started.push(ids.iter().position(|x| *x == id).unwrap());
        }
    }
    assert_eq!(started, [0, 3, 2, 1]);
    e.shutdown().await;
}

#[tokio::test]
async fn invalid_requests_are_rejected() {
    let env = Env::new();
    let e = env.engine().await;
    let unknown = TaskId::new();
    assert!(matches!(
        e.pause(unknown).await,
        Err(EngineError::NotFound(_))
    ));
    assert!(matches!(
        e.remove(unknown, false).await,
        Err(EngineError::NotFound(_))
    ));
    let mut relative = env.spec("https://example.com/x", 1);
    relative.dest_dir = PathBuf::from("relative");
    assert!(matches!(e.add(relative).await, Err(EngineError::Spec(_))));
    let magnet = TaskSpec::new(
        Source::magnet("magnet:?xt=urn:btih:abc").unwrap(),
        env.downloads(),
    );
    assert!(
        matches!(e.add(magnet).await, Err(EngineError::NoProtocol)),
        "torrents arrive in Phase 6"
    );
    let bad = Settings {
        max_concurrent_tasks: 0,
        ..Settings::default()
    };
    assert!(matches!(
        e.set_settings(bad).await,
        Err(EngineError::Settings(_))
    ));
    assert_eq!(e.settings().await.unwrap(), Settings::default());
    e.shutdown().await;
    assert!(matches!(
        e.add(env.spec("https://example.com/x", 1)).await,
        Err(EngineError::Stopped)
    ));
}

#[tokio::test]
async fn progress_is_batched_at_about_four_hertz() {
    let f = Fixture::random("b.bin", 600_000, 20);
    let s = TestServer::start([f]).await.unwrap();
    let env = Env::new();
    let e = env.engine_with(small_segments()).await;
    let mut events = e.subscribe();
    let id = e
        .add(env.spec(&s.url("/throttle/b.bin?bps=400000"), 1))
        .await
        .unwrap();
    wait_for(&e, id, TaskStateKind::Downloading, Duration::from_secs(5)).await;
    let window = Instant::now();
    let mut batches = 0;
    let mut last_speed = 0;
    while window.elapsed() < Duration::from_secs(1) {
        if let Ok(Ok(EngineEvent::ProgressBatch(b))) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            batches += 1;
            last_speed = b[0].speed_bps;
        }
    }
    assert!(
        (3..=6).contains(&batches),
        "{batches} batches in 1 s (target 4, max 10 Hz)"
    );
    assert!(last_speed > 100_000, "speed estimate is live: {last_speed}");
    e.shutdown().await;
}

/// Transfer rate between two exact progress samples taken mid-download (from ~20 % to
/// ~90 %). This measures what the limiter controls, excluding setup and the final
/// `sync_data`, which on macOS (`F_FULLFSYNC`) can take a large part of a second.
async fn measured_rate(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    id: TaskId,
    total: u64,
) -> f64 {
    let mut first: Option<(Instant, u64)> = None;
    let mut last: Option<(Instant, u64)> = None;
    while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
        let EngineEvent::ProgressBatch(batch) = ev else {
            continue;
        };
        let Some(p) = batch.iter().find(|p| p.task_id == id) else {
            continue;
        };
        let now = Instant::now();
        if p.downloaded >= total / 5 && first.is_none() {
            first = Some((now, p.downloaded));
        }
        if p.downloaded <= total * 9 / 10 {
            last = Some((now, p.downloaded));
        } else {
            break;
        }
    }
    let ((t0, b0), (t1, b1)) = (first.unwrap(), last.unwrap());
    assert!(
        t1 > t0 + Duration::from_millis(1000),
        "measurement window too short"
    );
    #[allow(clippy::cast_precision_loss, reason = "byte counts far below 2^52")]
    let rate = (b1 - b0) as f64 / (t1 - t0).as_secs_f64();
    rate
}
