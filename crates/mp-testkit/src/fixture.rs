//! Test fixtures with known SHA-256 digests.

use std::fmt::Write as _;
use std::io;
use std::path::Path;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::rng::SplitMix64;

/// A named blob served by [`crate::TestServer`], with its SHA-256 for verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Name used in URLs (`/file/{name}`).
    pub name: String,
    /// Content. `Bytes` makes clones and range slices zero-copy.
    pub bytes: Bytes,
    /// Lower-case hex SHA-256 of `bytes`.
    pub sha256: String,
}

impl Fixture {
    /// Deterministic pseudo-random content: the same `(size, seed)` always yields the
    /// same bytes, so failures reproduce. Random (not zero) bytes catch offset bugs that
    /// zero-filled files would hide.
    #[must_use]
    pub fn random(name: impl Into<String>, size: usize, seed: u64) -> Self {
        let mut buf = vec![0u8; size];
        SplitMix64::new(seed).fill(&mut buf);
        Self::from_bytes(name, buf)
    }

    /// Wraps given content.
    #[must_use]
    pub fn from_bytes(name: impl Into<String>, bytes: impl Into<Bytes>) -> Self {
        let bytes = bytes.into();
        Self {
            name: name.into(),
            sha256: sha256_hex(&bytes),
            bytes,
        }
    }

    /// Size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the fixture is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Lower-case hex SHA-256 of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    to_hex(&Sha256::digest(data))
}

/// Streams a file through SHA-256 (constant memory) and returns lower-case hex.
///
/// # Errors
/// Any I/O error opening or reading the file.
pub async fn sha256_file(path: impl AsRef<Path>) -> io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing to a String cannot fail.
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sha256_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn random_fixtures_are_deterministic() {
        let a = Fixture::random("a", 100_003, 7);
        let b = Fixture::random("b", 100_003, 7);
        let c = Fixture::random("c", 100_003, 8);
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(a.sha256, b.sha256);
        assert_ne!(a.sha256, c.sha256);
        assert_eq!(a.len(), 100_003);
        assert!(!a.is_empty());
        assert!(Fixture::from_bytes("e", Vec::new()).is_empty());
    }

    #[tokio::test]
    async fn sha256_file_matches_in_memory_hash() {
        let dir = tempfile::tempdir().unwrap();
        let f = Fixture::random("x", 300_000, 1); // > one 64 KiB read buffer
        let path = dir.path().join("x.bin");
        tokio::fs::write(&path, &f.bytes).await.unwrap();
        assert_eq!(sha256_file(&path).await.unwrap(), f.sha256);
        assert!(sha256_file(dir.path().join("missing")).await.is_err());
    }
}
