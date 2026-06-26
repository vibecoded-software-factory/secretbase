//! Structured error type for [`crate::ports::KeybasePort`].
//!
//! Previously every method returned `Result<T, String>` — pragmatic,
//! but opaque. Callers could not distinguish a transient timeout
//! from a semantic "conversation not found" from a missing binary,
//! so the UI surfaced every failure with the same generic banner
//! and had to parse the string to behave differently. Replacing the
//! payload with a typed enum unlocks per-kind handling (auth →
//! redirect to login, timeout → suggest raising the cap, …) without
//! forcing every change today: existing call sites continue working
//! via [`Display`] which renders each variant to a human-readable
//! line.

use std::fmt;

/// What went wrong while talking to the local `keybased` service.
///
/// The variants are ordered by where in the pipeline the failure
/// originated, from "couldn't even start the subprocess" down to
/// "subprocess ran fine but the wire payload was wrong".
#[derive(Debug, Clone)]
pub enum KeybaseError {
    /// Could not spawn the `keybase` binary at all — typically a
    /// PATH / permission issue. The wrapped string carries the
    /// underlying `io::Error` message.
    Spawn(String),

    /// The subprocess exceeded its wall-clock budget and was
    /// killed. `label` identifies the call site
    /// (`"keybase chat api"` / `"keybase status"`), `secs` is the
    /// budget that fired.
    Timeout { label: String, secs: u64 },

    /// The subprocess returned a non-zero exit status. `stderr` is
    /// the captured stderr text (trimmed), empty when the backend
    /// printed nothing.
    Exit { stderr: String, status: i32 },

    /// stdout was not valid JSON. `detail` carries the parser's
    /// error message (`serde_json::Error::to_string()`).
    InvalidJson { family: String, detail: String },

    /// Keybase reported a semantic error in the JSON body (exit was
    /// 0). `code` is the optional numeric error code; `message` is
    /// the human-readable text the service emitted.
    Api { code: Option<i64>, message: String },

    /// The JSON parsed, exit was 0, but the expected schema is
    /// missing (e.g. `/result/conversations` is not an array). The
    /// wrapped string identifies which pointer.
    Shape(String),

    /// The adapter call panicked. The wrapped string is the panic
    /// payload as text. The worker thread isolates panics with
    /// `std::panic::catch_unwind` so a buggy parser or third-party
    /// crate update cannot wedge the whole TUI — the immediate
    /// caller gets this error, the worker keeps serving subsequent
    /// requests.
    Internal(String),
}

impl KeybaseError {
    /// Convenience constructor for the most common case: a Keybase
    /// API error with just a message, no numeric code.
    pub fn api_message(msg: impl Into<String>) -> Self {
        Self::Api {
            code: None,
            message: msg.into(),
        }
    }

    /// Convenience constructor for shape mismatches.
    pub fn shape(detail: impl Into<String>) -> Self {
        Self::Shape(detail.into())
    }
}

impl fmt::Display for KeybaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "Could not run keybase: {e}"),
            Self::Timeout { label, secs } => {
                write!(f, "{label} timed out after {secs}s")
            }
            Self::Exit { stderr, status } => {
                if stderr.is_empty() {
                    write!(f, "keybase exited with status {status}")
                } else {
                    f.write_str(stderr)
                }
            }
            Self::InvalidJson { family, detail } => {
                write!(f, "keybase {family} api: invalid JSON reply ({detail})")
            }
            Self::Api { message, .. } => f.write_str(message),
            Self::Shape(detail) => f.write_str(detail),
            Self::Internal(msg) => write!(f, "internal adapter panic: {msg}"),
        }
    }
}

impl std::error::Error for KeybaseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_spawn_includes_underlying_cause() {
        let e = KeybaseError::Spawn("No such file or directory".into());
        assert_eq!(
            e.to_string(),
            "Could not run keybase: No such file or directory"
        );
    }

    #[test]
    fn display_timeout_mentions_label_and_secs() {
        let e = KeybaseError::Timeout {
            label: "keybase".into(),
            secs: 30,
        };
        assert_eq!(e.to_string(), "keybase timed out after 30s");
    }

    #[test]
    fn display_exit_with_stderr_passes_it_through() {
        let e = KeybaseError::Exit {
            stderr: "session expired".into(),
            status: 1,
        };
        assert_eq!(e.to_string(), "session expired");
    }

    #[test]
    fn display_exit_without_stderr_falls_back_to_status() {
        let e = KeybaseError::Exit {
            stderr: String::new(),
            status: 7,
        };
        assert_eq!(e.to_string(), "keybase exited with status 7");
    }

    #[test]
    fn display_invalid_json_prefixes_family() {
        let e = KeybaseError::InvalidJson {
            family: "chat".into(),
            detail: "expected value at line 1 column 1".into(),
        };
        assert_eq!(
            e.to_string(),
            "keybase chat api: invalid JSON reply (expected value at line 1 column 1)"
        );
    }

    #[test]
    fn display_api_uses_message_verbatim() {
        let e = KeybaseError::api_message("conv not found");
        assert_eq!(e.to_string(), "conv not found");
    }

    #[test]
    fn display_shape_uses_detail_verbatim() {
        let e = KeybaseError::shape("missing result.conversations");
        assert_eq!(e.to_string(), "missing result.conversations");
    }

    #[test]
    fn error_trait_is_implemented() {
        // Sanity check — KeybaseError must be usable wherever
        // std::error::Error is expected.
        fn assert_error<T: std::error::Error>(_: T) {}
        assert_error(KeybaseError::api_message("x"));
    }
}
