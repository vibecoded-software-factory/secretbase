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

#[cfg(test)]
mod tests {
    use super::*;

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
