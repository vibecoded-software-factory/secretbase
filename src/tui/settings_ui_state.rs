//! Settings overlay UI state — the modal's navigation (which pane/section/row
//! has focus, the live theme-picker index) and the secret-editing sub-popup.
//!
//! # Why this is its own type
//!
//! A cohesive seam off the [`App`](crate::tui::app::App) god object: seven
//! fields that only the `F10` Settings overlay reads. The **applying** methods
//! (`settings_adjust`, `apply_theme_idx`, `settings_secret_save`, …) stay on
//! `App` because they persist to `settings_cache` and re-resolve the theme /
//! image pipeline — the same bridge pattern as the other seams. This type
//! holds only the overlay's navigation state.

use crate::domain::LineEditor;
use crate::tui::screens::Screen;
use crate::tui::settings_model::{SettingId, SettingsFocus};

/// The Settings overlay's navigation + secret-editing state. See the
/// [module docs](self) for the split with the applying `App` methods.
pub struct SettingsUiState {
    /// Which pane of the overlay holds focus.
    pub focus: SettingsFocus,
    /// Highlighted section in the sidebar (index into `SettingsSection::ALL`).
    pub section: usize,
    /// Highlighted row within the active section's panel.
    pub item: usize,
    /// Highlighted preset in the Theme panel (index into `theme::Preset::ALL`).
    /// Applies live as it moves.
    pub theme_idx: usize,
    /// Screen the overlay was opened from (returned to on close).
    pub from: Screen,
    /// Secret setting being edited in the input popup, if any — with
    /// [`Self::input`] as its editor.
    pub editing: Option<SettingId>,
    /// Editor for the secret-setting popup. `ZeroizeOnDrop` like every other
    /// input — it holds an API key while open.
    pub input: LineEditor,
}

impl Default for SettingsUiState {
    fn default() -> Self {
        Self {
            focus: SettingsFocus::Sidebar,
            section: 0,
            item: 0,
            theme_idx: 0,
            // The overlay opens over the inbox by default; `open_settings`
            // overrides this with the real return screen.
            from: Screen::Inbox,
            editing: None,
            input: LineEditor::default(),
        }
    }
}
