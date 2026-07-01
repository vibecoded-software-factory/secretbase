//! Login screen (signed-out) — a bytewarden-style form over the
//! figlet/starfield backdrop. Three fields (Username / Device / Paper key,
//! the last masked unless F2-revealed) and two action buttons: "Log in"
//! (non-interactive paper-key login) and "Log in in terminal" (cede the
//! terminal to interactive `keybase login` for an already-provisioned device).

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::tui::app::{App, LoginField};
use crate::tui::theme::Theme;
use crate::tui::view::widgets::{
    center_rect_abs, editor_spans, editor_spans_masked, rounded_block,
};
use crate::tui::view::{action, logo};

/// Label column width so the field values line up.
const LABEL_W: usize = 13;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let t = app.theme.clone();

    let layout = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);
    let body = layout[0];
    let bar = layout[1];

    logo::render(frame, app, body);

    let focus = app.login_focus;
    let reveal = app.login_reveal;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw(""));
    lines.push(field_line(
        "Username:",
        &app.login_username,
        focus == LoginField::Username,
        false,
        None,
        &t,
    ));
    lines.push(field_line(
        "Device name:",
        &app.login_device,
        focus == LoginField::Device,
        false,
        None,
        &t,
    ));
    lines.push(field_line(
        "Paper key:",
        &app.login_paperkey,
        focus == LoginField::PaperKey,
        !reveal,
        Some(if reveal { "(F2: hide)" } else { "(F2: reveal)" }),
        &t,
    ));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        button("Log in", focus == LoginField::SubmitPaperkey, &t),
        Span::raw("     "),
        button("Log in in terminal", focus == LoginField::SubmitNative, &t),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        " New device → paper key · already provisioned (logged out) → terminal ",
        Style::default().fg(t.dim),
    )));

    // Content-sized, responsive box (see widgets::center_rect_abs): wide enough
    // for the fields, clamped to the terminal.
    let inner_w: u16 = 66;
    let box_w = inner_w.min(body.width.saturating_sub(4)).max(30);
    let box_h = lines.len() as u16 + 2;
    let panel = center_rect_abs(box_w, box_h, body);

    let block = rounded_block(Style::default().fg(t.accent)).title(Span::styled(
        " Login ",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    // Clear the panel so the starfield behind doesn't bleed through the gaps
    // to the right of each field (the box is an opaque form over the backdrop).
    frame.render_widget(Clear, panel);
    frame.render_widget(Paragraph::new(lines).block(block), panel);

    // Bottom hint strip.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Tab field · ↑/↓ move · Enter login · F2 reveal · F5 retry · Esc quit · F1 help",
            Style::default().fg(t.dim),
        ))),
        bar,
    );

    // Spinner / toast (e.g. "Logging in…", errors) over the body.
    action::render(frame, app, body);
}

/// One `label + value` form row. `masked` renders the value as `●` (secret);
/// `suffix` appends a dim hint (e.g. the reveal toggle) after the value.
fn field_line<'a>(
    label: &str,
    editor: &crate::domain::LineEditor,
    focused: bool,
    masked: bool,
    suffix: Option<&'a str>,
    t: &Theme,
) -> Line<'a> {
    let label_style = if focused {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.dim)
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{label:<LABEL_W$}"), label_style),
    ];
    if masked {
        spans.extend(editor_spans_masked(editor, focused, t));
    } else {
        spans.extend(editor_spans(editor, focused, t));
    }
    if let Some(s) = suffix {
        spans.push(Span::styled(format!("   {s}"), Style::default().fg(t.dim)));
    }
    Line::from(spans)
}

/// A focus-highlighted action button (`[ label ]`).
fn button(label: &str, focused: bool, t: &Theme) -> Span<'static> {
    let style = if focused {
        Style::default()
            .bg(t.selected_bg)
            .fg(t.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.dim)
    };
    Span::styled(format!("[ {label} ]"), style)
}
