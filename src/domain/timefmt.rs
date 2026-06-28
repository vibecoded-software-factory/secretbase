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
/// the **day + local clock time** (`yest 14:30`, `12/06 14:30`, or
/// `12/06/24 14:30` for a different year). Empty when the timestamp is unknown
/// or in the future.
pub fn message_time(sent_at_s: u64, now_s: u64) -> String {
    use chrono::{Datelike, Local, TimeZone};
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
    let hm = sent.format("%H:%M");
    let days = (now.date_naive() - sent.date_naive()).num_days();
    if days == 1 {
        format!("yest {hm}")
    } else if sent.year() == now.year() {
        sent.format("%d/%m %H:%M").to_string()
    } else {
        sent.format("%d/%m/%y %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
}
