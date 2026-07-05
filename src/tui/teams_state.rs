//! Teams screen state — the loaded team memberships plus the list's cursor,
//! scroll offset and `/` name filter.
//!
//! # Why this is its own type
//!
//! The third decomposition seam off the [`App`](crate::tui::app::App) god
//! object, after [`ImagePipeline`](crate::tui::image_pipeline) and
//! [`EmojiCatalog`](crate::tui::emoji_catalog). The Teams screen's five
//! fields (the membership list + its selection, scroll, filter editor and
//! filtering flag) are a self-contained list-view state: they change
//! together and nothing else reads them. Grouping them de-stutters the field
//! names (`app.teams.list` vs `app.teams` + `app.teams_selected` + …) and
//! gives the screen one place to hold its state.
//!
//! # What stays outside
//!
//! The **load** ([`request_load_teams`](crate::tui::flows::teams::request_load_teams))
//! runs on the worker and lives in the flow layer, same split as the other
//! seams; it assigns the parsed rows into [`TeamsState::list`]. This type
//! holds only the view state + the pure filter projection.

use crate::domain::LineEditor;
use crate::domain::TeamMembership;

/// The Teams screen's list state: the memberships and the list's cursor,
/// scroll and `/` filter. See the [module docs](self) for the design split.
#[derive(Default)]
pub struct TeamsState {
    /// Team memberships from `keybase team api list-user-memberships`, one row
    /// per team the user is in. Assigned by the flow-layer load.
    pub list: Vec<TeamMembership>,
    /// Currently selected row (indexes the *filtered* projection —
    /// [`Self::filtered`]).
    pub selected: usize,
    /// Persisted list viewport offset (it used to reset every frame — the
    /// list snapped to the top on each redraw).
    pub scroll: usize,
    /// `/` filter over the list (name substring, case-insensitive).
    pub filter: LineEditor,
    /// Whether the `/` filter input owns typing.
    pub filtering: bool,
}

impl TeamsState {
    /// Indices into [`Self::list`] matching the `/` filter (all when the query
    /// is empty) — what the Teams list renders and [`Self::selected`] indexes.
    pub fn filtered(&self) -> Vec<usize> {
        let q = self.filter.text().trim().to_lowercase();
        (0..self.list.len())
            .filter(|&i| q.is_empty() || self.list[i].name.to_lowercase().contains(&q))
            .collect()
    }
}
