//! Seeding limits. librqbit has none, so the adapter enforces them (ADR-016).

use std::time::Duration;

use pv_core::TorrentSettings;

/// Whether a finished torrent should stop seeding now.
///
/// - `never_seed` → stop immediately.
/// - Otherwise stop when **any** configured limit is reached: upload ratio
///   (`uploaded / size ≥ seed_ratio_limit_percent / 100`) or seeding time.
/// - With no limit configured, seed until the user pauses or removes the task.
#[must_use]
pub fn seeding_done(
    settings: &TorrentSettings,
    uploaded: u64,
    size: u64,
    seeding_for: Duration,
) -> bool {
    if settings.never_seed {
        return true;
    }
    let ratio_reached = settings.seed_ratio_limit_percent.is_some_and(|percent| {
        // uploaded / size >= percent / 100, in integers (no float rounding).
        u128::from(uploaded) * 100 >= u128::from(size) * u128::from(percent)
    });
    let time_reached = settings
        .seed_time_limit_mins
        .is_some_and(|mins| seeding_for >= Duration::from_secs(u64::from(mins) * 60));
    ratio_reached || time_reached
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(never: bool, ratio: Option<u32>, mins: Option<u32>) -> TorrentSettings {
        TorrentSettings {
            never_seed: never,
            seed_ratio_limit_percent: ratio,
            seed_time_limit_mins: mins,
            ..TorrentSettings::default()
        }
    }

    #[test]
    fn limits() {
        let minute = Duration::from_secs(60);
        assert!(seeding_done(
            &settings(true, None, None),
            0,
            100,
            Duration::ZERO
        ));
        assert!(
            !seeding_done(&settings(false, None, None), u64::MAX, 100, minute * 10_000),
            "no limits: seed forever"
        );
        let ratio = settings(false, Some(150), None);
        assert!(!seeding_done(&ratio, 149, 100, Duration::ZERO));
        assert!(seeding_done(&ratio, 150, 100, Duration::ZERO));
        assert!(
            seeding_done(&ratio, 0, 0, Duration::ZERO),
            "empty torrent: any ratio is met"
        );
        let time = settings(false, None, Some(30));
        assert!(!seeding_done(&time, 0, 100, minute * 29));
        assert!(seeding_done(&time, 0, 100, minute * 30));
        let both = settings(false, Some(100), Some(30));
        assert!(
            seeding_done(&both, 100, 100, Duration::ZERO),
            "either limit stops seeding"
        );
        assert!(seeding_done(&both, 0, 100, minute * 30));
        assert!(
            !seeding_done(
                &settings(false, Some(u32::MAX), None),
                u64::MAX,
                u64::MAX,
                Duration::ZERO
            ),
            "huge values do not overflow (ratio 1.0 < limit)"
        );
    }
}
