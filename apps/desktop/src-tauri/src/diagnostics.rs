//! A diagnostics report the user can read, save and attach to a bug report (ADR-026).
//!
//! It must be safe to share: no file names, links, peers, tokens or user names. Settings
//! are copied field by field (a new secret setting cannot slip in by accident), tasks are
//! only counted, and log text goes through [`redact`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use pv_core::{Settings, TaskKind, TaskSnapshot};
use regex::{Captures, Regex};

/// Most log text included, from the end.
const MAX_LOG: usize = 512 * 1024;

#[allow(
    clippy::expect_used,
    reason = "constant patterns, compiled in a unit test"
)]
fn pattern(p: &str) -> Regex {
    Regex::new(p).expect("valid pattern")
}

static URL: LazyLock<Regex> =
    LazyLock::new(|| pattern(r#"(?i)\b(https?|ftp|udp|wss?)://[^\s"'<>)\]]+|\bmagnet:[^\s"'<>]+"#));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"[\w.+-]+@[\w-]+(\.[\w-]+)+"));
static HEX: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b[0-9a-fA-F]{32,}\b"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b(\d{1,3}\.){3}\d{1,3}(:\d+)?\b"));
static IPV6: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\[?\b([0-9a-f]{0,4}:){3,7}[0-9a-f]{0,4}\b\]?(:\d+)?"));
/// Windows and Unix absolute paths, up to a character that cannot be part of one.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?i)(\b[a-z]:[\\/]|\\\\|/(home|users|mnt|media|tmp|var|root|private|volumes)/)[^"'\n<>|*?]*"#,
    )
});
/// Quoted text (names of files and torrents), and `name=value` pairs of librqbit's logs.
static QUOTED: LazyLock<Regex> = LazyLock::new(|| pattern(r#""[^"\n]*"|`[^`\n]*`"#));

/// Whether an address is this computer itself, which tells nothing about anyone.
fn is_loopback(address: &str) -> bool {
    address.starts_with("127.") || address.starts_with("0.0.0.0")
}

/// Removes everything personal from free text.
#[must_use]
pub fn redact(text: &str) -> String {
    let text = URL.replace_all(text, |c: &Captures<'_>| match c.get(1) {
        Some(scheme) => format!("{}://<link>", scheme.as_str()),
        None => "<magnet>".to_owned(),
    });
    let text = QUOTED.replace_all(&text, "<text>");
    let text = PATH.replace_all(&text, "<path>");
    let text = EMAIL.replace_all(&text, "<email>");
    let text = HEX.replace_all(&text, "<hash>");
    let text = IPV4.replace_all(&text, |c: &Captures<'_>| {
        if is_loopback(&c[0]) {
            c[0].to_owned()
        } else {
            "<address>".to_owned()
        }
    });
    IPV6.replace_all(&text, "<address>").into_owned()
}

/// What the report is made of.
pub struct Input<'a> {
    /// Persivio's version.
    pub version: &'a str,
    /// Settings (only harmless fields are used).
    pub settings: &'a Settings,
    /// Tasks (only counted).
    pub tasks: &'a [TaskSnapshot],
    /// Why the engine is not running, if it is not.
    pub engine_error: Option<&'a str>,
    /// Log text, oldest first.
    pub log: &'a str,
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn or_none(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "none".to_owned(), |v| v.to_string())
}

fn settings_text(s: &Settings) -> String {
    let (t, d) = (&s.torrent, &s.desktop);
    let rows: [(&str, String); 24] = [
        ("max concurrent tasks", s.max_concurrent_tasks.to_string()),
        ("default connections", s.default_connections.to_string()),
        ("min segment size", s.min_segment_size.to_string()),
        ("global speed limit", or_none(s.global_speed_limit_bps)),
        ("max retries", s.max_retries.to_string()),
        ("idle timeout (s)", s.idle_timeout_secs.to_string()),
        ("connect timeout (s)", s.connect_timeout_secs.to_string()),
        ("auto resume", on_off(s.auto_resume_on_startup).into()),
        ("custom folder", on_off(s.download_dir.is_some()).into()),
        ("categories", s.categories.len().to_string()),
        (
            "torrent: listen port set",
            on_off(t.listen_port.is_some()).into(),
        ),
        ("torrent: DHT", on_off(t.dht).into()),
        ("torrent: UPnP", on_off(t.upnp).into()),
        ("torrent: never seed", on_off(t.never_seed).into()),
        (
            "torrent: seed ratio (%)",
            or_none(t.seed_ratio_limit_percent),
        ),
        ("torrent: seed time (min)", or_none(t.seed_time_limit_mins)),
        // Trackers of private sites contain a personal key: only their number.
        (
            "torrent: extra trackers",
            t.extra_trackers.len().to_string(),
        ),
        ("close to tray", on_off(d.close_to_tray).into()),
        ("prevent sleep", on_off(d.prevent_sleep).into()),
        ("watch clipboard", on_off(d.watch_clipboard).into()),
        ("autostart", on_off(d.autostart).into()),
        ("magnet links", on_off(d.handle_magnet_links).into()),
        ("browser bridge", on_off(d.browser_bridge).into()),
        ("bridge port", d.bridge_port.to_string()),
    ];
    let mut out = String::new();
    for (name, value) in rows {
        let _ = writeln!(out, "{name}: {value}");
    }
    out
}

