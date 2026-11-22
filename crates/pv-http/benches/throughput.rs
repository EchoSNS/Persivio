//! Throughput of segmented downloads from the local test server, by connection count.
//!
//! Run: `cargo bench -p pv-http --bench throughput`
//! (Locally this measures our overhead: loopback network + disk, no throttling.)

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs,
    reason = "benchmark harness code"
)]

use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use pv_core::{ProgressSink, Settings, Unlimited};
use pv_http::{
    ClientConfig, DownloadRequest, HttpClient, Segmented, SegmentedConfig, SegmentedOutcome, probe,
    run_segmented,
};
use pv_testkit::{Fixture, TestServer};
use tokio_util::sync::CancellationToken;
use url::Url;

const SIZE: usize = 32 * 1024 * 1024;

fn throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = Fixture::random("bench.bin", SIZE, 1);
    let server = rt.block_on(TestServer::start([fixture])).unwrap();
    let url = Url::parse(&server.url("/file/bench.bin")).unwrap();
    let client = HttpClient::new(&ClientConfig::from_settings(&Settings::default())).unwrap();
    let request = DownloadRequest::new(url.clone());
    let validator = rt.block_on(probe(&client, &request)).unwrap().etag.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let mut group = c.benchmark_group("segmented_download");
    group.throughput(Throughput::Bytes(SIZE as u64));
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    for connections in [1u8, 4, 8] {
        let cfg = SegmentedConfig {
            min_segment: 1024 * 1024,
            ..SegmentedConfig::from_settings(&Settings::default(), Some(connections))
        };
        group.bench_with_input(BenchmarkId::from_parameter(connections), &cfg, |b, cfg| {
            b.iter(|| {
                let part = dir.path().join("bench.part");
                let cancel = CancellationToken::new();
                let progress = ProgressSink::new();
                let t = Segmented {
                    client: &client,
                    request: &request,
                    url: &url,
                    part_path: &part,
                    total: SIZE as u64,
                    validator: &validator,
                    segments: Vec::new(),
                    cancel: &cancel,
                    progress: &progress,
                    limiter: Arc::new(Unlimited),
                    checkpoint: None,
                    writer_stats: None,
                };
                let outcome = rt.block_on(run_segmented(t, cfg)).unwrap();
                assert_eq!(outcome, SegmentedOutcome::Completed);
            });
        });
    }
    group.finish();
    drop(server);
}

criterion_group!(benches, throughput);
criterion_main!(benches);
