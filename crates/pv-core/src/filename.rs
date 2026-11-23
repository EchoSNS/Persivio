//! Pure file-name safety rules shared by every protocol (HTTP and BitTorrent).
//!
//! Every name that comes from a server, a torrent or a user passes through
//! [`sanitize_filename`] and then [`contained_path`] before any file is created. A
//! malicious server can send `filename="../../.bashrc"`; after this module it becomes
//! `bashrc` inside the destination directory.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::protocol::ProtocolError;

/// Longest sanitized name in bytes. Kept well below the common 255-byte limit so that
/// a de-duplication suffix (` (9999)`) and `.part` still fit.
pub const MAX_NAME_BYTES: usize = 200;

/// Name used when nothing better is available.
pub const FALLBACK_NAME: &str = "download";

/// Device names that Windows refuses as file names, with or without an extension.
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4",
    "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Characters invalid in Windows file names (we apply the strictest platform everywhere,
/// so a file downloaded on Linux can be copied to Windows).
pub const INVALID_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Invisible bidirectional-control characters. `U+202E` can make `evil<U+202E>fdp.exe` display
/// as `evilexe.pdf`, so these are removed outright.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Turns an untrusted name into a safe single path component.
///
/// Guarantees (checked by property tests): the result is non-empty, at most
/// [`MAX_NAME_BYTES`] bytes, contains no path separators, control characters or
/// Windows-invalid characters, is not `.`/`..`, is not a Windows reserved device name,
/// and `sanitize_filename(sanitize_filename(x)) == sanitize_filename(x)`.
#[must_use]
pub fn sanitize_filename(raw: &str) -> String {
    // Only the last path component survives: kills `../../x` and `C:\x`.
    let last = raw
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or("");
    let replaced: String = last
        .chars()
        .filter(|&c| !is_bidi_control(c))
        .map(|c| {
            if c.is_control() || INVALID_CHARS.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Leading dots would create hidden files (or `..`); Windows strips trailing dots/spaces.
    let mut name = trim_name(&replaced).to_owned();
    if name.len() > MAX_NAME_BYTES {
        name = truncate_keeping_extension(&name, MAX_NAME_BYTES);
    }
    if name.is_empty() {
        return FALLBACK_NAME.to_owned();
    }
    let stem = name.split('.').next().unwrap_or("");
    if WINDOWS_RESERVED
        .iter()
        .any(|r| r.eq_ignore_ascii_case(stem.trim_end()))
    {
        name.insert(0, '_');
        // The prefix can push a truncated name one byte over the limit. Truncating again
        // keeps the `_`, so the stem can no longer be a reserved name.
        if name.len() > MAX_NAME_BYTES {
            name = truncate_keeping_extension(&name, MAX_NAME_BYTES);
        }
    }
    name
}

fn trim_name(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '.')
}

/// Shortens `name` to at most `max` bytes on a char boundary, keeping a short extension.
fn truncate_keeping_extension(name: &str, max: usize) -> String {
    let (stem, ext) = split_extension(name);
    let ext = if ext.len() <= 16 { ext } else { "" };
    let budget = max.saturating_sub(ext.len());
    let mut cut = budget.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    let stem = trim_name(&stem[..cut]);
    let joined = format!("{stem}{ext}");
    trim_name(&joined).to_owned()
}

/// Splits `archive.tar.gz` into (`archive.tar`, `.gz`); names without a dot (or with
/// only a leading dot) have no extension.
#[must_use]
pub fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

/// Joins `name` onto `dir`, refusing anything that is not exactly one normal path
/// component (defense in depth on top of [`sanitize_filename`]).
///
/// # Errors
/// [`ProtocolError::InvalidSource`] if the name could escape `dir`.
pub fn contained_path(dir: &Path, name: &str) -> Result<PathBuf, ProtocolError> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(c)), None) if c == OsStr::new(name) => Ok(dir.join(name)),
        _ => Err(ProtocolError::InvalidSource(format!(
            "unsafe file name `{name}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (found by the `sanitize_filename` fuzzer): a long name whose truncated
    /// stem is a Windows device name got its `_` prefix after truncation, ending one byte
    /// over the limit.
    #[test]
    fn reserved_names_stay_within_the_length_limit_after_truncation() {
        let fuzz_input: Vec<u8> = [
            b"coN.\xa6\0\0".as_slice(),
            &[0xff; 45],
            &[0; 60],
            &[0xff; 69],
            b"oN.\xa6\0\0",
        ]
        .concat();
        let cases = [
            format!("CON.{}", "x".repeat(300)),
            format!("nul {}.txt", " ".repeat(400)),
            String::from_utf8_lossy(&fuzz_input).into_owned(),
        ];
        for raw in cases {
            let name = sanitize_filename(&raw);
            assert!(
                name.len() <= MAX_NAME_BYTES,
                "{} bytes: {name:?}",
                name.len()
            );
            assert_ne!(name, "");
            let stem = name.split('.').next().unwrap_or("");
            assert!(
                !WINDOWS_RESERVED
                    .iter()
                    .any(|r| r.eq_ignore_ascii_case(stem.trim_end())),
                "{name:?}"
            );
            assert_eq!(sanitize_filename(&name), name, "idempotent");
        }
    }
}
