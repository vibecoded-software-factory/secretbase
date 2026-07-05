//! Team-channel flows: the channel browser (join/leave/create/rename/
//! delete/defaults) and the members view.

use crate::ports::KeybaseError;
use crate::ports::keybase::{ListConversationsOk, ReadChannel};
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};

#[allow(unused_imports)]
use super::*;

// ── Channel browser (`c` on a team) ───────────────────────────────

/// The channel name of a team-channel conversation (its `topic_name`), used for
/// display and to build the `join`/`leave` channel.
fn channel_topic(c: &crate::domain::Conversation) -> String {
    c.channel.topic_name.clone().unwrap_or_default()
}

/// Whether a channel row is one you're a member of.
fn channel_joined(c: &crate::domain::Conversation) -> bool {
    c.member_status == crate::domain::MemberStatus::Active
}

/// Opens the channel browser for the team of the selected tree row (a team
/// group header, or a team channel). Errors when the cursor isn't on a team.
pub fn open_channel_browser(app: &mut App) {
    use crate::tui::app::TreeRow;
    let team = match app.tree_rows().get(app.tree_selected) {
        Some(TreeRow::Group {
            is_team: true, key, ..
        }) => Some(key.clone()),
        Some(TreeRow::Conv { idx }) => {
            let c = &app.conversations[*idx];
            c.channel
                .members_type
                .is_team()
                .then(|| c.channel.name.clone())
        }
        _ => None,
    };
    let Some(team) = team else {
        app.set_action(ActionState::Error(
            "Select a team (or a team channel) first".into(),
        ));
        return;
    };
    open_channel_browser_for_team(app, team);
}

/// Opens the channel browser for a named team (e.g. from the Teams screen).
pub fn open_channel_browser_for_team(app: &mut App, team: String) {
    app.channel_browser_team = Some(team);
    app.channels.clear();
    app.channel_selected = 0;
    app.default_channels.clear();
    clear_channel_input(app);
    app.screen = crate::tui::screens::Screen::ChannelBrowser;
    request_load_channels(app);
}

pub fn close_channel_browser(app: &mut App) {
    app.channel_browser_team = None;
    app.channels.clear();
    app.channel_selected = 0;
    app.default_channels.clear();
    clear_channel_input(app);
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Clears every inline browser mode (create / rename / delete-confirm).
fn clear_channel_input(app: &mut App) {
    app.channel_creating = false;
    app.channel_renaming = None;
    app.channel_confirm_delete = None;
    app.channel_new_name.clear();
}

/// Enters create mode in the browser (`Alt+N`): the new-channel-name input.
pub fn open_channel_create(app: &mut App) {
    if app.channel_browser_team.is_none() {
        return;
    }
    clear_channel_input(app);
    app.channel_creating = true;
}

/// Cancels create mode, back to the channel list.
pub fn cancel_channel_create(app: &mut App) {
    clear_channel_input(app);
}

// ── Rename channel (r) ───────────────────────────────────────────────

/// Enters rename mode on the selected channel (`r`), pre-filling its name.
pub fn open_channel_rename(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let old = channel_topic(c);
    clear_channel_input(app);
    app.channel_new_name.set(old.clone());
    app.channel_renaming = Some(old);
}

pub fn cancel_channel_rename(app: &mut App) {
    clear_channel_input(app);
}

pub fn request_rename_channel(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let Some(old) = app.channel_renaming.clone() else {
        return;
    };
    let new = app.channel_new_name.text().trim().to_lowercase();
    if new.is_empty() {
        app.set_action(ActionState::Error("New channel name is empty".into()));
        return;
    }
    if new == old {
        cancel_channel_rename(app);
        return;
    }
    app.submit(
        InFlight::RenameChannel { topic: new.clone() },
        &format!("Renaming #{old} → #{new}…"),
        WorkerRequest::RenameChannel { team, old, new },
    );
}

pub fn handle_rename_channel_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    topic: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Renamed to #{topic}")));
            app.push_cmd("keybase chat rename-channel", true, format!("#{topic}"));
            clear_channel_input(app);
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat rename-channel", false, e.to_string());
        }
    }
}

