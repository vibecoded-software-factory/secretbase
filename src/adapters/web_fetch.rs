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

#[cfg(test)]
mod tests {
    use super::*;

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
