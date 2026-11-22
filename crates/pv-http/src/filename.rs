//! File name resolution, sanitization, containment and de-duplication.
//!
//! Every name that comes from a server (or a user) passes through
//! [`sanitize_filename`] and then [`contained_path`] before any file is created. A
//! malicious server can send `filename="../../.bashrc"`; after this module it becomes
//! `bashrc` inside the destination directory.

use std::io;
use std::path::Path;

use percent_encoding::percent_decode_str;
use pv_core::filename::split_extension;
pub use pv_core::filename::{
    FALLBACK_NAME, INVALID_CHARS, MAX_NAME_BYTES, contained_path, sanitize_filename,
};

use url::Url;

use crate::disposition::filename_from_disposition;

/// Suffix of in-progress files (ADR-008).
pub const PART_SUFFIX: &str = ".part";

/// Last non-empty path segment of a URL, percent-decoded (lossy for invalid UTF-8).
#[must_use]
pub fn name_from_url(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode_str(segment).decode_utf8_lossy().into_owned();
    (!decoded.trim().is_empty()).then_some(decoded)
}

/// A file extension for common `Content-Type`s (used only for the fallback name).
#[must_use]
pub fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    Some(match mime.as_str() {
        "application/zip" => ".zip",
        "application/pdf" => ".pdf",
        "application/gzip" | "application/x-gzip" => ".gz",
        "application/x-bittorrent" => ".torrent",
        "application/x-iso9660-image" => ".iso",
        "application/json" => ".json",
        "text/plain" => ".txt",
        "text/html" => ".html",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "video/mp4" => ".mp4",
        "audio/mpeg" => ".mp3",
        _ => return None,
    })
}

/// Picks a file name using the manual guide's priority order (M4.3), then sanitizes it:
/// 1. user-specified name, 2. `Content-Disposition` (`filename*` before `filename`),
/// 3. last segment of the **final** URL, 4. `download` + extension from `Content-Type`.
#[must_use]
pub fn resolve_filename(
    user: Option<&str>,
    content_disposition: Option<&str>,
    final_url: &Url,
    content_type: Option<&str>,
) -> String {
    let chosen = user
        .map(str::to_owned)
        .or_else(|| content_disposition.and_then(filename_from_disposition))
        .or_else(|| name_from_url(final_url))
        .unwrap_or_else(|| {
            let ext = content_type
                .and_then(extension_for_content_type)
                .unwrap_or("");
            format!("{FALLBACK_NAME}{ext}")
        });
    sanitize_filename(&chosen)
}

/// `name` + [`PART_SUFFIX`].
#[must_use]
pub fn part_name(name: &str) -> String {
    format!("{name}{PART_SUFFIX}")
}