// ── Delete channel (d, inline confirm) ───────────────────────────────

/// Opens the inline delete confirm for the selected channel (`d`).
pub fn open_channel_delete_confirm(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let topic = channel_topic(c);
    clear_channel_input(app);
    app.channel_confirm_delete = Some(topic);
    app.channel_delete_yes = false; // destructive → default highlight = cancel
}

pub fn cancel_channel_delete(app: &mut App) {
    clear_channel_input(app);
}

/// Commits the pending channel delete (`y`). Destructive + irreversible.
pub fn confirm_channel_delete(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let Some(topic) = app.channel_confirm_delete.clone() else {
        return;
    };
    app.channel_confirm_delete = None;
    app.submit(
        InFlight::DeleteChannel {
            topic: topic.clone(),
        },
        &format!("Deleting #{topic}…"),
        WorkerRequest::DeleteChannel {
            team,
            channel: topic,
        },
    );
}

pub fn handle_delete_channel_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    topic: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Deleted #{topic}")));
            app.push_cmd("keybase chat delete-channel", true, format!("#{topic}"));
            clear_channel_input(app);
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat delete-channel", false, e.to_string());
        }
    }
}

// ── Default channels (t: toggle) ─────────────────────────────────────

/// Fetches the team's default channels (get-only), chained after the list load
/// so the browser can badge them. Quiet — no error toast if it fails (badges
/// just stay empty).
pub fn request_get_default_channels(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    if !app.begin(InFlight::DefaultChannels { setting: false }) {
        return;
    }
    let _ = app.worker_tx.send(WorkerRequest::DefaultChannels {
        team,
        set: Vec::new(),
    });
}

/// `t`: toggles the selected channel in the team's default set (SET replaces the
/// whole set, so we recompute it). `#general` is always default; and the CLI
/// can't clear the set to empty (`--channel`-less = get), so removing the last
/// one is refused with an explanation.
pub fn toggle_default_channel(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let topic = channel_topic(c);
    if topic == "general" {
        app.set_action(ActionState::Error(
            "#general is always a default channel".into(),
        ));
        return;
    }
    let mut set = app.default_channels.clone();
    if let Some(pos) = set.iter().position(|x| *x == topic) {
        set.remove(pos);
    } else {
        set.push(topic);
    }
    if set.is_empty() {
        app.set_action(ActionState::Error(
            "Keybase can't clear the last default channel via the CLI".into(),
        ));
        return;
    }
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    app.submit(
        InFlight::DefaultChannels { setting: true },
        "Updating default channels…",
        WorkerRequest::DefaultChannels { team, set },
    );
}

pub fn handle_default_channels_response(
    app: &mut App,
    result: Result<Vec<String>, KeybaseError>,
    setting: bool,
) {
    match result {
        Ok(names) => {
            let n = names.len();
            app.default_channels = names;
            if setting {
                app.set_action(ActionState::Done("Default channels updated".into()));
                app.push_cmd("keybase chat default-channels", true, "updated");
            } else {
                // Quiet load-time get — just clear the spinner.
                app.set_action(ActionState::Idle);
                app.push_cmd(
                    "keybase chat default-channels",
                    true,
                    format!("{n} default(s)"),
                );
            }
        }
        Err(e) => {
            // A get failure is non-critical (badges stay empty); a set failure
            // is a real error the user should see.
            if setting {
                app.set_action(ActionState::Error(e.to_string()));
            } else {
                app.set_action(ActionState::Idle);
            }
            app.push_cmd("keybase chat default-channels", false, e.to_string());
        }
    }
}

// ── Members view (listmembers / addtochannel / removefromchannel) ────

/// Clears the Members view's inline modes (add input / remove confirm).
fn clear_member_input(app: &mut App) {
    app.member_adding = false;
    app.member_add_input.clear();
    app.member_confirm_remove = None;
}

/// Opens the Members view for `channel` (labelled `label`), returning to
/// `return_to` on close, and loads the member list.
fn open_members(app: &mut App, channel: ReadChannel, label: String, return_to: Screen) {
    app.members_channel = Some(channel);
    app.members_label = label;
    app.members.clear();
    app.members_selected = 0;
    app.members_return = return_to;
    clear_member_input(app);
    app.screen = Screen::Members;
    request_load_members(app);
}

