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
    match app.settings_focus {
        SettingsFocus::Sidebar => handle_sidebar(app, key),
        SettingsFocus::Panel => handle_panel(app, key),
    }
}

fn handle_sidebar(app: &mut App, key: KeyEvent) {
    let len = SettingsSection::ALL.len();
    match key.code {
        KeyCode::Esc | KeyCode::F(10) => app.close_settings(),
        KeyCode::Char('j') | KeyCode::Down if app.settings_section + 1 < len => {
            app.settings_section += 1;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.settings_section = app.settings_section.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => {
            app.settings_item = 0;
            app.settings_focus = SettingsFocus::Panel;
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
            app.settings_focus = SettingsFocus::Sidebar;
        }
        KeyCode::Char('j') | KeyCode::Down if n > 0 && app.settings_item + 1 < n => {
            app.settings_item += 1;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.settings_item = app.settings_item.saturating_sub(1);
        }
        KeyCode::Left | KeyCode::Char('h') => adjust(app, rows, -1),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char(' ') => {
            adjust(app, rows, 1);
        }
        _ => {}
    }
}

fn adjust(app: &mut App, rows: &[SettingId], delta: isize) {
    if let Some(&id) = rows.get(app.settings_item) {
        app.settings_adjust(id, delta);
    }
}

fn handle_theme_panel(app: &mut App, key: KeyEvent) {
    let len = theme::Preset::ALL.len();
    match key.code {
        KeyCode::F(10) => app.close_settings(),
        // Esc steps back to the section sidebar (a second Esc there closes).
        KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
            app.settings_focus = SettingsFocus::Sidebar;
        }
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('l') | KeyCode::Right
            if app.settings_theme_idx + 1 < len =>
        {
            app.apply_theme_idx(app.settings_theme_idx + 1);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('h') | KeyCode::Left => {
            app.apply_theme_idx(app.settings_theme_idx.saturating_sub(1));
        }
        KeyCode::Enter => app.apply_theme_idx(app.settings_theme_idx),
        _ => {}
    }
}
