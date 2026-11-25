//! Phase 10 acceptance: the bridge rejects requests without the token (or from web pages,
//! or with a foreign Host), and a download behind a login succeeds with the cookies the
//! extension forwards.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns are not covered by allow-unwrap-in-tests"
)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use persivio_desktop::bridge::{self, BridgeState, DownloadRequest};
use persivio_desktop::offers::{AcceptOffer, MAX_PENDING, Offers};
use pv_core::TaskStateKind;
use pv_engine::{Engine, EngineConfig, EngineHandle};
use pv_testkit::{AUTH_COOKIE, Fixture, TestServer, sha256_file};
use tower::ServiceExt;

const PORT: u16 = 47_321;
const ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

struct Env {
    _dir: tempfile::TempDir,
    downloads: std::path::PathBuf,
    engine: EngineHandle,
    token: String,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(EngineConfig::new(dir.path().join("db.sqlite")))
        .await
        .unwrap();
    Env {
        downloads: dir.path().join("dl"),
        _dir: dir,
        engine,
        token: bridge::new_token().unwrap(),
    }
}

impl Env {
    /// Without the question, as with the setting off.
    fn router(&self) -> Router {
        self.asking(false).0
    }

    fn asking(&self, ask: bool) -> (Router, Arc<Offers>) {
        let offers = Arc::new(Offers::new(|| {}));
        let (_, settings) = tokio::sync::watch::channel(pv_core::DesktopSettings {
            confirm_browser_downloads: ask,
            ..pv_core::DesktopSettings::default()
        });
        let router = bridge::router(BridgeState {
            token: self.token.clone().into(),
            port: PORT,
            engine: Arc::new(self.engine.clone()),
            os_dir: Some(self.downloads.clone()),
            offers: Arc::clone(&offers),
            settings,
        });
        (router, offers)
    }
}

fn request(
    method: &str,
    path: &str,
    host: &str,
    origin: Option<&str>,
    token: Option<&str>,
    body: Option<String>,
) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("host", host);
    if let Some(o) = origin {
        r = r.header("origin", o);
    }
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    if body.is_some() {
        r = r.header("content-type", "application/json");
    }
    r.body(body.map_or_else(Body::empty, Body::from)).unwrap()
}

async fn status(router: &Router, req: Request<Body>) -> StatusCode {
    router.clone().oneshot(req).await.unwrap().status()
}

fn host() -> String {
    format!("127.0.0.1:{PORT}")
}