/// The report as plain text.
#[must_use]
pub fn report(input: &Input<'_>) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for task in input.tasks {
        let kind = match task.kind() {
            TaskKind::Http => "http",
            TaskKind::Torrent => "torrent",
        };
        *counts
            .entry(format!("{kind}, {}", task.state.kind()))
            .or_default() += 1;
    }
    let mut out = String::from(
        "Persivio diagnostics\n\
         Links, file names, addresses, paths and tokens are removed. Please read it before\n\
         sharing.\n\n",
    );
    let _ = writeln!(out, "version: {}", input.version);
    let _ = writeln!(
        out,
        "system: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some(error) = input.engine_error {
        let _ = writeln!(out, "engine: not running: {}", redact(error));
    }
    out.push_str("\n== Downloads ==\n");
    if counts.is_empty() {
        out.push_str("none\n");
    }
    for (what, n) in counts {
        let _ = writeln!(out, "{what}: {n}");
    }
    out.push_str("\n== Settings ==\n");
    out.push_str(&settings_text(input.settings));
    out.push_str("\n== Log ==\n");
    out.push_str(&redact(input.log));
    out
}

/// Reads the newest [`MAX_LOG`] bytes of the log files in `dir`, oldest first.
#[must_use]
pub fn read_logs(dir: &Path) -> String {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "log"))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .collect();
    files.sort();
    let mut text = String::new();
    for (_, path) in files {
        text.push_str(&String::from_utf8_lossy(
            &std::fs::read(path).unwrap_or_default(),
        ));
    }
    if text.len() > MAX_LOG {
        let mut start = text.len() - MAX_LOG;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        // Begin with a whole line.
        start += text[start..].find('\n').map_or(0, |i| i + 1);
        text.drain(..start);
    }
    text
}

#[cfg(test)]
mod tests {
    use pv_core::{DesktopSettings, Source, TaskSpec, TorrentSettings};

    use super::*;

    #[test]
    fn personal_data_is_removed_from_text() {
        for (text, expected) in [
            (
                "GET https://me:pw@example.com/a.zip?token=1 failed",
                "GET https://<link> failed",
            ),
            ("magnet:?xt=urn:btih:abc&dn=My+File end", "<magnet> end"),
            (
                "cannot open C:\\Users\\yvonne\\My Files\\a.zip",
                "cannot open <path>",
            ),
            ("cannot open /home/yvonne/a.zip", "cannot open <path>"),
            ("addr=81.96.116.148:26763 closed", "addr=<address> closed"),
            (
                "peer [2001:db8::1:2:3]:51413 closed",
                "peer <address> closed",
            ),
            (
                "info_hash=f2772646161a6bba31d352f1a3f7394e6428be83",
                "info_hash=<hash>",
            ),
            (
                "name=\"Holiday.mp4\" mail a.b@c.de",
                "name=<text> mail <email>",
            ),
            (
                "listening on 127.0.0.1:47321 since 12:30:05",
                "listening on 127.0.0.1:47321 since 12:30:05",
            ),
            (
                "[2026-11-24][03:50:44][pv_http][WARN] timed out",
                "[2026-11-24][03:50:44][pv_http][WARN] timed out",
            ),
        ] {
            assert_eq!(redact(text), expected);
        }
    }

    #[test]
    fn the_report_contains_no_secrets() {
        const SECRETS: [&str; 8] = [
            "tok3n-secret",
            "passkey-secret",
            "private-name",
            "cookie-secret",
            "secret-host",
            "agent-secret",
            "folder-secret",
            "category-secret",
        ];
        let settings = Settings {
            user_agent: "agent-secret".into(),
            download_dir: Some("/data/folder-secret".into()),
            categories: vec!["category-secret".into()],
            torrent: TorrentSettings {
                extra_trackers: vec!["https://secret-host/passkey-secret/announce".into()],
                ..TorrentSettings::default()
            },
            desktop: DesktopSettings {
                bridge_token: "tok3n-secret".into(),
                ..DesktopSettings::default()
            },
            ..Settings::default()
        };
        let mut spec = TaskSpec::new(
            Source::Http {
                url: "https://secret-host/private-name.zip".parse().unwrap(),
                headers: vec![],
                cookies: Some("cookie-secret".into()),
                referer: None,
            },
            "/data/folder-secret",
        );
        spec.file_name = Some("private-name.zip".into());
        let tasks = [TaskSnapshot::new(spec)];
        let text = report(&Input {
            version: "1.2.3",
            settings: &settings,
            tasks: &tasks,
            engine_error: Some("cannot open C:\\x\\folder-secret"),
            log: "fetching https://secret-host/private-name.zip \"private-name.zip\"",
        });
        for secret in SECRETS {
            assert!(!text.contains(secret), "{secret} leaked:\n{text}");
        }
        for wanted in [
            "version: 1.2.3",
            "http, queued: 1",
            "torrent: DHT: on",
            "== Log ==",
        ] {
            assert!(text.contains(wanted), "{wanted} missing:\n{text}");
        }
    }

    #[test]
    fn logs_are_read_oldest_first_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str, secs: u64| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
                .unwrap();
        };
        write("Persivio.log", "new\n", 200);
        write("Persivio_old.log", "old\n", 100);
        write("other.txt", "no\n", 300);
        assert_eq!(read_logs(dir.path()), "old\nnew\n");
        write("Persivio_old.log", &"é line\n".repeat(200_000), 100);
        let text = read_logs(dir.path());
        assert!(text.len() <= MAX_LOG && text.len() > MAX_LOG - 100);
        assert!(text.starts_with("é line\n") && text.ends_with("new\n"));
        assert_eq!(read_logs(&dir.path().join("missing")), "");
    }
}
