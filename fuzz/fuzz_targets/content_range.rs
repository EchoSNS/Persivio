//! Content-Range must never panic, and anything it accepts must be self-consistent.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pv_http::{ContentRange, parse_content_range};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };
    if let Ok(ContentRange::Satisfied { start, end, complete }) = parse_content_range(s) {
        assert!(start <= end);
        if let Some(c) = complete {
            assert!(end < c);
        }
    }
});
