//! Post-download checksum verification (SHA-256 / MD5).

use std::io::Read;
use std::path::{Path, PathBuf};

use md5::Md5;
use pv_core::{Checksum, ChecksumAlgorithm, ProtocolError};
use sha2::{Digest, Sha256};

/// Hashes `path` with `algorithm` and returns lower-case hex. Runs on tokio's blocking
/// pool: hashing a multi-GB file must not stall the async runtime.
///
/// # Errors
/// I/O errors reading the file.
pub async fn hash_file(path: &Path, algorithm: ChecksumAlgorithm) -> Result<String, ProtocolError> {
    let path: PathBuf = path.to_path_buf();
    tokio::task::spawn_blocking(move || match algorithm {
        ChecksumAlgorithm::Sha256 => digest_file::<Sha256>(&path),
        ChecksumAlgorithm::Md5 => digest_file::<Md5>(&path),
    })
    .await
    .map_err(|e| ProtocolError::Network(format!("hashing task failed: {e}")))?
    .map_err(ProtocolError::from)
}

fn digest_file<D: Digest>(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().fold(String::new(), |mut hex, b| {
        hex.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
        hex.push(char::from(b"0123456789abcdef"[usize::from(b & 0x0f)]));
        hex
    }))
}

/// Verifies `path` against the expected checksum.
///
/// # Errors
/// [`ProtocolError::ChecksumMismatch`] if the digests differ, or I/O errors.
pub async fn verify_file(path: &Path, expected: &Checksum) -> Result<(), ProtocolError> {
    let actual = hash_file(path, expected.algorithm()).await?;
    if expected.matches(&actual) {
        Ok(())
    } else {
        Err(ProtocolError::ChecksumMismatch {
            expected: expected.expected().to_owned(),
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn known_vectors_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, b"abc").unwrap();
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let md5 = "900150983cd24fb0d6963f7d28e17f72";
        assert_eq!(
            hash_file(&path, ChecksumAlgorithm::Sha256).await.unwrap(),
            sha
        );
        assert_eq!(hash_file(&path, ChecksumAlgorithm::Md5).await.unwrap(), md5);
        verify_file(
            &path,
            &Checksum::new(ChecksumAlgorithm::Sha256, sha).unwrap(),
        )
        .await
        .unwrap();
        verify_file(
            &path,
            &Checksum::new(ChecksumAlgorithm::Md5, &md5.to_uppercase()).unwrap(),
        )
        .await
        .unwrap();
        let wrong = Checksum::new(ChecksumAlgorithm::Md5, &"0".repeat(32)).unwrap();
        let err = verify_file(&path, &wrong).await.unwrap_err();
        assert!(matches!(err, ProtocolError::ChecksumMismatch { ref actual, .. } if actual == md5));
        assert!(
            hash_file(&dir.path().join("missing"), ChecksumAlgorithm::Md5)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn large_files_stream_through_the_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        let data = vec![7u8; 1_000_003]; // several buffer fills + a partial one
        std::fs::write(&path, &data).unwrap();
        let hex = pv_testkit::sha256_hex(&data);
        assert_eq!(
            hash_file(&path, ChecksumAlgorithm::Sha256).await.unwrap(),
            hex
        );
    }
}
