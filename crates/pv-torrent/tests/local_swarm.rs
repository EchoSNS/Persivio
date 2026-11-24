//! Phase 6 acceptance: a second librqbit session seeds on 127.0.0.1 (DHT off) and the
//! Persivio adapter downloads from it. Every file is checked by SHA-256.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use pv_core::{
    CheckpointError, CheckpointSink, Outcome, ProgressSink, Protocol, ProtocolError,
    ResolvedMetadata, ResumeState, RunContext, RunEvents, RunPhase, Segment, Settings, Source,
    TaskId, TaskSnapshot, TaskSpec, TorrentResume, TorrentSettings, Unlimited,
};
use pv_testkit::{Fixture, sha256_file};
use pv_torrent::TorrentProtocol;
use pv_torrent::testing::LocalSeeder;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Sink(Mutex<Option<TorrentResume>>, Mutex<u64>);

#[async_trait]
impl CheckpointSink for Sink {
    async fn save_segments(&self, _: TaskId, _: &[Segment]) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn save_progress(&self, _: TaskId, _: &[(u32, u64)]) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn save_torrent(
        &self,
        _: TaskId,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(), CheckpointError> {
        *self.0.lock().unwrap() = Some(TorrentResume {
            info_hash: hash.to_owned(),
            torrent_bytes: bytes.to_vec(),
            have: None,
        });
        Ok(())
    }
    async fn save_torrent_progress(
        &self,
        _: TaskId,
        downloaded: u64,
        have: Option<&[u8]>,
    ) -> Result<(), CheckpointError> {
        self.0.lock().unwrap().as_mut().unwrap().have = have.map(<[u8]>::to_vec);
        *self.1.lock().unwrap() = downloaded;
        Ok(())
    }
}

#[derive(Default)]
struct Events {
    phases: Mutex<Vec<RunPhase>>,
    meta: Mutex<Vec<ResolvedMetadata>>,
}

impl RunEvents for Events {
    fn phase(&self, p: RunPhase) {
        self.phases.lock().unwrap().push(p);
    }
    fn metadata(&self, m: ResolvedMetadata) {
        self.meta.lock().unwrap().push(m);
    }
    fn warning(&self, _: String) {}
}

async fn seed(path: &Path, name: &str) -> LocalSeeder {
    LocalSeeder::start(path, name).await
}

fn settings(never_seed: bool) -> Settings {
    Settings {
        torrent: TorrentSettings {
            dht: false,
            upnp: false,
            never_seed,
            ..TorrentSettings::default()
        },
        ..Settings::default()
    }
}

struct Run {
    ctx: RunContext,
    sink: Arc<Sink>,
    events: Arc<Events>,
}

fn run_ctx(source: Source, dest: &Path, settings: Settings) -> Run {
    let sink = Arc::new(Sink::default());
    let events = Arc::new(Events::default());
    let ctx = RunContext {
        task: TaskSnapshot::new(TaskSpec::new(source, dest)),
        resume: ResumeState::default(),
        settings: Arc::new(settings),
        cancel: CancellationToken::new(),
        progress: ProgressSink::new(),
        limiter: Arc::new(Unlimited),
        checkpoint: sink.clone(),
        events: events.clone(),
    };
    Run { ctx, sink, events }
}

async fn run(protocol: &TorrentProtocol, ctx: RunContext) -> Result<Outcome, ProtocolError> {
    tokio::time::timeout(Duration::from_secs(60), protocol.run(ctx))
        .await
        .expect("torrent run timed out")
}

fn completed(o: Outcome) -> PathBuf {
    match o {
        Outcome::Completed { path } => path,
        Outcome::Stopped => panic!("expected completion"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_file_single_file_downloads_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("single.bin", 700_000, 1);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/single.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "single.bin").await;

    let protocol = TorrentProtocol::new().with_initial_peers(vec![seeder.addr]);
    let dest = dir.path().join("dl");
    let r = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dest,
        settings(true),
    );
    let path = completed(run(&protocol, r.ctx).await.unwrap());
    assert_eq!(path, dest.join("single.bin"));
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    let phases = r.events.phases.lock().unwrap().clone();
    assert_eq!(
        phases,
        [RunPhase::Downloading],
        "no metadata phase for .torrent files"
    );
    let meta = r.events.meta.lock().unwrap()[0].clone();
    assert_eq!(
        (meta.name.as_deref(), meta.total_size, meta.files.len()),
        (Some("single.bin"), Some(700_000), 1)
    );
    assert!(
        r.sink.0.lock().unwrap().is_some(),
        "metadata persisted for resume"
    );
    protocol.stop().await;
}

/// Sizes are multiples of the 16 KiB piece length, so no piece spans two files: in
/// BitTorrent a piece shared with a selected file is downloaded whole, which would
/// otherwise complete small unselected neighbours (seen on macOS, whose file system
/// orders the files differently).
fn make_pack(root: &Path) -> Vec<Fixture> {
    let files = vec![
        Fixture::random("a.bin", 18 * 16_384, 2),
        Fixture::random("b.bin", 28 * 16_384, 3),
        Fixture::random("c.txt", 2 * 16_384, 4),
    ];
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.bin"), &files[0].bytes).unwrap();
    std::fs::write(root.join("sub/b.bin"), &files[1].bytes).unwrap();
    std::fs::write(root.join("c.txt"), &files[2].bytes).unwrap();
    files
}

#[tokio::test(flavor = "multi_thread")]
async fn magnet_multi_file_fetches_metadata_then_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("seed/pack");
    let files = make_pack(&pack);
    let seeder = seed(&pack, "pack").await;

