//! Compact, single-unit duration formatting for the command log.
//!
//! Always shows the *smallest* unit that hasn't yet rolled over into the
//! next one (sub-second → `ms`, then `s` until 60, `m` until 60, `h`
//! until 24, `d` until 365, then `y`). Floored to a single unit so the
//! log stays scannable (`90s` → `1m`, not `1m 30s`).

use std::time::Duration;

const MIN: u64 = 60;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;
const YEAR: u64 = 365 * DAY;

/// Formats `d` as a compact single-unit string: `230ms`, `5s`, `3m`,
/// `2h`, `4d`, `1y`.
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 1 {
        // Sub-second: report milliseconds so fast ops aren't all "0s".
        return format!("{}ms", d.subsec_millis());
    }
    if secs < MIN {
        format!("{secs}s")
    } else if secs < HOUR {
        format!("{}m", secs / MIN)
    } else if secs < DAY {
        format!("{}h", secs / HOUR)
    } else if secs < YEAR {
        format!("{}d", secs / DAY)
    } else {
        format!("{}y", secs / YEAR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_second_is_milliseconds() {
        assert_eq!(format_duration(Duration::from_millis(0)), "0ms");
        assert_eq!(format_duration(Duration::from_millis(7)), "7ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn seconds_until_sixty() {
        assert_eq!(format_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn ascends_at_each_threshold() {
        assert_eq!(format_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m"); // floored
        assert_eq!(format_duration(Duration::from_secs(59 * 60)), "59m");
        assert_eq!(format_duration(Duration::from_secs(60 * 60)), "1h");
        assert_eq!(format_duration(Duration::from_secs(23 * 3600)), "23h");
        assert_eq!(format_duration(Duration::from_secs(24 * 3600)), "1d");
        assert_eq!(format_duration(Duration::from_secs(364 * 86_400)), "364d");
        assert_eq!(format_duration(Duration::from_secs(365 * 86_400)), "1y");
        assert_eq!(format_duration(Duration::from_secs(2 * 365 * 86_400)), "2y");
    }
}
