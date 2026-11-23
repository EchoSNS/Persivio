//! Writes the Mark-of-the-Web on a finished torrent's files (ADR-017).

use std::path::{Path, PathBuf};

use pv_core::motw::{ZONE_IDENTIFIER_STREAM, zone_identifier};

/// Marks `root` (a file, or every file under a directory) as downloaded from the
/// internet. Windows only; elsewhere a no-op. Failures are logged, never fatal.
pub(crate) async fn mark_all(root: PathBuf) {
    if !cfg!(windows) {
        return;
    }
    let walked = tokio::task::spawn_blocking(move || {
        let text = zone_identifier(None);
        let mut pending = vec![root];
        while let Some(path) = pending.pop() {
            // symlink_metadata: never follow a link out of the download folder.
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                match std::fs::read_dir(&path) {
                    Ok(entries) => pending.extend(entries.flatten().map(|e| e.path())),
                    Err(e) => warn(&path, &e),
                }
            } else if meta.is_file() {
                let mut stream = path.as_os_str().to_owned();
                stream.push(":");
                stream.push(ZONE_IDENTIFIER_STREAM);
                if let Err(e) = std::fs::write(&stream, &text) {
                    warn(&path, &e);
                }
            }
        }
    })
    .await;
    if let Err(e) = walked {
        tracing::warn!(error = %e, "Mark-of-the-Web task failed");
    }
}

fn warn(path: &Path, e: &std::io::Error) {
    tracing::warn!(path = %path.display(), error = %e, "cannot write Mark-of-the-Web");
}