    let protocol = TorrentProtocol::new().with_initial_peers(vec![seeder.addr]);
    let dest = dir.path().join("dl");
    let r = run_ctx(
        Source::magnet(seeder.magnet.clone()).unwrap(),
        &dest,
        settings(true),
    );
    let root = completed(run(&protocol, r.ctx).await.unwrap());
    assert_eq!(
        root,
        dest.join("pack"),
        "multi-file torrents get their own folder"
    );
    assert_eq!(
        sha256_file(root.join("a.bin")).await.unwrap(),
        files[0].sha256
    );
    assert_eq!(
        sha256_file(root.join("sub/b.bin")).await.unwrap(),
        files[1].sha256
    );
    assert_eq!(
        sha256_file(root.join("c.txt")).await.unwrap(),
        files[2].sha256
    );
    assert_marked(&root.join("sub/b.bin"), "about:internet");
    let phases = r.events.phases.lock().unwrap().clone();
    assert_eq!(phases, [RunPhase::FetchingMetadata, RunPhase::Downloading]);
    let meta = r.events.meta.lock().unwrap()[0].clone();
    let mut paths: Vec<String> = meta.files.iter().map(|f| f.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, ["a.bin", "c.txt", "sub/b.bin"]);
    protocol.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_selected_files_are_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("seed/pack");
    let files = make_pack(&pack);
    let seeder = seed(&pack, "pack").await;

