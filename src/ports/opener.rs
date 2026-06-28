//! Opener port — abstraction over "open this URL in the user's browser".

/// Opens a URL with the operating system's default handler.
///
/// The concrete implementation is
/// [`crate::adapters::opener_system::SystemOpener`]; tests inject a fake.
pub trait OpenerPort {
    /// Hands `url` to the OS opener (`xdg-open` / `open` / `cmd /c start`).
    /// Returns an error string when no opener is available or it fails to
    /// launch.
    fn open(&self, url: &str) -> Result<(), String>;
}
