//! Writes the Mark-of-the-Web on finished downloads (ADR-017).

use std::path::Path;

use pv_core::motw::{ZONE_IDENTIFIER_STREAM, zone_identifier};
use url::Url;

/// Marks `path` as downloaded from `origin`. Windows only; elsewhere a no-op.
///
/// Failure is logged, never fatal: FAT32/exFAT drives and network shares may not
/// support alternate data streams, and the download itself is fine.
pub(crate) async fn mark(path: &Path, origin: &Url) {
    if cfg!(windows) {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":");
        stream.push(ZONE_IDENTIFIER_STREAM);
        if let Err(e) = tokio::fs::write(&stream, zone_identifier(Some(origin))).await {
            tracing::warn!(path = %path.display(), error = %e, "cannot write Mark-of-the-Web");
        }
    }
}
