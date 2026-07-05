//! Tiny URL extractor for chat message bodies.
//!
//! Deliberately conservative: it only recognises `http://` / `https://`
//! tokens, strips common surrounding punctuation, and never tries to parse
//! the URL — the opener adapter re-checks the scheme before launching.

/// Returns the `http(s)` URLs found in `text`, in order of appearance.
pub fn extract_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for word in text.split_whitespace() {
        // Strip opening punctuation/brackets that often hug a pasted link.
        let w = word.trim_start_matches(['(', '<', '"', '\'', '[', '{']);
        if !(w.starts_with("http://") || w.starts_with("https://")) {
            continue;
        }
        // Strip trailing punctuation a sentence might leave on the link.
        let url = w.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '"', '\'']);
        if url.len() > "https://".len() {
            urls.push(url.to_string());
        }
    }
    urls
}

/// Finds a **giphy media** URL in a message body and returns its `.gif`
/// rendition, or `None`. Giphy serves the same media path under several
/// renditions (`giphy.mp4`, `giphy.gif`, `giphy.webp`); the GUI posts the
/// `.mp4` link, but the TUI's image pipeline animates GIFs — so the URL is
/// normalised: fragment dropped, a trailing `giphy.mp4` swapped to
/// `giphy.gif`. Host-allowlisted to `media[.N].giphy.com` / `i.giphy.com`
/// so this can never turn into a general web fetch.
pub fn giphy_gif_url(text: &str) -> Option<String> {
    for url in extract_urls(text) {
        let Some(rest) = url.strip_prefix("https://") else {
            continue; // https only — the fetch adapter enforces it too
        };
        let host = rest.split('/').next().unwrap_or("");
        let allowed = host == "media.giphy.com"
            || host == "i.giphy.com"
            || (host.starts_with("media")
                && host.ends_with(".giphy.com")
                && host["media".len()..host.len() - ".giphy.com".len()]
                    .chars()
                    .all(|c| c.is_ascii_digit()));
        if !allowed || !rest.contains("/media/") {
            continue;
        }
        let base = url.split(['#', '?']).next().unwrap_or(&url);
        if let Some(stem) = base.strip_suffix(".mp4") {
            return Some(format!("{stem}.gif"));
        }
        if base.ends_with(".gif") || base.ends_with(".webp") {
            return Some(
                base.strip_suffix(".webp")
                    .map(|s| format!("{s}.gif"))
                    .unwrap_or_else(|| base.to_string()),
            );
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn giphy_gif_url_normalises_and_allowlists() {
        // The GUI's giphy link: mp4 rendition + fragment → the .gif rendition.
        let body =
            "https://media2.giphy.com/media/v1.Abc/xYz/giphy.mp4#height=480&width=270&isvideo=true";
        assert_eq!(
            giphy_gif_url(body).as_deref(),
            Some("https://media2.giphy.com/media/v1.Abc/xYz/giphy.gif")
        );
        // Already a gif — passes through (fragment stripped).
        assert_eq!(
            giphy_gif_url("https://media.giphy.com/media/xYz/giphy.gif#x=1").as_deref(),
            Some("https://media.giphy.com/media/xYz/giphy.gif")
        );
        // Non-giphy hosts, lookalikes and http are rejected.
        assert_eq!(giphy_gif_url("https://example.com/media/a/giphy.mp4"), None);
        assert_eq!(
            giphy_gif_url("https://mediaX.giphy.com/media/a/giphy.mp4"),
            None
        );
        assert_eq!(
            giphy_gif_url("https://media2.giphy.com.evil.io/media/a/giphy.mp4"),
            None
        );
        assert_eq!(
            giphy_gif_url("http://media2.giphy.com/media/a/giphy.mp4"),
            None
        );
        // Giphy host but not a media path.
        assert_eq!(giphy_gif_url("https://media.giphy.com/other/a.mp4"), None);
    }

    #[test]
    fn finds_and_cleans_urls() {
        assert_eq!(
            extract_urls("see https://example.com/a, and http://x.io!"),
            vec!["https://example.com/a", "http://x.io"]
        );
        // Wrapped in parens.
        assert_eq!(
            extract_urls("(https://keybase.io)"),
            vec!["https://keybase.io"]
        );
        // Nothing to find.
        assert!(extract_urls("just words, no links").is_empty());
        // A bare scheme with no host is rejected.
        assert!(extract_urls("https://").is_empty());
    }
}
