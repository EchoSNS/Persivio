//! The local bridge for the browser extension (Phase 10, ADR-021).
//!
//! A small HTTP server bound to `127.0.0.1` only. Every request must pass, in order:
//! 1. **Host check**: `Host` is `127.0.0.1:<port>` or `localhost:<port>`. This defeats
//!    DNS rebinding (a web page whose domain resolves to 127.0.0.1).
//! 2. **Origin check**: `Origin` is a browser extension (`chrome-extension://` or
//!    `moz-extension://`), never a web page. Chromium leaves `Origin` out of an
//!    extension's GET requests; then `Sec-Fetch-Site` must not name a web page either.
//! 3. **Token**: `Authorization: Bearer <pairing token>`, compared in constant time.
//! 4. **Size and content limits**: bodies up to 64 KiB, validated fields.
//!
//! CORS preflights are answered only for extension origins. A download from the browser
//! waits for the user's answer ([`crate::offers`]), unless that is turned off.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pv_core::filename::sanitize_filename;
use pv_core::{DesktopSettings, Source, TaskSpec};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::api::EngineApi;
use crate::offers::Offers;

/// Largest request body.
pub const MAX_BODY: usize = 64 * 1024;
/// Largest `Cookie` value accepted from the extension.
const MAX_COOKIES: usize = 16 * 1024;

/// What the extension sends for one download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadRequest {
    /// The file's URL (http or https).
    pub url: String,
    /// Page the download started from.
    #[serde(default)]
    pub referrer: Option<String>,
    /// The browser's `User-Agent` (some sites tie cookies to it).
    #[serde(default)]
    pub user_agent: Option<String>,
    /// `Cookie` header value for the URL (from the browser's cookie store).
    #[serde(default)]
    pub cookies: Option<String>,
    /// File name the browser suggested.
    #[serde(default)]
    pub filename: Option<String>,
    /// Bytes, if the browser knows (shown in the question, never trusted).
    #[serde(default)]
    pub size: Option<u64>,
    /// Media type, if the browser knows (shown in the question).
    #[serde(default)]
    pub mime: Option<String>,
}

/// Reply to a successful add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Added {
    /// Id of the new task, or of the offer that waits for the user.
    pub id: String,
}

/// Everything a request handler needs.
#[derive(Clone)]
pub struct BridgeState {
    /// The pairing token.
    pub token: Arc<str>,
    /// Port the server listens on (for the Host check).
    pub port: u16,
    /// The engine.
    pub engine: Arc<dyn EngineApi>,
    /// Fallback download folder (the OS Downloads folder).
    pub os_dir: Option<PathBuf>,
    /// Where downloads wait for the user's answer.
    pub offers: Arc<Offers>,
    /// Current settings: whether to ask.
    pub settings: watch::Receiver<DesktopSettings>,
}

/// The bridge's routes with all checks applied.
pub fn router(state: BridgeState) -> Router {
    Router::new()
        .route("/v1/ping", get(ping))
        .route("/v1/offers", post(add_offer))
        // Extensions before 0.7.
        .route("/v1/downloads", post(add_download))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(state)
}

fn reject(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

fn is_extension_origin(origin: &str) -> bool {
    let rest = origin
        .strip_prefix("chrome-extension://")
        .or_else(|| origin.strip_prefix("moz-extension://"));
    rest.is_some_and(|id| {
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

/// Compares without an early exit, so response timing reveals nothing about the token.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Who calls: `Some(Some(origin))` for an extension that names itself, `Some(None)` for a
/// request that no web page can have made, `None` for everything else.
#[allow(clippy::option_option, reason = "three cases, explained above")]
fn allowed_caller(headers: &HeaderMap, preflight: bool) -> Option<Option<HeaderValue>> {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let ok = origin.to_str().is_ok_and(is_extension_origin);
        return ok.then(|| Some(origin.clone()));
    }
    // Browsers add `Sec-Fetch-Site` to what pages request, and pages cannot change it.
    // `none` (or no such header: not a browser) is not from a page. The token decides.
    let site = headers.get("sec-fetch-site").map(HeaderValue::as_bytes);
    (!preflight && matches!(site, None | Some(b"none"))).then_some(None)
}

async fn guard(State(state): State<BridgeState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host_ok = header_str(headers, header::HOST).is_some_and(|h| {
        h == format!("127.0.0.1:{}", state.port) || h == format!("localhost:{}", state.port)
    });
    if !host_ok {
        return reject(StatusCode::FORBIDDEN, "invalid host");
    }
    let preflight = request.method() == Method::OPTIONS;
    let Some(allow_origin) = allowed_caller(headers, preflight) else {
        return reject(
            StatusCode::FORBIDDEN,
            "only the Persivio browser extension may call this",
        );
    };
    if preflight {
        let mut response = StatusCode::NO_CONTENT.into_response();
        let h = response.headers_mut();
        if let Some(o) = allow_origin {
            h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
        }
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        return response;
    }
    let authorized = header_str(headers, header::AUTHORIZATION)
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| constant_time_eq(t.as_bytes(), state.token.as_bytes()));
    if !authorized {
        return reject(StatusCode::UNAUTHORIZED, "missing or wrong pairing token");
    }
    let mut response = next.run(request).await;
    if let Some(o) = allow_origin {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
    }
    response
}

async fn ping() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "app": "persivio", "version": env!("CARGO_PKG_VERSION") }))
}

