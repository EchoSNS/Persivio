//! Where torrent files go, and the checks that keep them inside the destination.

use std::path::{Path, PathBuf};

use librqbit::ValidatedTorrentMetaV1Info;
use pv_core::ProtocolError;
use pv_core::filename::{contained_path, split_extension};

/// Characters that make a name display differently from what it is.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Validates one torrent file path (its components) and returns it joined with `/`.
///
/// Unlike HTTP names, torrent names cannot be rewritten (librqbit writes exactly the
/// names in the metainfo), so instead of sanitizing we **reject** any torrent whose
/// paths could escape the destination or deceive the user:
/// empty / `.` / `..` components, separators inside a component, absolute paths or
/// drive prefixes, control characters, and bidirectional-override characters.
///
/// # Errors
/// [`ProtocolError::InvalidSource`] naming the offending path.
pub fn validate_torrent_path<S: AsRef<str>>(components: &[S]) -> Result<String, ProtocolError> {
    let joined = components
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join("/");
    let unsafe_path =
        || ProtocolError::InvalidSource(format!("torrent contains an unsafe path `{joined}`"));
    if components.is_empty() {
        return Err(unsafe_path());
    }
    for c in components.iter().map(AsRef::as_ref) {
        if c.is_empty()
            || c == "."
            || c == ".."
            || c.contains(['/', '\\'])
            || c.chars().any(|ch| ch.is_control() || is_bidi_control(ch))
            || contained_path(Path::new("."), c).is_err()
        {
            return Err(unsafe_path());
        }
    }
    Ok(joined)
}

/// The on-disk layout of one torrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Name shown to the user and persisted as the task's file name: the single file,
    /// or the folder holding a multi-file torrent.
    pub root_name: String,
    /// `dest/root_name`: what `Outcome::Completed` points at.
    pub root_path: PathBuf,
    /// The folder librqbit writes the torrent's relative paths into.
    pub output_folder: PathBuf,
    /// Validated relative paths (`/`-separated) and sizes, in torrent order.
    pub files: Vec<(String, u64)>,
}

impl Layout {
    /// Decides the layout. On a fresh start an existing file or folder with the same
    /// name is never reused (that could overwrite unrelated data): the torrent gets a
    /// de-duplicated folder instead. On resume, the persisted name is reused.
    pub(crate) async fn from_info<B: AsRef<[u8]>>(
        info: &ValidatedTorrentMetaV1Info<B>,
        persisted_name: Option<&str>,
        dest: &Path,
        resuming: bool,
    ) -> Result<Self, ProtocolError> {
        let mut files = Vec::new();
        for details in info.iter_file_details() {
            let components: Vec<String> = details
                .filename
                .iter_components()
                .map(std::borrow::Cow::into_owned)
                .collect();
            files.push((validate_torrent_path(&components)?, details.len));
        }
        let single = files.len() == 1 && !files[0].0.contains('/');
        let natural = if single {
            files[0].0.clone()
        } else {
            let name = info
                .name()
                .map_or_else(|| "torrent".to_owned(), std::borrow::Cow::into_owned);
            validate_torrent_path(&[name.as_str()])?;
            name
        };

        let root_name = match persisted_name {
            Some(saved) if resuming => {
                validate_torrent_path(&[saved])?;
                saved.to_owned()
            }
            _ => unique(dest, &natural).await?,
        };
        let root_path = dest.join(&root_name);
        // A single file lives directly in `dest` unless it had to be put in its own
        // de-duplicated folder; multi-file torrents always get their folder.
        let output_folder = if single && root_name == natural {
            dest.to_path_buf()
        } else {
            root_path.clone()
        };
        Ok(Self {
            root_name,
            root_path,
            output_folder,
            files,
        })
    }
}

/// `name`, or `stem (n).ext` for the first `n` not present in `dir`.
async fn unique(dir: &Path, name: &str) -> Result<String, ProtocolError> {
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
    Err(ProtocolError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free name for `{name}`"),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths_pass_and_join() {
        assert_eq!(validate_torrent_path(&["a.iso"]).unwrap(), "a.iso");
        assert_eq!(
            validate_torrent_path(&["dir", "sub", "What?.mp3"]).unwrap(),
            "dir/sub/What?.mp3"
        );
        assert_eq!(
            validate_torrent_path(&["日本語", "ファイル.txt"]).unwrap(),
            "日本語/ファイル.txt"
        );
    }

    #[test]
    fn escaping_or_deceptive_paths_are_rejected() {
        let bad: &[&[&str]] = &[
            &[],
            &[""],
            &[".."],
            &["dir", "..", "x"],
            &["."],
            &["a/b"],
            &["a\\b"],
            &["/etc", "passwd"],
            &["tab\there"],
            &["evil\u{202E}fdp.exe"],
        ];
        for path in bad {
            assert!(
                validate_torrent_path(path).is_err(),
                "{path:?} must be rejected"
            );
        }
        #[cfg(windows)]
        assert!(
            validate_torrent_path(&["C:x"]).is_err(),
            "drive-relative path"
        );
    }

    #[tokio::test]
    async fn unique_names_never_reuse_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique(dir.path(), "a.iso").await.unwrap(), "a.iso");
        std::fs::write(dir.path().join("a.iso"), b"x").unwrap();
        assert_eq!(unique(dir.path(), "a.iso").await.unwrap(), "a (1).iso");
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        assert_eq!(unique(dir.path(), "folder").await.unwrap(), "folder (1)");
    }
}
