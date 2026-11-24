//! The HTTP client used for probing and downloading.

use std::future::Future;
use std::time::Duration;

use pv_core::{ProtocolError, Settings, Source};
use reqwest::header::{
    ACCEPT_ENCODING, COOKIE, HeaderMap, HeaderName, HeaderValue, LOCATION, REFERER,
};
use reqwest::{Client, Response, StatusCode, redirect};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Maximum redirects followed per request.
pub const MAX_REDIRECTS: usize = 10;

/// Request headers a user (or the browser extension) may not override, because the
/// download logic depends on controlling them.
const RESERVED_HEADERS: &[&str] = &[
    "range",
    "if-range",
    "accept-encoding",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
];

/// Client settings derived from [`Settings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// `User-Agent` header.
    pub user_agent: String,
    /// TCP + TLS connect timeout.
    pub connect_timeout: Duration,
}

impl ClientConfig {
    /// Takes the relevant fields from the app settings.
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            user_agent: settings.user_agent.clone(),
            connect_timeout: Duration::from_secs(u64::from(settings.connect_timeout_secs)),
        }
    }
}

/// What to download and with which request context (cookies, referer, headers).
#[derive(Clone, PartialEq, Eq)]
pub struct DownloadRequest {
    /// URL to request.
    pub url: Url,
    /// Extra headers (reserved ones are ignored).
    pub headers: Vec<(String, String)>,
    /// `Cookie` header value.
    pub cookies: Option<String>,
    /// `Referer` header value.
    pub referer: Option<Url>,
}

impl std::fmt::Debug for DownloadRequest {
    /// Delegates to [`Source`]'s redacting `Debug` so secrets never reach logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Source::Http {
            url: self.url.clone(),
            headers: self.headers.clone(),
            cookies: self.cookies.clone(),
            referer: self.referer.clone(),
        }
        .fmt(f)
    }
}

impl DownloadRequest {
    /// A bare GET of `url`.
    #[must_use]
    pub const fn new(url: Url) -> Self {
        Self {
            url,
            headers: Vec::new(),
            cookies: None,
            referer: None,
        }
    }

    /// Builds a request from an HTTP [`Source`], validating it.
    ///
    /// # Errors
    /// [`ProtocolError::InvalidSource`] for non-HTTP or invalid sources.
    pub fn from_source(source: &Source) -> Result<Self, ProtocolError> {
        source
            .validate()
            .map_err(|e| ProtocolError::InvalidSource(e.to_string()))?;
        match source {
            Source::Http {
                url,
                headers,
                cookies,
                referer,
            } => Ok(Self {
                url: url.clone(),
                headers: headers.clone(),
                cookies: cookies.clone(),
                referer: referer.clone(),
            }),
            _ => Err(ProtocolError::InvalidSource(
                "not an HTTP source".to_owned(),
            )),
        }
    }
}

/// A shared, connection-pooling HTTP client configured per ADR-003 and ADR-011:
/// HTTP/1.1 only, `Accept-Encoding: identity`, no automatic decompression, at most
/// [`MAX_REDIRECTS`] redirects, followed and checked by [`HttpClient::send`]. TLS is
/// rustls with certificate validation always on.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Client,
}

impl HttpClient {
    /// Builds the client.
    ///
    /// # Errors
    /// [`ProtocolError::Network`] if the TLS backend cannot be initialized or the user
    /// agent is not a valid header value.
    pub fn new(config: &ClientConfig) -> Result<Self, ProtocolError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let inner = Client::builder()
            .http1_only()
            .user_agent(config.user_agent.as_str())
            .default_headers(headers)
            .connect_timeout(config.connect_timeout)
            // Redirects are followed by `send`, which decides per hop what to send.
            .redirect(redirect::Policy::none())
            .referer(false) // we send the task's own referer, never an automatic one
            .tcp_nodelay(true)
            .build()
            .map_err(|e| ProtocolError::Network(format!("cannot build HTTP client: {e}")))?;
        Ok(Self { inner })
    }

    /// Sends a GET for `url` with `extra` headers (`Range`, `If-Range`), following
    /// redirects. The returned response's `url()` is the final URL.
    ///
    /// Redirects are followed here rather than by reqwest so that every hop is checked:
    /// - only `http`/`https` targets (never `file:`, `ftp:`, `data:`, ...);
    /// - no downgrade from `https` to `http` (a network attacker could swap the file);
    /// - at most [`MAX_REDIRECTS`] hops;
    /// - the user's cookies and custom headers are sent only while the URL has the same
    ///   origin as the task's URL. A CDN on another host never receives them, neither
    ///   while following the redirect nor in the later range requests to the final URL.
    ///   The referer is kept: hosts that check it (hotlink protection) are usually CDNs.
    ///
    /// # Errors
    /// [`ProtocolError::Network`] for transport errors and refused redirects.
    pub(crate) async fn send(
        &self,
        request: &DownloadRequest,
        url: &Url,
        extra: &[(HeaderName, String)],
    ) -> Result<Response, ProtocolError> {
        let mut url = url.clone();
        let mut hops = 0;
        loop {
            let headers = request_headers(request, same_origin(&url, &request.url));
            let mut builder = self.inner.get(url.clone()).headers(headers);
            for (name, value) in extra {
                builder = builder.header(name, value.as_str());
            }
            let response = builder.send().await.map_err(network_error)?;
            let Some(location) = redirect_location(&response) else {
                return Ok(response);
            };
            if hops >= MAX_REDIRECTS {
                return Err(ProtocolError::Network("too many redirects".to_owned()));
            }
            hops += 1;
            url = next_hop(&url, &location)?;
        }
    }
}

