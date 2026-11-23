//! Phase 3 integration tests: probe + single-connection download + resume against the
//! `pv-testkit` servers. Every download asserts the SHA-256 of the result.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use pv_core::{
    CheckpointError, CheckpointSink, HttpResumeInfo, Outcome, ProgressSink, Protocol,
    ProtocolError, ResolvedMetadata, ResumeState, RunContext, RunEvents, RunPhase, Segment,
    Settings, Source, TaskId, TaskSnapshot, TaskSpec, Unlimited,
};
use pv_http::{
    ClientConfig, DownloadRequest, HttpClient, HttpProtocol, Transfer, TransferOutcome, probe,
    run_single,
};
use pv_testkit::{Fixture, TestServer, sha256_file};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use url::Url;

const NAME: &str = "data.bin";

/// In-memory stand-in for the store: keeps the segment table like SQLite would.
#[derive(Default)]
struct MemSink(Mutex<Vec<Segment>>);

#[async_trait]
impl CheckpointSink for MemSink {
    async fn save_segments(&self, _: TaskId, segments: &[Segment]) -> Result<(), CheckpointError> {
        *self.0.lock().unwrap() = segments.to_vec();
        Ok(())
    }
    async fn save_progress(
        &self,
        _: TaskId,
        progress: &[(u32, u64)],
    ) -> Result<(), CheckpointError> {
        let mut table = self.0.lock().unwrap();
        for &(index, downloaded) in progress {
            let seg = table
                .iter_mut()
                .find(|s| s.index == index)
                .ok_or_else(|| CheckpointError(format!("segment {index} missing")))?;
            seg.downloaded = downloaded;
        }
        Ok(())
    }
}

impl MemSink {
    fn segments(&self) -> Vec<Segment> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct Events {
    phases: Mutex<Vec<RunPhase>>,
    metadata: Mutex<Vec<ResolvedMetadata>>,
    warnings: Mutex<Vec<String>>,
}

impl RunEvents for Events {
    fn phase(&self, phase: RunPhase) {
        self.phases.lock().unwrap().push(phase);
    }
    fn metadata(&self, meta: ResolvedMetadata) {
        self.metadata.lock().unwrap().push(meta);
    }
    fn warning(&self, message: String) {
        self.warnings.lock().unwrap().push(message);
    }
}

struct Harness {
    _dir: TempDir,
    dest: PathBuf,
    sink: Arc<MemSink>,
    events: Arc<Events>,
    progress: ProgressSink,
    cancel: CancellationToken,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            dest: dir.path().join("downloads"),
            _dir: dir,
            sink: Arc::default(),
            events: Arc::default(),
            progress: ProgressSink::new(),
            cancel: CancellationToken::new(),
        }
    }

    fn ctx(
        &self,
        source: Source,
        resume: ResumeState,
        file_name: Option<String>,
        total: Option<u64>,
    ) -> RunContext {
        let mut spec = TaskSpec::new(source, &self.dest);
        spec.file_name = file_name;
        let mut task = TaskSnapshot::new(spec);
        task.total_size = total;
        let settings = Settings {
            idle_timeout_secs: 5,
            ..Settings::default()
        };
        RunContext {
            task,
            resume,
            settings: Arc::new(settings),
            cancel: self.cancel.clone(),
            progress: self.progress.clone(),
            limiter: Arc::new(Unlimited),
            checkpoint: self.sink.clone(),
            events: self.events.clone(),
        }
    }

    async fn run(&self, url: &str) -> Result<Outcome, ProtocolError> {
        let source = Source::http(Url::parse(url).unwrap()).unwrap();
        self.run_source(source).await
    }

    async fn run_source(&self, source: Source) -> Result<Outcome, ProtocolError> {
        HttpProtocol
            .run(self.ctx(source, ResumeState::default(), None, None))
            .await
    }

    fn last_metadata(&self) -> ResolvedMetadata {
        self.events
            .metadata
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap()
    }
}

fn completed_path(outcome: Outcome) -> PathBuf {
    match outcome {
        Outcome::Completed { path } => path,
        Outcome::Stopped => panic!("expected completion"),
    }
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig::from_settings(&Settings::default())).unwrap()
}