/// `m` in the channel browser: members of the selected channel.
pub fn open_members_from_browser(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let topic = channel_topic(c);
    let channel = ReadChannel {
        name: team.clone(),
        members_type: "team".into(),
        topic_name: Some(topic.clone()),
    };
    open_members(
        app,
        channel,
        format!("{team}#{topic}"),
        Screen::ChannelBrowser,
    );
}

/// `Alt+P` on an open team channel: its members. DMs/non-team convs have a
/// fixed membership, so it's refused there.
pub fn open_members_from_conversation(app: &mut App) {
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    if conv.channel.members_type != crate::domain::MembersType::Team {
        app.set_action(ActionState::Error(
            "Members is only available for team channels".into(),
        ));
        return;
    }
    let label = format!(
        "{}#{}",
        conv.channel.name,
        conv.channel.topic_name.as_deref().unwrap_or("general")
    );
    let channel = match read_channel_from_conv(conv) {
        Ok(ch) => ch,
        Err(_) => return,
    };
    open_members(app, channel, label, Screen::Inbox);
}

pub fn close_members(app: &mut App) {
    let return_to = app.members_return;
    app.members_channel = None;
    app.members.clear();
    app.members_selected = 0;
    clear_member_input(app);
    app.screen = return_to;
}

pub fn request_load_members(app: &mut App) {
    let Some(channel) = app.members_channel.clone() else {
        return;
    };
    app.submit(
        InFlight::LoadMembers,
        "Loading members…",
        WorkerRequest::LoadMembers { channel },
    );
}

pub fn handle_load_members_response(
    app: &mut App,
    result: Result<Vec<crate::domain::ChatMember>, KeybaseError>,
) {
    match result {
        Ok(mut members) => {
            // Higher-privilege roles first, then alphabetical by username.
            members.sort_by(|a, b| {
                a.role
                    .sort_rank()
                    .cmp(&b.role.sort_rank())
                    .then_with(|| a.username.cmp(&b.username))
            });
            let n = members.len();
            app.members = members;
            app.members_selected = app
                .members_selected
                .min(app.members.len().saturating_sub(1));
            app.set_action(ActionState::Done(format!("{n} members")));
            app.push_cmd("keybase chat api listmembers", true, format!("{n} members"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api listmembers", false, e.to_string());
        }
    }
}

pub fn members_move(app: &mut App, delta: isize) {
    let len = app.members.len();
    if len == 0 {
        return;
    }
    app.members_selected =
        (app.members_selected as isize + delta).clamp(0, len as isize - 1) as usize;
}

// add member(s)

pub fn open_member_add(app: &mut App) {
    if app.members_channel.is_none() {
        return;
    }
    clear_member_input(app);
    app.member_adding = true;
}

pub fn cancel_member_add(app: &mut App) {
    clear_member_input(app);
}

pub fn request_add_members(app: &mut App) {
    use crate::domain::is_valid_keybase_identity;
    let Some(channel) = app.members_channel.clone() else {
        return;
    };
    // Accept comma / whitespace separated usernames.
    let usernames: Vec<String> = app
        .member_add_input
        .text()
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    if usernames.is_empty() {
        app.set_action(ActionState::Error("No usernames entered".into()));
        return;
    }
    if let Some(bad) = usernames.iter().find(|u| !is_valid_keybase_identity(u)) {
        app.set_action(ActionState::Error(format!("Invalid username: {bad}")));
        return;
    }
    let count = usernames.len();
    app.submit(
        InFlight::AddToChannel { count },
        &format!("Adding {count} member(s)…"),
        WorkerRequest::AddToChannel { channel, usernames },
    );
}

pub fn handle_add_members_response(app: &mut App, result: Result<(), KeybaseError>, count: usize) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Added {count} member(s)")));
            app.push_cmd(
                "keybase chat api addtochannel",
                true,
                format!("{count} added"),
            );
            clear_member_input(app);
            request_load_members(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api addtochannel", false, e.to_string());
        }
    }
}

// remove member (inline confirm)

