//! Extracting the suggested file name from `Content-Disposition` (RFC 6266 + RFC 8187).
//!
//! The result is **untrusted**: it must still go through
//! [`crate::filename::sanitize_filename`] before touching the file system.

use percent_encoding::percent_decode_str;

/// Returns the file name suggested by a `Content-Disposition` header value, preferring
/// the RFC 8187 `filename*` form (which carries a charset, so non-ASCII names survive)
/// over the plain `filename` parameter.
#[must_use]
pub fn filename_from_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for (name, val) in parse_params(value) {
        if name.eq_ignore_ascii_case("filename*") {
            if extended.is_none() {
                extended = decode_ext_value(&val);
            }
        } else if name.eq_ignore_ascii_case("filename") && plain.is_none() {
            plain = Some(val);
        }
    }
    extended.or(plain).filter(|name| !name.trim().is_empty())
}

/// Decodes an RFC 8187 `ext-value`: `charset'[language]'percent-encoded`.
/// Only UTF-8 and ISO-8859-1 are supported (the two the RFC requires).
fn decode_ext_value(value: &str) -> Option<String> {
    let (charset, rest) = value.split_once('\'')?;
    let (_language, encoded) = rest.split_once('\'')?;
    let bytes: Vec<u8> = percent_decode_str(encoded).collect();
    if charset.eq_ignore_ascii_case("utf-8") {
        String::from_utf8(bytes).ok()
    } else if charset.eq_ignore_ascii_case("iso-8859-1") {
        // Latin-1 bytes map 1:1 to the first 256 Unicode code points.
        Some(bytes.into_iter().map(char::from).collect())
    } else {
        None
    }
}

/// Splits `type; a=b; c="quoted \"value\""` into `(name, value)` pairs, honoring quoted
/// strings (so a `;` inside quotes does not end the parameter). Malformed parameters are
/// skipped rather than failing the whole header.
fn parse_params(input: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    // Skip the disposition type (`attachment` / `inline`).
    for c in chars.by_ref() {
        if c == ';' {
            break;
        }
    }
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ';').is_some() {}
        let mut name = String::new();
        while let Some(c) = chars.next_if(|c| *c != '=' && *c != ';') {
            name.push(c);
        }
        let name = name.trim().to_owned();
        if chars.next_if_eq(&'=').is_none() {
            if chars.peek().is_none() {
                break;
            }
            continue; // `;` with no value: skip this parameter
        }
        while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'"').is_some() {
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(escaped) = chars.next() {
                            value.push(escaped);
                        }
                    }
                    '"' => break,
                    _ => value.push(c),
                }
            }
            // Ignore anything between the closing quote and the next `;`.
            while chars.next_if(|c| *c != ';').is_some() {}
        } else {
            while let Some(c) = chars.next_if(|c| *c != ';') {
                value.push(c);
            }
            // Leading blanks were skipped above; drop trailing ones.
            value.truncate(value.trim_end().len());
        }
        if !name.is_empty() {
            out.push((name, value));
        }
        if chars.peek().is_none() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(3000))]
        #[test]
        fn never_panics(s in "(?s).{0,60}", q in "[a-z*=;\"\'% ]{0,40}") {
            let _ = filename_from_disposition(&s);
            let _ = filename_from_disposition(&format!("attachment; {q}"));
        }
    }

    #[test]
    fn plain_and_quoted_filenames() {
        let cases = [
            ("attachment; filename=report.pdf", Some("report.pdf")),
            (
                "attachment; filename=\"my report.pdf\"",
                Some("my report.pdf"),
            ),
            ("attachment;filename=\"a;b.txt\"", Some("a;b.txt")),
            (
                r#"attachment; filename="say \"hi\".txt""#,
                Some(r#"say "hi".txt"#),
            ),
            ("ATTACHMENT; FILENAME=Up.TXT", Some("Up.TXT")),
            ("inline", None),
            ("attachment; filename=\"\"", None),
            ("attachment; filename=   ", None),
            ("attachment; size=10; filename=x.bin", Some("x.bin")),
            ("attachment; novalue; filename=y.bin", Some("y.bin")),
            ("", None),
            ("attachment; filename=\"unterminated", Some("unterminated")),
        ];
        for (header, expected) in cases {
            assert_eq!(
                filename_from_disposition(header).as_deref(),
                expected,
                "{header}"
            );
        }
    }

    #[test]
    fn rfc8187_extended_names_win() {
        let cases = [
            (
                "attachment; filename=\"fallback.txt\"; filename*=UTF-8''na%C3%AFve%20file.txt",
                Some("naïve file.txt"),
            ),
            // Order does not matter: filename* still wins.
            (
                "attachment; filename*=utf-8'en'%E6%97%A5%E6%9C%AC.zip; filename=ascii.zip",
                Some("日本.zip"),
            ),
            (
                "attachment; filename*=ISO-8859-1''caf%E9.txt",
                Some("café.txt"),
            ),
            // Unsupported charset or invalid UTF-8: fall back to `filename`.
            (
                "attachment; filename=f.txt; filename*=KOI8-R''%E1",
                Some("f.txt"),
            ),
            (
                "attachment; filename=g.txt; filename*=UTF-8''%FF%FE",
                Some("g.txt"),
            ),
            ("attachment; filename*=UTF-8''", None),
            ("attachment; filename*=no-quotes", None),
        ];
        for (header, expected) in cases {
            assert_eq!(
                filename_from_disposition(header).as_deref(),
                expected,
                "{header}"
            );
        }
    }

    #[test]
    fn traversal_is_returned_raw_for_the_sanitizer_to_handle() {
        // This module only parses; sanitization is a separate, mandatory step.
        assert_eq!(
            filename_from_disposition("attachment; filename=\"../../.bashrc\"").as_deref(),
            Some("../../.bashrc")
        );
    }
}