async fn start(fixture: &Fixture) -> TestServer {
    TestServer::start([fixture.clone()]).await.unwrap()
}

fn part_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "part"))
        .map(|p| p.display().to_string())
        .collect()
}

#[tokio::test]
async fn downloads_a_file_end_to_end() {
    let f = Fixture::random(NAME, 300_000, 1);
    let s = start(&f).await;
    let h = Harness::new();
    let path = completed_path(h.run(&s.url("/file/data.bin")).await.unwrap());

    assert_eq!(path, h.dest.join(NAME));
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    assert!(part_files(&h.dest).is_empty(), ".part renamed away");
    assert_eq!(
        *h.events.phases.lock().unwrap(),
        [RunPhase::Probing, RunPhase::Downloading]
    );
    let meta = h.last_metadata();
    assert_eq!(meta.name.as_deref(), Some(NAME));
    assert_eq!(meta.total_size, Some(300_000));
    assert!(meta.http.unwrap().etag.is_some());
    assert_eq!(
        h.sink.segments(),
        [Segment {
            index: 0,
            start: 0,
            end: 299_999,
            downloaded: 300_000
        }]
    );
    assert_eq!(h.progress.sample().downloaded, 300_000);

    // The probe asked for one byte; every request asked for identity encoding.
    let reqs = s.requests();
    assert_eq!(reqs[0].header("range"), Some("bytes=0-0"));
    for r in &reqs {
        assert_eq!(r.header("accept-encoding"), Some("identity"), "{}", r.path);
        assert!(r.header("user-agent").unwrap().starts_with("Persivio/"));
    }
}

#[tokio::test]
async fn request_context_is_sent_and_reserved_headers_are_protected() {
    let f = Fixture::random(NAME, 10_000, 2);
    let s = start(&f).await;
    let h = Harness::new();
    let source = Source::Http {
        url: Url::parse(&s.url("/file/data.bin")).unwrap(),
        headers: vec![
            ("X-Token".into(), "abc".into()),
            ("Accept-Encoding".into(), "gzip".into()), // must be ignored
            ("Range".into(), "bytes=5-".into()),       // must be ignored
        ],
        cookies: Some("session=xyz".into()),
        referer: Some(Url::parse("https://origin.example/page").unwrap()),
    };
    let path = completed_path(
        HttpProtocol
            .run(h.ctx(source, ResumeState::default(), None, None))
            .await
            .unwrap(),
    );
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    for r in s.requests() {
        assert_eq!(r.header("x-token"), Some("abc"));
        assert_eq!(r.header("cookie"), Some("session=xyz"));
        assert_eq!(r.header("referer"), Some("https://origin.example/page"));
        assert_eq!(r.header("accept-encoding"), Some("identity"));
        assert_ne!(r.header("range"), Some("bytes=5-"));
    }
}