pub fn open_member_remove_confirm(app: &mut App) {
    let Some(m) = app.members.get(app.members_selected) else {
        return;
    };
    let username = m.username.clone();
    clear_member_input(app);
    app.member_confirm_remove = Some(username);
    app.member_remove_yes = false; // destructive → default highlight = cancel
}

pub fn cancel_member_remove(app: &mut App) {
    clear_member_input(app);
}

pub fn confirm_remove_member(app: &mut App) {
    let Some(channel) = app.members_channel.clone() else {
        return;
    };
    let Some(username) = app.member_confirm_remove.clone() else {
        return;
    };
    app.member_confirm_remove = None;
    app.submit(
        InFlight::RemoveFromChannel {
            username: username.clone(),
        },
        &format!("Removing {username}…"),
        WorkerRequest::RemoveFromChannel {
            channel,
            usernames: vec![username],
        },
    );
}

pub fn handle_remove_member_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    username: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Removed {username}")));
            app.push_cmd("keybase chat api removefromchannel", true, username);
            clear_member_input(app);
            request_load_members(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api removefromchannel", false, e.to_string());
        }
    }
}

/// Creates a new channel on the browsed team (`newconv` with a team channel).
pub fn request_create_channel(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let topic = app.channel_new_name.text().trim().to_lowercase();
    if topic.is_empty() {
        app.set_action(ActionState::Error("Channel name is empty".into()));
        return;
    }
    let channel = ReadChannel {
        name: team,
        members_type: "team".into(),
        topic_name: Some(topic.clone()),
    };
    // Reuses the newconv worker request; routed to the channel handler by the
    // CreateChannel in-flight slot.
    app.submit(
        InFlight::CreateChannel {
            topic: topic.clone(),
        },
        &format!("Creating #{topic}…"),
        WorkerRequest::NewConversation { channel },
    );
}

pub fn handle_create_channel_response(
    app: &mut App,
    result: Result<String, KeybaseError>,
    topic: String,
) {
    match result {
        Ok(_) => {
            app.set_action(ActionState::Done(format!("Created #{topic}")));
            app.push_cmd(
                "keybase chat api newconv (channel)",
                true,
                format!("#{topic}"),
            );
            app.channel_creating = false;
            app.channel_new_name.clear();
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api newconv (channel)", false, e.to_string());
        }
    }
}

/// Loads every channel of `channel_browser_team` (`listconvsonname`).
pub fn request_load_channels(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    app.submit(
        InFlight::LoadChannels,
        &format!("Loading channels of {team}…"),
        WorkerRequest::LoadChannels { team },
    );
}

pub fn handle_load_channels_response(
    app: &mut App,
    result: Result<ListConversationsOk, KeybaseError>,
) {
    match result {
        Ok(load) => {
            let n = load.conversations.len();
            app.channels = load.conversations;
            // Joined channels first, then alphabetical by channel name.
            app.channels.sort_by(|a, b| {
                channel_joined(b)
                    .cmp(&channel_joined(a))
                    .then_with(|| channel_topic(a).cmp(&channel_topic(b)))
            });
            app.channel_selected = app
                .channel_selected
                .min(app.channels.len().saturating_sub(1));
            app.set_action(ActionState::Done(format!("{n} channels")));
            app.push_cmd(
                "keybase chat api listconvsonname",
                true,
                format!("{n} channels"),
            );
            for diag in load.skipped {
                app.push_cmd("channel parse warning", false, diag);
            }
            // Chain a get of the team's default channels to badge them.
            request_get_default_channels(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api listconvsonname", false, e.to_string());
        }
    }
}

pub fn channel_browser_move(app: &mut App, delta: isize) {
    let len = app.channels.len();
    if len == 0 {
        return;
    }
    app.channel_selected =
        (app.channel_selected as isize + delta).clamp(0, len as isize - 1) as usize;
}

/// `Enter` in the browser: open a channel you're in, or join one you're not.
pub fn channel_browser_activate(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    if channel_joined(c) {
        let id = c.id.clone();
        close_channel_browser(app);
        open_conversation_by_id(app, id);
    } else {
        request_join_selected_channel(app);
    }
}

/// Builds the `team#channel` [`ReadChannel`] for the selected browser row.
fn selected_channel_read(app: &App) -> Option<(String, ReadChannel)> {
    let team = app.channel_browser_team.clone()?;
    let c = app.channels.get(app.channel_selected)?;
    let topic = channel_topic(c);
    Some((
        topic.clone(),
        ReadChannel {
            name: team,
            members_type: "team".into(),
            topic_name: Some(topic),
        },
    ))
}

pub fn request_join_selected_channel(app: &mut App) {
    let Some((topic, channel)) = selected_channel_read(app) else {
        return;
    };
    app.submit(
        InFlight::JoinChannel {
            topic: topic.clone(),
        },
        &format!("Joining #{topic}…"),
        WorkerRequest::JoinChannel { channel },
    );
}

pub fn handle_join_response(app: &mut App, result: Result<(), KeybaseError>, topic: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Joined #{topic}")));
            app.push_cmd("keybase chat api join", true, format!("#{topic}"));
            // Pull the new channel into the inbox so it's openable, and refresh
            // the browser so its membership flips.
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api join", false, e.to_string());
        }
    }
}

