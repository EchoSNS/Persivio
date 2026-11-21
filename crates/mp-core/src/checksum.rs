//! Expected file hashes for post-download verification.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Supported hash algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChecksumAlgorithm {
    /// SHA-256 (preferred).
    Sha256,
    /// MD5 (legacy; only for matching published checksums, never for security).
    Md5,
}

impl ChecksumAlgorithm {
    /// Length of the digest in hex characters.
    #[must_use]
    pub const fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Md5 => 32,
        }
    }
}

impl fmt::Display for ChecksumAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sha256 => "SHA-256",
            Self::Md5 => "MD5",
        })
    }
}

/// An expected digest. Always valid: the hex string is checked and lower-cased on
/// construction **and** on deserialization (`serde(try_from)`), so no invalid
/// `Checksum` can exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawChecksum")]
pub struct Checksum {
    algorithm: ChecksumAlgorithm,
    expected: String,
}

/// Why a checksum string was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChecksumError {
    /// Wrong number of hex digits for the algorithm.
    #[error("{algorithm} digest must be {expected} hex characters, got {actual}")]
    WrongLength {
        /// The algorithm the digest was given for.
        algorithm: ChecksumAlgorithm,
        /// Required length.
        expected: usize,
        /// Provided length.
        actual: usize,
    },
    /// Contains a non-hex character.
    #[error("digest contains non-hex characters")]
    NotHex,
}

#[derive(Deserialize)]
struct RawChecksum {
    algorithm: ChecksumAlgorithm,
    expected: String,
}

impl TryFrom<RawChecksum> for Checksum {
    type Error = ChecksumError;

    fn try_from(raw: RawChecksum) -> Result<Self, Self::Error> {
        Self::new(raw.algorithm, &raw.expected)
    }
}

impl Checksum {
    /// Validates and normalizes (trims, lower-cases) a hex digest.
    ///
    /// # Errors
    /// [`ChecksumError`] if the digest has the wrong length or non-hex characters.
    pub fn new(algorithm: ChecksumAlgorithm, hex: &str) -> Result<Self, ChecksumError> {
        let hex = hex.trim();
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ChecksumError::NotHex);
        }
        if hex.len() != algorithm.hex_len() {
            return Err(ChecksumError::WrongLength {
                algorithm,
                expected: algorithm.hex_len(),
                actual: hex.len(),
            });
        }
        Ok(Self {
            algorithm,
            expected: hex.to_ascii_lowercase(),
        })
    }

    /// The hash algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> ChecksumAlgorithm {
        self.algorithm
    }

    /// The expected digest, lower-case hex.
    #[must_use]
    pub fn expected(&self) -> &str {
        &self.expected
    }

    /// Whether a computed hex digest matches (case-insensitive).
    #[must_use]
    pub fn matches(&self, actual_hex: &str) -> bool {
        self.expected.eq_ignore_ascii_case(actual_hex.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn new_normalizes_and_validates() {
        let c = Checksum::new(
            ChecksumAlgorithm::Sha256,
            &format!("  {}  ", SHA_EMPTY.to_uppercase()),
        )
        .unwrap();
        assert_eq!(c.expected(), SHA_EMPTY);
        assert_eq!(c.algorithm(), ChecksumAlgorithm::Sha256);
        assert!(c.matches(&SHA_EMPTY.to_uppercase()));
        assert!(!c.matches("00"));

        assert_eq!(
            Checksum::new(ChecksumAlgorithm::Md5, SHA_EMPTY),
            Err(ChecksumError::WrongLength {
                algorithm: ChecksumAlgorithm::Md5,
                expected: 32,
                actual: 64
            })
        );
        assert_eq!(
            Checksum::new(ChecksumAlgorithm::Md5, "zz"),
            Err(ChecksumError::NotHex)
        );
        assert!(Checksum::new(ChecksumAlgorithm::Md5, "d41d8cd98f00b204e9800998ecf8427e").is_ok());
    }

    #[test]
    fn serde_round_trip_and_rejects_invalid() {
        let c = Checksum::new(ChecksumAlgorithm::Sha256, SHA_EMPTY).unwrap();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"algorithm":"sha256","expected":"{SHA_EMPTY}"}}"#)
        );
        assert_eq!(serde_json::from_str::<Checksum>(&json).unwrap(), c);
        assert!(
            serde_json::from_str::<Checksum>(r#"{"algorithm":"md5","expected":"abc"}"#).is_err()
        );
    }

    #[test]
    fn display_names() {
        assert_eq!(ChecksumAlgorithm::Sha256.to_string(), "SHA-256");
        assert_eq!(ChecksumAlgorithm::Md5.to_string(), "MD5");
        let err = Checksum::new(ChecksumAlgorithm::Md5, "ab").unwrap_err();
        assert_eq!(
            err.to_string(),
            "MD5 digest must be 32 hex characters, got 2"
        );
    }
}
