//! Whatever a server sends, the sanitized name stays inside the destination directory.
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;
use pv_http::filename::{MAX_NAME_BYTES, contained_path, sanitize_filename};

fuzz_target!(|data: &[u8]| {
    let raw = String::from_utf8_lossy(data);
    let name = sanitize_filename(&raw);
    assert!(!name.is_empty() && name.len() <= MAX_NAME_BYTES);
    let dest = Path::new("/dest");
    let path = contained_path(dest, &name).expect("sanitized names are always contained");
    assert_eq!(path.parent(), Some(dest));
    assert_eq!(sanitize_filename(&name), name);
});
