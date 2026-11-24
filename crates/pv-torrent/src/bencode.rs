//! A strict structural check of `.torrent` bytes **before** librqbit parses them.
//!
//! librqbit's parser multiplies digits without checking for overflow: a build with
//! overflow checks panics on a huge number (found by fuzzing), a release build wraps it
//! to another value. This check refuses numbers that cannot be what they claim to be, so
//! neither can happen. A piece length of zero is divided by there. It is iterative: deep nesting cannot exhaust the stack.

/// Deeper than any real torrent (about 6 levels).
const MAX_DEPTH: usize = 64;
/// Digits of `u64::MAX`.
const MAX_DIGITS: usize = 20;

/// Why the bytes are refused.
pub(crate) type Problem = &'static str;

/// Reads an unsigned decimal number at the start of `bytes`, ended by `end`. Returns it
/// and how many bytes were used, including `end`.
fn number(bytes: &[u8], end: u8) -> Result<(u64, usize), Problem> {
    let digits = bytes
        .iter()
        .take(MAX_DIGITS + 1)
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits == 0 {
        return Err("a number is missing");
    }
    if digits > MAX_DIGITS {
        return Err("a number is too large");
    }
    if bytes.get(digits) != Some(&end) {
        return Err("a number is malformed");
    }
    let value = std::str::from_utf8(&bytes[..digits])
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or("a number is too large")?;
    Ok((value, digits + 1))
}

/// Largest value librqbit's field of this name can hold.
fn limit(key: &[u8]) -> u64 {
    match key {
        b"piece length" => u64::from(u32::MAX),
        b"private" => u64::from(u8::MAX),
        // Sizes and dates: also positive as a signed number, and safe to add up.
        _ => i64::MAX.unsigned_abs(),
    }
}

/// Checks that `bytes` start with one complete, well-formed value whose numbers fit.
pub(crate) fn check(bytes: &[u8]) -> Result<(), Problem> {
    let mut at = 0;
    let mut depth = 0usize;
    // The last string seen: the key, when a number follows it in a dictionary.
    let mut last: &[u8] = b"";
    let mut total_length = 0u64;
    loop {
        let rest = bytes.get(at..).unwrap_or_default();
        match rest.first() {
            None => return Err("the file ends too early"),
            Some(b'd' | b'l') => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err("it is nested too deeply");
                }
                at += 1;
            }
            Some(b'e') => {
                depth = depth.checked_sub(1).ok_or("an unexpected end")?;
                at += 1;
            }
            Some(b'i') => {
                let negative = rest.get(1) == Some(&b'-');
                let start = if negative { 2 } else { 1 };
                let (value, used) = number(&rest[start..], b'e')?;
                if last == b"piece length" && (value == 0 || negative) {
                    return Err("the piece length is zero");
                }
                if value > limit(last) {
                    return Err("a number is too large");
                }
                if last == b"length" {
                    total_length = total_length
                        .checked_add(value)
                        .filter(|t| *t <= i64::MAX.unsigned_abs())
                        .ok_or("the files are too large together")?;
                }
                at += start + used;
            }
            Some(_) => {
                let (len, used) = number(rest, b':')?;
                let len = usize::try_from(len).map_err(|_| "a text is too long")?;
                let end = used.checked_add(len).ok_or("a text is too long")?;
                last = rest.get(used..end).ok_or("the file ends too early")?;
                at += end;
            }
        }
        if depth == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_structures_pass() {
        for ok in [
            b"d4:infod6:lengthi5e4:name1:a12:piece lengthi16384e6:pieces0:ee".as_slice(),
            b"d7:privatei1e13:creation datei1795490000ee",
            b"i-5e",
            b"le",
            b"3:abc and trailing bytes are the parser's business",
            b"d5:filesld6:lengthi1e4:pathl1:aeed6:lengthi2e4:pathl1:beeee",
        ] {
            assert_eq!(check(ok), Ok(()), "{}", String::from_utf8_lossy(ok));
        }
    }

    #[test]
    fn numbers_that_cannot_fit_are_refused() {
        // Found by fuzzing: overflowed inside librqbit.
        assert!(check(b"d77777777777777777777777777777777777777779").is_err());
        assert!(check(b"i99999999999999999999e").is_err(), "more than u64");
        assert!(check(b"i18446744073709551615e").is_err(), "more than i64");
        assert!(check(b"d12:piece lengthi4294967296ee").is_err(), "u32");
        assert!(check(b"d7:privatei256ee").is_err(), "u8");
        // Found by fuzzing: a division by zero, also in release builds.
        assert!(check(b"d12:piece lengthi0e6:lengthi4e6:pieces0:e").is_err());
        assert!(
            check(b"ld6:lengthi9223372036854775807eed6:lengthi1eee").is_err(),
            "the sum overflows"
        );
    }

    #[test]
    fn broken_structures_are_refused() {
        for bad in [
            b"".as_slice(),
            b"d",
            b"e",
            b"ie",
            b"i-e",
            b"i5",
            b"5:abc",
            b"18446744073709551615:a",
            b"x",
            b"i 5e",
        ] {
            assert!(check(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        assert_eq!(
            check(&vec![b'l'; 1_000_000]),
            Err("it is nested too deeply")
        );
    }
}
