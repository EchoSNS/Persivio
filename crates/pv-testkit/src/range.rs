//! Server-side `Range` / `If-Range` evaluation (RFC 9110 §14), single range only.
//!
//! Implemented by hand (rather than via `tower-http`'s `ServeDir`) so every route can
//! share it and so the edge cases are explicit and tested.

/// What to send for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOutcome {
    /// `200` with the whole representation.
    Full,
    /// `206` with bytes `start..=end`.
    Partial {
        /// First byte.
        start: u64,
        /// Last byte (inclusive).
        end: u64,
    },
    /// `416` with `Content-Range: bytes */len`.
    Unsatisfiable,
}

/// Decides the response for a `Range` header value against a representation of `len`
/// bytes. Per RFC 9110, a syntactically invalid header, an unknown unit or a
/// multi-range request may be **ignored** (→ [`RangeOutcome::Full`]).
#[must_use]
pub fn evaluate(range: Option<&str>, len: u64) -> RangeOutcome {
    let Some(spec) = range.and_then(parse_bytes_unit) else {
        return RangeOutcome::Full;
    };
    if spec.contains(',') {
        return RangeOutcome::Full;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RangeOutcome::Full;
    };
    let (first, last) = (first.trim(), last.trim());

    if first.is_empty() {
        // Suffix range: last N bytes.
        let Ok(n) = last.parse::<u64>() else {
            return RangeOutcome::Full;
        };
        if n == 0 || len == 0 {
            return RangeOutcome::Unsatisfiable;
        }
        return RangeOutcome::Partial {
            start: len.saturating_sub(n),
            end: len - 1,
        };
    }

    let Ok(start) = first.parse::<u64>() else {
        return RangeOutcome::Full;
    };
    let end = if last.is_empty() {
        None
    } else {
        match last.parse::<u64>() {
            Ok(e) if e >= start => Some(e),
            _ => return RangeOutcome::Full, // e.g. "5-2": invalid → ignore
        }
    };
    if start >= len {
        return RangeOutcome::Unsatisfiable;
    }
    RangeOutcome::Partial {
        start,
        end: end.map_or(len - 1, |e| e.min(len - 1)),
    }
}

fn parse_bytes_unit(header: &str) -> Option<&str> {
    let (unit, spec) = header.split_once('=')?;
    unit.trim().eq_ignore_ascii_case("bytes").then_some(spec)
}

/// Whether an `If-Range` value still matches the current representation, i.e. the
/// range may be honored. Weak `ETag`s never match (RFC 9110 §13.1.5).
#[must_use]
pub fn if_range_matches(if_range: &str, etag: &str, last_modified: &str) -> bool {
    let v = if_range.trim();
    if v.starts_with("W/") {
        false
    } else if v.starts_with('"') {
        v == etag
    } else {
        v == last_modified
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RangeOutcome::{Full, Partial, Unsatisfiable};

    #[test]
    fn range_table() {
        let len = 100;
        let cases: &[(Option<&str>, RangeOutcome)] = &[
            (None, Full),
            (Some("bytes=0-0"), Partial { start: 0, end: 0 }),
            (Some("bytes=10-19"), Partial { start: 10, end: 19 }),
            (Some("bytes=90-"), Partial { start: 90, end: 99 }),
            (Some("bytes=-5"), Partial { start: 95, end: 99 }),
            (Some("bytes=-500"), Partial { start: 0, end: 99 }),
            (Some("bytes=50-1000"), Partial { start: 50, end: 99 }),
            (Some("bytes=99-99"), Partial { start: 99, end: 99 }),
            (Some("BYTES = 1-2"), Partial { start: 1, end: 2 }),
            (Some("bytes=100-"), Unsatisfiable),
            (Some("bytes=100-200"), Unsatisfiable),
            (Some("bytes=-0"), Unsatisfiable),
            (Some("bytes=5-2"), Full),
            (Some("bytes=0-1,4-5"), Full),
            (Some("items=0-1"), Full),
            (Some("bytes=abc"), Full),
            (Some("bytes=a-1"), Full),
            (Some("bytes=-x"), Full),
            (Some("bytes=1-x"), Full),
            (Some("garbage"), Full),
        ];
        for &(header, expected) in cases {
            assert_eq!(evaluate(header, len), expected, "{header:?}");
        }
    }

    #[test]
    fn empty_representation_cannot_satisfy_ranges() {
        assert_eq!(evaluate(Some("bytes=0-0"), 0), Unsatisfiable);
        assert_eq!(evaluate(Some("bytes=-1"), 0), Unsatisfiable);
        assert_eq!(evaluate(None, 0), Full);
    }

    #[test]
    fn if_range_rules() {
        let (etag, lm) = ("\"abc\"", "Wed, 21 Oct 2015 07:28:00 GMT");
        assert!(if_range_matches("\"abc\"", etag, lm));
        assert!(!if_range_matches("\"xyz\"", etag, lm));
        assert!(
            !if_range_matches("W/\"abc\"", etag, lm),
            "weak never matches"
        );
        assert!(if_range_matches(lm, etag, lm));
        assert!(!if_range_matches("Thu, 22 Oct 2015 07:28:00 GMT", etag, lm));
    }
}
