//! Phase 4 acceptance tests for segmented downloads. Every test checks SHA-256.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use pv_core::{
    CheckpointError, CheckpointSink, HttpResumeInfo, Outcome, ProgressSink, Protocol,
    ResolvedMetadata, ResumeState, RunContext, RunEvents, RunPhase, Segment, Settings, Source,
    TaskId, TaskSnapshot, TaskSpec, Unlimited,
};
use pv_http::segmented::WriterStats;
use pv_http::{
    ClientConfig, DownloadRequest, HttpClient, HttpProtocol, RetryPolicy, Segmented,
    SegmentedConfig, SegmentedOutcome, probe, run_segmented,
};
use pv_testkit::{Fixture, TestServer, sha256_file};
use tokio_util::sync::CancellationToken;
use url::Url;

const KIB: u64 = 1024;

#[derive(Default)]
struct MemSink(Mutex<Vec<Segment>>);

#[async_trait]
impl CheckpointSink for MemSink {
    async fn save_segments(&self, _: TaskId, s: &[Segment]) -> Result<(), CheckpointError> {
        *self.0.lock().unwrap() = s.to_vec();
        Ok(())
    }
    async fn save_progress(&self, _: TaskId, p: &[(u32, u64)]) -> Result<(), CheckpointError> {
        let mut table = self.0.lock().unwrap();
        for &(i, d) in p {
            table
                .iter_mut()
                .find(|s| s.index == i)
                .ok_or_else(|| CheckpointError(format!("segment {i} missing")))?
                .downloaded = d;
        }
        Ok(())
    }
}

impl MemSink {
    fn segments(&self) -> Vec<Segment> {
        self.0.lock().unwrap().clone()
    }
}

fn config(connections: u8) -> SegmentedConfig {
    SegmentedConfig {
        connections,
        min_segment: 64 * KIB,
        idle_timeout: Duration::from_secs(5),
        retry: RetryPolicy {
            max_retries: 30,
            base: Duration::from_millis(5),
            cap: Duration::from_millis(50),
        },
        checkpoint_interval: Duration::from_millis(50),
        write_queue: 512,
        write_delay: None,
    }
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig::from_settings(&Settings::default())).unwrap()
}

/// The strong `ETag` the server uses for `name` (identical on every route).
async fn etag(s: &TestServer, name: &str) -> String {
    let url = Url::parse(&s.url(&format!("/file/{name}"))).unwrap();
    probe(&client(), &DownloadRequest::new(url))
        .await
        .unwrap()
        .etag
        .unwrap()
}

struct Job {
    url: Url,
    part: PathBuf,
    total: u64,
    validator: String,
    segments: Vec<Segment>,
    sink: Arc<MemSink>,
    progress: ProgressSink,
    cancel: CancellationToken,
    stats: Arc<WriterStats>,
}

impl Job {
    fn new(url: &str, part: &Path, total: u64, validator: &str) -> Self {
        Self {
            url: Url::parse(url).unwrap(),
            part: part.to_path_buf(),
            total,
            validator: validator.to_owned(),
            segments: Vec::new(),
            sink: Arc::default(),
            progress: ProgressSink::new(),
            cancel: CancellationToken::new(),
            stats: Arc::default(),
        }
    }

    async fn run(&self, cfg: &SegmentedConfig) -> Result<SegmentedOutcome, pv_core::ProtocolError> {
        let client = client();
        let request = DownloadRequest::new(self.url.clone());
        let t = Segmented {
            client: &client,
            request: &request,
            url: &self.url,
            part_path: &self.part,
            total: self.total,
            validator: &self.validator,
            segments: self.segments.clone(),
            cancel: &self.cancel,
            progress: &self.progress,
            limiter: Arc::new(Unlimited),
            checkpoint: Some((TaskId::new(), self.sink.clone())),
            writer_stats: Some(self.stats.clone()),
        };
        run_segmented(t, cfg).await
    }
}

