//! Search flows: the Ctrl+G server-side inbox search and the Ctrl+F
//! in-conversation regexp search.

use crate::domain::InboxHit;
use crate::ports::KeybaseError;
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::worker::{InFlight, WorkerRequest};

#[allow(unused_imports)]
use super::*;

// ── Server-side search popup ────────────────────────────────────────

pub fn open_search_global(app: &mut App) {
    app.search_global_input.clear();
    app.search_global_results.clear();
    app.search_global_selected = 0;
    app.screen = crate::tui::screens::Screen::SearchGlobal;
}

pub fn close_search_global(app: &mut App) {
    app.search_global_input.clear();
    app.search_global_results.clear();
    app.search_global_selected = 0;
    app.screen = crate::tui::screens::Screen::Inbox;
}

pub fn request_search_inbox_remote(app: &mut App) {
    let q = app.search_global_input.text().trim().to_string();
    if q.is_empty() {
        app.set_action(ActionState::Error("Query is empty".into()));
        return;
    }
    app.submit(
        InFlight::SearchInboxRemote,
        "Searching…",
        WorkerRequest::SearchInboxHits {
            query: q,
            max_hits: 30,
        },
    );
}

pub fn handle_search_inbox_response(app: &mut App, result: Result<Vec<InboxHit>, KeybaseError>) {
    match result {
        Ok(hits) => {
            let n = hits.len();
            app.search_global_results = hits;
            app.search_global_selected = 0;
            app.set_action(ActionState::Done(format!("{n} matches")));
            app.push_cmd("keybase chat api searchinbox", true, format!("{n} hits"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api searchinbox", false, e.to_string());
        }
    }
}

pub fn open_selected_search_result(app: &mut App) {
    let Some(hit_ref) = app.search_global_results.get(app.search_global_selected) else {
        return;
    };
    let conv_id = hit_ref.conv_id.clone();
    let target = hit_ref.message_id;
    if !app.conversations.iter().any(|c| c.id == conv_id) {
        app.set_action(ActionState::Error(
            "Conversation not in cached inbox — refresh first".into(),
        ));
        return;
    }
    close_search_global(app);
    enter_conversation(app, conv_id);
    // Land on the matched message once the history loads (try_jump_to_search
    // _target, called from the read handlers, paginates older if needed).
    app.pending_search_jump = Some(target);
}

/// After a (paginated) read, jump to + highlight the message the global
/// search targeted. If it's older than what's loaded, pull the next older
/// page and re-check when it arrives; give up once history is exhausted.
pub(crate) fn try_jump_to_search_target(app: &mut App) {
    let Some(target) = app.pending_search_jump else {
        return;
    };
    if let Some(idx) = app.messages.iter().position(|m| m.id == target) {
        // Selecting the message makes the renderer scroll it into view and
        // highlight it (same as Select mode).
        app.pending_search_jump = None;
        app.compose_open = false;
        app.select_from_compose = false;
        app.selected_msg_idx = Some(idx);
        app.set_action(ActionState::Done("Jumped to message".into()));
    } else if app.messages_next.is_some() && !app.messages_loading_older {
        app.messages_loading_older = true;
        request_load_older_messages(app);
    } else if !app.messages_loading_older {
        app.pending_search_jump = None;
        app.set_action(ActionState::Error("Message not found in history".into()));
    }
}

// ── In-conversation search (Ctrl+F → searchregexp) ───────────────────

/// Max matches requested from `searchregexp` for the in-conversation search.
const CONV_SEARCH_MAX_HITS: u32 = 50;

/// Opens the in-conversation search **modal** (`Screen::ConvSearch`, `Ctrl+F`) —
/// a centered `searchregexp` box + results, like the global-search modal. No-op
/// when no conversation is open.
pub fn open_conv_search(app: &mut App) {
    if app.open_conv_id.is_none() {
        return;
    }
    // The previous query + hits are retained (vim keeps the last search
    // pattern): reopening shows them pre-selected, editing the query
    // invalidates them (the input handler clears on change), and `Ctrl+U`
    // clears the line. Switching/closing the conversation resets everything.
    app.screen = crate::tui::screens::Screen::ConvSearch;
}

/// Closes the search modal, back to the inbox/chat. Query + hits are
/// **retained** so `n`/`N` can cycle them from Select mode and a reopen
/// picks up where the search left off — a conversation switch/close is
/// what resets them.
pub fn close_conv_search(app: &mut App) {
    if app.screen == crate::tui::screens::Screen::ConvSearch {
        app.screen = crate::tui::screens::Screen::Inbox;
    }
}

/// Runs `searchregexp` over the open conversation for the current query.
pub fn request_conv_search(app: &mut App) {
    let q = app.conv_search.text().trim().to_string();
    if q.is_empty() {
        app.set_action(ActionState::Error("Search is empty".into()));
        return;
    }
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    app.submit(
        InFlight::ConvSearch,
        "Searching conversation…",
        WorkerRequest::SearchRegexp {
            channel,
            query: q,
            max_hits: CONV_SEARCH_MAX_HITS,
        },
    );
}

pub fn handle_conv_search_response(app: &mut App, result: Result<Vec<InboxHit>, KeybaseError>) {
    match result {
        Ok(hits) => {
            let n = hits.len();
            app.conv_search_results = hits;
            app.conv_search_selected = 0;
            app.set_action(ActionState::Done(format!("{n} matches")));
            app.push_cmd("keybase chat api searchregexp", true, format!("{n} hits"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api searchregexp", false, e.to_string());
        }
    }
}

/// Jumps to the highlighted in-conversation match: closes the search and
/// reuses the message-jump path (finds it in the loaded history, paginating
/// older if needed).
/// `n`/`N` in Select mode — jump to the next/previous retained search hit,
/// wrapping (vim's post-`/` motion). Points at `Ctrl+F` when there is
/// nothing to cycle.
pub fn conv_search_cycle(app: &mut App, delta: isize) {
    let len = app.conv_search_results.len();
    if len == 0 {
        app.set_action(ActionState::Error(
            "No search hits — Ctrl+F (or / in Select) to search".into(),
        ));
        return;
    }
    let cur = app.conv_search_selected as isize;
    app.conv_search_selected = (cur + delta).rem_euclid(len as isize) as usize;
    let Some(hit) = app.conv_search_results.get(app.conv_search_selected) else {
        return;
    };
    let target = hit.message_id;
    app.focus = crate::tui::screens::Focus::Chat;
    app.pending_search_jump = Some(target);
    try_jump_to_search_target(app);
    // Position within the hit list, tig/less-style — the jump toast alone
    // says nothing about how many matches remain.
    if app.pending_search_jump.is_none() {
        app.set_action(ActionState::Done(format!(
            "Hit {} of {len}",
            app.conv_search_selected + 1
        )));
    }
}

pub fn conv_search_jump_selected(app: &mut App) {
    let Some(hit) = app.conv_search_results.get(app.conv_search_selected) else {
        return;
    };
    let target = hit.message_id;
    close_conv_search(app);
    // Land focus on the chat so the jumped-to message is in Select mode.
    app.focus = crate::tui::screens::Focus::Chat;
    app.pending_search_jump = Some(target);
    try_jump_to_search_target(app);
}

// ── GIF search (giphy, user API key) ─────────────────────────────────

/// Opens the GIF-search popup (the compose bar's `GIF` chip / `Alt+G`).
/// Needs an open conversation and a configured `giphy_api_key` — without a
/// key the toast points at Settings instead of opening a dead popup.
pub fn open_giphy_search(app: &mut App) {
    if app.open_conv_id.is_none() {
        return;
    }
    if app.settings_cache.giphy_api_key.trim().is_empty() {
        app.set_action(ActionState::Error(
            "GIF search needs a Giphy API key — Settings → Images (free at developers.giphy.com)"
                .into(),
        ));
        return;
    }
    app.giphy_input.clear();
    app.giphy_results.clear();
    app.giphy_selected = 0;
    app.screen = crate::tui::screens::Screen::GiphySearch;
}

pub fn close_giphy_search(app: &mut App) {
    app.giphy_input.clear();
    app.giphy_results.clear();
    app.giphy_selected = 0;
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Runs the search for the typed query (Enter with no results yet, or F5).
pub fn request_giphy_search(app: &mut App) {
    let query = app.giphy_input.text().trim().to_string();
    if query.is_empty() {
        app.set_action(ActionState::Error("Type something to search".into()));
        return;
    }
    let api_key = zeroize::Zeroizing::new(app.settings_cache.giphy_api_key.trim().to_string());
    app.submit(
        crate::tui::worker::InFlight::GiphySearch,
        "Searching giphy…",
        WorkerRequest::GiphySearch { api_key, query },
    );
}

pub fn handle_giphy_search_response(
    app: &mut App,
    result: Result<Vec<crate::domain::GiphyHit>, KeybaseError>,
) {
    match result {
        Ok(hits) => {
            app.set_action(ActionState::Done(format!("{} GIFs", hits.len())));
            // The key never reaches the log — only the outcome.
            app.push_cmd("giphy search", true, format!("{} hits", hits.len()));
            app.giphy_selected = 0;
            app.giphy_results = hits;
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("giphy search", false, e.to_string());
        }
    }
}

/// Enter on a hit: sends its media URL to the open conversation as a text
/// message — the GUI unfurls it, this TUI renders it inline. Goes through
/// the same optimistic outbox as a normal send, but **never touches the
/// compose draft** (typed text is sacred).
pub fn giphy_send_selected(app: &mut App) {
    let Some(hit) = app.giphy_results.get(app.giphy_selected).cloned() else {
        return;
    };
    let Some((conv_id, channel)) = super::open_channel(app) else {
        return;
    };
    if !app.begin(crate::tui::worker::InFlight::SendMessage {
        body_len: hit.url.len(),
        was_reply: false,
    }) {
        return;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    app.outbox.push(crate::tui::app::PendingSend {
        conv_id,
        body: hit.url.clone(),
        reply_to: None,
        sent_at_ms: now_ms,
        state: crate::tui::app::SendState::Pending,
    });
    app.messages_scroll = 0;
    close_giphy_search(app);
    app.set_action(ActionState::Running("Sending GIF…".into()));
    let _ = app.worker_tx.send(WorkerRequest::SendMessage {
        channel,
        body: hit.url,
        reply_to: None,
    });
}