pub fn request_leave_selected_channel(app: &mut App) {
    let joined = app
        .channels
        .get(app.channel_selected)
        .is_some_and(channel_joined);
    if !joined {
        app.set_action(ActionState::Error("Not a member of this channel".into()));
        return;
    }
    let Some((topic, channel)) = selected_channel_read(app) else {
        return;
    };
    app.submit(
        InFlight::LeaveChannel {
            topic: topic.clone(),
        },
        &format!("Leaving #{topic}…"),
        WorkerRequest::LeaveChannel { channel },
    );
}

pub fn handle_leave_response(app: &mut App, result: Result<(), KeybaseError>, topic: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Left #{topic}")));
            app.push_cmd("keybase chat api leave", true, format!("#{topic}"));
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api leave", false, e.to_string());
        }
    }
}

pub fn request_create_new_conversation(app: &mut App) {
    use crate::domain::is_valid_keybase_identity;

    let raw = app.new_conv.text().trim().to_string();
    if raw.is_empty() {
        app.set_action(ActionState::Error("Username list is empty".into()));
        return;
    }
    let mut names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // Client-side validation: surface obvious typos before the worker
    // round-trips a confusing server error. Accepts bare usernames and
    // social-proof identities (`alice@twitter`).
    let invalid: Vec<&str> = names
        .iter()
        .filter(|n| !is_valid_keybase_identity(n))
        .map(String::as_str)
        .collect();
    if !invalid.is_empty() {
        let bad = invalid.join(", ");
        app.set_action(ActionState::Error(format!("Invalid username(s): {bad}")));
        app.push_cmd("new conversation", false, format!("invalid: {bad}"));
        return;
    }

    if !app.identity.username.is_empty() && !names.contains(&app.identity.username) {
        names.insert(0, app.identity.username.clone());
    }
    let channel = ReadChannel {
        name: names.join(","),
        members_type: "impteamnative".into(),
        topic_name: None,
    };
    app.submit(
        InFlight::NewConversation,
        "Creating conversation…",
        WorkerRequest::NewConversation { channel },
    );
}

pub fn handle_new_conversation_response(app: &mut App, result: Result<String, KeybaseError>) {
    match result {
        Ok(id) => {
            app.set_action(ActionState::Done("Conversation created".into()));
            app.push_cmd(
                "keybase chat api newconv",
                true,
                if id.is_empty() {
                    "ok".into()
                } else {
                    // Char-boundary-safe truncation: a non-ASCII id with
                    // a multi-byte codepoint straddling byte 16 would
                    // panic a byte slice on the render thread.
                    format!("id={}", id.chars().take(16).collect::<String>())
                },
            );
            close_new_conversation(app);
            // Re-fetch the inbox so the new conv shows up; stay on the
            // inbox. (Do NOT set open_conv_id here — the conv isn't in
            // `conversations` until the refresh lands, and leaving it set
            // on the inbox screen desyncs open-conversation state.)
            request_load_inbox(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api newconv", false, e.to_string());
        }
    }
}
