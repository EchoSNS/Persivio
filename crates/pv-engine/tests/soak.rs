//! Phase 11 soak test: rounds of 100 mixed downloads (plain, throttled, flaky, without
//! ranges, torrents), some paused and resumed, until the time is up. Every file is
//! checked by SHA-256, and the process's memory must not keep growing.
//!
//! Ignored by default. One hour, as in the plan:
//! `cargo nextest run --release -p pv-engine --test soak --run-ignored only --no-capture`
//! Shorter: set `PERSIVIO_SOAK_SECS`. The report is written to `target/soak.csv`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    reason = "test code; memory figures need no exact precision"
)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pv_core::{Settings, Source, TaskId, TaskSpec, TaskState, TaskStateKind, TorrentSettings};
use pv_engine::{Engine, EngineConfig, EngineHandle};
use pv_http::HttpProtocol;
use pv_testkit::{Fixture, TestServer, sha256_file};
use pv_torrent::TorrentProtocol;
use pv_torrent::testing::LocalSeeder;
use url::Url;

const PER_ROUND: usize = 100;
/// Rounds needed before the memory trend is judged.
const MIN_ROUNDS: usize = 8;

fn memory_mib() -> f64 {
    let stats = memory_stats::memory_stats().expect("memory statistics");
    // Virtual memory on Windows is committed ("private") memory, which no OS trims: the
    // better leak signal there. Resident memory elsewhere.
    let bytes = if cfg!(windows) {
        stats.virtual_mem
    } else {
        stats.physical_mem
    };
    bytes as f64 / 1_048_576.0
}

fn settings() -> Settings {
    Settings {
        max_concurrent_tasks: 12,
        min_segment_size: 64 * 1024,
        max_retries: 50,
        torrent: TorrentSettings {
            dht: false,
            upnp: false,
            never_seed: true,
            ..TorrentSettings::default()
        },
        ..Settings::default()
    }
}

struct World {
    server: TestServer,
    files: Vec<Fixture>,
    seeders: Vec<(LocalSeeder, Fixture)>,
}

impl World {
    async fn new(dir: &Path) -> Self {
        let files: Vec<Fixture> = (0..8u64)
            .map(|i| {
                let size = 200_000 + usize::try_from(i).unwrap() * 350_000;
                Fixture::random(format!("f{i}.bin"), size, 100 + i)
            })
            .collect();
        let server = TestServer::start(files.clone()).await.unwrap();
        let mut seeders = Vec::new();
        for i in 0..5u64 {
            let name = format!("t{i}.bin");
            let f = Fixture::random(&name, 900_000, 200 + i);
            let folder = dir.join(format!("seed{i}"));
            std::fs::create_dir_all(&folder).unwrap();
            let path = folder.join(&name);
            std::fs::write(&path, &f.bytes).unwrap();
            seeders.push((LocalSeeder::start(&path, &name).await, f));
        }
        Self {
            server,
            files,
            seeders,
        }
    }

    /// Download number `i` of a round: its source and expected hash.
    fn pick(&self, i: usize, round: usize) -> (Source, String) {
        // One torrent per seeder and round: one torrent can only run once at a time.
        if i < self.seeders.len() {
            let (seeder, f) = &self.seeders[i];
            let bytes = seeder.torrent.clone();
            return (Source::TorrentFile { bytes }, f.sha256.clone());
        }
        let f = &self.files[(i * 7 + round) % self.files.len()];
        let name = &f.name;
        let seed = round * PER_ROUND + i;
        let path = match i % 5 {
            0 => format!("/flaky/{name}?fail_rate=0.1&drop_after=90000&drop_rate=0.3&seed={seed}"),
            1 => format!("/norange/{name}"),
            2 => format!("/throttle/{name}?bps=3000000"),
            _ => format!("/file/{name}"),
        };
        let url = Url::parse(&self.server.url(&path)).unwrap();
        (Source::http(url).unwrap(), f.sha256.clone())
    }
}