#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one table of refused requests")]
async fn requests_without_the_token_or_from_web_pages_are_rejected() {
    let env = env().await;
    let r = env.router();
    let token = env.token.as_str();
    let h = host();
    // Correct request works.
    assert_eq!(
        status(
            &r,
            request("GET", "/v1/ping", &h, Some(ORIGIN), Some(token), None)
        )
        .await,
        StatusCode::OK
    );
    // No token, wrong token, token with one character changed.
    assert_eq!(
        status(&r, request("GET", "/v1/ping", &h, Some(ORIGIN), None, None)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(
            &r,
            request("GET", "/v1/ping", &h, Some(ORIGIN), Some("0000"), None)
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let mut almost = token.to_owned();
    almost.replace_range(0..1, if token.starts_with('a') { "b" } else { "a" });
    assert_eq!(
        status(
            &r,
            request("GET", "/v1/ping", &h, Some(ORIGIN), Some(&almost), None)
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // A web page, even with the token (e.g. leaked).
    for origin in ["https://evil.example", "null", ""].map(Some) {
        assert_eq!(
            status(
                &r,
                request("GET", "/v1/ping", &h, origin, Some(token), None)
            )
            .await,
            StatusCode::FORBIDDEN,
            "{origin:?}"
        );
    }
    // What Brave really sends for an extension's GET (recorded on 2026-11-24): no Origin,
    // `Sec-Fetch-Site: none`. The token is still required.
    let without_origin = |site: Option<&str>, token: Option<&str>| {
        let mut req = request("GET", "/v1/ping", &h, None, token, None);
        if let Some(site) = site {
            req.headers_mut()
                .insert("sec-fetch-site", site.parse().unwrap());
        }
        req
    };
    for (site, token, expected) in [
        (Some("none"), Some(token), StatusCode::OK),
        (None, Some(token), StatusCode::OK),
        (Some("none"), None, StatusCode::UNAUTHORIZED),
        (Some("none"), Some("wrong"), StatusCode::UNAUTHORIZED),
        // A page's request without Origin (an image, a navigation) says where it is from.
        (Some("cross-site"), Some(token), StatusCode::FORBIDDEN),
        (Some("same-site"), Some(token), StatusCode::FORBIDDEN),
        (Some("same-origin"), Some(token), StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            status(&r, without_origin(site, token)).await,
            expected,
            "{site:?} {token:?}"
        );
    }
    // A preflight always has an Origin.
    assert_eq!(
        status(
            &r,
            request("OPTIONS", "/v1/downloads", &h, None, None, None)
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // DNS rebinding: a foreign Host name pointing at 127.0.0.1.
    assert_eq!(
        status(
            &r,
            request(
                "GET",
                "/v1/ping",
                "evil.example:47321",
                Some(ORIGIN),
                Some(token),
                None
            )
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // Preflight: allowed for the extension only, without the token.
    let pre = r
        .clone()
        .oneshot(request(
            "OPTIONS",
            "/v1/downloads",
            &h,
            Some(ORIGIN),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(pre.status(), StatusCode::NO_CONTENT);
    assert_eq!(pre.headers()["access-control-allow-origin"], ORIGIN);
    assert_eq!(
        status(
            &r,
            request(
                "OPTIONS",
                "/v1/downloads",
                &h,
                Some("https://evil.example"),
                None,
                None
            )
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(env.engine.list().await.unwrap(), [], "nothing was added");
    env.engine.shutdown().await;
}

#[tokio::test]
async fn invalid_payloads_are_rejected() {
    let env = env().await;
    let r = env.router();
    let post = |body: String| {
        request(
            "POST",
            "/v1/downloads",
            &host(),
            Some(ORIGIN),
            Some(&env.token),
            Some(body),
        )
    };
    for body in [
        "not json".to_owned(),
        r#"{"url": "file:///etc/passwd"}"#.to_owned(),
        r#"{"url": "https://a.example/f", "surprise": 1}"#.to_owned(),
        r#"{"url": "https://a.example/f", "cookies": "a=1\r\nX: y"}"#.to_owned(),
    ] {
        assert_eq!(
            status(&r, post(body.clone())).await,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }
    let huge = format!(
        r#"{{"url": "https://a.example/f", "cookies": "{}"}}"#,
        "x".repeat(70_000)
    );
    assert_eq!(status(&r, post(huge)).await, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(env.engine.list().await.unwrap(), []);
    env.engine.shutdown().await;
}

#[tokio::test]
async fn a_download_behind_a_login_succeeds_with_forwarded_cookies() {
    let f = Fixture::random("private.bin", 300_000, 9);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let env = env().await;
    let r = env.router();
    let send = |cookies: Option<&str>| {
        let body = DownloadRequest {
            url: server.url("/auth/private.bin"),
            referrer: Some(server.url("/account")),
            user_agent: Some("Mozilla/5.0 (Brave test)".into()),
            cookies: cookies.map(str::to_owned),
            filename: Some("private.bin".into()),
            size: None,
            mime: None,
        };
        request(
            "POST",
            "/v1/downloads",
            &host(),
            Some(ORIGIN),
            Some(&env.token),
            Some(serde_json::to_string(&body).unwrap()),
        )
    };

    let response = r
        .clone()
        .oneshot(send(Some(&format!("theme=dark; {AUTH_COOKIE}"))))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let added: bridge::Added = serde_json::from_slice(&bytes).unwrap();
    let id = added.id.parse().unwrap();
    let done = wait_for(&env.engine, id).await;
    assert_eq!(
        done.state.kind(),
        TaskStateKind::Completed,
        "{:?}",
        done.state
    );
    assert_eq!(
        sha256_file(env.downloads.join("private.bin"))
            .await
            .unwrap(),
        f.sha256
    );
    let seen = server.requests();
    assert!(
        seen.iter()
            .all(|q| q.header("user-agent") == Some("Mozilla/5.0 (Brave test)"))
    );

    // Without the cookie the site refuses: the download fails instead of saving a login page.
    let response = r.clone().oneshot(send(None)).await.unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let id = serde_json::from_slice::<bridge::Added>(&bytes)
        .unwrap()
        .id
        .parse()
        .unwrap();
    assert_eq!(
        wait_for(&env.engine, id).await.state.kind(),
        TaskStateKind::Failed
    );
    env.engine.shutdown().await;
}

#[tokio::test]
async fn a_download_waits_for_the_user_and_keeps_its_cookies() {
    let f = Fixture::random("private.bin", 200_000, 11);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let env = env().await;
    let (r, offers) = env.asking(true);
    let send = |path: &str, file: &str| {
        let body = DownloadRequest {
            url: server.url(file),
            referrer: None,
            user_agent: None,
            cookies: Some(AUTH_COOKIE.to_owned()),
            filename: Some("private.bin".into()),
            size: Some(200_000),
            mime: Some("application/zip".into()),
        };
        request(
            "POST",
            path,
            &host(),
            Some(ORIGIN),
            Some(&env.token),
            Some(serde_json::to_string(&body).unwrap()),
        )
    };
    // Nothing is added or requested from the site before the answer.
    let req = send("/v1/offers", "/auth/private.bin");
    assert_eq!(status(&r, req).await, StatusCode::ACCEPTED);
    // An older extension takes only 201 for "Persivio has it".
    let req = send("/v1/downloads", "/auth/private.bin");
    assert_eq!(status(&r, req).await, StatusCode::CREATED);
    assert_eq!(env.engine.list().await.unwrap(), []);
    assert_eq!(server.requests().len(), 0);
    let offer = offers.list().remove(0);
    assert_eq!(offers.list().len(), 1, "the same link once");
    assert_eq!(
        (offer.file_name.as_str(), offer.size),
        ("private.bin", Some(200_000))
    );

    // A folder that cannot be used: the question stays.
    let bad = AcceptOffer {
        dest_dir: Some("relative".into()),
        ..AcceptOffer::default()
    };
    assert!(offers.accept(&env.engine, &offer.id, bad).await.is_err());
    assert_eq!(offers.list().len(), 1);

    let answer = AcceptOffer {
        file_name: Some("../renamed.bin".into()),
        ..AcceptOffer::default()
    };
    let id = offers.accept(&env.engine, &offer.id, answer).await.unwrap();
    let done = wait_for(&env.engine, id.parse().unwrap()).await;
    assert_eq!(
        done.state.kind(),
        TaskStateKind::Completed,
        "{:?}",
        done.state
    );
    let saved = sha256_file(env.downloads.join("renamed.bin"))
        .await
        .unwrap();
    assert_eq!(saved, f.sha256);
    assert_eq!(offers.list(), []);
    let again = offers.accept(&env.engine, &offer.id, AcceptOffer::default());
    assert!(again.await.is_err(), "answered once");

    // A page that starts downloads by itself cannot fill the queue.
    for i in 0..MAX_PENDING {
        let req = send("/v1/offers", &format!("/auth/{i}"));
        assert_eq!(status(&r, req).await, StatusCode::ACCEPTED);
    }
    let req = send("/v1/offers", "/auth/one-more");
    assert_eq!(status(&r, req).await, StatusCode::TOO_MANY_REQUESTS);
    env.engine.shutdown().await;
}

async fn wait_for(engine: &EngineHandle, id: pv_core::TaskId) -> pv_core::TaskSnapshot {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let task = engine
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.id == id)
            .unwrap();
        if task.state.kind().is_terminal()
            || task.state.kind() == TaskStateKind::Failed
            || tokio::time::Instant::now() > deadline
        {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A raw HTTP/1.1 request over a real socket; returns the status code.
fn raw_get(port: u16, token: &str) -> std::io::Result<u16> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    write!(
        s,
        "GET /v1/ping HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {ORIGIN}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    Ok(reply[9..12].parse().unwrap_or(0))
}

/// Nothing listens: the connection itself is refused (a hang or bad reply would not count).
fn refused(result: std::io::Result<u16>, what: &str) {
    let kind = result
        .map(|s| panic!("{what}: got HTTP {s}"))
        .unwrap_err()
        .kind();
    assert_eq!(kind, std::io::ErrorKind::ConnectionRefused, "{what}");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_supervisor_follows_the_settings_and_reports_busy_ports() {
    let env = env().await;
    let status = Arc::new(std::sync::Mutex::new(bridge::BridgeStatus::default()));
    let port = free_port();
    let (tx, rx) = tokio::sync::watch::channel(pv_core::DesktopSettings {
        browser_bridge: true,
        bridge_port: port,
        bridge_token: env.token.clone(),
        ..pv_core::DesktopSettings::default()
    });
    let supervisor = tokio::spawn(bridge::supervise(
        rx,
        Arc::new(env.engine.clone()),
        Some(env.downloads.clone()),
        Arc::new(Offers::new(|| {})),
        Arc::clone(&status),
    ));
    let settle = || tokio::time::sleep(Duration::from_millis(200));
    let get = |port: u16, token: String| tokio::task::spawn_blocking(move || raw_get(port, &token));
    settle().await;
    assert!(status.lock().unwrap().running);
    assert_eq!(get(port, env.token.clone()).await.unwrap().unwrap(), 200);
    assert_eq!(get(port, "wrong".into()).await.unwrap().unwrap(), 401);

    // A new port: the old one closes, the new one answers.
    let new_port = free_port();
    tx.send_modify(|s| s.bridge_port = new_port);
    settle().await;
    assert_eq!(
        get(new_port, env.token.clone()).await.unwrap().unwrap(),
        200
    );
    refused(
        get(port, env.token.clone()).await.unwrap(),
        "old port closed",
    );

    // A busy port is reported, not fatal.
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = blocker.local_addr().unwrap().port();
    tx.send_modify(|s| s.bridge_port = busy);
    settle().await;
    let s = status.lock().unwrap().clone();
    assert!(!s.running && s.error.unwrap().contains(&busy.to_string()));

    // Disabled: nothing listens.
    drop(blocker);
    tx.send_modify(|s| {
        s.bridge_port = new_port;
        s.browser_bridge = false;
    });
    settle().await;
    assert!(!status.lock().unwrap().running);
    refused(get(new_port, env.token.clone()).await.unwrap(), "disabled");

    drop(tx);
    supervisor.await.unwrap();
    env.engine.shutdown().await;
}

/// For checking the extension in a real browser by hand or with a script: serves a bridge
/// and a site with a login until stdin closes. Prints one line of JSON with the details.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: serves until stdin closes"]
async fn serve_for_a_real_browser() {
    let f = Fixture::random("private.zip", 3_000_000, 9);
    let server = TestServer::start([f.clone()]).await.unwrap();
    let env = env().await;
    let port = free_port();
    // PERSIVIO_TEST_ASK: wait for the user, and print what would be asked.
    let ask = std::env::var_os("PERSIVIO_TEST_ASK").is_some();
    let (_tx, rx) = tokio::sync::watch::channel(pv_core::DesktopSettings {
        browser_bridge: true,
        bridge_port: port,
        bridge_token: env.token.clone(),
        confirm_browser_downloads: ask,
        ..pv_core::DesktopSettings::default()
    });
    let offers = Arc::new_cyclic(|me: &std::sync::Weak<Offers>| {
        let me = me.clone();
        Offers::new(move || {
            let list = me.upgrade().map_or_else(Vec::new, |o| o.list());
            println!("OFFER {}", serde_json::to_string(&list).unwrap());
        })
    });
    tokio::spawn(bridge::supervise(
        rx,
        Arc::new(env.engine.clone()),
        Some(env.downloads.clone()),
        offers,
        Arc::default(),
    ));
    println!(
        "SERVING {}",
        serde_json::json!({
            "port": port,
            "token": env.token,
            "url": server.url("/auth/private.zip"),
            "cookie": AUTH_COOKIE,
            "sha256": f.sha256,
            "dir": env.downloads,
        })
    );
    tokio::task::spawn_blocking(|| std::io::stdin().read_line(&mut String::new()))
        .await
        .unwrap()
        .unwrap();
    env.engine.shutdown().await;
}