/// Acceptance: 8 connections at least 5x faster than 1 on a per-connection-throttled server.
#[tokio::test]
async fn eight_connections_are_at_least_five_times_faster() {
    let f = Fixture::random("big.bin", 1_000_000, 1);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let v = etag(&s, "big.bin").await;
    let url = s.url("/throttle/big.bin?bps=250000"); // 250 KB/s per connection
    let dir = tempfile::tempdir().unwrap();

    let mut times = Vec::new();
    for n in [1u8, 8] {
        let part = dir.path().join(format!("n{n}.part"));
        let job = Job::new(&url, &part, 1_000_000, &v);
        let started = Instant::now();
        assert_eq!(
            job.run(&config(n)).await.unwrap(),
            SegmentedOutcome::Completed
        );
        times.push(started.elapsed());
        assert_eq!(
            sha256_file(&part).await.unwrap(),
            f.sha256,
            "{n} connections"
        );
    }
    let speedup = times[0].as_secs_f64() / times[1].as_secs_f64();
    println!(
        "1 conn: {:?}, 8 conns: {:?}, speedup {speedup:.1}x",
        times[0], times[1]
    );
    assert!(speedup >= 5.0, "speedup only {speedup:.2}x ({times:?})");
}

/// Acceptance: 20/20 runs against random 503s and mid-stream drops produce the exact file.
#[tokio::test]
async fn flaky_server_twenty_runs_all_correct() {
    let f = Fixture::random("flaky.bin", 512 * 1024, 2);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let v = etag(&s, "flaky.bin").await;
    let dir = tempfile::tempdir().unwrap();
    for seed in 0..20 {
        let url = s.url(&format!(
            "/flaky/flaky.bin?fail_rate=0.2&drop_after=40000&seed={seed}"
        ));
        let part = dir.path().join(format!("run{seed}.part"));
        let job = Job::new(&url, &part, f.len() as u64, &v);
        assert_eq!(
            job.run(&config(8)).await.unwrap(),
            SegmentedOutcome::Completed,
            "seed {seed}"
        );
        assert_eq!(sha256_file(&part).await.unwrap(), f.sha256, "seed {seed}");
        assert_eq!(
            job.sink
                .segments()
                .iter()
                .map(|s| s.downloaded)
                .sum::<u64>(),
            f.len() as u64
        );
    }
}

/// Acceptance: kill mid-flight (abort, no graceful shutdown), resume from persisted
/// checkpoints, exact file. 50 consecutive runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_and_resume_fifty_times() {
    let f = Fixture::random("crash.bin", 600_000, 3);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let v = etag(&s, "crash.bin").await;
    let dir = tempfile::tempdir().unwrap();
    let slow = s.url("/throttle/crash.bin?bps=600000");
    let fast = s.url("/file/crash.bin");

    for run in 0..50u64 {
        let part = dir.path().join(format!("crash{run}.part"));
        let job = Arc::new(Job::new(&slow, &part, 600_000, &v));
        let runner = job.clone();
        let handle = tokio::spawn(async move { runner.run(&config(4)).await });
        // Kill somewhere between 10 % and ~60 %, varying per run.
        let kill_at = 60_000 + (run * 7_919) % 300_000;
        while job.progress.sample().downloaded < kill_at {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        handle.abort();
        assert!(
            handle.await.unwrap_err().is_cancelled(),
            "run {run}: killed"
        );

        // "Restart": only the persisted checkpoints survive.
        let persisted = job.sink.segments();
        let mut resumed = Job::new(&fast, &part, 600_000, &v);
        resumed.segments = persisted.clone();
        resumed.sink = job.sink.clone();
        let outcome = resumed.run(&config(4)).await.unwrap();
        assert_eq!(outcome, SegmentedOutcome::Completed, "run {run}");
        assert_eq!(
            sha256_file(&part).await.unwrap(),
            f.sha256,
            "run {run} resumed from {persisted:?}"
        );
    }
}

