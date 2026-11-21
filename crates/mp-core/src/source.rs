//! Where a download comes from.

use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::task::TaskKind;

/// Header names whose values are secrets and must never appear in logs.
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "x-api-key",
    "x-auth-token",
];

/// Placeholder printed instead of secret values.
const REDACTED: &str = "<redacted>";

/// The origin of a download task.
///
/// `Debug` is implemented by hand so cookies, credential headers and URL passwords are
/// **redacted**: it is safe to log a `Source` with `{:?}` (security checklist §11).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// An HTTP or HTTPS URL, with optional request context (often forwarded by the
    /// browser extension so downloads behind a login work).
    Http {
        /// The URL to download. Must use the `http` or `https` scheme.
        url: Url,
        /// Extra request headers as `(name, value)` pairs.
        #[serde(default)]
        headers: Vec<(String, String)>,
        /// A `Cookie` header value to send.
        #[serde(default)]
        cookies: Option<String>,
        /// The `Referer` to send.
        #[serde(default)]
        referer: Option<Url>,
    },
    /// A `magnet:?` URI.
    Magnet {
        /// The full magnet URI.
        uri: String,
    },
    /// The raw bytes of a `.torrent` file (stored hex-encoded in JSON).
    TorrentFile {
        /// Bencoded torrent metainfo.
        #[serde(with = "hex_bytes")]
        bytes: Vec<u8>,
    },
}

/// Why a [`Source`] is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// HTTP sources must be `http://` or `https://` (no `file:`, `ftp:`, `javascript:` ...).
    #[error("unsupported URL scheme `{0}` (only http and https are allowed)")]
    UnsupportedScheme(String),
    /// The referer, if given, must also be http(s).
    #[error("unsupported referer scheme `{0}`")]
    UnsupportedRefererScheme(String),
    /// Header names/values must not contain control characters (header injection).
    #[error("invalid header `{0}`")]
    InvalidHeader(String),
    /// Not a `magnet:?` URI.
    #[error("not a magnet URI")]
    InvalidMagnet,
    /// An empty `.torrent` payload.
    #[error("torrent file is empty")]
    EmptyTorrent,
}

impl Source {
    /// Builds a plain HTTP(S) source with no extra headers, validating the scheme.
    ///
    /// # Errors
    /// [`SourceError::UnsupportedScheme`] for anything but `http`/`https`.
    pub fn http(url: Url) -> Result<Self, SourceError> {
        let source = Self::Http {
            url,
            headers: Vec::new(),
            cookies: None,
            referer: None,
        };
        source.validate()?;
        Ok(source)
    }

    /// Builds a magnet source, validating the `magnet:?` prefix.
    ///
    /// # Errors
    /// [`SourceError::InvalidMagnet`] if the URI does not start with `magnet:?`.
    pub fn magnet(uri: impl Into<String>) -> Result<Self, SourceError> {
        let source = Self::Magnet { uri: uri.into() };
        source.validate()?;
        Ok(source)
    }

