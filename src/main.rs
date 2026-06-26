#![forbid(unsafe_code)]

//! Composition root — instantiates concrete adapters and starts the
//! [`secretbase::tui`] event loop.

use secretbase::adapters::{KeybaseCliAdapter, SystemClipboardAdapter, TomlSettingsAdapter};
use secretbase::ports::{KeybasePort, SettingsPort};
use secretbase::tui;

use color_eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;

    // Read settings before constructing the keybase adapter so the
    // configurable `keybase chat api list` timeout (used by users with
    // very large inboxes) is in effect from the very first call. The
    // TUI also reads the same `UserSettings` via its own port, so the
    // double-read is intentional — keeps the composition root the
    // single source of truth for adapter wiring without coupling the
    // TUI to the adapter constructor.
    let settings_adapter = TomlSettingsAdapter::new();
    let cfg = settings_adapter.read();

    // `Send` bound: the keybase port crosses into the worker thread
    // owned by `tui::run`. `KeybaseCliAdapter` is Send by default
    // (only `u64` fields); the annotation here makes the requirement
    // explicit at the composition root.
    let keybase: Box<dyn KeybasePort + Send> = Box::new(
        KeybaseCliAdapter::new()
            .with_list_inbox_timeout(cfg.list_inbox_timeout_secs)
            .with_download_timeout(cfg.download_timeout_secs),
    );
    // Second, independent adapter for the background lane (idle inbox
    // auto-refresh) so it never head-of-line-blocks the user's lane.
    let keybase_bg: Box<dyn KeybasePort + Send> =
        Box::new(KeybaseCliAdapter::new().with_list_inbox_timeout(cfg.list_inbox_timeout_secs));
    let clipboard = Box::new(SystemClipboardAdapter::new());
    let settings = Box::new(settings_adapter);

    tui::run(keybase, keybase_bg, clipboard, settings)
}