/// Acceptance (bounded memory): with a disk much slower than the network, bytes queued
/// for the writer never exceed the configured budget, regardless of file size.
#[tokio::test]
async fn slow_disk_keeps_memory_bounded() {
    let f = Fixture::random("mem.bin", 24 * 1024 * 1024, 4);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let v = etag(&s, "mem.bin").await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("mem.part");
    let job = Job::new(&s.url("/file/mem.bin"), &part, f.len() as u64, &v);
    let cfg = SegmentedConfig {
        write_queue: 16,                               // 16 x 64 KiB = 1 MiB budget
        write_delay: Some(Duration::from_micros(500)), // ~128 MB/s disk vs. unthrottled network
        ..config(8)
    };
    assert_eq!(job.run(&cfg).await.unwrap(), SegmentedOutcome::Completed);
    assert_eq!(sha256_file(&part).await.unwrap(), f.sha256);
    let peak = job.stats.max_in_flight.load(Ordering::Relaxed);
    // Queue + one message being written + one blocked sender per connection.
    let bound = (16 + 1 + 8) * pv_http::segmented::MAX_MESSAGE as u64;
    println!(
        "peak in flight: {} KiB (bound {} KiB, file {} KiB)",
        peak / KIB,
        bound / KIB,
        f.len() as u64 / KIB
    );
    assert!(peak <= bound, "peak {peak} > bound {bound}");
    assert!(
        peak < f.len() as u64 / 8,
        "memory did not scale with file size"
    );
}

/// Dynamic splitting: idle workers steal halves of the remaining big segment.
#[tokio::test]
async fn idle_workers_steal_work_from_the_largest_segment() {
    let f = Fixture::random("steal.bin", 1_000_000, 5);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let v = etag(&s, "steal.bin").await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("steal.part");
    // A resumed download: segment 0 done, segment 1 (750 KB) untouched.
    let mut prefilled = f.bytes[..250_000].to_vec();
    prefilled.resize(1_000_000, 0);
    std::fs::write(&part, prefilled).unwrap();
    let mut job = Job::new(
        &s.url("/throttle/steal.bin?bps=300000"),
        &part,
        1_000_000,
        &v,
    );
    job.segments = vec![
        Segment {
            index: 0,
            start: 0,
            end: 249_999,
            downloaded: 250_000,
        },
        Segment {
            index: 1,
            start: 250_000,
            end: 999_999,
            downloaded: 0,
        },
    ];
    let started = Instant::now();
    assert_eq!(
        job.run(&config(4)).await.unwrap(),
        SegmentedOutcome::Completed
    );
    let elapsed = started.elapsed();
    assert_eq!(sha256_file(&part).await.unwrap(), f.sha256);

    let table = job.sink.segments();
    assert!(table.len() >= 4, "segment 1 was split: {table:?}");
    let covered: u64 = table.iter().map(Segment::len).sum();
    assert_eq!(
        covered, 1_000_000,
        "split segments still cover the file exactly"
    );
    // One connection would need 750 KB / 300 KB/s = 2.5 s.
    assert!(
        elapsed < Duration::from_millis(1600),
        "no speedup from stealing: {elapsed:?}"
    );
}

#[derive(Default)]
struct Events(Mutex<Vec<ResolvedMetadata>>);

impl RunEvents for Events {
    fn phase(&self, _: RunPhase) {}
    fn metadata(&self, meta: ResolvedMetadata) {
        self.0.lock().unwrap().push(meta);
    }
    fn warning(&self, _: String) {}
}

fn ctx(
    dest: &Path,
    url: &str,
    resume: ResumeState,
    name: Option<&str>,
    total: Option<u64>,
    sink: Arc<MemSink>,
) -> (RunContext, Arc<Events>) {
    let mut spec = TaskSpec::new(Source::http(Url::parse(url).unwrap()).unwrap(), dest);
    spec.file_name = name.map(str::to_owned);
    spec.connections = Some(4);
    let mut task = TaskSnapshot::new(spec);
    task.total_size = total;
    let events = Arc::new(Events::default());
    let settings = Settings {
        min_segment_size: 64 * KIB,
        ..Settings::default()
    };
    let ctx = RunContext {
        task,
        resume,
        settings: Arc::new(settings),
        cancel: CancellationToken::new(),
        progress: ProgressSink::new(),
        limiter: Arc::new(Unlimited),
        checkpoint: sink,
        events: events.clone(),
    };
    (ctx, events)
}

