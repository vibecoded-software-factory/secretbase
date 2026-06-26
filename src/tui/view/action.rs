//! Feedback strip (spinner + status message) overlaid on the bottom of
//! every screen.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::action::ActionState;
use crate::tui::app::App;

/// Four-frame Braille spinner.
const SPINNER: [&str; 4] = ["⠋", "⠙", "⠸", "⠴"];

/// Returns the spinner glyph for `tick` (used by the splash screen,
/// which draws its own centered status line below the logo).
pub fn spinner_frame(tick: u8) -> &'static str {
    SPINNER[tick as usize % SPINNER.len()]
}

/// Renders the feedback strip at the very last row of `area`.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if area.height < 2 {
        return;
    }
    let strip = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area)[1];
    let t = &app.theme;
    let (icon, msg, color) = match &app.action_state {
        ActionState::Idle => return,
        ActionState::Running(s) => (SPINNER[app.action_tick as usize % 4], s.as_str(), t.accent),
        ActionState::Done(s) => ("✓", s.as_str(), t.success),
        ActionState::Error(s) => ("✗", s.as_str(), t.error),
    };
    let line = Line::from(vec![
        Span::styled(format!(" {icon} "), Style::default().fg(color)),
        Span::styled(msg.to_string(), Style::default().fg(t.foreground)),
    ]);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Left), strip);
}
