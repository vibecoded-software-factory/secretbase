#![forbid(unsafe_code)]

//! Secretbase — TUI for the Keybase CLI.
//!
//! # Architecture
//!
//! The crate follows a hexagonal (ports & adapters) layout. Reading
//! order matches the dependency direction:
//!
//! 1. [`domain`]   — pure types and rules (no I/O).
//! 2. [`ports`]    — trait abstractions over external dependencies.
//! 3. [`adapters`] — concrete adapters: `keybase` CLI, system clipboard,
//!    TOML settings file.
//! 4. [`tui`]      — the terminal UI (state, input, view, flows). This
//!    is itself a *driving* adapter — it calls into the ports through
//!    trait objects.
//!
//! `main.rs` is the composition root: it wires the concrete adapters to
//! [`tui::run`] and starts the loop.
//!
//! ```text
//!  main ──► tui ──► flows ──► ports ◄── adapters
//!                       ▲
//!                       └── domain (used by every layer above)
//! ```
//!
//! Every layer above `domain` depends on `domain`. No layer depends on
//! the `tui` layer except `main`. Adapters depend on `ports` and
//! `domain` only — never on the `tui`. This makes it possible to swap
//! any adapter (or replace the TUI with a different driver entirely)
//! without touching the rest of the code.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod tui;
