//! Mark-of-the-Web (ADR-017): the note Windows attaches to files that came from the
//! internet, so that SmartScreen and Office warn before running or opening them.
//!
//! Browsers write it as an NTFS alternate data stream named [`ZONE_IDENTIFIER_STREAM`].
//! This module only builds the stream's text; the protocol crates write it.

use url::Url;

/// Name of the alternate data stream (`file.exe:Zone.Identifier`).
pub const ZONE_IDENTIFIER_STREAM: &str = "Zone.Identifier";

/// The `Zone.Identifier` contents for a file downloaded from `origin`.
///
/// `ZoneId=3` is the Internet zone. `HostUrl` is the origin without credentials, query
/// or fragment: the stream is readable by every local program and signed URLs carry
/// tokens in their query strings. Torrents have no URL and use `about:internet`, as
/// browsers do when the origin is unknown.
#[must_use]
pub fn zone_identifier(origin: Option<&Url>) -> String {
    let host_url = origin.map_or_else(
        || "about:internet".to_owned(),
        |url| {
            let mut url = url.clone();
            // Both setters only fail for URLs that cannot have credentials (e.g.
            // `data:`), which then have none to remove.
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        },
    );
    format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={host_url}\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_url_drops_secrets() {
        let url = Url::parse("https://user:pw@cdn.example/f.exe?token=abc#frag").unwrap();
        assert_eq!(
            zone_identifier(Some(&url)),
            "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://cdn.example/f.exe\r\n"
        );
    }

    #[test]
    fn unknown_origin_is_still_the_internet_zone() {
        let text = zone_identifier(None);
        assert!(text.contains("ZoneId=3\r\n"));
        assert!(text.contains("HostUrl=about:internet\r\n"));
    }
}
