//! Rebuilds a magnet link from its checked parts **before** librqbit parses it.
//!
//! librqbit's parser panics on a base32 hash with padding (found by fuzzing), and lists
//! every number of a file range (`so=0-99999999999`), which can exhaust the memory. Only
//! the fields Persivio uses are passed on, so nothing else can reach that parser.

use pv_core::ProtocolError;
use url::Url;
use url::form_urlencoded::Serializer;

/// Trackers of one link that are used. Real links have a few dozen at most.
const MAX_TRACKERS: usize = 200;

fn invalid(problem: &str) -> ProtocolError {
    ProtocolError::InvalidSource(format!("not a valid magnet link: {problem}"))
}

/// Whether this is a v1 info hash: 40 hex digits, or 32 upper-case base32 digits without
/// padding.
fn is_info_hash(hash: &str) -> bool {
    match hash.len() {
        40 => hash.bytes().all(|b| b.is_ascii_hexdigit()),
        32 => hash.bytes().all(|b| matches!(b, b'A'..=b'Z' | b'2'..=b'7')),
        _ => false,
    }
}

/// The same link with only its v1 info hash, name and trackers.
pub(crate) fn clean(uri: &str) -> Result<String, ProtocolError> {
    let url = Url::parse(uri).map_err(|e| invalid(&e.to_string()))?;
    if url.scheme() != "magnet" {
        return Err(invalid("it does not start with magnet:"));
    }
    let (mut hash, mut name, mut trackers) = (None, None, Vec::new());
    for (key, value) in url.query_pairs() {
        match &*key {
            "xt" => {
                // Other kinds (v2 hashes of hybrid links) are not used.
                if let Some(h) = value.strip_prefix("urn:btih:") {
                    let h = h.to_ascii_uppercase();
                    if !is_info_hash(&h) {
                        return Err(invalid("its info hash is malformed"));
                    }
                    hash.get_or_insert(h);
                }
            }
            "dn" if !value.is_empty() => {
                name.get_or_insert(value);
            }
            "tr" if trackers.len() < MAX_TRACKERS => trackers.push(value),
            _ => {}
        }
    }
    let hash = hash.ok_or_else(|| invalid("it has no v1 info hash"))?;
    let mut query = Serializer::new(String::new());
    query.append_pair("xt", &format!("urn:btih:{hash}"));
    if let Some(name) = name {
        query.append_pair("dn", &name);
    }
    for tracker in trackers {
        query.append_pair("tr", &tracker);
    }
    Ok(format!("magnet:?{}", query.finish()))
}

#[cfg(test)]
mod tests {
    use super::{MAX_TRACKERS, clean};
    use crate::parse_magnet;

    const HEX: &str = "0123456789abcdef0123456789abcdef01234567";
    /// The same hash in base32.
    const BASE32: &str = "AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH";

    /// Found by fuzzing: `=` is base32 padding, so this decodes to fewer than 20 bytes.
    #[test]
    fn a_padded_base32_hash_is_refused() {
        for hash in [
            "YYYYYYY=YYYYYYYYYYYYYYYYYYYYYYYY",
            "YYYYYYYYYYYYYYYYYYYYYYYYYYYYYYY=",
            "================================",
        ] {
            assert!(parse_magnet(&format!("magnet:?xt=urn:btih:{hash}")).is_err());
        }
    }

    /// librqbit would list every number of the range.
    #[test]
    fn a_huge_file_selection_is_ignored() {
        let m = parse_magnet(&format!(
            "magnet:?xt=urn:btih:{HEX}&so=0-18446744073709551615"
        ))
        .unwrap();
        assert_eq!(m.info_hash, HEX);
    }

    #[test]
    fn hashes_are_accepted_in_every_valid_form() {
        for hash in [
            HEX.to_owned(),
            HEX.to_ascii_uppercase(),
            BASE32.to_owned(),
            BASE32.to_ascii_lowercase(),
        ] {
            for scheme in ["magnet", "MAGNET"] {
                let m = parse_magnet(&format!("{scheme}:?xt=urn:btih:{hash}")).unwrap();
                assert_eq!(m.info_hash, HEX);
            }
        }
    }

    #[test]
    fn the_used_parts_survive_and_the_rest_is_dropped() {
        let cleaned = clean(&format!(
            "magnet:?xt=urn:btmh:1220ab&xt=urn:btih:{HEX}&dn=a%26b%20c&so=1-3&x.pe=h:1\
             &tr=udp%3A%2F%2Ft.example%3A80%2Fa%3Fk%3D1%26l"
        ))
        .unwrap();
        for dropped in ["so=", "x.pe", "btmh"] {
            assert!(!cleaned.contains(dropped), "{cleaned}");
        }
        let m = parse_magnet(&cleaned).unwrap();
        assert_eq!(m.name.as_deref(), Some("a&b c"));
        assert_eq!(m.trackers, ["udp://t.example:80/a?k=1&l"]);
    }

    #[test]
    fn the_number_of_trackers_is_limited() {
        let trackers = "&tr=udp%3A%2F%2Fa".repeat(5000);
        let m = parse_magnet(&format!("magnet:?xt=urn:btih:{HEX}{trackers}")).unwrap();
        assert_eq!(m.trackers.len(), MAX_TRACKERS);
    }
}
