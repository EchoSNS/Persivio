//! Magnet links (from the browser, clipboard, command line) must never panic, and an
//! accepted one has a well-formed info hash.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pv_torrent::parse_magnet;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    // With and without the prefix, so the fuzzer does not have to discover it.
    for uri in [s.to_owned(), format!("magnet:?xt=urn:btih:{s}")] {
        if let Ok(info) = parse_magnet(&uri) {
            assert_eq!(info.info_hash.len(), 40);
            assert!(info.info_hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }
});