/// Validates a request and turns it into a task spec (pure; tested).
///
/// # Errors
/// A human-readable reason for a 422 response.
pub fn spec_for(req: &DownloadRequest, dest_dir: PathBuf) -> Result<TaskSpec, String> {
    let url = Url::parse(&req.url).map_err(|_| "invalid url".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https downloads are supported".to_owned());
    }
    let referer = req
        .referrer
        .as_deref()
        .filter(|r| !r.is_empty())
        .and_then(|r| Url::parse(r).ok())
        .filter(|r| matches!(r.scheme(), "http" | "https"));
    let cookies = req.cookies.clone().filter(|c| !c.trim().is_empty());
    if cookies.as_ref().is_some_and(|c| c.len() > MAX_COOKIES) {
        return Err("cookies too large".to_owned());
    }
    let headers = req
        .user_agent
        .iter()
        .filter(|ua| !ua.is_empty() && ua.len() <= 512)
        .map(|ua| ("User-Agent".to_owned(), ua.clone()))
        .collect();
    let source = Source::Http {
        url,
        headers,
        cookies,
        referer,
    };
    source.validate().map_err(|e| e.to_string())?;
    let mut spec = TaskSpec::new(source, dest_dir);
    spec.file_name = req
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.len() <= 1024)
        .map(sanitize_filename);
    Ok(spec)
}

type Body = Result<Json<DownloadRequest>, axum::extract::rejection::JsonRejection>;

async fn add_offer(State(state): State<BridgeState>, body: Body) -> Response {
    receive(&state, body, StatusCode::ACCEPTED).await
}

async fn add_download(State(state): State<BridgeState>, body: Body) -> Response {
    // What old extensions take for "Persivio has it".
    receive(&state, body, StatusCode::CREATED).await
}

/// Queues the download for the user's answer (`waiting`), or adds it at once.
async fn receive(state: &BridgeState, body: Body, waiting: StatusCode) -> Response {
    let req = match body {
        Ok(Json(req)) => req,
        // Too large stays 413; malformed or unknown fields become 422.
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return reject(StatusCode::PAYLOAD_TOO_LARGE, "request too large");
        }
        Err(_) => return reject(StatusCode::UNPROCESSABLE_ENTITY, "invalid request body"),
    };
    let dest = match crate::ops::default_download_dir(&*state.engine, state.os_dir.clone()).await {
        Ok(dir) => dir,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.message).into_response(),
    };
    let spec = match spec_for(&req, dest) {
        Ok(spec) => spec,
        Err(message) => return (StatusCode::UNPROCESSABLE_ENTITY, message).into_response(),
    };
    if state.settings.borrow().confirm_browser_downloads {
        return match state.offers.push(spec, req.size, req.mime.as_deref()) {
            Some(id) => (waiting, Json(Added { id })).into_response(),
            None => reject(
                StatusCode::TOO_MANY_REQUESTS,
                "too many downloads wait for an answer",
            ),
        };
    }
    match state.engine.add(spec).await {
        Ok(id) => (StatusCode::CREATED, Json(Added { id: id.to_string() })).into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    }
}

/// A new random pairing token (256 bits, hex).
///
/// # Errors
/// If the OS random source fails.
pub fn new_token() -> Result<String, getrandom::Error> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(bytes
        .iter()
        .flat_map(|b| {
            [
                char::from(HEX[usize::from(b >> 4)]),
                char::from(HEX[usize::from(b & 15)]),
            ]
        })
        .collect())
}

/// Whether the bridge runs, for the settings page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BridgeStatus {
    /// Listening now.
    pub running: bool,
    /// Why it could not start (e.g. the port is in use).
    pub error: Option<String>,
}

