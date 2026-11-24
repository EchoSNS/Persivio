//! The data directory shared by every Persivio front end (CLI and desktop app).

use std::fs::File;
use std::path::{Path, PathBuf};

/// Why the data directory cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum DataDirError {
    /// The OS did not report a home/app-data directory.
    #[error("cannot determine the app data directory")]
    NoDefault,
    /// Another Persivio process (the app or the CLI) is using the directory.
    #[error("another Persivio instance is using {}", .0.display())]
    Locked(PathBuf),
    /// The directory or lock file cannot be created.
    #[error("cannot use the data directory {}: {source}", path.display())]
    Io {
        /// The directory.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// An exclusively locked data directory. Only one Persivio process may use one: two
/// engines on one database would both "own" the same downloads. The lock is released
/// when this value is dropped (or the process exits).
#[derive(Debug)]
pub struct DataDir {
    path: PathBuf,
    _lock: File,
}

impl DataDir {
    /// The OS default: `%APPDATA%\Persivio\data` on Windows,
    /// `~/Library/Application Support/Persivio` on macOS, `~/.local/share/persivio` on Linux.
    ///
    /// # Errors
    /// [`DataDirError::NoDefault`] if the OS reports no home directory.
    pub fn default_path() -> Result<PathBuf, DataDirError> {
        directories::ProjectDirs::from("", "", "Persivio")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .ok_or(DataDirError::NoDefault)
    }

    /// Creates `path` if needed and locks it.
    ///
    /// # Errors
    /// [`DataDirError::Locked`] if another process holds the lock, or
    /// [`DataDirError::Io`].
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DataDirError> {
        let path = path.into();
        let io = |source| DataDirError::Io {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(&path).map_err(io)?;
        // It holds cookies of unfinished downloads and the browser pairing token. (On
        // Windows, the user profile is private already.)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        let file = File::create(path.join("persivio.lock")).map_err(io)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { path, _lock: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(DataDirError::Locked(path)),
            Err(std::fs::TryLockError::Error(e)) => Err(io(e)),
        }
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The database file inside it.
    #[must_use]
    pub fn db_path(&self) -> PathBuf {
        self.path.join("persivio.db")
    }
}

/// The OS "Downloads" folder, if the platform has one.
#[must_use]
pub fn os_download_dir() -> Option<PathBuf> {
    directories::UserDirs::new()?
        .download_dir()
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_can_be_opened_once_at_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("data");
        let first = DataDir::open(&dir).unwrap();
        assert_eq!(first.db_path(), dir.join("persivio.db"));
        assert_eq!(first.path(), dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "private to the user");
        }
        assert!(matches!(DataDir::open(&dir), Err(DataDirError::Locked(_))));
        drop(first);
        DataDir::open(&dir).unwrap();
        // A file where the directory should be is an error, not a panic.
        let file = tmp.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        let err = DataDir::open(&file).unwrap_err();
        assert!(matches!(err, DataDirError::Io { .. }), "{err}");
    }

    #[test]
    fn the_default_locations_are_named_after_the_app() {
        if let Ok(path) = DataDir::default_path() {
            let text = path.to_string_lossy().to_lowercase();
            assert!(text.contains("persivio"), "{text}");
        }
    }
}
