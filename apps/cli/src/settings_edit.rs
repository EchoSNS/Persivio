//! `persivio settings set KEY VALUE`, generic over every field of `Settings`.

use anyhow::{Context, Result, bail};
use pv_core::Settings;
use serde_json::Value;

/// Returns `current` with `key` (dot-separated for nested fields, e.g. `torrent.dht`)
/// set to `value`. The value is parsed as JSON when possible (`5`, `true`, `null`), with
/// `none` meaning `null` and anything else taken as a string. The result is
/// type-checked by deserializing it back into [`Settings`]; range checks happen in the
/// engine.
pub fn set(current: &Settings, key: &str, value: &str) -> Result<Settings> {
    let mut json = serde_json::to_value(current)?;
    let mut slot = &mut json;
    for part in key.split('.') {
        slot = match slot {
            Value::Object(map) if map.contains_key(part) => map
                .get_mut(part)
                .context("unreachable: key checked above")?,
            _ => bail!("unknown setting `{key}` (see `persivio settings show`)"),
        };
    }
    *slot = if value.eq_ignore_ascii_case("none") {
        Value::Null
    } else {
        serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()))
    };
    serde_json::from_value(json).with_context(|| format!("invalid value `{value}` for `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_top_level_nested_optional_and_string_values() {
        let s = Settings::default();
        assert_eq!(
            set(&s, "max_concurrent_tasks", "7")
                .unwrap()
                .max_concurrent_tasks,
            7
        );
        assert!(!set(&s, "torrent.dht", "false").unwrap().torrent.dht);
        let limited = set(&s, "global_speed_limit_bps", "1000000").unwrap();
        assert_eq!(limited.global_speed_limit_bps, Some(1_000_000));
        assert_eq!(
            set(&limited, "global_speed_limit_bps", "none")
                .unwrap()
                .global_speed_limit_bps,
            None
        );
        assert_eq!(
            set(&s, "user_agent", "Mozilla/5.0").unwrap().user_agent,
            "Mozilla/5.0"
        );
        let dir = set(&s, "download_dir", "C:/Users/me/Downloads").unwrap();
        assert!(dir.download_dir.is_some());
    }

    #[test]
    fn rejects_unknown_keys_and_wrong_types() {
        let s = Settings::default();
        assert!(
            set(&s, "nope", "1")
                .unwrap_err()
                .to_string()
                .contains("unknown setting")
        );
        assert!(set(&s, "torrent.nope", "1").is_err());
        assert!(set(&s, "max_concurrent_tasks.x", "1").is_err());
        assert!(
            set(&s, "max_concurrent_tasks", "lots")
                .unwrap_err()
                .to_string()
                .contains("invalid value")
        );
    }
}
