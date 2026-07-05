//! Boot splash screen — logo + spinner shown while `keybase status`
//! runs at boot.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
    widgets::Paragraph,
};

use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::view::action::spinner_frame;
use crate::tui::view::logo;
use crate::tui::view::starfield::fill_stars;

const LOGO_HEIGHT: u16 = 16;

/// Renders the splash screen.
pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = frame.area();

    // Center the logo vertically (leave room for the spinner below).
    let top = area.height.saturating_sub(LOGO_HEIGHT + 3) / 2;
    let logo_area = Rect {
        x: 0,
        y: top,
        width: area.width,
        height: LOGO_HEIGHT.min(area.height.saturating_sub(top)),
    };

    fill_stars(
        frame,
        Rect {
            x: 0,
            y: 0,
            width: area.width,
            height: top,
        },
        t,
    );
    logo::render(frame, app, logo_area);
    let below_y = top + LOGO_HEIGHT;
    if below_y < area.height {
        fill_stars(
            frame,
            Rect {
                x: 0,
                y: below_y,
                width: area.width,
                height: area.height - below_y,
            },
            t,
        );
    }

    // Spinner / status text just below the logo.
    let sp = spinner_frame(app.action_tick);
    let msg = match &app.action_state {
        ActionState::Running(m) => format!(" {sp}  {m}"),
        ActionState::Done(m) => format!(" ✓  {m}"),
        ActionState::Error(m) => format!(" ✗  {m}"),
        ActionState::Idle => String::new(),
    };
    let col = match &app.action_state {
        ActionState::Running(_) => t.accent,
        ActionState::Done(_) => t.success,
        ActionState::Error(_) => t.error,
        ActionState::Idle => t.dim,
    };
    if !msg.is_empty() {
        let y = (top + LOGO_HEIGHT + 1).min(area.height.saturating_sub(1));
        let w = msg.chars().count() as u16;
        let x = area.width.saturating_sub(w) / 2;
        frame.render_widget(
            Paragraph::new(Span::styled(
                msg,
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            )),
            Rect {
                x,
                y,
                width: w,
                height: 1,
            },
        );
    }
    // A failed boot is not a dead end: say how to retry (and how to leave).
    if matches!(app.action_state, ActionState::Error(_)) && !app.is_busy() {
        let hint = "r / Enter retry · Ctrl+C quit";
        let y = (top + LOGO_HEIGHT + 3).min(area.height.saturating_sub(1));
        let w = hint.chars().count() as u16;
        let x = area.width.saturating_sub(w) / 2;
        frame.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(t.dim))),
            Rect {
                x,
                y,
                width: w,
                height: 1,
            },
        );
    }
}
