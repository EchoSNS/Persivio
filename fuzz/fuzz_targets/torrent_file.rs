//! `.torrent` files are untrusted: reading one must never panic, and every path it
//! yields must stay inside the download folder.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pv_torrent::inspect_torrent;

fuzz_target!(|data: &[u8]| {
    // Wrapped as an `info` dictionary too, so random bytes reach the deeper checks.
    let mut wrapped = b"d4:info".to_vec();
    wrapped.extend_from_slice(data);
    wrapped.push(b'e');
    for bytes in [data, &wrapped] {
        let Ok(info) = inspect_torrent(bytes) else { continue };
        for file in &info.files {
            assert!(!file.path.is_empty());
            for part in file.path.split('/') {
                assert!(!part.is_empty() && part != "." && part != "..");
                assert!(!part.contains(['\\', '\0']));
            }
        }
    }
});
