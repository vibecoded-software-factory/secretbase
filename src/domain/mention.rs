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

#[cfg(test)]
mod tests {
    use super::*;

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