/// Returns `name`, or `stem (n).ext` for the smallest `n` such that no file with that
/// name exists in `dir` yet. Existing files are never overwritten.
///
/// # Errors
/// I/O errors while checking, or if 9999 candidates are all taken.
pub async fn unique_name(dir: &Path, name: &str) -> io::Result<String> {
    if !tokio::fs::try_exists(dir.join(name)).await? {
        return Ok(name.to_owned());
    }
    let (stem, ext) = split_extension(name);
    for n in 1..=9999 {
        let candidate = format!("{stem} ({n}){ext}");
        if !tokio::fs::try_exists(dir.join(&candidate)).await? {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free name for `{name}`"),
    ))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn sanitize_examples() {
        let cases = [
            ("report.pdf", "report.pdf"),
            ("../../.bashrc", "bashrc"),
            ("..\\..\\Windows\\System32\\cmd.exe", "cmd.exe"),
            ("C:\\evil.txt", "evil.txt"),
            ("/etc/passwd", "passwd"),
            ("dir/", "dir"),
            ("a<b>c:d\"e|f?g*h.txt", "a_b_c_d_e_f_g_h.txt"),
            ("tab\there\n.txt", "tab_here_.txt"),
            ("  spaced name.txt  ", "spaced name.txt"),
            ("trailing dots...", "trailing dots"),
            ("...", FALLBACK_NAME),
            ("..", FALLBACK_NAME),
            ("", FALLBACK_NAME),
            ("   ", FALLBACK_NAME),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("LPT9.tar.gz", "_LPT9.tar.gz"),
            ("NUL .txt", "_NUL .txt"),
            ("CONSOLE.txt", "CONSOLE.txt"),
            ("evil\u{202E}fdp.exe", "evilfdp.exe"),
            ("日本語ファイル.zip", "日本語ファイル.zip"),
            ("naïve café.txt", "naïve café.txt"),
        ];
        for (raw, expected) in cases {
            assert_eq!(sanitize_filename(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = format!("{}.iso", "a".repeat(500));
        let s = sanitize_filename(&long);
        assert_eq!(s.len(), MAX_NAME_BYTES);
        assert_eq!(&s[s.len() - 4..], ".iso");
        // Multi-byte chars are never split.
        let wide = format!("{}.txt", "日".repeat(200));
        let s = sanitize_filename(&wide);
        assert!(s.len() <= MAX_NAME_BYTES);
        assert_eq!(&s[s.len() - 4..], ".txt");
        // A huge "extension" is not preserved.
        let weird = format!("x.{}", "e".repeat(300));
        assert!(sanitize_filename(&weird).len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn url_and_content_type_fallbacks() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(
            name_from_url(&u("https://e.x/a/b/file%20name.iso")).as_deref(),
            Some("file name.iso")
        );
        assert_eq!(
            name_from_url(&u("https://e.x/dir/")).as_deref(),
            Some("dir")
        );
        assert_eq!(name_from_url(&u("https://e.x/")), None);
        assert_eq!(
            name_from_url(&u("https://e.x/%FF.bin")).as_deref(),
            Some("\u{FFFD}.bin")
        );
        assert_eq!(
            extension_for_content_type("application/zip; charset=binary"),
            Some(".zip")
        );
        assert_eq!(extension_for_content_type("TEXT/PLAIN"), Some(".txt"));
        assert_eq!(extension_for_content_type("application/x-unknown"), None);
    }

    #[test]
    fn resolve_priority_order() {
        let url = Url::parse("https://e.x/path/from-url.bin").unwrap();
        let root = Url::parse("https://e.x/").unwrap();
        let cd = Some("attachment; filename=from-cd.bin; filename*=UTF-8''from-cd-ext.bin");
        assert_eq!(
            resolve_filename(Some("user.bin"), cd, &url, None),
            "user.bin"
        );
        assert_eq!(resolve_filename(None, cd, &url, None), "from-cd-ext.bin");
        assert_eq!(
            resolve_filename(None, Some("attachment; filename=plain.bin"), &url, None),
            "plain.bin"
        );
        assert_eq!(
            resolve_filename(None, Some("inline"), &url, None),
            "from-url.bin"
        );
        assert_eq!(
            resolve_filename(None, None, &root, Some("application/pdf")),
            "download.pdf"
        );
        assert_eq!(resolve_filename(None, None, &root, None), "download");
        // User names are sanitized too.
        assert_eq!(
            resolve_filename(Some("../x.bin"), None, &url, None),
            "x.bin"
        );
    }

    #[test]
    fn containment_rejects_anything_but_one_component() {
        let dir = std::env::temp_dir();
        assert_eq!(contained_path(&dir, "ok.bin").unwrap(), dir.join("ok.bin"));
        for bad in ["", ".", "..", "a/b", "../x", "/abs"] {
            assert!(contained_path(&dir, bad).is_err(), "{bad:?}");
        }
        #[cfg(windows)]
        for bad in ["a\\b", "C:x", "C:\\x"] {
            assert!(contained_path(&dir, bad).is_err(), "{bad:?}");
        }
        assert_eq!(part_name("a.iso"), "a.iso.part");
    }

    #[tokio::test]
    async fn unique_name_never_reuses_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(unique_name(d, "a.tar.gz").await.unwrap(), "a.tar.gz");
        tokio::fs::write(d.join("a.tar.gz"), b"x").await.unwrap();
        assert_eq!(unique_name(d, "a.tar.gz").await.unwrap(), "a.tar (1).gz");
        tokio::fs::write(d.join("a.tar (1).gz"), b"x")
            .await
            .unwrap();
        assert_eq!(unique_name(d, "a.tar.gz").await.unwrap(), "a.tar (2).gz");
        tokio::fs::write(d.join("noext"), b"x").await.unwrap();
        assert_eq!(unique_name(d, "noext").await.unwrap(), "noext (1)");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        /// The core security property: whatever a server sends, the result is one
        /// harmless path component that stays inside the destination directory.
        #[test]
        fn sanitized_names_never_escape(raw in any::<String>()) {
            let dest = std::env::temp_dir().join("persivio-dest");
            let name = sanitize_filename(&raw);
            prop_assert!(!name.is_empty());
            prop_assert!(name.len() <= MAX_NAME_BYTES);
            prop_assert!(!name.contains(['/', '\\']));
            prop_assert!(name != "." && name != "..");
            prop_assert!(!name.chars().any(|c| c.is_control() || INVALID_CHARS.contains(&c)));
            let path = contained_path(&dest, &name).unwrap();
            prop_assert_eq!(path.parent(), Some(dest.as_path()));
            prop_assert_eq!(path.file_name(), Some(OsStr::new(&name)));
            prop_assert_eq!(sanitize_filename(&name), name.clone(), "idempotent");
        }

        /// Same property with inputs biased toward traversal tricks.
        #[test]
        fn traversal_shaped_inputs(parts in prop::collection::vec(
            prop_oneof![Just("..".to_owned()), Just(".".to_owned()), Just(String::new()),
                        Just("CON".to_owned()), "[a-z\u{202E}.: ]{0,6}"], 0..6),
            sep in prop_oneof![Just("/"), Just("\\")]
        ) {
            let raw = parts.join(sep);
            let dest = std::env::temp_dir();
            let name = sanitize_filename(&raw);
            let path = contained_path(&dest, &name).unwrap();
            prop_assert_eq!(path.parent(), Some(dest.as_path()));
        }
    }
}
