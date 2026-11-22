//! Discovering what a URL serves before downloading it.

use pv_core::{HttpResumeInfo, ProtocolError};
use reqwest::header::{
    CONTENT_DISPOSITION, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderMap,
    LAST_MODIFIED, RANGE,
};
use reqwest::{Response, StatusCode};
use url::Url;

use crate::client::{DownloadRequest, HttpClient, network_error};
use crate::content_range::{ContentRange, parse_content_range};

/// What the server told us about the resource.
#[derive(Debug)]
pub struct Probe {
    /// URL after redirects; later requests go straight here.
    pub final_url: Url,
    /// Full size in bytes, if known.
    pub total_size: Option<u64>,
    /// Whether the server honors byte ranges (answered our probe with `206`).
    pub accepts_ranges: bool,
    /// Strong `ETag` (weak ones are useless for `If-Range` and are dropped).
    pub etag: Option<String>,
    /// `Last-Modified` value.
    pub last_modified: Option<String>,
    /// Raw `Content-Disposition` (untrusted; parse + sanitize before use).
    pub content_disposition: Option<String>,
    /// `Content-Type`.
    pub content_type: Option<String>,
    /// When the server ignored our range and sent the whole body (`200`), the live
    /// response, so the download can **reuse** it instead of requesting again.
    pub body: Option<Response>,
}

impl Probe {
    /// The value to send as `If-Range` on resume: strong `ETag`, else `Last-Modified`.
    #[must_use]
    pub fn validator(&self) -> Option<&str> {
        self.etag.as_deref().or(self.last_modified.as_deref())
    }

    /// Whether an interrupted download of this resource can be resumed safely: the
    /// server supports ranges **and** gave us a validator to detect changes (ADR-007).
    #[must_use]
    pub fn resumable(&self) -> bool {
        self.accepts_ranges && self.validator().is_some()
    }

    /// Validators to persist for a later resume.
    #[must_use]
    pub fn resume_info(&self) -> HttpResumeInfo {
        HttpResumeInfo {
            final_url: Some(self.final_url.clone()),
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
        }
    }
}

fn header(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Like [`header`], but tolerates raw non-ASCII bytes. Many servers put UTF-8 directly
/// in `Content-Disposition: filename="…"`; like browsers, decode it as UTF-8 and fall
/// back to Latin-1 (where every byte is a valid character).
fn header_text(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    let bytes = headers.get(name)?.as_bytes();
    let text = String::from_utf8(bytes.to_vec())
        .unwrap_or_else(|_| bytes.iter().copied().map(char::from).collect());
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Returns the `ETag` only if it is strong (`"…"`, not `W/"…"`).
pub(crate) fn strong_etag(headers: &HeaderMap) -> Option<String> {
    header(headers, ETAG).filter(|e| e.len() >= 2 && e.starts_with('"') && e.ends_with('"'))
}

/// Validators carried by a response (used to refresh them after a restart).
pub(crate) fn resume_info_from(response: &Response) -> HttpResumeInfo {
    HttpResumeInfo {
        final_url: Some(response.url().clone()),
        etag: strong_etag(response.headers()),
        last_modified: header(response.headers(), LAST_MODIFIED),
    }
}

/// Rejects responses whose body is content-coded despite `Accept-Encoding: identity`:
/// byte offsets would refer to the compressed stream (ADR-011).
pub(crate) fn ensure_identity(headers: &HeaderMap) -> Result<(), ProtocolError> {
    match header(headers, CONTENT_ENCODING) {
        Some(enc) if !enc.eq_ignore_ascii_case("identity") => {
            Err(ProtocolError::InvalidSource(format!(
                "server compressed the download (Content-Encoding: {enc}) although we asked for identity"
            )))
        }
        _ => Ok(()),
    }
}

/// Probes `request` with `GET` + `Range: bytes=0-0`.
///
/// Why GET and not HEAD: many servers answer HEAD differently (or not at all), and if
/// the server ignores the range we already hold the full response stream and reuse it.
///
/// # Errors
/// Network errors, unexpected statuses ([`ProtocolError::HttpStatus`]), a malformed
/// `Content-Range`, or a compressed body.
pub async fn probe(client: &HttpClient, request: &DownloadRequest) -> Result<Probe, ProtocolError> {
    let response = client
        .get(request, &request.url)
        .header(RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(network_error)?;
    let headers = response.headers().clone();
    ensure_identity(&headers)?;

    let mut probe = Probe {
        final_url: response.url().clone(),
        total_size: None,
        accepts_ranges: false,
        etag: strong_etag(&headers),
        last_modified: header(&headers, LAST_MODIFIED),
        content_disposition: header_text(&headers, CONTENT_DISPOSITION),
        content_type: header(&headers, CONTENT_TYPE),
        body: None,
    };

    match response.status() {
        StatusCode::PARTIAL_CONTENT => {
            let raw = header(&headers, CONTENT_RANGE)
                .ok_or_else(|| ProtocolError::Network("206 without Content-Range".to_owned()))?;
            match parse_content_range(&raw).map_err(|e| ProtocolError::Network(e.to_string()))? {
                ContentRange::Satisfied {
                    start: 0, complete, ..
                } => {
                    probe.accepts_ranges = true;
                    probe.total_size = complete;
                }
                _ => {
                    return Err(ProtocolError::Network(format!(
                        "probe asked for byte 0 but got `{raw}`"
                    )));
                }
            }
            // The 1-byte body is discarded with the response.
        }
        StatusCode::OK => {
            probe.total_size = response.content_length();
            probe.body = Some(response);
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            // `bytes=0-0` is unsatisfiable only for an empty resource.
            match header(&headers, CONTENT_RANGE).map(|r| parse_content_range(&r)) {
                Some(Ok(ContentRange::Unsatisfied { complete: 0 })) => {
                    probe.accepts_ranges = true;
                    probe.total_size = Some(0);
                }
                _ => return Err(ProtocolError::HttpStatus { status: 416 }),
            }
        }
        status if status.is_redirection() => {
            // reqwest hands back a redirect it refused to follow (unsafe scheme such as
            // `file:`, or a missing/invalid Location).
            return Err(ProtocolError::Network(format!(
                "redirect not followed (HTTP {}): blocked or unsupported location",
                status.as_u16()
            )));
        }
        status => {
            return Err(ProtocolError::HttpStatus {
                status: status.as_u16(),
            });
        }
    }
    Ok(probe)
}