/// Starts, restarts and stops the bridge as the settings change, until the settings
/// sender is dropped. Binds only to `127.0.0.1`.
pub async fn supervise(
    mut settings: watch::Receiver<DesktopSettings>,
    engine: Arc<dyn EngineApi>,
    os_dir: Option<PathBuf>,
    offers: Arc<Offers>,
    status: Arc<Mutex<BridgeStatus>>,
) {
    let live = settings.clone();
    let set_status = |s: BridgeStatus| {
        *status.lock().unwrap_or_else(PoisonError::into_inner) = s;
    };
    let mut current: Option<((u16, String), CancellationToken)> = None;
    loop {
        let wanted = {
            let s = settings.borrow_and_update();
            s.browser_bridge
                .then(|| (s.bridge_port, s.bridge_token.clone()))
        };
        if current.as_ref().map(|(config, _)| config) != wanted.as_ref() {
            if let Some((_, stop)) = current.take() {
                stop.cancel();
            }
            set_status(BridgeStatus::default());
            if let Some((port, token)) = wanted {
                match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
                    Ok(listener) => {
                        let stop = CancellationToken::new();
                        let app = router(BridgeState {
                            token: token.clone().into(),
                            port,
                            engine: Arc::clone(&engine),
                            os_dir: os_dir.clone(),
                            offers: Arc::clone(&offers),
                            settings: live.clone(),
                        });
                        let until = stop.clone();
                        tokio::spawn(async move {
                            let served = axum::serve(listener, app)
                                .with_graceful_shutdown(async move { until.cancelled().await })
                                .await;
                            if let Err(e) = served {
                                tracing::warn!("browser bridge stopped: {e}");
                            }
                        });
                        set_status(BridgeStatus {
                            running: true,
                            error: None,
                        });
                        current = Some(((port, token), stop));
                    }
                    Err(e) => {
                        tracing::warn!("browser bridge cannot listen on port {port}: {e}");
                        set_status(BridgeStatus {
                            running: false,
                            error: Some(format!("cannot listen on 127.0.0.1:{port}: {e}")),
                        });
                    }
                }
            }
        }
        if settings.changed().await.is_err() {
            if let Some((_, stop)) = current.take() {
                stop.cancel();
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> DownloadRequest {
        DownloadRequest {
            url: url.to_owned(),
            referrer: None,
            user_agent: None,
            cookies: None,
            filename: None,
            size: None,
            mime: None,
        }
    }

    #[test]
    fn origins_must_be_extensions() {
        assert!(is_extension_origin("chrome-extension://abcdefghijklmnop"));
        assert!(is_extension_origin(
            "moz-extension://6f1b2c3d-1111-2222-3333-444455556666"
        ));
        for bad in [
            "https://evil.example",
            "http://127.0.0.1:47321",
            "null",
            "chrome-extension://",
            "chrome-extension://abc/../x",
            "file://",
        ] {
            assert!(!is_extension_origin(bad), "{bad}");
        }
    }

    #[test]
    fn tokens_are_random_hex_and_compared_exactly() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, new_token().unwrap());
        assert!(constant_time_eq(a.as_bytes(), a.as_bytes()));
        assert!(!constant_time_eq(a.as_bytes(), &a.as_bytes()[..63]));
        assert!(!constant_time_eq(b"abc", b"abd"));
    }

    #[test]
    fn requests_become_http_specs_with_context() {
        let dir = std::env::temp_dir();
        let mut r = req("https://files.example/a.iso");
        r.referrer = Some("https://files.example/page".into());
        r.user_agent = Some("Mozilla/5.0 Brave".into());
        r.cookies = Some("session=1".into());
        r.filename = Some("../../evil.exe".into());
        let spec = spec_for(&r, dir.clone()).unwrap();
        assert_eq!(spec.file_name.as_deref(), Some("evil.exe"), "sanitized");
        match spec.source {
            Source::Http {
                headers,
                cookies,
                referer,
                ..
            } => {
                assert_eq!(headers, [("User-Agent".into(), "Mozilla/5.0 Brave".into())]);
                assert_eq!(cookies.as_deref(), Some("session=1"));
                assert_eq!(referer.unwrap().path(), "/page");
            }
            other => panic!("{other:?}"),
        }

        for bad in [
            "ftp://x/y",
            "file:///etc/passwd",
            "not a url",
            "javascript:alert(1)",
        ] {
            assert!(spec_for(&req(bad), dir.clone()).is_err(), "{bad}");
        }
        let mut big = req("https://a.example/f");
        big.cookies = Some("x".repeat(MAX_COOKIES + 1));
        assert!(spec_for(&big, dir.clone()).is_err());
        let mut newline = req("https://a.example/f");
        newline.cookies = Some("a=1\r\nX-Evil: 1".into());
        assert!(spec_for(&newline, dir).is_err(), "header injection");
    }
}
