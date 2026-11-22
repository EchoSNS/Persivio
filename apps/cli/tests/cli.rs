//! End-to-end: the real `persivio` binary, as a separate process, against a local server.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::path::{Path, PathBuf};
use std::process::Output;

use pv_testkit::{Fixture, TestServer, sha256_file};

async fn persivio(data: &Path, args: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_persivio"));
    cmd.arg("--data-dir").arg(data).args(args);
    // The binary runs in its own process; wait for it off the async runtime.
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn add_watch_list_settings_and_remove() {
    let f = Fixture::random("cli.bin", 400_000, 1);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let downloads: PathBuf = dir.path().join("downloads");

    let out = persivio(&data, &["list"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("No downloads."));

    let out = persivio(&data, &["settings", "set", "max_concurrent_tasks", "4"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    let shown = stdout(&persivio(&data, &["settings", "show"]).await);
    assert!(shown.contains("\"max_concurrent_tasks\": 4"), "{shown}");
    let bad = persivio(&data, &["settings", "set", "max_concurrent_tasks", "0"]).await;
    assert!(!bad.status.success(), "range-checked by the engine");

    let url = server.url("/file/cli.bin");
    let out = persivio(
        &data,
        &[
            "add",
            &url,
            "--dir",
            downloads.to_str().unwrap(),
            "--no-wait",
        ],
    )
    .await;
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("added "));
    assert!(
        stdout(&persivio(&data, &["list"]).await).contains("queued"),
        "--no-wait only queues"
    );

    let out = persivio(&data, &["watch"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    let listed = stdout(&persivio(&data, &["list"]).await);
    assert!(
        listed.contains("completed") && listed.contains("cli.bin"),
        "{listed}"
    );
    assert_eq!(
        sha256_file(downloads.join("cli.bin")).await.unwrap(),
        f.sha256
    );

    let id = listed
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let out = persivio(&data, &["rm", &id, "--delete-files"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!downloads.join("cli.bin").exists());
    assert!(stdout(&persivio(&data, &["list"]).await).contains("No downloads."));

    let out = persivio(&data, &["pause", "zzzzzz"]).await;
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no download matches"),
        "{}",
        stderr(&out)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_instance_on_the_same_data_dir_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    let lock = std::fs::File::create(dir.path().join("persivio.lock")).unwrap();
    lock.lock().unwrap(); // simulate a running instance
    let out = persivio(dir.path(), &["list"]).await;
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("another Persivio instance"),
        "{}",
        stderr(&out)
    );
    lock.unlock().unwrap();
    assert!(persivio(dir.path(), &["list"]).await.status.success());
}

/// Phase 5 acceptance: several concurrent downloads, the process is killed hard
/// (no graceful shutdown), and a restart resumes everything to exact files.
#[tokio::test(flavor = "multi_thread")]
async fn hard_kill_mid_download_then_restart_resumes_everything() {
    let fixtures: Vec<Fixture> = (0..3)
        .map(|i| Fixture::random(format!("k{i}.bin"), 900_000, 100 + i))
        .collect();
    let server = TestServer::start(fixtures.clone()).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let downloads = dir.path().join("downloads");
    let settings = persivio(&data, &["settings", "set", "min_segment_size", "65536"]).await;
    assert!(settings.status.success(), "{}", stderr(&settings));

    let urls: Vec<String> = (0..3)
        .map(|i| server.url(&format!("/throttle/k{i}.bin?bps=200000")))
        .collect();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_persivio"));
    cmd.arg("--data-dir").arg(&data).arg("add");
    cmd.args(&urls)
        .arg("--dir")
        .arg(&downloads)
        .args(["--connections", "2"]);
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    child.kill().unwrap(); // TerminateProcess / SIGKILL: no cleanup runs
    child.wait().unwrap();

    let listed = stdout(&persivio(&data, &["list"]).await);
    assert_eq!(
        listed.matches("paused").count(),
        3,
        "all three recovered as paused:\n{listed}"
    );

    let out = persivio(&data, &["resume", "all"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    for f in &fixtures {
        assert_eq!(
            sha256_file(downloads.join(&f.name)).await.unwrap(),
            f.sha256,
            "{}",
            f.name
        );
    }
    let resumed_from_zero = server
        .requests()
        .iter()
        .filter(|r| {
            r.header("range")
                .is_some_and(|h| h.starts_with("bytes=0-") && h != "bytes=0-0")
        })
        .count();
    assert_eq!(
        resumed_from_zero, 3,
        "only the first run starts segments at 0, not the resume"
    );
}