#[tokio::test]
async fn norange_reuses_the_probe_response() {
    let f = Fixture::random(NAME, 200_000, 3);
    let s = start(&f).await;
    let h = Harness::new();
    let path = completed_path(h.run(&s.url("/norange/data.bin")).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    assert_eq!(
        s.requests().len(),
        1,
        "the 200 body of the probe is the download"
    );
    assert!(h.events.warnings.lock().unwrap()[0].contains("does not support resuming"));
}

#[tokio::test]
async fn follows_redirects_and_records_final_url() {
    let f = Fixture::random(NAME, 50_000, 4);
    let s = start(&f).await;
    let h = Harness::new();
    let path = completed_path(h.run(&s.url("/redirect/3/data.bin")).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    let http = h.last_metadata().http.unwrap();
    assert_eq!(http.final_url.unwrap().path(), "/file/data.bin");
}

#[tokio::test]
async fn refuses_redirects_to_non_http_schemes() {
    let s = TestServer::start([]).await.unwrap();
    let h = Harness::new();
    for target in ["file:///etc/passwd", "ftp://example.com/x"] {
        let url = Url::parse_with_params(&s.url("/redirect-to"), &[("location", target)]).unwrap();
        let err = h.run(url.as_str()).await.unwrap_err();
        assert!(
            matches!(&err, ProtocolError::Network(m) if m.contains("blocked")),
            "{target}: {err}"
        );
    }
    assert!(!h.dest.exists(), "nothing written");
}

#[tokio::test]
async fn finished_files_carry_the_mark_of_the_web_on_windows() {
    let f = Fixture::random(NAME, 20_000, 8);
    let s = start(&f).await;
    let h = Harness::new();
    let url = s.url("/file/data.bin?token=secret");
    let path = completed_path(h.run(&url).await.unwrap());
    if cfg!(windows) {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let text = std::fs::read_to_string(&stream).unwrap();
        let host_url = url.split('?').next().unwrap();
        assert_eq!(
            text,
            format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={host_url}\r\n"),
            "query (and its token) stripped"
        );
    }
}

/// Regression: the downloader requested the post-redirect URL directly with the user's
/// cookies and custom headers, so a redirect to another host (a CDN) received them.
#[tokio::test]
async fn credentials_are_not_sent_to_another_host_after_a_redirect() {
    let f = Fixture::random(NAME, 300_000, 6);
    let s = start(&f).await;
    let origin = s.url("/file/data.bin");
    let elsewhere = origin.replace("127.0.0.1", "localhost");
    assert_ne!(origin, elsewhere, "test server must listen on 127.0.0.1");
    let start_url =
        Url::parse_with_params(&s.url("/redirect-to"), &[("location", &elsewhere)]).unwrap();
    let source = Source::Http {
        url: start_url,
        headers: vec![("X-Api-Key".into(), "key-123".into())],
        cookies: Some("session=secret".into()),
        referer: None,
    };
    let h = Harness::new();
    let path = completed_path(h.run_source(source).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);

    let requests = s.requests();
    let (first, rest) = requests.split_first().unwrap();
    assert_eq!(first.header("cookie"), Some("session=secret"));
    assert_eq!(first.header("x-api-key"), Some("key-123"));
    assert!(rest.len() >= 2, "probe and download went to the other host");
    for r in rest {
        assert!(r.header("host").unwrap().starts_with("localhost"), "{r:?}");
        assert_eq!(r.header("cookie"), None, "cookie leaked: {r:?}");
        assert_eq!(r.header("x-api-key"), None, "header leaked: {r:?}");
    }
}

#[tokio::test]
async fn credentials_are_kept_on_same_host_redirects() {
    let f = Fixture::random(NAME, 300_000, 7);
    let s = start(&f).await;
    let source = Source::Http {
        url: Url::parse(&s.url("/redirect/2/data.bin")).unwrap(),
        headers: vec![("X-Api-Key".into(), "key-123".into())],
        cookies: Some("session=secret".into()),
        referer: None,
    };
    let h = Harness::new();
    completed_path(h.run_source(source).await.unwrap());
    for r in s.requests() {
        assert_eq!(r.header("cookie"), Some("session=secret"), "{r:?}");
        assert_eq!(r.header("x-api-key"), Some("key-123"), "{r:?}");
    }
}

#[tokio::test]
async fn content_disposition_names_are_decoded_and_sanitized() {
    let f = Fixture::random(NAME, 20_000, 5);
    let s = start(&f).await;
    let cases = [
        (
            "attachment; filename*=UTF-8''na%C3%AFve%20%E6%97%A5%E6%9C%AC.txt",
            "naïve 日本.txt",
        ),
        ("attachment; filename=\"../../evil.sh\"", "evil.sh"),
        ("attachment; filename=\"CON.txt\"", "_CON.txt"),
        (
            "attachment; filename=\"caf\u{e9} report.pdf\"",
            "café report.pdf",
        ),
    ];
    for (cd, expected) in cases {
        let h = Harness::new();
        let url = Url::parse_with_params(&s.url("/disposition/data.bin"), &[("cd", cd)]).unwrap();
        let path = completed_path(h.run(url.as_str()).await.unwrap());
        assert_eq!(path, h.dest.join(expected), "{cd}");
        assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    }
}

#[tokio::test]
async fn gzip_route_serves_identity_and_misconfigured_servers_are_rejected() {
    let f = Fixture::random(NAME, 80_000, 6);
    let s = start(&f).await;
    let h = Harness::new();
    let path = completed_path(h.run(&s.url("/gzip/data.bin")).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);

    let err = Harness::new()
        .run(&s.url("/gzip-always/data.bin"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ProtocolError::InvalidSource(m) if m.contains("Content-Encoding: gzip")),
        "{err}"
    );
}

#[tokio::test]
async fn http_errors_are_reported() {
    let s = TestServer::start([]).await.unwrap();
    let err = Harness::new()
        .run(&s.url("/file/missing.bin"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ProtocolError::HttpStatus { status: 404 }),
        "{err}"
    );
}

#[tokio::test]
async fn empty_files_download_as_empty_files() {
    let f = Fixture::from_bytes("empty.txt", Vec::new());
    let s = start(&f).await;
    let h = Harness::new();
    let path = completed_path(h.run(&s.url("/file/empty.txt")).await.unwrap());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    assert_eq!(h.last_metadata().total_size, Some(0));
}

#[tokio::test]
async fn existing_files_are_never_overwritten() {
    let f = Fixture::random(NAME, 10_000, 7);
    let s = start(&f).await;
    let h = Harness::new();
    std::fs::create_dir_all(&h.dest).unwrap();
    std::fs::write(h.dest.join(NAME), b"precious").unwrap();
    let path = completed_path(h.run(&s.url("/file/data.bin")).await.unwrap());
    assert_eq!(path, h.dest.join("data (1).bin"));
    assert_eq!(std::fs::read(h.dest.join(NAME)).unwrap(), b"precious");
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
}

/// Cancel at ~40%, then resume in a new run from the persisted checkpoint.
#[tokio::test]
async fn cancel_and_resume_continue_from_the_checkpoint() {
    let f = Fixture::random(NAME, 400_000, 8);
    let s = start(&f).await;
    let url = s.url("/throttle/data.bin?bps=500000");
    let source = Source::http(Url::parse(&url).unwrap()).unwrap();

    let h = Harness::new();
    let run =
        tokio::spawn(HttpProtocol.run(h.ctx(source.clone(), ResumeState::default(), None, None)));
    while h.progress.sample().downloaded < 160_000 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    h.cancel.cancel();
    assert_eq!(run.await.unwrap().unwrap(), Outcome::Stopped);

    let segments = h.sink.segments();
    let durable = segments[0].downloaded;
    assert!(
        (160_000..400_000).contains(&durable),
        "checkpoint at cancel: {durable}"
    );
    let part = h.dest.join("data.bin.part");
    assert!(std::fs::metadata(&part).unwrap().len() >= durable);

    // "Restart the app": a fresh context built only from what was persisted.
    let meta = h.last_metadata();
    let h2 = Harness {
        sink: h.sink.clone(),
        ..Harness::new()
    };
    let h2 = Harness {
        dest: h.dest.clone(),
        ..h2
    };
    s.clear_requests();
    let resume = ResumeState {
        torrent: None,
        http: meta.http.clone(),
        segments,
    };
    let ctx = h2.ctx(source, resume, meta.name.clone(), meta.total_size);
    let path = completed_path(HttpProtocol.run(ctx).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);

    let reqs = s.requests();
    assert_eq!(reqs.len(), 1, "no new probe on resume");
    assert_eq!(
        reqs[0].header("range"),
        Some(format!("bytes={durable}-399999").as_str()),
        "resume starts exactly at the checkpoint (segment requests name their end)"
    );
    assert_eq!(
        reqs[0].header("if-range"),
        meta.http.unwrap().etag.as_deref()
    );
    assert_eq!(
        *h2.events.phases.lock().unwrap(),
        [RunPhase::Downloading],
        "resume skips probing"
    );
}

async fn transfer(
    url: &Url,
    part: &Path,
    resume_from: u64,
    validator: Option<&str>,
    expected_total: Option<u64>,
    sink: Option<&MemSink>,
    idle: Duration,
) -> Result<TransferOutcome, ProtocolError> {
    let client = client();
    let request = DownloadRequest::new(url.clone());
    let progress = ProgressSink::new();
    let cancel = CancellationToken::new();
    let t = Transfer {
        client: &client,
        request: &request,
        url,
        part_path: part,
        resume_from,
        validator,
        expected_total,
        idle_timeout: idle,
        cancel: &cancel,
        progress: &progress,
        limiter: &Unlimited,
        checkpoint: sink.map(|sink| pv_http::Checkpointing {
            task: TaskId::new(),
            sink,
            interval: Duration::from_millis(100),
        }),
    };
    run_single(&t, None).await
}

#[tokio::test]
async fn changed_remote_file_restarts_from_zero() {
    let f = Fixture::random(NAME, 120_000, 9);
    let s = start(&f).await;
    let url = Url::parse(&s.url("/mutable/data.bin")).unwrap();
    let p = probe(&client(), &DownloadRequest::new(url.clone()))
        .await
        .unwrap();
    let old_etag = p.etag.clone().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("data.bin.part");
    std::fs::write(&part, &f.bytes[..50_000]).unwrap(); // a previous partial download
    let v1 = s.mutate(NAME).unwrap();

    let out = transfer(
        &url,
        &part,
        50_000,
        Some(&old_etag),
        Some(120_000),
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    let TransferOutcome::Completed {
        size,
        restarted,
        validators,
    } = out
    else {
        panic!("{out:?}")
    };
    assert!(restarted, "If-Range mismatch must restart");
    assert_eq!(size, 120_000);
    assert_ne!(
        validators.etag.as_deref(),
        Some(old_etag.as_str()),
        "new validators reported"
    );
    assert_eq!(
        sha256_file(&part).await.unwrap(),
        v1.sha256,
        "the new file, not a mix"
    );
}

#[tokio::test]
async fn already_complete_part_is_recognized_via_416() {
    let f = Fixture::random(NAME, 30_000, 10);
    let s = start(&f).await;
    let url = Url::parse(&s.url("/file/data.bin")).unwrap();
    let etag = probe(&client(), &DownloadRequest::new(url.clone()))
        .await
        .unwrap()
        .etag
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("x.part");
    std::fs::write(&part, &f.bytes).unwrap();
    let out = transfer(
        &url,
        &part,
        30_000,
        Some(&etag),
        Some(30_000),
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            out,
            TransferOutcome::Completed {
                size: 30_000,
                restarted: false,
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(sha256_file(&part).await.unwrap(), f.sha256);
}

#[tokio::test]
async fn stalled_server_hits_the_idle_timeout_with_a_durable_checkpoint() {
    let f = Fixture::random(NAME, 100_000, 11);
    let s = start(&f).await;
    let url = Url::parse(&s.url("/hang/data.bin?after=40000")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("x.part");
    let sink = MemSink::default();
    let err = transfer(
        &url,
        &part,
        0,
        None,
        None,
        Some(&sink),
        Duration::from_millis(300),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, ProtocolError::Network(m) if m.contains("no data received")),
        "{err}"
    );
    assert_eq!(sink.segments()[0].downloaded, 40_000);
    assert_eq!(std::fs::read(&part).unwrap(), f.bytes[..40_000]);
}

#[tokio::test]
async fn dropped_connection_then_resume_produces_the_exact_file() {
    let f = Fixture::random(NAME, 150_000, 12);
    let s = start(&f).await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("x.part");
    let sink = MemSink::default();

    let flaky = Url::parse(&s.url("/flaky/data.bin?drop_after=70000")).unwrap();
    let err = transfer(
        &flaky,
        &part,
        0,
        None,
        None,
        Some(&sink),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ProtocolError::Network(_)), "{err}");
    let durable = sink.segments()[0].downloaded;
    assert_eq!(durable, 70_000);

    // Same representation (same ETag) on /file: resume from the checkpoint.
    let file = Url::parse(&s.url("/file/data.bin")).unwrap();
    let etag = probe(&client(), &DownloadRequest::new(file.clone()))
        .await
        .unwrap()
        .etag
        .unwrap();
    let out = transfer(
        &file,
        &part,
        durable,
        Some(&etag),
        Some(150_000),
        Some(&sink),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            out,
            TransferOutcome::Completed {
                size: 150_000,
                restarted: false,
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(sha256_file(&part).await.unwrap(), f.sha256);
    assert_eq!(sink.segments()[0].downloaded, 150_000);
}

#[tokio::test]
async fn resume_without_validator_starts_over_safely() {
    let f = Fixture::random(NAME, 40_000, 13);
    let s = start(&f).await;
    let url = Url::parse(&s.url("/file/data.bin")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("x.part");
    std::fs::write(&part, vec![0u8; 20_000]).unwrap(); // garbage we cannot validate
    let out = transfer(
        &url,
        &part,
        20_000,
        None,
        Some(40_000),
        None,
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert!(matches!(
        out,
        TransferOutcome::Completed { size: 40_000, .. }
    ));
    assert_eq!(sha256_file(&part).await.unwrap(), f.sha256);
    assert_eq!(
        s.requests()[0].header("range"),
        None,
        "no range without a validator"
    );
}

#[tokio::test]
async fn probe_reports_capabilities() {
    let f = Fixture::random(NAME, 1_000, 14);
    let s = start(&f).await;
    let c = client();
    let ranged = probe(
        &c,
        &DownloadRequest::new(Url::parse(&s.url("/file/data.bin")).unwrap()),
    )
    .await
    .unwrap();
    assert!(ranged.accepts_ranges && ranged.resumable());
    assert_eq!(ranged.total_size, Some(1_000));
    assert!(ranged.body.is_none());
    assert_eq!(ranged.validator(), ranged.etag.as_deref());
    assert_eq!(
        ranged.resume_info(),
        HttpResumeInfo {
            final_url: Some(ranged.final_url.clone()),
            etag: ranged.etag.clone(),
            last_modified: ranged.last_modified.clone(),
        }
    );

    let plain = probe(
        &c,
        &DownloadRequest::new(Url::parse(&s.url("/norange/data.bin")).unwrap()),
    )
    .await
    .unwrap();
    assert!(!plain.accepts_ranges && !plain.resumable());
    assert_eq!(plain.total_size, Some(1_000));
    assert!(plain.body.is_some());
}

#[tokio::test]
async fn checksums_are_verified_before_the_rename() {
    use md5::Digest as _;
    use pv_core::{Checksum, ChecksumAlgorithm};
    let f = Fixture::random(NAME, 120_000, 30);
    let s = start(&f).await;
    let md5_hex: String = md5::Md5::digest(&f.bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .concat();
    let source = Source::http(Url::parse(&s.url("/file/data.bin")).unwrap()).unwrap();

    for good in [
        Checksum::new(ChecksumAlgorithm::Sha256, &f.sha256).unwrap(),
        Checksum::new(ChecksumAlgorithm::Md5, &md5_hex).unwrap(),
    ] {
        let h = Harness::new();
        let mut ctx = h.ctx(source.clone(), ResumeState::default(), None, None);
        ctx.task.spec.checksum = Some(good);
        let path = completed_path(HttpProtocol.run(ctx).await.unwrap());
        assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
        assert!(
            h.events
                .phases
                .lock()
                .unwrap()
                .contains(&RunPhase::Verifying)
        );
    }

    let h = Harness::new();
    let mut ctx = h.ctx(source, ResumeState::default(), None, None);
    ctx.task.spec.checksum =
        Some(Checksum::new(ChecksumAlgorithm::Sha256, &"ab".repeat(32)).unwrap());
    let err = HttpProtocol.run(ctx).await.unwrap_err();
    assert!(
        matches!(err, ProtocolError::ChecksumMismatch { ref actual, .. } if *actual == f.sha256),
        "{err}"
    );
    assert!(
        !h.dest.join(NAME).exists(),
        "a wrong file never gets its final name"
    );
    assert_eq!(
        part_files(&h.dest),
        Vec::<String>::new(),
        "the bad .part is deleted so a retry starts clean"
    );
}