    let protocol = TorrentProtocol::new().with_initial_peers(vec![seeder.addr]);
    let dest = dir.path().join("dl");
    let mut r = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dest,
        settings(true),
    );
    // Pick b.bin by its index in the torrent's own order.
    let probe = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dir.path().join("probe"),
        settings(true),
    );
    let _ = run(&protocol, probe.ctx).await.unwrap();
    let order = probe.events.meta.lock().unwrap()[0].files.clone();
    let b = order.iter().find(|f| f.path == "sub/b.bin").unwrap().index;

    r.ctx.task.spec.selected_files = Some(vec![b]);
    let root = completed(run(&protocol, r.ctx).await.unwrap());
    assert_eq!(
        sha256_file(root.join("sub/b.bin")).await.unwrap(),
        files[1].sha256
    );
    let meta = r.events.meta.lock().unwrap()[0].clone();
    assert_eq!(
        meta.total_size,
        Some(28 * 16_384),
        "total counts only selected files"
    );
    assert!(meta.files.iter().all(|f| f.selected == (f.index == b)));
    let others_complete = ["a.bin", "c.txt"].iter().any(|n| {
        std::fs::read(root.join(n)).is_ok_and(|d| {
            pv_testkit::sha256_hex(&d) == files[if *n == "a.bin" { 0 } else { 2 }].sha256
        })
    });
    assert!(!others_complete, "an unselected file was downloaded");
    protocol.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn seeding_phase_then_completion_when_the_limit_is_met() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("s.bin", 200_000, 5);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/s.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "s.bin").await;
    let protocol = TorrentProtocol::new().with_initial_peers(vec![seeder.addr]);
    let mut s = settings(false);
    s.torrent.seed_ratio_limit_percent = Some(0); // limit met as soon as seeding starts
    let r = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dir.path().join("dl"),
        s,
    );
    let path = completed(run(&protocol, r.ctx).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    assert_eq!(
        *r.events.phases.lock().unwrap(),
        [RunPhase::Downloading, RunPhase::Seeding]
    );
    protocol.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_keep_torrent_progress() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("big.bin", 2_000_000, 6);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/big.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "big.bin").await;
    let protocol = Arc::new(TorrentProtocol::new().with_initial_peers(vec![seeder.addr]));
    let dest = dir.path().join("dl");

    let mut first = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dest,
        settings(true),
    );
    first.ctx.task.spec.speed_limit_bps = Some(800_000);
    let progress = first.ctx.progress.clone();
    let cancel = first.ctx.cancel.clone();
    let p = protocol.clone();
    let handle = tokio::spawn(async move { p.run(first.ctx).await });
    while progress.sample().downloaded < 600_000 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cancel.cancel();
    assert_eq!(handle.await.unwrap().unwrap(), Outcome::Stopped);
    let paused_at = progress.sample().downloaded;
    let saved = first.sink.0.lock().unwrap().clone().unwrap();
    let name = first.events.meta.lock().unwrap()[0].name.clone();

    // "Restart": only the persisted torrent metadata and name survive.
    let mut second = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dest,
        settings(true),
    );
    second.ctx.resume.torrent = Some(saved);
    second.ctx.task.spec.file_name = name;
    let progress = second.ctx.progress.clone();
    let p = protocol.clone();
    let handle = tokio::spawn(async move { p.run(second.ctx).await });
    let mut first_seen = 0;
    while first_seen == 0 {
        first_seen = progress.sample().downloaded;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let path = completed(handle.await.unwrap().unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    assert!(
        first_seen >= paused_at / 2,
        "resume re-used existing pieces: first sample {first_seen}, paused at {paused_at}"
    );
    protocol.stop().await;
}

/// A hand-crafted malicious torrent whose file path climbs out of the folder.
#[tokio::test(flavor = "multi_thread")]
async fn path_traversal_torrents_are_rejected_before_writing() {
    let mut t = b"d4:infod5:filesld6:lengthi5e4:pathl2:..8:evil.txteeed6:lengthi5e4:pathl6:ok.txteee4:name4:pack12:piece lengthi16384e6:pieces20:".to_vec();
    t.extend_from_slice(&[0u8; 20]);
    t.extend_from_slice(b"ee");
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("dl");
    let protocol = TorrentProtocol::new();
    let r = run_ctx(Source::TorrentFile { bytes: t }, &dest, settings(true));
    let err = run(&protocol, r.ctx).await.unwrap_err();
    println!("rejected with: {err}");
    assert!(
        matches!(
            err,
            ProtocolError::InvalidSource(_) | ProtocolError::Torrent(_)
        ),
        "{err}"
    );
    assert!(!dir.path().join("evil.txt").exists());
    assert!(!dest.join("evil.txt").exists());
    assert!(
        r.events.meta.lock().unwrap().is_empty(),
        "rejected before metadata was published"
    );
    protocol.stop().await;
}

/// A name librqbit accepts but Persivio refuses: a right-to-left override that makes
/// `evil<RLO>txt.exe` display as `evilexe.txt`.
#[tokio::test(flavor = "multi_thread")]
async fn deceptive_names_are_rejected_by_persivio_validation() {
    let name = "evil\u{202E}txt.exe".as_bytes().to_vec();
    let mut t = b"d4:infod6:lengthi5e4:name".to_vec();
    t.extend_from_slice(format!("{}:", name.len()).as_bytes());
    t.extend_from_slice(&name);
    t.extend_from_slice(b"12:piece lengthi16384e6:pieces20:");
    t.extend_from_slice(&[0u8; 20]);
    t.extend_from_slice(b"ee");
    let dir = tempfile::tempdir().unwrap();
    let protocol = TorrentProtocol::new();
    let r = run_ctx(
        Source::TorrentFile { bytes: t },
        &dir.path().join("dl"),
        settings(true),
    );
    let err = run(&protocol, r.ctx).await.unwrap_err();
    assert!(
        matches!(&err, ProtocolError::InvalidSource(m) if m.contains("unsafe path")),
        "{err}"
    );
    assert!(r.events.meta.lock().unwrap().is_empty());
    protocol.stop().await;
}

/// On Windows, finished files carry the Mark-of-the-Web (ADR-017).
fn assert_marked(path: &Path, host_url: &str) {
    if cfg!(windows) {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let text = std::fs::read_to_string(&stream).unwrap();
        assert!(text.contains("ZoneId=3"), "{text}");
        assert!(text.contains(&format!("HostUrl={host_url}")), "{text}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn torrent_files_can_be_inspected_without_downloading() {
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("seed/pack");
    let files = make_pack(&pack);
    let seeder = seed(&pack, "pack").await;
    let info = pv_torrent::inspect_torrent(&seeder.torrent).unwrap();
    assert_eq!(info.name, "pack");
    assert!(seeder.magnet.contains(&info.info_hash), "{}", seeder.magnet);
    let mut listed: Vec<(String, u64)> = info
        .files
        .iter()
        .map(|f| (f.path.clone(), f.size))
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        [
            ("a.bin".to_owned(), files[0].len() as u64),
            ("c.txt".to_owned(), files[2].len() as u64),
            ("sub/b.bin".to_owned(), files[1].len() as u64),
        ]
    );
    assert!(info.files.iter().all(|f| f.selected));
    assert!(pv_torrent::inspect_torrent(b"not a torrent").is_err());
    assert!(
        pv_torrent::inspect_torrent(&vec![b'd'; pv_torrent::MAX_TORRENT_FILE_SIZE + 1]).is_err()
    );
}

/// Regression: while librqbit re-checks existing data, its counter means "bytes hashed".
/// Showing that as downloaded looked like a very fast download that then started over.
#[tokio::test(flavor = "multi_thread")]
async fn checking_existing_data_is_not_reported_as_downloading() {
    let dir = tempfile::tempdir().unwrap();
    // Large enough that the check lasts several progress polls.
    let f = Fixture::random("check.bin", 96 * 1024 * 1024, 7);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/check.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "check.bin").await;
    let dest = dir.path().join("dl");
    std::fs::create_dir_all(&dest).unwrap();
    // A wrong first half: hashing reads all of it, but only the second half counts.
    let mut half = vec![0u8; f.bytes.len()];
    let mid = f.bytes.len() / 2;
    half[mid..].copy_from_slice(&f.bytes[mid..]);
    std::fs::write(dest.join("check.bin"), &half).unwrap();

    let protocol = Arc::new(TorrentProtocol::new().with_initial_peers(vec![seeder.addr]));
    let mut r = run_ctx(
        Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        },
        &dest,
        settings(true),
    );
    r.ctx.resume.torrent = Some(TorrentResume {
        info_hash: String::new(),
        torrent_bytes: seeder.torrent.clone(),
        have: None,
    });
    r.ctx.task.spec.file_name = Some("check.bin".into());
    r.ctx.task.spec.speed_limit_bps = Some(4_000_000);
    let progress = r.ctx.progress.clone();
    let cancel = r.ctx.cancel.clone();
    let events = r.events.clone();
    let p = protocol.clone();
    let handle = tokio::spawn(async move { p.run(r.ctx).await });
    let mut checked = 0;
    loop {
        let phases = events.phases.lock().unwrap().clone();
        if phases.contains(&RunPhase::Downloading) {
            break;
        }
        let s = progress.sample();
        assert_eq!(s.downloaded, 0, "nothing is downloaded while checking");
        checked = checked.max(s.check_percent.unwrap_or(0));
        assert!(!handle.is_finished(), "ended early");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(checked > 0, "check progress is reported separately");
    let s = progress.sample();
    assert_eq!(s.check_percent, None);
    assert!(
        s.downloaded >= mid as u64 - 16_384,
        "the valid half is kept: {}",
        s.downloaded
    );
    cancel.cancel();
    assert_eq!(handle.await.unwrap().unwrap(), Outcome::Stopped);
    assert_eq!(
        *events.phases.lock().unwrap(),
        [RunPhase::Checking, RunPhase::Downloading]
    );
    protocol.stop().await;
}

/// Runs `r` until `bytes` are downloaded, then pauses it. Returns what was saved.
async fn run_until(protocol: &Arc<TorrentProtocol>, r: Run, bytes: u64) -> (TorrentResume, u64) {
    let progress = r.ctx.progress.clone();
    let cancel = r.ctx.cancel.clone();
    let p = protocol.clone();
    let handle = tokio::spawn(async move { p.run(r.ctx).await });
    while progress.sample().downloaded < bytes {
        assert!(!handle.is_finished(), "ended early");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cancel.cancel();
    assert_eq!(handle.await.unwrap().unwrap(), Outcome::Stopped);
    let saved = r.sink.0.lock().unwrap().clone().unwrap();
    let downloaded = *r.sink.1.lock().unwrap();
    (saved, downloaded)
}

/// Runs a resume until it starts downloading; returns how much it found on disk.
async fn found_on_resume(protocol: &Arc<TorrentProtocol>, r: Run) -> (u64, Vec<RunPhase>) {
    let (events, progress) = (r.events.clone(), r.ctx.progress.clone());
    let (cancel, p) = (r.ctx.cancel.clone(), protocol.clone());
    let handle = tokio::spawn(async move { p.run(r.ctx).await });
    while !events
        .phases
        .lock()
        .unwrap()
        .contains(&RunPhase::Downloading)
    {
        assert!(!handle.is_finished(), "ended early");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let found = progress.sample().downloaded;
    cancel.cancel();
    handle.await.unwrap().unwrap();
    let phases = events.phases.lock().unwrap().clone();
    (found, phases)
}

/// After a clean pause, even across a restart, a resume trusts the saved piece map instead
/// of reading everything again. Proven with a map edited to claim less than is on disk:
/// a full check would find all of it. Once the file changed, the map is ignored.
#[tokio::test(flavor = "multi_thread")]
async fn clean_resumes_skip_the_full_check_until_the_files_change() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("fast.bin", 8_000_000, 8);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/fast.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "fast.bin").await;
    let dest = dir.path().join("dl");
    let new_protocol = || {
        Arc::new(
            TorrentProtocol::with_work_dir(dir.path().join("work"))
                .with_initial_peers(vec![seeder.addr]),
        )
    };
    let new_run = |resume: Option<TorrentResume>| {
        let source = Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        };
        let mut r = run_ctx(source, &dest, settings(true));
        r.ctx.task.spec.speed_limit_bps = Some(1_500_000);
        if resume.is_some() {
            r.ctx.task.spec.file_name = Some("fast.bin".into());
            // Also the burst size: little new data hides what was found on disk.
            r.ctx.task.spec.speed_limit_bps = Some(100_000);
        }
        r.ctx.resume.torrent = resume;
        r
    };
    let protocol = new_protocol();
    let (saved, downloaded) = run_until(&protocol, new_run(None), 2_000_000).await;
    assert!(downloaded >= 2_000_000, "{downloaded}");
    protocol.stop().await;

    let mut map: serde_json::Value =
        serde_json::from_slice(saved.have.as_deref().expect("a clean pause saves the map"))
            .unwrap();
    for (i, byte) in map["bits"].as_array_mut().unwrap().iter_mut().enumerate() {
        if i % 2 == 1 {
            *byte = 0.into();
        }
    }
    let mut less = saved.clone();
    less.have = Some(serde_json::to_vec(&map).unwrap());
    let protocol = new_protocol();
    let (found, phases) = found_on_resume(&protocol, new_run(Some(less))).await;
    assert!(
        found > 0 && found < downloaded * 3 / 4,
        "the map was used: found {found} of {downloaded}"
    );
    assert_eq!(phases, [RunPhase::Checking, RunPhase::Downloading]);

    // That run wrote to the file, so the first map no longer matches: full check.
    let (found, _) = found_on_resume(&protocol, new_run(Some(saved))).await;
    assert!(found >= downloaded, "found {found} of {downloaded}");

    // And everything still adds up to the right file.
    let mut last = new_run(Some(TorrentResume {
        info_hash: String::new(),
        torrent_bytes: seeder.torrent.clone(),
        have: None,
    }));
    last.ctx.task.spec.speed_limit_bps = None;
    let path = completed(run(&protocol, last.ctx).await.unwrap());
    assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
    protocol.stop().await;
}

