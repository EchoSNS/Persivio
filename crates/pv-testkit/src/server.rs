//! The in-process test HTTP server.

use std::collections::HashMap;
use std::io::{self, Write as _};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header, response::Builder};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::body::{self, Faults};
use crate::fixture::{Fixture, sha256_hex};
use crate::range::{self, RangeOutcome};
use crate::rng::SplitMix64;

/// Longest redirect chain `/redirect/{n}/...` accepts.
const MAX_REDIRECTS: u32 = 50;

/// One request as seen by the server, for assertions in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// HTTP method.
    pub method: String,
    /// Path and query.
    pub path: String,
    /// All request headers, names lower-case.
    pub headers: Vec<(String, String)>,
}

impl RecordedRequest {
    /// First value of a header (case-insensitive name).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A served representation: content plus validators.
#[derive(Debug)]
struct Entry {
    bytes: Bytes,
    etag: String,
    last_modified: String,
}

impl Entry {
    fn new(bytes: Bytes, version: u32) -> Self {
        let hash = sha256_hex(&bytes);
        Self {
            etag: format!("\"{}-v{version}\"", &hash[..16]),
            // A valid IMF-fixdate that changes with the version.
            last_modified: format!(
                "Wed, 21 Oct 2015 07:{:02}:{:02} GMT",
                (version / 60) % 60,
                version % 60
            ),
            bytes,
        }
    }
}

struct AppState {
    files: RwLock<HashMap<String, Arc<Entry>>>,
    /// Current versions of files served by `/mutable/`.
    mutable: RwLock<HashMap<String, (u32, Arc<Entry>)>>,
    requests: Mutex<Vec<RecordedRequest>>,
    request_counter: AtomicU64,
    cancel: CancellationToken,
}

impl AppState {
    fn file(&self, name: &str) -> Option<Arc<Entry>> {
        self.files
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    fn mutable(&self, name: &str) -> Option<Arc<Entry>> {
        let current = self
            .mutable
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .map(|(_, e)| e.clone());
        current.or_else(|| self.file(name))
    }

    /// Replaces the `/mutable/` content of `name` with a same-length variant and a new
    /// `ETag`/`Last-Modified`. Same length on purpose: a client must detect the change
    /// through validators, not by size.
    fn mutate(&self, name: &str) -> Option<Bytes> {
        let original = self.file(name)?;
        let mut map = self.mutable.write().unwrap_or_else(PoisonError::into_inner);
        let (version, current) = map
            .get(name)
            .map_or((0, original), |(v, e)| (*v, e.clone()));
        let next: Bytes = current.bytes.iter().map(|b| b.wrapping_add(1)).collect();
        let version = version + 1;
        map.insert(
            name.to_owned(),
            (version, Arc::new(Entry::new(next.clone(), version))),
        );
        Some(next)
    }
}

/// An HTTP server on `127.0.0.1:<ephemeral port>`, running inside the test's runtime.
///
/// Each test starts its own server, so tests run in parallel without port conflicts.
/// Dropping the server stops it; [`TestServer::shutdown`] stops it and waits.
pub struct TestServer {
    addr: SocketAddr,
    state: Arc<AppState>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Starts a server serving `fixtures`.
    ///
    /// # Errors
    /// If binding the listener fails.
    pub async fn start(fixtures: impl IntoIterator<Item = Fixture>) -> io::Result<Self> {
        let cancel = CancellationToken::new();
        let state = Arc::new(AppState {
            files: RwLock::new(HashMap::new()),
            mutable: RwLock::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
            request_counter: AtomicU64::new(0),
            cancel: cancel.clone(),
        });
        for f in fixtures {
            insert_fixture(&state, f);
        }

        let app = Router::new()
            .route("/file/{name}", get(file))
            .route("/norange/{name}", get(norange))
            .route("/throttle/{name}", get(throttle))
            .route("/flaky/{name}", get(flaky))
            .route("/hang/{name}", get(hang))
            .route("/redirect/{n}/{name}", get(redirect))
            .route("/disposition/{name}", get(disposition))
            .route("/mutable/{name}", get(mutable))
            .route("/control/mutate/{name}", post(control_mutate))
            .route("/gzip/{name}", get(gzip))
            .layer(middleware::from_fn_with_state(state.clone(), record))
            .with_state(state.clone());

        // Port 0: the OS picks a free ephemeral port, so parallel tests never collide.
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
        let addr = listener.local_addr()?;
        let handle = tokio::spawn(async move {
            // Serve errors after startup only happen on shutdown; nothing to report to.
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { cancel.cancelled().await })
                .await;
        });
        Ok(Self {
            addr,
            state,
            handle: Some(handle),
        })
    }

