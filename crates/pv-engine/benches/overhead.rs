//! What 200 paused tasks cost the engine.
//!
//! Run: `cargo bench -p pv-engine --bench overhead`
//! The last line printed is CPU-independent: wake-ups are zero by design (the engine's
//! timer only runs while something is running), so idle cost shows up as `list` time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs,
    reason = "benchmark harness code"
)]

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use pv_core::{Source, TaskSpec};
use pv_engine::{Engine, EngineConfig};
use url::Url;

const TASKS: u32 = 200;

fn overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let start = |db: std::path::PathBuf| {
        let mut config = EngineConfig::new(db);
        config.autostart = false;
        rt.block_on(Engine::start(config)).unwrap()
    };
    let db = dir.path().join("db.sqlite");
    let engine = start(db.clone());
    rt.block_on(async {
        for i in 0..TASKS {
            let url = Url::parse(&format!("https://example.invalid/{i}.bin")).unwrap();
            let id = engine
                .add(TaskSpec::new(Source::http(url).unwrap(), dir.path()))
                .await
                .unwrap();
            engine.pause(id).await.unwrap();
        }
    });

    let mut group = c.benchmark_group("engine_200_paused_tasks");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(5));
    group.bench_function("list", |b| {
        b.iter(|| {
            let tasks = rt.block_on(engine.list()).unwrap();
            assert_eq!(tasks.len(), TASKS as usize);
        });
    });
    rt.block_on(engine.shutdown());
    group.bench_function("start_and_shutdown", |b| {
        b.iter(|| {
            let engine = start(db.clone());
            rt.block_on(engine.shutdown());
        });
    });
    group.finish();
}

criterion_group!(benches, overhead);
criterion_main!(benches);
