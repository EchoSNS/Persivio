//! The "have map": which pieces of a torrent are on disk, saved when a run stops
//! cleanly so the next one can continue without re-reading everything (ADR-023).
//!
//! **Durability (ADR-005):** a map is only [`capture`](HaveMap::capture)d after every
//! file was synced, and carries each file's size and modification time. If anything
//! touched the files since (a crash mid-download, another program), they differ, the
//! map is ignored, and librqbit re-checks every piece as before.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// Bumped when the format changes; other versions are ignored.
const VERSION: u8 = 1;

/// A file as it was when the map was captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    /// Nanoseconds since the Unix epoch, as text: JSON numbers do not hold a `u128`.
    modified: String,
}

/// Which pieces are on disk, and the files that claim rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HaveMap {
    version: u8,
    /// One bit per piece, most significant bit first (the BitTorrent bitfield order).
    bits: Vec<u8>,
    files: Vec<FileStamp>,
}

fn stamp(path: &Path) -> std::io::Result<Option<FileStamp>> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        // Files that were never selected may not exist.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let modified = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos()
        .to_string();
    Ok(Some(FileStamp {
        path: path.to_path_buf(),
        len: meta.len(),
        modified,
    }))
}

impl HaveMap {
    /// Syncs `paths` to disk, then records `bits` for them. Blocking: call it from
    /// `spawn_blocking`.
    pub(crate) fn capture(bits: Vec<u8>, paths: &[PathBuf]) -> std::io::Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            match std::fs::File::options().write(true).open(path) {
                Ok(f) => f.sync_all()?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
            files.extend(stamp(path)?);
        }
        Ok(Self {
            version: VERSION,
            bits,
            files,
        })
    }

    /// The bits, if every file of the map is unchanged and nothing but `paths` is meant.
    /// Blocking.
    pub(crate) fn validated_bits(self, paths: &[PathBuf]) -> Option<Vec<u8>> {
        let mut now = Vec::new();
        for path in paths {
            now.extend(stamp(path).ok()?);
        }
        (self.version == VERSION && now == self.files).then_some(self.bits)
    }

    pub(crate) fn to_bytes(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(self).ok()
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_is_valid_only_while_its_files_are_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let (a, missing) = (dir.path().join("a"), dir.path().join("missing"));
        std::fs::write(&a, b"12345").unwrap();
        let paths = [a.clone(), missing.clone()];
        let map = HaveMap::capture(vec![0b1010_0000], &paths).unwrap();
        let map = HaveMap::from_bytes(&map.to_bytes().unwrap()).unwrap();
        assert_eq!(map.clone().validated_bits(&paths), Some(vec![0b1010_0000]));

        assert_eq!(
            map.clone().validated_bits(std::slice::from_ref(&missing)),
            None,
            "different files"
        );
        std::fs::write(&missing, b"x").unwrap();
        assert_eq!(map.clone().validated_bits(&paths), None, "a file appeared");
        std::fs::remove_file(&missing).unwrap();

        // Same size, different time: something wrote to the file.
        let f = std::fs::File::options().write(true).open(&a).unwrap();
        f.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(map.clone().validated_bits(&paths), None);
        let other = HaveMap {
            version: VERSION + 1,
            ..map
        };
        assert_eq!(other.validated_bits(&paths), None);
        assert_eq!(HaveMap::from_bytes(b"not json"), None);
    }
}