/// The task's headers: the referer always; cookies and custom headers only when
/// `credentials` is true (same origin as the task's URL).
fn request_headers(request: &DownloadRequest, credentials: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(referer) = request
        .referer
        .as_ref()
        .and_then(|r| HeaderValue::from_str(r.as_str()).ok())
    {
        headers.insert(REFERER, referer);
    }
    if !credentials {
        return headers;
    }
    for (name, value) in &request.headers {
        if RESERVED_HEADERS
            .iter()
            .any(|r| r.eq_ignore_ascii_case(name))
        {
            continue;
        }
        // Source::validate already rejected invalid names/values; skip defensively.
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.append(n, v);
        }
    }
    if let Some(cookie) = request
        .cookies
        .as_deref()
        .and_then(|c| HeaderValue::from_str(c).ok())
    {
        headers.insert(COOKIE, cookie);
    }
    headers
}

/// The `Location` of a redirect we should follow. Other 3xx responses (304, 300, or a
/// redirect without a usable `Location`) are returned to the caller unchanged.
fn redirect_location(response: &Response) -> Option<String> {
    let redirect = matches!(
        response.status(),
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    );
    if !redirect {
        return None;
    }
    let location = response.headers().get(LOCATION)?.to_str().ok()?;
    Some(location.to_owned())
}

/// Resolves and checks one redirect hop from `current` to `location`.
fn next_hop(current: &Url, location: &str) -> Result<Url, ProtocolError> {
    let next = current
        .join(location)
        .map_err(|e| ProtocolError::Network(format!("redirect to an invalid URL: {e}")))?;
    if !matches!(next.scheme(), "http" | "https") {
        // Security checklist: never follow a redirect to file:, ftp:, data:, ...
        return Err(ProtocolError::Network(
            "redirect to a non-HTTP(S) URL was blocked".to_owned(),
        ));
    }
    if current.scheme() == "https" && next.scheme() == "http" {
        return Err(ProtocolError::Network(
            "redirect from HTTPS to insecure HTTP was blocked".to_owned(),
        ));
    }
    Ok(next)
}

/// Same scheme, host and port.
fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Converts a reqwest error into a protocol error **without the URL** (signed URLs carry
/// tokens in their query strings and must not end up in logs or the UI).
/// Waits for `response`, at most `timeout`, and only until `cancel` fires (`Ok(None)`).
/// A server may accept a connection and then never answer.
pub(crate) async fn guarded<T>(
    cancel: &CancellationToken,
    timeout: Duration,
    response: impl Future<Output = Result<T, ProtocolError>>,
) -> Result<Option<T>, ProtocolError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Ok(None),
        answered = tokio::time::timeout(timeout, response) => match answered {
            Err(_) => Err(ProtocolError::Network(format!(
                "timed out: no response for {}s",
                timeout.as_secs_f32()
            ))),
            Ok(result) => result.map(Some),
        },
    }
}

pub(crate) fn network_error(err: reqwest::Error) -> ProtocolError {
    let timeout = err.is_timeout();
    let err = err.without_url();
    if timeout {
        ProtocolError::Network(format!("timed out: {err}"))
    } else {
        // Include the source chain: reqwest's top-level message is often generic.
        let mut msg = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(s) = source {
            msg.push_str(": ");
            msg.push_str(&s.to_string());
            source = s.source();
        }
        ProtocolError::Network(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn redirect_hops_are_resolved_and_checked() {
        let base = url("https://a.example/dir/file?x=1");
        assert_eq!(
            next_hop(&base, "/other").unwrap().as_str(),
            "https://a.example/other"
        );
        assert_eq!(
            next_hop(&base, "https://cdn.example/f").unwrap().host_str(),
            Some("cdn.example")
        );
        for refused in [
            "http://a.example/f",
            "file:///etc/passwd",
            "ftp://a/f",
            "data:,x",
        ] {
            assert!(next_hop(&base, refused).is_err(), "{refused}");
        }
        // Upgrading is fine, and plain HTTP may stay on HTTP.
        let plain = url("http://a.example/f");
        assert!(next_hop(&plain, "https://a.example/f").is_ok());
        assert!(next_hop(&plain, "http://b.example/f").is_ok());
    }

    #[test]
    fn origin_comparison_includes_scheme_and_port() {
        let a = url("https://a.example/x");
        assert!(same_origin(&a, &url("https://a.example:443/y?z")));
        assert!(!same_origin(&a, &url("http://a.example/x")));
        assert!(!same_origin(&a, &url("https://a.example:8443/x")));
        assert!(!same_origin(&a, &url("https://b.a.example/x")));
    }

    #[test]
    fn credentials_only_for_the_same_origin() {
        let request = DownloadRequest {
            url: url("https://a.example/f"),
            headers: vec![
                ("X-Token".into(), "t".into()),
                ("Range".into(), "bytes=9-".into()),
            ],
            cookies: Some("s=1".into()),
            referer: Some(url("https://a.example/page")),
        };
        let with = request_headers(&request, true);
        assert_eq!(with.get(COOKIE).unwrap(), "s=1");
        assert_eq!(with.get("x-token").unwrap(), "t");
        assert!(
            with.get("range").is_none(),
            "reserved headers are never overridden"
        );
        let without = request_headers(&request, false);
        assert!(without.get(COOKIE).is_none());
        assert!(without.get("x-token").is_none());
        assert_eq!(without.get(REFERER).unwrap(), "https://a.example/page");
    }
}
