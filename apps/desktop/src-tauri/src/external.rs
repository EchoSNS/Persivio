//! Requests from outside the app: command-line arguments of the first or a second launch
//! (a magnet link clicked in the browser, a `.torrent` opened from the file manager) and
//! macOS "open URL / open file" events.
//!
//! They are never added directly: the UI opens its Add dialog prefilled, and the user
//! confirms (ADR-020). Any local program can launch Persivio with arguments, and web pages
//! can trigger `magnet:` links.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use specta::Type;
use url::Url;

/// A `.torrent` file read for the Add dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TorrentDraftView {
    /// File name, for display.
    pub name: String,
    /// Base64 of the file's bytes (the same form the UI sends back in `AddSource`).
    pub data: String,
}

/// What an external request asks the UI to add.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ExternalAddView {
    /// `http(s)` and `magnet:` links.
    pub links: Vec<String>,
    /// `.torrent` files.
    pub torrents: Vec<TorrentDraftView>,
    /// Problems to show (unreadable or oversized files).
    pub errors: Vec<String>,
}

impl ExternalAddView {
    /// Whether there is anything to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty() && self.torrents.is_empty() && self.errors.is_empty()
    }
}

/// An argument classified without touching the file system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// An `http(s)://` or `magnet:?` link.
    Link(String),
    /// A path ending in `.torrent` (`file://` URLs included).
    TorrentPath(PathBuf),
}

/// Classifies process arguments (without the program name). Flags (`--minimized`) and
/// anything else are ignored.
pub fn classify_args<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> Vec<Arg> {
    args.into_iter()
        .filter_map(|a| classify(a.as_ref()))
        .collect()
}

fn classify(arg: &str) -> Option<Arg> {
    let arg = arg.trim();
    if arg.starts_with('-') || arg.is_empty() {
        return None;
    }
    if arg
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("magnet:?"))
    {
        return Some(Arg::Link(arg.to_owned()));
    }
    if let Ok(url) = Url::parse(arg) {
        match url.scheme() {
            "http" | "https" => return Some(Arg::Link(url.to_string())),
            "file" => {
                return url
                    .to_file_path()
                    .ok()
                    .filter(|p| is_torrent(p))
                    .map(Arg::TorrentPath);
            }
            // A Windows path like `C:\x.torrent` parses as scheme `c`: fall through.
            s if s.len() > 1 => return None,
            _ => {}
        }
    }
    let path = PathBuf::from(arg);
    is_torrent(&path).then_some(Arg::TorrentPath(path))
}

fn is_torrent(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("torrent"))
}

/// Reads the `.torrent` files and groups everything for the UI.
#[must_use]
pub fn resolve(args: Vec<Arg>) -> ExternalAddView {
    let mut view = ExternalAddView::default();
    for arg in args {
        match arg {
            Arg::Link(link) => {
                if !view.links.contains(&link) {
                    view.links.push(link);
                }
            }
            Arg::TorrentPath(path) => match read_torrent(&path) {
                Ok(draft) => view.torrents.push(draft),
                Err(e) => view.errors.push(e),
            },
        }
    }
    view
}

fn read_torrent(path: &Path) -> Result<TorrentDraftView, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{name}: {e}"))?
        .len();
    if size > pv_engine::MAX_TORRENT_FILE_SIZE as u64 {
        return Err(format!("{name}: the .torrent file is too large"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    Ok(TorrentDraftView {
        name,
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_classified() {
        let tmp = std::env::temp_dir().join("x.torrent");
        let file_url = Url::from_file_path(&tmp).unwrap().to_string();
        let args = classify_args([
            "--minimized",
            "magnet:?xt=urn:btih:abc&dn=a",
            "MAGNET:?xt=urn:btih:def",
            "https://example.com/a.iso",
            "ftp://example.com/a",
            "javascript:alert(1)",
            "C:\\Users\\me\\Downloads\\ubuntu.TORRENT",
            "/home/me/debian.torrent",
            &file_url,
            "notes.txt",
            "",
        ]);
        assert_eq!(
            args,
            [
                Arg::Link("magnet:?xt=urn:btih:abc&dn=a".into()),
                Arg::Link("MAGNET:?xt=urn:btih:def".into()),
                Arg::Link("https://example.com/a.iso".into()),
                Arg::TorrentPath("C:\\Users\\me\\Downloads\\ubuntu.TORRENT".into()),
                Arg::TorrentPath("/home/me/debian.torrent".into()),
                Arg::TorrentPath(tmp),
            ]
        );
    }

    #[test]
    fn torrent_files_are_read_with_a_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("a.torrent");
        std::fs::write(&ok, b"d4:infoe").unwrap();
        let missing = dir.path().join("missing.torrent");
        let view = resolve(vec![
            Arg::Link("https://e.com/a".into()),
            Arg::Link("https://e.com/a".into()),
            Arg::TorrentPath(ok),
            Arg::TorrentPath(missing),
        ]);
        assert_eq!(view.links, ["https://e.com/a"]);
        assert_eq!(view.torrents.len(), 1);
        assert_eq!(view.torrents[0].name, "a.torrent");
        assert_eq!(view.torrents[0].data, "ZDQ6aW5mb2U=");
        assert_eq!(view.errors.len(), 1);
        assert!(
            view.errors[0].starts_with("missing.torrent:"),
            "{:?}",
            view.errors
        );

        let big = dir.path().join("big.torrent");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(pv_engine::MAX_TORRENT_FILE_SIZE as u64 + 1)
            .unwrap();
        assert!(resolve(vec![Arg::TorrentPath(big)]).errors[0].contains("too large"));
        assert!(resolve(vec![]).is_empty());
    }
}
