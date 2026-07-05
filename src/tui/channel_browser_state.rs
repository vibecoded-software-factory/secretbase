//! Channel browser state — the modal listing a team's channels (`c` on a
//! team), with its inline create / rename / delete / filter modes and the
//! team's default-channel set.
//!
//! # Why this is its own type
//!
//! The fifth decomposition seam off the [`App`](crate::tui::app::App) god
//! object, closing the browser family started by
//! [`TeamsState`](crate::tui::teams_state) and
//! [`MembersState`](crate::tui::members_state). The channel browser owned
//! eleven loose fields on `App` — the team being browsed, its channels and
//! cursor, the shared create/rename name editor, the `/` filter, the
//! delete-confirm, and the team's default-channel set — plus two projection
//! methods. They change together and nothing else reads them, so grouping
//! them gives the modal one home and de-stutters the names
//! (`app.channel_browser.channels` vs `app.channels` + `app.channel_selected`
//! + …).
//!
//! # What stays outside
//!
//! The `listconvsonname` / `join` / `leave` / `newconv` /
//! `rename-channel` / `delete-channel` / `default-channels` calls run on the
//! worker and live in the flow layer ([`crate::tui::flows::chat`]), same split
//! as every seam; they read/assign this state but the I/O boundary stays
//! there. This type holds only the view state + the pure filter projections.

use crate::domain::{Conversation, LineEditor};

/// The channel browser modal's state: the team being browsed, its channels
/// and cursor, the inline create / rename / delete / filter modes, and the
/// team's default-channel set. See the [module docs](self) for the split.
#[derive(Default)]
pub struct ChannelBrowserState {
    /// Team whose channels the browser is showing (`None` while closed).
    pub team: Option<String>,
    /// Channels of [`Self::team`] from `listconvsonname` — **all** of them,
    /// joined or not (`member_status == Active` means you're a member).
    pub channels: Vec<Conversation>,
    /// Selected row (indexes the *filtered* projection — [`Self::filtered`]).
    pub selected: usize,
    /// Whether the inline **create** mode (`n`) is open — the new-channel name
    /// input is shown; `Enter` creates it (`newconv`), `Esc` cancels.
    pub creating: bool,
    /// New-channel name typed in create/rename mode (shared input).
    pub new_name: LineEditor,
    /// `/` filter over the browser (topic substring).
    pub filter: LineEditor,
    /// Whether the `/` filter input owns typing.
    pub filtering: bool,
    /// `Some(old)` while **renaming** a channel (`r`): the old channel name;
    /// the new name is typed into [`Self::new_name`].
    pub renaming: Option<String>,
    /// `Some(topic)` while an inline **delete** confirm (`x`) is showing —
    /// destructive + irreversible, so `y` confirms / `n`/`Esc` cancels.
    pub confirm_delete: Option<String>,
    /// Highlighted button of the inline delete confirm (`false` = cancel —
    /// the destructive default).
    pub delete_yes: bool,
    /// The team's **default channels** (new members auto-join these), fetched
    /// alongside the browser list; `#general` is always default and omitted.
    /// `t` toggles the selected channel's membership in this set.
    pub defaults: Vec<String>,
}

impl ChannelBrowserState {
    /// Indices into [`Self::channels`] matching the `/` filter (all when the
    /// query is empty) — what the browser renders and [`Self::selected`]
    /// indexes.
    pub fn filtered(&self) -> Vec<usize> {
        let q = self.filter.text().trim().to_lowercase();
        (0..self.channels.len())
            .filter(|&i| {
                q.is_empty()
                    || self.channels[i]
                        .channel
                        .topic_name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
            })
            .collect()
    }

    /// The channel the cursor is on, through the filter projection.
    pub fn selected_idx(&self) -> Option<usize> {
        self.filtered().get(self.selected).copied()
    }
}
