//! Input validation helpers used before dispatching to the Keybase CLI.

/// Keybase usernames are `^[a-z0-9_]{2,16}$` (lower-case ASCII letters,
/// digits, underscore — 2 to 16 chars). Social-network proofs use the
/// form `name@network`; this validator accepts the bare username form
/// only. See [`is_valid_keybase_identity`] for the form accepted by
/// the new-conversation popup.
pub fn is_valid_keybase_username(s: &str) -> bool {
    let len = s.chars().count();
    if !(2..=16).contains(&len) {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// One participant entry of an impteam conversation.
///
/// Accepts either a bare Keybase username
/// ([`is_valid_keybase_username`]) or a social-proof identity of the
/// form `user@network` — `network` being a short ASCII-alphabetic
/// label (`alice@twitter`, `bob@reddit`, …). Anything with
/// whitespace, control characters, or other punctuation is rejected
/// so the new-conversation popup can flag obvious typos before
/// Keybase does, instead of waiting for the CLI to surface a
/// cryptic server error.
///
/// The validator is intentionally conservative: Keybase accepts a
/// superset of what this function allows (phone numbers, emails,
/// etc.), but those are uncommon in the TUI flow and easy to add
/// later. Erring on the strict side is safer than erring permissive.
pub fn is_valid_keybase_identity(s: &str) -> bool {
    match s.split_once('@') {
        Some((user, network)) => is_valid_keybase_username(user) && is_valid_proof_network(network),
        None => is_valid_keybase_username(s),
    }
}

/// Network label of a social-proof identity. 2-32 ASCII-alphabetic
/// chars — matches the conservative subset of what Keybase accepts
/// (twitter / reddit / github / hackernews / mastodon / …).
fn is_valid_proof_network(s: &str) -> bool {
    let len = s.chars().count();
    if !(2..=32).contains(&len) {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_username() {
        assert!(is_valid_keybase_username("alice"));
        assert!(is_valid_keybase_username("ab"));
        assert!(is_valid_keybase_username("a_b_1"));
    }

    #[test]
    fn rejects_too_short() {
        assert!(!is_valid_keybase_username("a"));
        assert!(!is_valid_keybase_username(""));
    }

    #[test]
    fn rejects_too_long() {
        assert!(!is_valid_keybase_username("a".repeat(17).as_str()));
    }

    #[test]
    fn rejects_uppercase_and_punctuation() {
        assert!(!is_valid_keybase_username("Alice"));
        assert!(!is_valid_keybase_username("alice!"));
        assert!(!is_valid_keybase_username("alice.bob"));
    }

    // ── is_valid_keybase_identity ───────────────────────────────

    #[test]
    fn identity_accepts_bare_username() {
        assert!(is_valid_keybase_identity("alice"));
        assert!(is_valid_keybase_identity("a_b_1"));
    }

    #[test]
    fn identity_accepts_proof_form() {
        assert!(is_valid_keybase_identity("alice@twitter"));
        assert!(is_valid_keybase_identity("bob@reddit"));
        assert!(is_valid_keybase_identity("dev_42@github"));
    }

    #[test]
    fn identity_rejects_shell_metacharacters() {
        assert!(!is_valid_keybase_identity("; rm -rf /"));
        assert!(!is_valid_keybase_identity("alice'or'1=1"));
        assert!(!is_valid_keybase_identity("alice bob"));
    }

    #[test]
    fn identity_rejects_dots_and_other_punctuation() {
        assert!(!is_valid_keybase_identity("alice.bob"));
        assert!(!is_valid_keybase_identity("alice-bob"));
        assert!(!is_valid_keybase_identity("alice/bob"));
    }

    #[test]
    fn identity_rejects_invalid_proof_network() {
        // Network must be short ASCII letters only.
        assert!(!is_valid_keybase_identity("alice@x"));
        assert!(!is_valid_keybase_identity("alice@twitter123"));
        assert!(!is_valid_keybase_identity("alice@two words"));
        // Empty network after `@`.
        assert!(!is_valid_keybase_identity("alice@"));
    }

    #[test]
    fn identity_rejects_invalid_user_in_proof() {
        assert!(!is_valid_keybase_identity("Alice@twitter")); // uppercase
        assert!(!is_valid_keybase_identity("alice.bob@twitter")); // dot
        assert!(!is_valid_keybase_identity("@twitter")); // empty user
    }
}
