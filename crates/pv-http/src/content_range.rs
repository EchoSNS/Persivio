//! Strict parser for the `Content-Range` response header (RFC 9110 §14.4).
//!
//! A segment worker trusts this header to know *where* incoming bytes belong, so a
//! lenient parser here would turn a server bug into silent file corruption. Anything
//! unusual is rejected. The parser is fuzzed (`fuzz/fuzz_targets/content_range.rs`).

/// A parsed `Content-Range` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRange {
    /// `bytes start-end/complete` (or `/*` when the total is unknown).
    Satisfied {
        /// First byte offset.
        start: u64,
        /// Last byte offset (inclusive).
        end: u64,
        /// Full representation length, if the server knows it.
        complete: Option<u64>,
    },
    /// `bytes */complete`, sent with `416 Range Not Satisfiable`.
    Unsatisfied {
        /// Full representation length.
        complete: u64,
    },
}

impl ContentRange {
    /// Number of bytes in a satisfied range.
    #[must_use]
    pub fn len(&self) -> u64 {
        match self {
            Self::Satisfied { start, end, .. } => end - start + 1,
            Self::Unsatisfied { .. } => 0,
        }
    }

    /// Whether the range carries no bytes (only for `Unsatisfied`).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Unsatisfied { .. })
    }
}

/// Why a `Content-Range` value was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid Content-Range header: {0}")]
pub struct ContentRangeError(&'static str);

/// Parses a `Content-Range` header value.
///
/// # Errors
/// [`ContentRangeError`] for any syntax error, unknown unit, `start > end`, or an `end`
/// at or beyond the declared total.
pub fn parse_content_range(value: &str) -> Result<ContentRange, ContentRangeError> {
    let value = value.trim();
    let rest = value
        .get(..5)
        .filter(|unit| unit.eq_ignore_ascii_case("bytes"))
        .map(|_| &value[5..])
        .ok_or(ContentRangeError("unit must be `bytes`"))?;
    // The unit must be followed by at least one space.
    if !rest.starts_with(' ') {
        return Err(ContentRangeError("missing space after unit"));
    }
    let rest = rest.trim_start_matches(' ');

    let (range, complete) = rest
        .split_once('/')
        .ok_or(ContentRangeError("missing `/`"))?;
    let complete = match complete {
        "*" => None,
        digits => Some(parse_digits(digits).ok_or(ContentRangeError("invalid complete length"))?),
    };

    if range == "*" {
        return complete
            .map(|complete| ContentRange::Unsatisfied { complete })
            .ok_or(ContentRangeError("`*/*` is not allowed"));
    }

    let (start, end) = range
        .split_once('-')
        .ok_or(ContentRangeError("missing `-` in range"))?;
    let start = parse_digits(start).ok_or(ContentRangeError("invalid range start"))?;
    let end = parse_digits(end).ok_or(ContentRangeError("invalid range end"))?;
    if start > end {
        return Err(ContentRangeError("range start is after range end"));
    }
    if complete.is_some_and(|c| end >= c) {
        return Err(ContentRangeError("range end is beyond the complete length"));
    }
    Ok(ContentRange::Satisfied {
        start,
        end,
        complete,
    })
}

/// ASCII digits only. `str::parse::<u64>` alone would also accept a leading `+`.
fn parse_digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ContentRange::{Satisfied, Unsatisfied};
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(5000))]
        /// Never panics; accepted values are self-consistent (mirrors the fuzz target).
        #[test]
        fn never_panics(s in "(?s).{0,40}", tail in "[0-9*/ -]{0,24}") {
            for input in [s.clone(), format!("bytes {tail}")] {
                if let Ok(Satisfied { start, end, complete }) = parse_content_range(&input) {
                    prop_assert!(start <= end);
                    prop_assert!(complete.is_none_or(|c| end < c));
                }
            }
        }
    }

    #[test]
    fn accepts_valid_forms() {
        let ok: &[(&str, ContentRange)] = &[
            (
                "bytes 0-0/1",
                Satisfied {
                    start: 0,
                    end: 0,
                    complete: Some(1),
                },
            ),
            (
                "bytes 0-499/1234",
                Satisfied {
                    start: 0,
                    end: 499,
                    complete: Some(1234),
                },
            ),
            (
                "bytes 500-999/*",
                Satisfied {
                    start: 500,
                    end: 999,
                    complete: None,
                },
            ),
            (
                "BYTES  7-7/8",
                Satisfied {
                    start: 7,
                    end: 7,
                    complete: Some(8),
                },
            ),
            (
                "  bytes 1-2/3  ",
                Satisfied {
                    start: 1,
                    end: 2,
                    complete: Some(3),
                },
            ),
            ("bytes */1234", Unsatisfied { complete: 1234 }),
            ("bytes */0", Unsatisfied { complete: 0 }),
            (
                "bytes 0-18446744073709551614/18446744073709551615",
                Satisfied {
                    start: 0,
                    end: u64::MAX - 1,
                    complete: Some(u64::MAX),
                },
            ),
        ];
        for &(input, expected) in ok {
            assert_eq!(parse_content_range(input), Ok(expected), "{input}");
        }
        assert_eq!(
            Satisfied {
                start: 2,
                end: 5,
                complete: None
            }
            .len(),
            4
        );
        assert!(Unsatisfied { complete: 1 }.is_empty());
        assert!(
            !Satisfied {
                start: 0,
                end: 0,
                complete: None
            }
            .is_empty()
        );
    }

    #[test]
    fn rejects_malformed_and_inconsistent_values() {
        for bad in [
            "",
            "bytes",
            "bytes ",
            "bytes0-1/2",
            "items 0-1/2",
            "bytes 0-1",
            "bytes 1-0/5",
            "bytes 0-5/5",
            "bytes 0-1/x",
            "bytes +0-1/2",
            "bytes 0-+1/2",
            "bytes 0-1/+2",
            "bytes -1-2/3",
            "bytes 0 - 1/2",
            "bytes */*",
            "bytes 0-1/2/3",
            "bytes 0-18446744073709551616/*",
            "bytés 0-1/2",
        ] {
            assert!(
                parse_content_range(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
        assert_eq!(
            parse_content_range("bytes 1-0/5").unwrap_err().to_string(),
            "invalid Content-Range header: range start is after range end"
        );
    }
}
