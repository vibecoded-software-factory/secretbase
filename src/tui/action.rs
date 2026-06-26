//! User-feedback strip + command log entries.
//!
//! The "queued action" concept moved to
//! [`crate::tui::worker::InFlight`] when the keybase port was lifted
//! into a dedicated thread. This module now only owns the visible
//! feedback strip state and the command-log row type.

/// Visible state of the feedback strip / status area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionState {
    /// Nothing is happening — the strip is blank.
    Idle,
    /// A worker call is in flight; the wrapped string is the label
    /// shown next to the spinner.
    Running(String),
    /// Last operation succeeded — auto-clears after ~1.5s.
    Done(String),
    /// Last operation failed — auto-clears after ~1.5s.
    Error(String),
}

/// Single entry in the command-log panel.
#[derive(Debug, Clone)]
pub struct CmdEntry {
    /// Verbatim shell representation of the call (no session keys to
    /// redact — Keybase holds those in the local service).
    pub cmd: String,
    /// Whether the command succeeded.
    pub ok: bool,
    /// Free-form trailing text — error message or summary.
    pub detail: String,
    /// How long the worker operation took (request → response), when
    /// this entry corresponds to one. `None` for purely local log
    /// entries (clipboard writes, parse warnings, refused requests).
    pub duration: Option<std::time::Duration>,
}
