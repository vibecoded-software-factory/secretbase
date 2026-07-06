//! Settings overlay input.
//!
//! Two panes: a section sidebar (left) and the active section's panel
//! (right). `Tab` moves between them; `↑/↓` navigate within them; `←/→`
//! adjust the focused setting. Every change applies and persists immediately
//! (apply-immediately — there is no separate confirm/cancel step). `Esc`
//! steps back: Panel → Sidebar → close. `F10` closes from anywhere.

use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::App;
use crate::tui::app::{SettingId, SettingsFocus, SettingsSection};
use crate::tui::theme;

pub fn handle(app: &mut App, key: KeyEvent) {
    // A secret's input popup owns the keys while open.
    if app.settings_ui.editing.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.settings_ui.editing = None;
                app.settings_ui.input = crate::domain::LineEditor::default();
            }
            KeyCode::Enter => app.settings_secret_save(),
            _ => {
                crate::tui::input::common::route_line_editor(&mut app.settings_ui.input, key);
            }
        }
        return;
    }
    match app.settings_ui.focus {
        SettingsFocus::Sidebar => handle_sidebar(app, key),
        SettingsFocus::Panel => handle_panel(app, key),
    }
}

/// A click inside the settings overlay: select a sidebar section, select a
/// panel row (a second click on the selected row cycles it), or apply a theme
/// preset — the mouse twin of the keyboard navigation.
pub fn mouse(app: &mut App, col: u16, row: u16) {
    use crate::tui::view::settings::{SettingsHit, settings_hit_at};
    let Some(hit) = settings_hit_at(col, row) else {
        return;
    };
    match hit {
        SettingsHit::Section(i) => {
            app.settings_ui.section = i;
            app.settings_ui.item = 0;
            app.settings_ui.focus = SettingsFocus::Sidebar;
        }
        SettingsHit::Row(i) => {
            let reselect =
                app.settings_ui.focus == SettingsFocus::Panel && app.settings_ui.item == i;
            app.settings_ui.item = i;
            app.settings_ui.focus = SettingsFocus::Panel;
            if reselect {
                // Clicking the already-selected row cycles / toggles it.
                let rows = app.settings_section_obj().rows();
                adjust(app, rows, 1);
            }
        }
        SettingsHit::Theme(i) => {
            app.settings_ui.focus = SettingsFocus::Panel;
            app.apply_theme_idx(i);
        }
    }
}

fn handle_sidebar(app: &mut App, key: KeyEvent) {
    let len = SettingsSection::ALL.len();
    match key.code {
        KeyCode::Esc | KeyCode::F(10) => app.close_settings(),
        KeyCode::Char('j') | KeyCode::Down if app.settings_ui.section + 1 < len => {
            app.settings_ui.section += 1;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.settings_ui.section = app.settings_ui.section.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => {
            app.settings_ui.item = 0;
            app.settings_ui.focus = SettingsFocus::Panel;
        }
        _ => {}
    }
}

fn handle_panel(app: &mut App, key: KeyEvent) {
    if app.settings_section_obj() == SettingsSection::Theme {
        return handle_theme_panel(app, key);
    }
    let rows = app.settings_section_obj().rows();
    let n = rows.len();
    match key.code {
        KeyCode::F(10) => app.close_settings(),
        // Esc steps back to the section sidebar (a second Esc there closes).
        KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
            app.settings_ui.focus = SettingsFocus::Sidebar;
        }
        KeyCode::Char('j') | KeyCode::Down if n > 0 && app.settings_ui.item + 1 < n => {
            app.settings_ui.item += 1;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.settings_ui.item = app.settings_ui.item.saturating_sub(1);
        }
        KeyCode::Left | KeyCode::Char('h') => adjust(app, rows, -1),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char(' ') => {
            adjust(app, rows, 1);
        }
        _ => {}
    }
}

fn adjust(app: &mut App, rows: &[SettingId], delta: isize) {
    if let Some(&id) = rows.get(app.settings_ui.item) {
        app.settings_adjust(id, delta);
        // The join/leave filter is applied at projection time — reload the
        // open conversation so the toggle is visible immediately.
        if id == SettingId::SmartJoins && app.open_conv_id.is_some() {
            crate::tui::flows::chat::request_reload_messages(app);
        }
    }
}

fn handle_theme_panel(app: &mut App, key: KeyEvent) {
    let len = theme::Preset::ALL.len();
    match key.code {
        KeyCode::F(10) => app.close_settings(),
        // Esc steps back to the section sidebar (a second Esc there closes).
        KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
            app.settings_ui.focus = SettingsFocus::Sidebar;
        }
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('l') | KeyCode::Right
            if app.settings_ui.theme_idx + 1 < len =>
        {
            app.apply_theme_idx(app.settings_ui.theme_idx + 1);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('h') | KeyCode::Left => {
            app.apply_theme_idx(app.settings_ui.theme_idx.saturating_sub(1));
        }
        KeyCode::Enter => app.apply_theme_idx(app.settings_ui.theme_idx),
        _ => {}
    }
}
