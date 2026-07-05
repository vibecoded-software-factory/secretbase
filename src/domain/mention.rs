//! Detecting an in-progress `@mention` in a compose buffer, for autocomplete.

/// If the cursor sits inside an `@mention` being typed, returns its
/// `(byte offset of '@', prefix typed so far)`. A mention starts at the
/// beginning of the buffer or after whitespace, and the prefix may only
/// contain username characters (`[a-z0-9_.]`) — so `email@host` and a
/// completed `@alice ` (trailing space) are **not** matches.
pub fn active_mention(text: &str, cursor: usize) -> Option<(usize, String)> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let at = before.rfind('@')?;
    let prefix = &before[at + 1..];
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    // The `@` must open a token: start of buffer or after whitespace.
    if at > 0
        && !before[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        return None;
    }
    Some((at, prefix.to_string()))
}

/// If the cursor sits inside a `:shortcode` being typed, returns its
/// `(byte offset of ':', prefix typed so far)`. Same shape as
/// [`active_mention`]: the `:` must open a token (start of buffer or after
/// whitespace — so clock times like `10:30` never match), the prefix may
/// only contain shortcode characters (`[a-z0-9_+-]`), and at least **two**
/// prefix chars are required before the popup fires (a lone `:` is
/// ordinary punctuation).
pub fn active_emoji_token(text: &str, cursor: usize) -> Option<(usize, String)> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let colon = before.rfind(':')?;
    let prefix = &before[colon + 1..];
    if prefix.len() < 2 {
        return None;
    }
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-')
    {
        return None;
    }
    if colon > 0
        && !before[..colon]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        return None;
    }
    Some((colon, prefix.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_emoji_token() {
        assert_eq!(
            active_emoji_token("nice :thu", 9),
            Some((5, "thu".to_string()))
        );
        // One char after the colon: still ordinary punctuation.
        assert_eq!(active_emoji_token("say :t", 6), None);
        // Clock times and mid-word colons never match.
        assert_eq!(active_emoji_token("at 10:30", 8), None);
        // A closed shortcode (trailing space) is not a token.
        assert_eq!(active_emoji_token(":wave: hi", 9), None);
    }

    #[test]
    fn detects_mention_token() {
        // Typing "@al" with the cursor at the end.
        assert_eq!(active_mention("hey @al", 7), Some((4, "al".to_string())));
        // Bare "@" right after a space.
        assert_eq!(active_mention("hi @", 4), Some((3, String::new())));
        // Start of buffer.
        assert_eq!(active_mention("@bob", 4), Some((0, "bob".to_string())));
    }

    #[test]
    fn rejects_non_mentions() {
        // Completed mention (trailing space).
        assert_eq!(active_mention("@alice ", 7), None);
        // Email — `@` not at a token boundary.
        assert_eq!(active_mention("me@host", 7), None);
        // No `@` before the cursor.
        assert_eq!(active_mention("plain text", 5), None);
    }
}
