//! Narrow public-web media fetcher (currently: giphy GIF renditions) for
//! inline previews.
//!
//! Deliberately **not** a general HTTP client: the URL must already come
//! from `domain::giphy_gif_url` (host-allowlisted), and this adapter
//! re-checks the allowlist before spawning — defense in depth, so no other
//! call site can turn it into an arbitrary fetcher. Shells out to `curl`
//! (ubiquitous on the headless targets) with a wall-clock timeout, an https
//! pin and a hard size cap; the body goes straight to the cache file, never
//! through memory.
//!
//! Privacy note (documented in UX.md): fetching reveals the client IP to
//! the media host, like any link-preview client. The GUI avoids this via
//! Keybase's encrypted re-host, which the JSON API cannot reach. The
//! `web_previews` setting turns the whole path off.

use std::process::Command;
use std::time::Duration;

use crate::ports::KeybaseError;

/// Hard cap on a fetched media file (bytes) — a giphy GIF rendition is a
/// few MB; anything larger is refused by curl before it fills the disk.
const MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Wall-clock budget for one fetch.
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether `url` is an `https` URL on a giphy media host — the same rule as
/// `domain::giphy_gif_url`, re-checked here so the adapter is safe on its
/// own.
fn allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    host == "media.giphy.com"
        || host == "i.giphy.com"
        || (host.starts_with("media")
            && host.ends_with(".giphy.com")
            && host["media".len()..host.len() - ".giphy.com".len()]
                .chars()
                .all(|c| c.is_ascii_digit()))
}

/// Downloads `url` to `output`. Returns `Ok(())` only when curl exited
/// zero and the file exists; any failure (missing curl, refused host,
/// timeout, HTTP error, size cap) surfaces as a `KeybaseError` so the
/// caller's ready/failed bookkeeping treats it like a failed attachment
/// download.
pub fn fetch_giphy_media(url: &str, output: &str) -> Result<(), KeybaseError> {
    if !allowed(url) {
        return Err(KeybaseError::Internal(format!(
            "refused non-giphy url: {url}"
        )));
    }
    let run = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            &FETCH_TIMEOUT.as_secs().to_string(),
            "--max-filesize",
            &MAX_BYTES.to_string(),
            "--output",
            output,
            "--",
            url,
        ])
        .output();
    match run {
        Ok(out) if out.status.success() && std::path::Path::new(output).exists() => Ok(()),
        Ok(out) => {
            // curl wrote a partial file on failure? Never serve it.
            let _ = std::fs::remove_file(output);
            let err = String::from_utf8_lossy(&out.stderr);
            Err(KeybaseError::Internal(format!(
                "curl exited {}: {}",
                out.status,
                err.trim()
            )))
        }
        Err(e) => Err(KeybaseError::Internal(format!("curl not runnable: {e}"))),
    }
}

/// Percent-encodes a search query for a URL query component.
fn encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    for b in q.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Searches giphy with the **user's own API key** (`giphy_api_key` setting;
/// Keybase's server-vended key isn't reachable over the JSON API). Returns
/// the hits' titles + clean `.gif` rendition URLs. The key never appears in
/// errors or logs.
pub fn giphy_search(
    api_key: &str,
    query: &str,
) -> Result<Vec<crate::domain::GiphyHit>, KeybaseError> {
    let endpoint = format!(
        "https://api.giphy.com/v1/gifs/search?api_key={}&q={}&limit=25",
        encode_query(api_key),
        encode_query(query)
    );
    let run = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-time",
            "20",
            "--max-filesize",
            "2097152",
            "--",
            &endpoint,
        ])
        .output();
    match run {
        Ok(out) if out.status.success() => {
            let body = String::from_utf8_lossy(&out.stdout);
            Ok(parse_giphy_search(&body))
        }
        // Redact: curl's stderr echoes the URL (which carries the key).
        Ok(out) => Err(KeybaseError::Internal(format!(
            "giphy search failed (curl exited {}; wrong key or no network?)",
            out.status
        ))),
        Err(e) => Err(KeybaseError::Internal(format!("curl not runnable: {e}"))),
    }
}

/// Parses a giphy `/v1/gifs/search` response body into hits — tolerant:
/// malformed entries are skipped, never the whole list. URLs are stripped
/// of their `?cid=…` tracking params so what gets posted is the stable
/// media path (which the inline renderer also produces for incoming links).
fn parse_giphy_search(body: &str) -> Vec<crate::domain::GiphyHit> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(data) = v.get("data").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|d| {
            let url = d
                .pointer("/images/original/url")
                .and_then(serde_json::Value::as_str)?;
            let clean = url.split(['?', '#']).next()?.to_string();
            if !clean.ends_with(".gif") {
                return None;
            }
            let title = d
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            Some(crate::domain::GiphyHit {
                title: if title.is_empty() {
                    "(untitled)".to_string()
                } else {
                    title
                },
                url: clean,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn giphy_search_parse_is_tolerant_and_cleans_urls() {
        let body = r#"{"data":[
            {"title":"Cat GIF","images":{"original":{"url":"https://media0.giphy.com/media/abc/giphy.gif?cid=xyz&rid=1"}}},
            {"title":"","images":{"original":{"url":"https://media1.giphy.com/media/def/giphy.gif"}}},
            {"title":"broken","images":{}},
            {"title":"not-gif","images":{"original":{"url":"https://media1.giphy.com/media/x/giphy.mp4"}}}
        ]}"#;
        let hits = parse_giphy_search(body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Cat GIF");
        assert_eq!(hits[0].url, "https://media0.giphy.com/media/abc/giphy.gif");
        assert_eq!(hits[1].title, "(untitled)");
        // Garbage in, empty out — never a panic.
        assert!(parse_giphy_search("not json").is_empty());
        assert!(parse_giphy_search("{}").is_empty());
    }

    #[test]
    fn query_encoding_is_conservative() {
        assert_eq!(encode_query("hola tarola"), "hola+tarola");
        assert_eq!(encode_query("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn allowlist_matches_domain_rule() {
        assert!(allowed("https://media2.giphy.com/media/a/giphy.gif"));
        assert!(allowed("https://media.giphy.com/media/a/giphy.gif"));
        assert!(allowed("https://i.giphy.com/a.gif"));
        assert!(!allowed("http://media2.giphy.com/media/a/giphy.gif"));
        assert!(!allowed("https://example.com/media/a/giphy.gif"));
        assert!(!allowed("https://media2.giphy.com.evil.io/a.gif"));
        assert!(!allowed("https://mediaX.giphy.com/a.gif"));
    }

    #[test]
    fn refused_host_fails_without_spawning() {
        assert!(fetch_giphy_media("https://example.com/x.gif", "/tmp/never").is_err());
    }
}
