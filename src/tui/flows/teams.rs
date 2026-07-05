//! Teams flows.
//!
//! Currently covers the "list self memberships" screen — the entry
//! point for the second pillar of the MVP. Per-team detail (members,
//! roles, sub-teams) is a follow-up.

use crate::ports::KeybaseError;
use crate::ports::keybase::ListTeamsOk;
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};

/// Switches to the teams screen and queues a fresh
/// `list-self-memberships` load. Selects row 0.
pub fn open_teams(app: &mut App) {
    app.teams_selected = 0;
    app.screen = Screen::Teams;
    request_load_teams(app);
}

/// Returns to the inbox.
pub fn close_teams(app: &mut App) {
    app.screen = Screen::Inbox;
}

/// Queues `keybase team api {"method":"list-self-memberships"}`.
pub fn request_load_teams(app: &mut App) {
    // `list-user-memberships` (see the port) needs our own username.
    let username = app.identity.username.clone();
    if username.is_empty() {
        app.set_action(ActionState::Error(
            "Not signed in — can't list teams".into(),
        ));
        return;
    }
    app.submit(
        InFlight::LoadTeams,
        "Loading teams…",
        WorkerRequest::ListSelfMemberships { username },
    );
}

/// Applies the worker response — populates [`App::teams`] and clamps
/// the row selection. Per-row decode warnings go into the command
/// log instead of failing the whole load.
pub fn handle_load_teams_response(app: &mut App, result: Result<ListTeamsOk, KeybaseError>) {
    match result {
        Ok(load) => {
            let skipped_count = load.skipped.len();
            // Safety net: collapse any duplicate team rows (one row per team is
            // expected from list-user-memberships, but never show the repeated
            // mess `list-self-memberships` produced).
            let mut teams = load.teams;
            let mut seen = std::collections::HashSet::new();
            teams.retain(|t| seen.insert(t.name.clone()));
            teams.sort_by(|a, b| a.name.cmp(&b.name));
            let n = teams.len();
            app.teams = teams;
            if app.teams_selected >= app.teams.len() {
                app.teams_selected = app.teams.len().saturating_sub(1);
            }
            let summary = if skipped_count == 0 {
                format!("{n} teams")
            } else {
                format!("{n} teams ({skipped_count} skipped)")
            };
            app.set_action(ActionState::Done(format!("Loaded {summary}")));
            app.push_cmd("keybase team api list-user-memberships", true, summary);
            for diag in load.skipped {
                app.push_cmd("team parse warning", false, diag);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd(
                "keybase team api list-self-memberships",
                false,
                e.to_string(),
            );
        }
    }
}

pub fn move_up(app: &mut App) {
    app.teams_selected = app.teams_selected.saturating_sub(1);
}

pub fn move_down(app: &mut App) {
    let max = app.teams.len().saturating_sub(1);
    app.teams_selected = (app.teams_selected + 1).min(max);
}
