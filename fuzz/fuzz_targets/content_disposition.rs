//! Content-Disposition parsing must never panic on arbitrary (even non-UTF-8) input.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(name) = pv_http::disposition::filename_from_disposition(&text) {
        assert!(!name.trim().is_empty());
    }
});
