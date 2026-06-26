//! Clipboard port — abstraction over the system clipboard.

/// Abstraction over the OS clipboard.
///
/// The concrete implementation is
/// [`crate::adapters::clipboard_system::SystemClipboardAdapter`].
pub trait ClipboardPort {
    /// Writes `text` to the system clipboard.
    fn write(&self, text: &str) -> Result<(), String>;

    /// Writes `text` to the system clipboard and clears it again after
    /// `clear_after_secs` seconds, **unless** the clipboard contents
    /// have changed in the meantime (so we never wipe out something
    /// the user copied themselves).
    ///
    /// Adapters that cannot implement the auto-clear safely fall back
    /// to the plain [`Self::write`] via the default impl.
    fn write_with_clear(&self, text: &str, _clear_after_secs: u64) -> Result<(), String> {
        self.write(text)
    }
}