    /// The bound address.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Absolute URL for a path, e.g. `url("/file/a.bin")`.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Registers (or replaces) a fixture.
    pub fn add_fixture(&self, fixture: Fixture) {
        insert_fixture(&self.state, fixture);
    }

    /// Changes the `/mutable/{name}` content (same as `POST /control/mutate/{name}`) and
    /// returns the new content as a fixture, or `None` for an unknown name.
    #[must_use]
    pub fn mutate(&self, name: &str) -> Option<Fixture> {
        self.state
            .mutate(name)
            .map(|b| Fixture::from_bytes(name, b))
    }

    /// Requests received so far, in arrival order.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Clears the request log.
    pub fn clear_requests(&self) {
        self.state
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Stops accepting connections, aborts in-flight (including hanging) bodies, and
    /// waits for the server task to finish.
    pub async fn shutdown(mut self) {
        self.state.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

type Shared = State<Arc<AppState>>;
type Params = Query<HashMap<String, String>>;

async fn record(State(state): Shared, req: Request, next: Next) -> Response {
    let recorded = RecordedRequest {
        method: req.method().to_string(),
        path: req
            .uri()
            .path_and_query()
            .map_or_else(|| req.uri().path().to_owned(), ToString::to_string),
        headers: req
            .headers()
            .iter()
            .map(|(n, v)| {
                (
                    n.as_str().to_owned(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect(),
    };
    state
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(recorded);
    next.run(req).await
}

fn build(builder: Builder, body: Body) -> Response {
    builder
        .body(body)
        .unwrap_or_else(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "no such fixture").into_response()
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn param<T: std::str::FromStr>(q: &HashMap<String, String>, key: &str) -> Option<T> {
    q.get(key).and_then(|v| v.parse().ok())
}

/// Options for [`serve`].
#[derive(Default)]
struct Opts {
    ranges: bool,
    faults: Faults,
    disposition: Option<HeaderValue>,
}

/// Serves an entry honoring `Range`/`If-Range` (when `opts.ranges`) with fault injection.
fn serve(state: &AppState, entry: &Entry, headers: &HeaderMap, opts: Opts) -> Response {
    let len = u64::try_from(entry.bytes.len()).unwrap_or(u64::MAX);
    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::ETAG, &entry.etag)
        .header(header::LAST_MODIFIED, &entry.last_modified);
    if let Some(cd) = opts.disposition {
        builder = builder.header(header::CONTENT_DISPOSITION, cd);
    }

    let mut outcome = RangeOutcome::Full;
    if opts.ranges {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
        let if_range_ok = header_str(headers, header::IF_RANGE)
            .is_none_or(|v| range::if_range_matches(v, &entry.etag, &entry.last_modified));
        if if_range_ok {
            outcome = range::evaluate(header_str(headers, header::RANGE), len);
        }
    } else {
        builder = builder.header(header::ACCEPT_RANGES, "none");
    }

    let (status, start, end) = match outcome {
        RangeOutcome::Full => (StatusCode::OK, 0, len),
        RangeOutcome::Partial { start, end } => {
            builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
            (StatusCode::PARTIAL_CONTENT, start, end + 1)
        }
        RangeOutcome::Unsatisfiable => {
            return build(
                builder
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{len}")),
                Body::empty(),
            );
        }
    };
    let slice = entry.bytes.slice(
        usize::try_from(start).unwrap_or(usize::MAX)..usize::try_from(end).unwrap_or(usize::MAX),
    );
    // Explicit Content-Length on a streamed body: hyper then sends a fixed-length body,
    // so an injected drop is seen by clients as a truncated download.
    build(
        builder
            .status(status)
            .header(header::CONTENT_LENGTH, end - start),
        body::stream(slice, opts.faults, state.cancel.clone()),
    )
}

async fn file(State(s): Shared, Path(name): Path<String>, headers: HeaderMap) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    serve(
        &s,
        &entry,
        &headers,
        Opts {
            ranges: true,
            ..Opts::default()
        },
    )
}

async fn norange(State(s): Shared, Path(name): Path<String>, headers: HeaderMap) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    serve(&s, &entry, &headers, Opts::default())
}

async fn throttle(
    State(s): Shared,
    Path(name): Path<String>,
    Query(q): Params,
    headers: HeaderMap,
) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    let Some(bps) = param::<u64>(&q, "bps").filter(|&b| b > 0) else {
        return (StatusCode::BAD_REQUEST, "bps must be a positive integer").into_response();
    };
    let faults = Faults {
        bps: Some(bps),
        ..Faults::default()
    };
    serve(
        &s,
        &entry,
        &headers,
        Opts {
            ranges: true,
            faults,
            ..Opts::default()
        },
    )
}

async fn flaky(
    State(s): Shared,
    Path(name): Path<String>,
    Query(q): Params,
    headers: HeaderMap,
) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    let seed = param::<u64>(&q, "seed").unwrap_or(0);
    // Each request draws from its own seeded stream: deterministic for a given order.
    let n = s.request_counter.fetch_add(1, Ordering::Relaxed);
    let mut rng = SplitMix64::new(seed ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    if rng.chance(param(&q, "fail_rate").unwrap_or(0.0)) {
        return build(
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::RETRY_AFTER, "0"),
            Body::from("injected failure"),
        );
    }
    let drop_after = param::<u64>(&q, "drop_after")
        .filter(|_| rng.chance(param(&q, "drop_rate").unwrap_or(1.0)));
    let faults = Faults {
        drop_after,
        ..Faults::default()
    };
    serve(
        &s,
        &entry,
        &headers,
        Opts {
            ranges: true,
            faults,
            ..Opts::default()
        },
    )
}

async fn hang(
    State(s): Shared,
    Path(name): Path<String>,
    Query(q): Params,
    headers: HeaderMap,
) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    let faults = Faults {
        hang_after: Some(param(&q, "after").unwrap_or(0)),
        ..Faults::default()
    };
    serve(
        &s,
        &entry,
        &headers,
        Opts {
            ranges: true,
            faults,
            ..Opts::default()
        },
    )
}

async fn redirect(Path((n, name)): Path<(u32, String)>) -> Response {
    if n > MAX_REDIRECTS {
        return (StatusCode::BAD_REQUEST, "too many redirects requested").into_response();
    }
    let location = if n == 0 {
        format!("/file/{name}")
    } else {
        format!("/redirect/{}/{name}", n - 1)
    };
    build(
        Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, location),
        Body::empty(),
    )
}

async fn disposition(
    State(s): Shared,
    Path(name): Path<String>,
    Query(q): Params,
    headers: HeaderMap,
) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    let value = q
        .get("cd")
        .cloned()
        .unwrap_or_else(|| format!("attachment; filename=\"{name}\""));
    let Ok(cd) = HeaderValue::from_str(&value) else {
        return (StatusCode::BAD_REQUEST, "cd is not a valid header value").into_response();
    };
    serve(
        &s,
        &entry,
        &headers,
        Opts {
            ranges: true,
            disposition: Some(cd),
            ..Opts::default()
        },
    )
}

async fn mutable(State(s): Shared, Path(name): Path<String>, headers: HeaderMap) -> Response {
    let Some(entry) = s.mutable(&name) else {
        return not_found();
    };
    serve(
        &s,
        &entry,
        &headers,
        Opts {
            ranges: true,
            ..Opts::default()
        },
    )
}

async fn control_mutate(State(s): Shared, Path(name): Path<String>) -> Response {
    match s.mutate(&name) {
        Some(bytes) => (StatusCode::OK, sha256_hex(&bytes)).into_response(),
        None => not_found(),
    }
}

async fn gzip(State(s): Shared, Path(name): Path<String>, headers: HeaderMap) -> Response {
    let Some(entry) = s.file(&name) else {
        return not_found();
    };
    let accept = header_str(&headers, header::ACCEPT_ENCODING)
        .unwrap_or("")
        .to_ascii_lowercase();
    if accept.contains("identity") && !accept.contains("gzip") {
        return serve(
            &s,
            &entry,
            &headers,
            Opts {
                ranges: true,
                ..Opts::default()
            },
        );
    }
    // Misbehaving-on-purpose: ignores Range and compresses the whole body.
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    let compressed = encoder
        .write_all(&entry.bytes)
        .and_then(|()| encoder.finish());
    match compressed {
        Ok(gz) => build(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_ENCODING, "gzip")
                .header(header::VARY, "Accept-Encoding")
                .header(header::ETAG, &entry.etag)
                .header(header::CONTENT_LENGTH, gz.len()),
            Body::from(gz),
        ),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn insert_fixture(state: &AppState, fixture: Fixture) {
    state
        .files
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(fixture.name, Arc::new(Entry::new(fixture.bytes, 0)));
}