    /// Checks invariants that serde cannot enforce. Call on every source that crosses a
    /// trust boundary (IPC, browser bridge, database).
    ///
    /// # Errors
    /// The first [`SourceError`] found.
    pub fn validate(&self) -> Result<(), SourceError> {
        match self {
            Self::Http {
                url,
                headers,
                cookies,
                referer,
            } => {
                if !is_http_scheme(url) {
                    return Err(SourceError::UnsupportedScheme(url.scheme().to_owned()));
                }
                if let Some(referer) = referer
                    && !is_http_scheme(referer)
                {
                    return Err(SourceError::UnsupportedRefererScheme(
                        referer.scheme().to_owned(),
                    ));
                }
                for (name, value) in headers {
                    if name.is_empty()
                        || !name.bytes().all(|b| b.is_ascii_graphic() && b != b':')
                        || value.bytes().any(|b| b.is_ascii_control() && b != b'\t')
                    {
                        return Err(SourceError::InvalidHeader(name.clone()));
                    }
                }
                if cookies
                    .as_deref()
                    .is_some_and(|c| c.bytes().any(|b| b.is_ascii_control()))
                {
                    return Err(SourceError::InvalidHeader("cookie".to_owned()));
                }
                Ok(())
            }
            Self::Magnet { uri } => {
                if uri.len() > "magnet:?".len()
                    && uri
                        .get(..8)
                        .is_some_and(|p| p.eq_ignore_ascii_case("magnet:?"))
                {
                    Ok(())
                } else {
                    Err(SourceError::InvalidMagnet)
                }
            }
            Self::TorrentFile { bytes } => {
                if bytes.is_empty() {
                    Err(SourceError::EmptyTorrent)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Which protocol family handles this source.
    #[must_use]
    pub const fn kind(&self) -> TaskKind {
        match self {
            Self::Http { .. } => TaskKind::Http,
            Self::Magnet { .. } | Self::TorrentFile { .. } => TaskKind::Torrent,
        }
    }
}

fn is_http_scheme(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Formats a URL with any password replaced, for logs.
fn redact_url(url: &Url) -> String {
    if url.password().is_some() {
        let mut copy = url.clone();
        // Cannot fail: a URL that has a password also has a host.
        let _ = copy.set_password(Some(REDACTED));
        copy.to_string()
    } else {
        url.to_string()
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http {
                url,
                headers,
                cookies,
                referer,
            } => {
                let headers: Vec<(&str, &str)> = headers
                    .iter()
                    .map(|(name, value)| {
                        let sensitive = SENSITIVE_HEADERS
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(name));
                        (
                            name.as_str(),
                            if sensitive { REDACTED } else { value.as_str() },
                        )
                    })
                    .collect();
                f.debug_struct("Http")
                    .field("url", &redact_url(url))
                    .field("headers", &headers)
                    .field("cookies", &cookies.as_ref().map(|_| REDACTED))
                    .field("referer", &referer.as_ref().map(redact_url))
                    .finish()
            }
            Self::Magnet { uri } => f.debug_struct("Magnet").field("uri", uri).finish(),
            Self::TorrentFile { bytes } => f
                .debug_struct("TorrentFile")
                .field("len", &bytes.len())
                .finish(),
        }
    }
}

/// Serde helper storing bytes as a lowercase hex string (compact and JSON-friendly,
/// unlike serde's default array-of-numbers).
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    // serde's `with` contract requires `&Vec<u8>` here.
    #[allow(clippy::ptr_arg, reason = "signature fixed by serde `with`")]
    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from(DIGITS[usize::from(b >> 4)]));
            out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
        }
        serializer.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s.len() % 2 != 0 {
            return Err(D::Error::custom("hex string has odd length"));
        }
        s.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let hi = nibble(pair[0]).ok_or_else(|| D::Error::custom("invalid hex digit"))?;
                let lo = nibble(pair[1]).ok_or_else(|| D::Error::custom("invalid hex digit"))?;
                Ok((hi << 4) | lo)
            })
            .collect()
    }

    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn http_constructor_accepts_only_http_schemes() {
        assert!(Source::http(url("https://example.com/a.iso")).is_ok());
        assert!(Source::http(url("http://example.com/a.iso")).is_ok());
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "javascript:alert(1)",
        ] {
            assert!(
                matches!(
                    Source::http(url(bad)),
                    Err(SourceError::UnsupportedScheme(_))
                ),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn validate_rejects_bad_referer_headers_and_cookies() {
        let mk =
            |headers: Vec<(String, String)>, cookies: Option<&str>, referer: &str| Source::Http {
                url: url("https://example.com/f"),
                headers,
                cookies: cookies.map(str::to_owned),
                referer: Some(url(referer)),
            };
        assert_eq!(
            mk(vec![], None, "file:///x").validate(),
            Err(SourceError::UnsupportedRefererScheme("file".into()))
        );
        let injected = vec![("X-Test".into(), "a\r\nEvil: 1".into())];
        assert!(matches!(
            mk(injected, None, "https://r.example").validate(),
            Err(SourceError::InvalidHeader(_))
        ));
        let bad_name = vec![("Bad Name".into(), "v".into())];
        assert!(mk(bad_name, None, "https://r.example").validate().is_err());
        assert!(
            mk(vec![], Some("a=1\nb=2"), "https://r.example")
                .validate()
                .is_err()
        );
        let ok = vec![("Accept".into(), "*/*".into())];
        assert!(
            mk(ok, Some("a=1; b=2"), "https://r.example")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn magnet_and_torrent_validation() {
        assert!(Source::magnet("magnet:?xt=urn:btih:abc").is_ok());
        assert!(Source::magnet("MAGNET:?xt=urn:btih:abc").is_ok());
        assert_eq!(Source::magnet("magnet:?"), Err(SourceError::InvalidMagnet));
        assert_eq!(Source::magnet("http://x"), Err(SourceError::InvalidMagnet));
        assert_eq!(
            Source::TorrentFile { bytes: vec![] }.validate(),
            Err(SourceError::EmptyTorrent)
        );
        assert!(Source::TorrentFile { bytes: vec![1] }.validate().is_ok());
    }

    #[test]
    fn kind_maps_sources_to_protocol_families() {
        assert_eq!(
            Source::http(url("https://e.x/f")).unwrap().kind(),
            TaskKind::Http
        );
        assert_eq!(
            Source::magnet("magnet:?xt=1").unwrap().kind(),
            TaskKind::Torrent
        );
        assert_eq!(
            Source::TorrentFile { bytes: vec![1] }.kind(),
            TaskKind::Torrent
        );
    }

    #[test]
    fn debug_redacts_secrets() {
        let source = Source::Http {
            url: url("https://user:hunter2@example.com/f"),
            headers: vec![
                ("Authorization".into(), "Bearer SECRET-TOKEN".into()),
                ("Accept".into(), "text/plain".into()),
            ],
            cookies: Some("session=SECRET-COOKIE".into()),
            referer: Some(url("https://u:refpass@example.com/")),
        };
        let printed = format!("{source:?}");
        for secret in ["hunter2", "SECRET-TOKEN", "SECRET-COOKIE", "refpass"] {
            assert!(!printed.contains(secret), "{secret} leaked: {printed}");
        }
        assert!(
            printed.contains("text/plain"),
            "non-sensitive headers stay visible"
        );
        assert!(printed.contains("example.com"));
        let torrent = format!("{:?}", Source::TorrentFile { bytes: vec![0; 3] });
        assert!(torrent.contains("len: 3"));
        assert!(format!("{:?}", Source::magnet("magnet:?xt=1").unwrap()).contains("magnet:?xt=1"));
        let plain = format!("{:?}", Source::http(url("https://e.x/f")).unwrap());
        assert!(plain.contains("cookies: None"));
    }

    #[test]
    fn serde_round_trip_all_variants() {
        let sources = [
            Source::Http {
                url: url("https://example.com/a b.iso"),
                headers: vec![("X-A".into(), "1".into())],
                cookies: Some("c=1".into()),
                referer: Some(url("https://ref.example/")),
            },
            Source::magnet("magnet:?xt=urn:btih:abc").unwrap(),
            Source::TorrentFile {
                bytes: vec![0x00, 0x7f, 0xff, 0xab],
            },
        ];
        for source in sources {
            let json = serde_json::to_string(&source).unwrap();
            let back: Source = serde_json::from_str(&json).unwrap();
            assert_eq!(back, source);
        }
    }

    #[test]
    fn torrent_bytes_are_hex_in_json() {
        let json = serde_json::to_string(&Source::TorrentFile {
            bytes: vec![0xde, 0xad, 0x01],
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"torrent_file","bytes":"dead01"}"#);
        let upper: Source =
            serde_json::from_str(r#"{"type":"torrent_file","bytes":"DEAD01"}"#).unwrap();
        assert_eq!(
            upper,
            Source::TorrentFile {
                bytes: vec![0xde, 0xad, 0x01]
            }
        );
        for bad in [r#""abc""#, r#""zz""#, "123"] {
            let doc = format!(r#"{{"type":"torrent_file","bytes":{bad}}}"#);
            assert!(serde_json::from_str::<Source>(&doc).is_err(), "{bad}");
        }
    }

    #[test]
    fn http_optional_fields_default_when_missing() {
        let s: Source = serde_json::from_str(r#"{"type":"http","url":"https://e.x/f"}"#).unwrap();
        assert_eq!(s, Source::http(url("https://e.x/f")).unwrap());
    }
}