/// With no torrent running, Persivio must leave the BitTorrent network.
#[tokio::test(flavor = "multi_thread")]
async fn the_session_closes_when_idle_and_reopens_when_needed() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("idle.bin", 200_000, 9);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/idle.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "idle.bin").await;
    let protocol =
        TorrentProtocol::with_options(Some(dir.path().join("work")), Duration::from_millis(200))
            .with_initial_peers(vec![seeder.addr]);
    for dest in ["one", "two"] {
        assert!(!protocol.is_online().await);
        let source = Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        };
        let r = run_ctx(source, &dir.path().join(dest), settings(true));
        let path = completed(run(&protocol, r.ctx).await.unwrap());
        assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
        assert!(protocol.is_online().await, "a quick resume reuses it");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while protocol.is_online().await {
            assert!(std::time::Instant::now() < deadline, "never closed");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Two tasks for one torrent would write to the same files.
#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_that_is_already_running_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::random("twice.bin", 4_000_000, 10);
    std::fs::create_dir_all(dir.path().join("seed")).unwrap();
    let src = dir.path().join("seed/twice.bin");
    std::fs::write(&src, &f.bytes).unwrap();
    let seeder = seed(&src, "twice.bin").await;
    let protocol = Arc::new(
        TorrentProtocol::with_work_dir(dir.path().join("work"))
            .with_initial_peers(vec![seeder.addr]),
    );
    let new_run = |dest: &str| {
        let source = Source::TorrentFile {
            bytes: seeder.torrent.clone(),
        };
        let mut r = run_ctx(source, &dir.path().join(dest), settings(true));
        r.ctx.task.spec.speed_limit_bps = Some(500_000);
        r
    };
    let first = new_run("one");
    let (cancel, progress, p) = (
        first.ctx.cancel.clone(),
        first.ctx.progress.clone(),
        protocol.clone(),
    );
    let handle = tokio::spawn(async move { p.run(first.ctx).await });
    while progress.sample().downloaded == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let err = run(&protocol, new_run("two").ctx).await.unwrap_err();
    assert!(
        err.to_string().contains("already being downloaded"),
        "{err}"
    );
    assert!(!handle.is_finished(), "the first one is not affected");
    cancel.cancel();
    assert_eq!(handle.await.unwrap().unwrap(), Outcome::Stopped);
    protocol.stop().await;
}
