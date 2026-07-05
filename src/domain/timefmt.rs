//! Compact, Discord-style timestamp formatting for chat messages.
//!
//! Pure formatting over Unix seconds + a "now" reference, using the local
//! timezone via `chrono`. Shared by the message header, the in-conversation
//! search results, and the copy-with-context flows.

/// Same-day relative age: `now` / `{n}m` / `{n}h`. The whole sub-minute range
/// reads `now` (Discord-style) — a per-second counter on a fresh message is
/// distracting, and the minute granularity is enough context.
pub fn relative_short(secs: u64) -> String {
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

/// Compact timestamp shown beside the sender (Discord-style): if the message
/// is from **today** it's a relative age (`now`/`{n}m`/`{n}h`); otherwise it's
/// the **full local date + clock with seconds** (`21/04/2025 12:02:34`).
/// Empty when the timestamp is unknown or in the future.
pub fn message_time(sent_at_s: u64, now_s: u64) -> String {
    use chrono::{Local, TimeZone};
    if sent_at_s == 0 || now_s == 0 || sent_at_s > now_s {
        return String::new();
    }
    let (Some(sent), Some(now)) = (
        Local.timestamp_opt(sent_at_s as i64, 0).single(),
        Local.timestamp_opt(now_s as i64, 0).single(),
    ) else {
        return String::new();
    };
    if sent.date_naive() == now.date_naive() {
        return relative_short(now_s - sent_at_s);
    }
    // Older than today → full day/month/year + hours:minutes:seconds.
    sent.format("%d/%m/%Y %H:%M:%S").to_string()
}

/// Local wall-clock `HH:MM` for a message timestamp (Unix seconds), or empty
/// when unknown. Used for the message-stream header once a **day divider**
/// carries the date, so the per-message stamp only needs the time of day.
pub fn clock_time(sent_at_s: u64) -> String {
    use chrono::{Local, TimeZone};
    if sent_at_s == 0 {
        return String::new();
    }
    Local
        .timestamp_opt(sent_at_s as i64, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// Inbox-row timestamp, matching the Keybase GUI's chat list: **today** →
/// the clock (`23:39`), **this week** (1–6 calendar days back) → the
/// weekday (`Tue`), **this year** → month + day (`Jun 3`), older →
/// day + month + 2-digit year (`22 Dec 25`). Empty when the timestamp is
/// unknown or in the future — the "no signal" the tree already renders.
pub fn inbox_stamp(active_at_s: u64, now_s: u64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    if active_at_s == 0 || now_s == 0 || active_at_s > now_s {
        return String::new();
    }
    let (Some(then), Some(now)) = (
        Local.timestamp_opt(active_at_s as i64, 0).single(),
        Local.timestamp_opt(now_s as i64, 0).single(),
    ) else {
        return String::new();
    };
    let days_back = (now.date_naive() - then.date_naive()).num_days();
    if days_back == 0 {
        then.format("%H:%M").to_string()
    } else if (1..7).contains(&days_back) {
        then.format("%a").to_string()
    } else if then.year() == now.year() {
        then.format("%b %-d").to_string()
    } else {
        then.format("%-d %b %y").to_string()
    }
}

/// Whether two Unix-second timestamps fall on the same **local** calendar day.
/// Used to decide where a day divider goes in the message stream.
pub fn same_local_day(a_s: u64, b_s: u64) -> bool {
    use chrono::{Local, TimeZone};
    match (
        Local.timestamp_opt(a_s as i64, 0).single(),
        Local.timestamp_opt(b_s as i64, 0).single(),
    ) {
        (Some(a), Some(b)) => a.date_naive() == b.date_naive(),
        _ => false,
    }
}

/// Label for a day divider between messages: `Today` / `Yesterday` /
/// `Mon 12 Feb` (localized weekday + date). Empty when the timestamp is
/// unknown.
pub fn day_divider_label(sent_at_s: u64, now_s: u64) -> String {
    use chrono::{Duration, Local, TimeZone};
    if sent_at_s == 0 || now_s == 0 {
        return String::new();
    }
    let (Some(sent), Some(now)) = (
        Local.timestamp_opt(sent_at_s as i64, 0).single(),
        Local.timestamp_opt(now_s as i64, 0).single(),
    ) else {
        return String::new();
    };
    let (sd, nd) = (sent.date_naive(), now.date_naive());
    if sd == nd {
        "Today".to_string()
    } else if sd == nd - Duration::days(1) {
        "Yesterday".to_string()
    } else {
        sent.format("%a %d %b").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn local_ts(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        use chrono::{Local, TimeZone};
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("unambiguous local time")
            .timestamp() as u64
    }

    #[test]
    fn inbox_stamp_matches_the_gui_buckets() {
        let now = local_ts(2026, 7, 4, 23, 50);
        // Today → the clock.
        assert_eq!(inbox_stamp(local_ts(2026, 7, 4, 9, 5), now), "09:05");
        // 1–6 calendar days back → the weekday (2026-07-01 is a Wednesday).
        assert_eq!(inbox_stamp(local_ts(2026, 7, 1, 12, 0), now), "Wed");
        // Same year, a week or more back → "Mon D".
        assert_eq!(inbox_stamp(local_ts(2026, 6, 3, 12, 0), now), "Jun 3");
        // Older years → "D Mon YY".
        assert_eq!(inbox_stamp(local_ts(2025, 12, 22, 12, 0), now), "22 Dec 25");
        // Unknown / future → empty.
        assert_eq!(inbox_stamp(0, now), "");
        assert_eq!(inbox_stamp(now + 60, now), "");
    }

    #[test]
    fn relative_short_buckets() {
        // The whole sub-minute range is "now" — no frenetic second counter.
        assert_eq!(relative_short(3), "now");
        assert_eq!(relative_short(42), "now");
        assert_eq!(relative_short(59), "now");
        assert_eq!(relative_short(60), "1m");
        assert_eq!(relative_short(5 * 60), "5m");
        assert_eq!(relative_short(3 * 3600), "3h");
    }

    #[test]
    fn message_time_guards_and_today_is_relative() {
        // No timestamp / future → empty.
        assert_eq!(message_time(0, 600), "");
        assert_eq!(message_time(200, 100), "");
        // A few seconds ago is the same local day → relative.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1_000_000);
        assert_eq!(message_time(now - 3, now), "now");
        assert_eq!(message_time(now - 42, now), "now");
        assert_eq!(message_time(now - 90, now), "1m");
    }

    #[test]
    fn clock_time_is_hh_mm_or_empty() {
        assert_eq!(clock_time(0), "");
        let s = clock_time(1_700_000_000);
        assert_eq!(s.chars().count(), 5, "{s} should be HH:MM");
        assert_eq!(s.matches(':').count(), 1, "{s} should have one colon");
    }

    #[test]
    fn same_local_day_matches_only_the_same_day() {
        let ts = 1_700_000_000;
        assert!(same_local_day(ts, ts));
        assert!(!same_local_day(ts, ts - 10 * 86_400));
    }

    #[test]
    fn day_divider_label_today_and_older() {
        let ts = 1_700_000_000;
        assert_eq!(day_divider_label(ts, ts), "Today");
        // Ten days earlier is neither Today nor Yesterday → a weekday+date label.
        let older = day_divider_label(ts - 10 * 86_400, ts);
        assert_ne!(older, "Today");
        assert_ne!(older, "Yesterday");
        assert!(older.contains(' '), "{older} should be a weekday + date");
        // Unknown timestamp → empty.
        assert_eq!(day_divider_label(0, ts), "");
    }

    #[test]
    fn message_time_older_is_full_date_with_seconds() {
        // A timestamp days earlier renders as DD/MM/YYYY HH:MM:SS — full year
        // and seconds (exact digits depend on the local timezone).
        let sent = 1_700_000_000; // 2023
        let now = sent + 10 * 86_400; // ten days later
        let s = message_time(sent, now);
        assert_eq!(s.matches(':').count(), 2, "{s} should carry seconds");
        assert_eq!(s.matches('/').count(), 2, "{s} should be day/month/year");
        assert!(s.contains("/2023 "), "{s} should carry the full year");
    }
}
