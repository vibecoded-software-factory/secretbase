//! Members view state — the modal listing a channel's members (`Alt+P` on a
//! team channel, or `m` in the channel browser).
//!
//! # Why this is its own type
//!
//! The fourth decomposition seam off the [`App`](crate::tui::app::App) god
//! object, after [`ImagePipeline`](crate::tui::image_pipeline),
//! [`EmojiCatalog`](crate::tui::emoji_catalog) and
//! [`TeamsState`](crate::tui::teams_state). The Members modal owned eleven
//! loose fields on `App` (its target channel + label, the member list and
//! cursor, the return screen, and the add / filter / remove-confirm inline
//! modes) plus two projection methods. They change together and nothing else
//! reads them, so grouping them gives the modal one home and de-stutters the
//! names (`app.members.list` vs `app.members` + `app.members_selected` + …).
//!
//! # What stays outside
//!
//! The `listmembers` / `addtochannel` / `removefromchannel` calls run on the
//! worker and live in the flow layer
//! ([`crate::tui::flows::chat`]), same split as every seam; they read/assign
//! this state but the I/O boundary stays there. This type holds only the view
//! state + the pure filter projections.

use crate::domain::{ChatMember, LineEditor};
use crate::ports::keybase::ReadChannel;
use crate::tui::screens::Screen;

/// The Members modal's state: its target channel, the member list + cursor,
/// the return screen, and the inline add / filter / remove-confirm modes.
/// See the [module docs](self) for the design split.
pub struct MembersState {
    /// Channel/conversation whose members are shown (`None` while closed).
    pub channel: Option<ReadChannel>,
    /// Display label for the title (e.g. `team#channel`).
    pub label: String,
    /// Members of [`Self::channel`] (from `listmembers`), sorted by role.
    pub list: Vec<ChatMember>,
    /// Selected row (indexes the *filtered* projection — [`Self::filtered`]).
    pub selected: usize,
    /// Screen to return to when the modal closes (browser or inbox).
    pub return_to: Screen,
    /// Whether the **add** mode (`a`) is open — a username input.
    pub adding: bool,
    /// Comma/space-separated usernames typed in add mode.
    pub add_input: LineEditor,
    /// `/` filter over the list (username/full-name substring).
    pub filter: LineEditor,
    /// Whether the `/` filter input owns typing.
    pub filtering: bool,
    /// `Some(username)` while an inline **remove** confirm (`x`) is showing.
    pub confirm_remove: Option<String>,
    /// Highlighted button of the inline remove confirm (`false` = cancel —
    /// the destructive default).
    pub remove_yes: bool,
}

impl Default for MembersState {
    fn default() -> Self {
        Self {
            channel: None,
            label: String::new(),
            list: Vec::new(),
            selected: 0,
            // The modal opens over the inbox by default; the opener overrides
            // this with the real return screen (browser or inbox).
            return_to: Screen::Inbox,
            adding: false,
            add_input: LineEditor::default(),
            filter: LineEditor::default(),
            filtering: false,
            confirm_remove: None,
            remove_yes: false,
        }
    }
}

impl MembersState {
    /// Indices into [`Self::list`] matching the `/` filter (all when the query
    /// is empty).
    pub fn filtered(&self) -> Vec<usize> {
        let q = self.filter.text().trim().to_lowercase();
        (0..self.list.len())
            .filter(|&i| q.is_empty() || self.list[i].username.to_lowercase().contains(&q))
            .collect()
    }

    /// The member the cursor is on, through the filter projection.
    pub fn selected_idx(&self) -> Option<usize> {
        self.filtered().get(self.selected).copied()
    }
}
