//! Login form state — the signed-out screen's three fields, focus and the
//! paper-key reveal toggle.
//!
//! A cohesive seam off the [`App`](crate::tui::app::App) god object: only the
//! Login screen reads these. The login *flows* (paper-key / native login,
//! status retry) stay in `flows::auth`. [`LoginField`] lives here and is
//! re-exported from `app`, so existing `app::LoginField` paths keep working.

use crate::domain::LineEditor;

/// Focusable element on the **Login** screen form. Tab / Shift+Tab cycle
/// through the three fields then the two action buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginField {
    /// The keybase username (the `[username]` argument).
    Username,
    /// Unique device name (`--devicename`), pre-filled with a default.
    Device,
    /// The paper key (secret, masked unless F2-revealed) — fed to `keybase
    /// login` on stdin.
    PaperKey,
    /// "Log in" button — runs the non-interactive paper-key login.
    SubmitPaperkey,
    /// "Log in in terminal" button — cedes the terminal to interactive
    /// `keybase login` (the only path when the device is already provisioned).
    SubmitNative,
}

impl LoginField {
    /// Cycle order for Tab / Shift+Tab.
    pub const ORDER: [LoginField; 5] = [
        LoginField::Username,
        LoginField::Device,
        LoginField::PaperKey,
        LoginField::SubmitPaperkey,
        LoginField::SubmitNative,
    ];
}

/// The signed-out Login form's state. See the [module docs](self) for the
/// split with the `flows::auth` login flows.
pub struct LoginState {
    /// Username field (the `keybase login [username]` argument). Pre-filled
    /// from `identity.default_username` when known.
    pub username: LineEditor,
    /// Device-name field (`--devicename`), pre-filled with a sensible default.
    pub device: LineEditor,
    /// Paper-key field — secret material, so it rides the `ZeroizeOnDrop`
    /// `LineEditor` like every other input and is masked unless revealed.
    pub paperkey: LineEditor,
    /// Which login element has focus (Tab cycles [`LoginField::ORDER`]).
    pub focus: LoginField,
    /// Whether the paper-key field is shown in clear (F2 toggles it).
    pub reveal: bool,
}

impl Default for LoginState {
    fn default() -> Self {
        Self {
            username: LineEditor::default(),
            device: LineEditor::default(),
            paperkey: LineEditor::default(),
            focus: LoginField::Username,
            reveal: false,
        }
    }
}
