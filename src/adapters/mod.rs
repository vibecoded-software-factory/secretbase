//! Adapters — concrete implementations of the [`crate::ports`] traits.
//!
//! This is the only layer allowed to import OS-level dependencies
//! (`std::process::Command`, the filesystem, environment variables).
//! The rest of the crate talks to these adapters through the trait
//! abstractions, so swapping or mocking them is straightforward.

pub mod clipboard_system;
pub mod keybase_cli;
pub mod settings_toml;

pub use clipboard_system::SystemClipboardAdapter;
pub use keybase_cli::KeybaseCliAdapter;
pub use settings_toml::TomlSettingsAdapter;