/// The server's file changed between runs: segments get `200` (If-Range mismatch), the
/// protocol starts over and delivers the new version.
#[tokio::test]
async fn file_changed_since_last_run_restarts_cleanly() {
    let f = Fixture::random("m.bin", 400_000, 6);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let old_etag = etag(&s, "m.bin").await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("dl");
    std::fs::create_dir_all(&dest).unwrap();
    let mut partial = f.bytes[..100_000].to_vec();
    partial.resize(400_000, 0);
    std::fs::write(dest.join("m.bin.part"), partial).unwrap();
    let new = s.mutate("m.bin").unwrap();

    let url = s.url("/mutable/m.bin");
    let resume = ResumeState {
        http: Some(HttpResumeInfo {
            final_url: Some(Url::parse(&url).unwrap()),
            etag: Some(old_etag),
            last_modified: None,
        }),
        segments: vec![
            Segment {
                index: 0,
                start: 0,
                end: 199_999,
                downloaded: 100_000,
            },
            Segment {
                index: 1,
                start: 200_000,
                end: 399_999,
                downloaded: 0,
            },
        ],
    };
    let (c, events) = ctx(
        &dest,
        &url,
        resume,
        Some("m.bin"),
        Some(400_000),
        Arc::default(),
    );
    let Outcome::Completed { path } = HttpProtocol.run(c).await.unwrap() else {
        panic!("not completed")
    };
    assert_eq!(
        sha256_file(&path).await.unwrap(),
        new.sha256,
        "the new version, not a mix"
    );
    assert!(
        !events.0.lock().unwrap().is_empty(),
        "fresh metadata after restart"
    );
}

/// The `.part` file vanished (user deleted it): start over instead of trusting checkpoints.
#[tokio::test]
async fn missing_part_file_restarts_instead_of_trusting_checkpoints() {
    let f = Fixture::random("gone.bin", 300_000, 7);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let v = etag(&s, "gone.bin").await;
    let dir = tempfile::tempdir().unwrap();
    let url = s.url("/file/gone.bin");
    let resume = ResumeState {
        http: Some(HttpResumeInfo {
            final_url: None,
            etag: Some(v),
            last_modified: None,
        }),
        segments: vec![Segment {
            index: 0,
            start: 0,
            end: 299_999,
            downloaded: 150_000,
        }],
    };
    let (c, _) = ctx(
        dir.path(),
        &url,
        resume,
        Some("gone.bin"),
        Some(300_000),
        Arc::default(),
    );
    let Outcome::Completed { path } = HttpProtocol.run(c).await.unwrap() else {
        panic!("not completed")
    };
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
}

/// End to end through the protocol: fresh segmented download with several connections.
#[tokio::test]
async fn protocol_uses_multiple_connections_for_ranged_servers() {
    let f = Fixture::random("multi.bin", 800_000, 8);
    let s = TestServer::start([f.clone()]).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sink = Arc::new(MemSink::default());
    let (c, _) = ctx(
        dir.path(),
        &s.url("/file/multi.bin"),
        ResumeState::default(),
        None,
        None,
        sink.clone(),
    );
    let Outcome::Completed { path } = HttpProtocol.run(c).await.unwrap() else {
        panic!("not completed")
    };
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    let ranges = s
        .requests()
        .iter()
        .filter_map(|r| r.header("range").map(str::to_owned))
        .filter(|r| r != "bytes=0-0")
        .count();
    assert!(ranges >= 4, "expected >= 4 segment requests, got {ranges}");
    for r in s.requests().iter().skip(1) {
        assert!(
            r.header("if-range").is_some(),
            "every segment request is validated"
        );
    }
    assert_eq!(
        sink.segments().iter().map(|s| s.downloaded).sum::<u64>(),
        800_000
    );
}
