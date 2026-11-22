//! The HTTP client used for probing and downloading.

use std::time::Duration;

use pv_core::{ProtocolError, Settings, Source};
use reqwest::header::{ACCEPT_ENCODING, COOKIE, HeaderMap, HeaderName, HeaderValue, REFERER};
use reqwest::{Client, RequestBuilder, redirect};
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
/// [`MAX_REDIRECTS`] redirects and only to `http`/`https` URLs. TLS is rustls with
/// certificate validation always on.
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
        let policy = redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if !matches!(attempt.url().scheme(), "http" | "https") {
                // Security checklist: never follow a redirect to file:, ftp:, data:, ...
                attempt.error("redirect to a non-HTTP(S) URL was blocked")
            } else {
                attempt.follow()
            }
        });
        let inner = Client::builder()
            .http1_only()
            .user_agent(config.user_agent.as_str())
            .default_headers(headers)
            .connect_timeout(config.connect_timeout)
            .redirect(policy)
            .referer(false) // we send the task's own referer, never an automatic one
            .tcp_nodelay(true)
            .build()
            .map_err(|e| ProtocolError::Network(format!("cannot build HTTP client: {e}")))?;
        Ok(Self { inner })
    }

    /// A GET for `url` carrying the request's headers, cookies and referer.
    pub(crate) fn get(&self, request: &DownloadRequest, url: &Url) -> RequestBuilder {
        let mut headers = HeaderMap::new();
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
        if let Some(referer) = request
            .referer
            .as_ref()
            .and_then(|r| HeaderValue::from_str(r.as_str()).ok())
        {
            headers.insert(REFERER, referer);
        }
        self.inner.get(url.clone()).headers(headers)
    }
}

/// Converts a reqwest error into a protocol error **without the URL** (signed URLs carry
/// tokens in their query strings and must not end up in logs or the UI).
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
