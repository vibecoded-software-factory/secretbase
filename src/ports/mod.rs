//! Ports — trait abstractions for everything outside the domain.
//!
//! In the hexagonal-architecture sense, a *port* is the inward-facing API
//! that the application uses to talk to the outside world. Concrete
//! implementations (the *adapters*) live in [`crate::adapters`].
//!
//! Adding a new backend (e.g. a direct HTTP adapter for the Keybase API,
//! or a fake adapter for tests) is a matter of implementing the relevant
//! trait and wiring it up at the composition root in `main.rs`.

pub mod clipboard;
pub mod error;
pub mod keybase;
pub mod settings;

pub use clipboard::ClipboardPort;
pub use error::KeybaseError;
pub use keybase::{KeybasePort, ListConversationsOk, ListTeamsOk, ParallelSessionData};
pub use settings::{SettingsPort, UserSettings};
