//! Conversions between Rust types and SQLite column values.
//!
//! SQLite integers are signed 64-bit; our sizes and offsets are `u64`. Converting with
//! `as` would silently wrap huge values into negative numbers, so every crossing goes
//! through a checked conversion.

use std::path::Path;

use crate::StoreError;

/// `u64` → SQLite INTEGER.
pub fn to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError::Overflow(v))
}

/// Optional `u64` → SQLite INTEGER.
pub fn opt_to_i64(v: Option<u64>) -> Result<Option<i64>, StoreError> {
    v.map(to_i64).transpose()
}

/// SQLite INTEGER → `u64`, rejecting negatives as corruption.
pub fn to_u64(v: i64, column: &str) -> Result<u64, StoreError> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt(format!("negative {column}: {v}")))
}

/// Optional SQLite INTEGER → `u64`.
pub fn opt_to_u64(v: Option<i64>, column: &str) -> Result<Option<u64>, StoreError> {
    v.map(|v| to_u64(v, column)).transpose()
}

/// SQLite INTEGER → any narrower unsigned type.
pub fn to_small<T: TryFrom<i64>>(v: i64, column: &str) -> Result<T, StoreError> {
    T::try_from(v).map_err(|_| StoreError::Corrupt(format!("{column} out of range: {v}")))
}

/// Path → TEXT. Non-UTF-8 paths are rejected rather than lossily converted, because a
/// lossy path would point somewhere else after reload.
pub fn path_to_str(path: &Path) -> Result<&str, StoreError> {
    path.to_str()
        .ok_or_else(|| StoreError::NonUtf8Path(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_conversions_are_checked() {
        assert_eq!(to_i64(5).unwrap(), 5);
        assert!(matches!(
            to_i64(u64::MAX),
            Err(StoreError::Overflow(u64::MAX))
        ));
        assert_eq!(opt_to_i64(None).unwrap(), None);
        assert_eq!(opt_to_i64(Some(7)).unwrap(), Some(7));
        assert_eq!(to_u64(9, "x").unwrap(), 9);
        assert!(matches!(to_u64(-1, "x"), Err(StoreError::Corrupt(_))));
        assert_eq!(opt_to_u64(Some(3), "x").unwrap(), Some(3));
        assert_eq!(to_small::<u8>(32, "c").unwrap(), 32);
        assert!(to_small::<u8>(300, "c").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_rejected() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let p = Path::new(OsStr::from_bytes(b"/tmp/\xff"));
        assert!(matches!(path_to_str(p), Err(StoreError::NonUtf8Path(_))));
    }

    #[test]
    fn utf8_paths_pass_through() {
        assert_eq!(path_to_str(Path::new("/tmp/ü")).unwrap(), "/tmp/ü");
    }
}
