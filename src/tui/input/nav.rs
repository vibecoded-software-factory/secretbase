//! Shared navigation primitives — arrow keys, PgUp/PgDn, Home/End.

use crate::tui::app::{App, PAGE_STEP};

/// Moves the inbox selection up by one row, scrolling the viewport
/// when the selection leaves it.
pub fn move_up(app: &mut App) {
    if app.list_selected > 0 {
        app.list_selected -= 1;
        if app.list_selected < app.list_scroll {
            app.list_scroll = app.list_selected;
        }
    }
}

/// Moves the inbox selection down by one row.
pub fn move_down(app: &mut App) {
    let max = app.filtered_cache.len().saturating_sub(1);
    if app.list_selected < max {
        app.list_selected += 1;
    }
}

/// Jumps up by [`PAGE_STEP`] rows.
pub fn page_up(app: &mut App) {
    app.list_selected = app.list_selected.saturating_sub(PAGE_STEP);
    if app.list_selected < app.list_scroll {
        app.list_scroll = app.list_selected;
    }
}

/// Jumps down by [`PAGE_STEP`] rows.
pub fn page_down(app: &mut App) {
    let max = app.filtered_cache.len().saturating_sub(1);
    app.list_selected = (app.list_selected + PAGE_STEP).min(max);
}

/// Jumps to the top of the list.
pub fn home(app: &mut App) {
    app.list_selected = 0;
    app.list_scroll = 0;
}

/// Jumps to the bottom of the list.
pub fn end(app: &mut App) {
    app.list_selected = app.filtered_cache.len().saturating_sub(1);
}