async fn round(e: &EngineHandle, world: &World, dest: &Path, round: usize) {
    let mut expected: HashMap<TaskId, String> = HashMap::new();
    for i in 0..PER_ROUND {
        let (source, hash) = world.pick(i, round);
        let id = e
            .add(TaskSpec::new(source, dest.join(i.to_string())))
            .await
            .unwrap();
        expected.insert(id, hash);
    }
    let deadline = Instant::now() + Duration::from_mins(5);
    let mut disturbed = 0;
    let started = Instant::now();
    let mut reported = 0;
    loop {
        let tasks = e.list().await.unwrap();
        // What a slow round is waiting for (a round normally takes a few seconds).
        if started.elapsed().as_secs() >= 10 + reported * 10 {
            reported += 1;
            for t in &tasks {
                if t.state != TaskState::Completed {
                    println!(
                        "  waiting: {} of {:?}, {:?}, {:?}",
                        t.downloaded, t.total_size, t.state, t.spec.source
                    );
                }
            }
        }
        let mut open = 0;
        for t in &tasks {
            match &t.state {
                TaskState::Completed => {}
                TaskState::Failed { reason, .. } => {
                    panic!("{:?} failed: {reason}", t.spec.source)
                }
                TaskState::Paused => {
                    open += 1;
                    let _ = e.resume(t.id).await;
                }
                state => {
                    open += 1;
                    // Pause some running downloads, so resuming is exercised too.
                    if disturbed < 15 && state.kind() == TaskStateKind::Downloading {
                        disturbed += 1;
                        let _ = e.pause(t.id).await;
                    }
                }
            }
        }
        if open == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "round {round}: {open} still open"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for t in e.list().await.unwrap() {
        let path = t.spec.dest_dir.join(
            t.spec
                .file_name
                .as_deref()
                .expect("a finished file has a name"),
        );
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            expected[&t.id],
            "round {round}: {:?}",
            t.spec.source
        );
        e.remove(t.id, true).await.unwrap();
        assert!(!path.exists(), "removed with its file");
    }
    assert_eq!(e.list().await.unwrap().len(), 0);
}

fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs for an hour"]
async fn a_hundred_mixed_downloads_at_a_time_without_leaks_or_corruption() {
    let secs = std::env::var("PERSIVIO_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);
    let duration = Duration::from_secs(secs);
    let dir = tempfile::tempdir().unwrap();
    let world = World::new(dir.path()).await;
    let peers = world.seeders.iter().map(|(s, _)| s.addr).collect();
    let mut config = EngineConfig::new(dir.path().join("persivio.db"));
    config.protocols = vec![
        Arc::new(HttpProtocol),
        Arc::new(TorrentProtocol::with_work_dir(dir.path().join("work")).with_initial_peers(peers)),
    ];
    let e = Engine::start(config).await.unwrap();
    e.set_settings(settings()).await.unwrap();

    let start = Instant::now();
    let mut memory = Vec::new();
    let mut csv = String::from("round,seconds,memory_mib\n");
    while start.elapsed() < duration || memory.len() < MIN_ROUNDS {
        let n = memory.len();
        round(&e, &world, &dir.path().join("dl"), n).await;
        world.server.clear_requests(); // the test server's own log, not a leak
        memory.push(memory_mib());
        writeln!(csv, "{n},{},{:.1}", start.elapsed().as_secs(), memory[n]).unwrap();
        println!(
            "round {n}: {} s, {:.1} MiB",
            start.elapsed().as_secs(),
            memory[n]
        );
    }
    e.shutdown().await;
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::write(target.join("soak.csv"), csv).unwrap();

    // Ignore the warm-up, then compare the first and last quarter.
    let settled = &memory[memory.len() / 10 + 1..];
    let quarter = (settled.len() / 4).max(1);
    let first = median(&settled[..quarter]);
    let last = median(&settled[settled.len() - quarter..]);
    println!(
        "{} downloads in {} rounds; memory {first:.1} MiB -> {last:.1} MiB",
        memory.len() * PER_ROUND,
        memory.len()
    );
    assert!(
        last <= first * 1.20 + 16.0,
        "memory keeps growing: {first:.1} MiB -> {last:.1} MiB"
    );
}
