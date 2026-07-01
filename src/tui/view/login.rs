//! Login screen (signed-out) — a bytewarden-style form over the
//! figlet/starfield backdrop. Each field is a **labelled, full-width bordered
//! input** (Username / Device name / Paper key, the last masked unless
//! F2-revealed); below them two action buttons: "Log in" (non-interactive
//! paper-key login) and "Log in in terminal" (cede the terminal to interactive
//! `keybase login` for an already-provisioned device).

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use crate::domain::LineEditor;
use crate::tui::app::{App, LoginField};
use crate::tui::theme::Theme;
use crate::tui::view::widgets::{editor_spans, editor_spans_masked, rounded_block};
use crate::tui::view::{action, logo};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let t = app.theme.clone();

    let outer = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);
    let body = outer[0];
    let bar = outer[1];

    // Starfield + figlet backdrop across the whole body; the form box sits over
    // its lower portion (cleared, opaque), leaving the wordmark up top.
    logo::render(frame, app, body);

    let focus = app.login_focus;
    let reveal = app.login_reveal;

    // The Login block: comfortably wide, bottom-aligned so the wordmark keeps
    // the top of the screen (like the reference design). Height fits its
    // contents; it collapses gracefully on a short terminal.
    let block_w = 72.min(body.width.saturating_sub(6)).max(40);
    let block_h = FORM_ROWS.min(body.height);
    let form = Rect {
        x: body.x + (body.width - block_w) / 2,
        y: body.y + body.height.saturating_sub(block_h),
        width: block_w,
        height: block_h,
    };
    frame.render_widget(Clear, form);

    let block = rounded_block(Style::default().fg(t.accent)).title(Span::styled(
        " Login ",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(form);
    frame.render_widget(block, form);

    // One row per element; the flexible spacer soaks up extra height on a tall
    // terminal so the fields stay grouped at the top and the buttons/hint sit
    // lower — the reference layout.
    let rows = Layout::vertical([
        Constraint::Length(1), // Username label
        Constraint::Length(3), // Username input
        Constraint::Length(1), // Device label
        Constraint::Length(3), // Device input
        Constraint::Length(1), // Paper key label
        Constraint::Length(3), // Paper key input
        Constraint::Min(0),    // flexible spacer
        Constraint::Length(1), // buttons
        Constraint::Length(1), // hint
    ])
    .horizontal_margin(2)
    .split(inner);

    render_label(
        frame,
        rows[0],
        "Username",
        None,
        focus == LoginField::Username,
        &t,
    );
    render_input(
        frame,
        rows[1],
        &app.login_username,
        focus == LoginField::Username,
        false,
        &t,
    );
    render_label(
        frame,
        rows[2],
        "Device name",
        None,
        focus == LoginField::Device,
        &t,
    );
    render_input(
        frame,
        rows[3],
        &app.login_device,
        focus == LoginField::Device,
        false,
        &t,
    );
    render_label(
        frame,
        rows[4],
        "Paper key",
        Some(if reveal { "(F2: hide)" } else { "(F2: reveal)" }),
        focus == LoginField::PaperKey,
        &t,
    );
    render_input(
        frame,
        rows[5],
        &app.login_paperkey,
        focus == LoginField::PaperKey,
        !reveal,
        &t,
    );

    // Buttons.
    let buttons = Line::from(vec![
        button("Log in", focus == LoginField::SubmitPaperkey, &t),
        Span::raw("    "),
        button("Log in in terminal", focus == LoginField::SubmitNative, &t),
    ]);
    frame.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[7],
    );

    // Hint (wraps so it never clips).
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "New device → paper key · already provisioned → Log in in terminal",
            Style::default().fg(t.dim),
        )))
        .wrap(Wrap { trim: true }),
        rows[8],
    );

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

/// Rows the form block needs at full size (borders + labels + bordered inputs
/// + spacer + buttons + hint). Shrinks to the terminal when shorter.
const FORM_ROWS: u16 = 19;

/// A field label row: accent+bold when its field is focused, else dim. `hint`
/// (e.g. the reveal toggle) is appended right-aligned-ish after the label.
fn render_label(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    hint: Option<&str>,
    focused: bool,
    t: &Theme,
) {
    let style = if focused {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.dim)
    };
    let mut spans = vec![Span::styled(format!("{label}:"), style)];
    if let Some(h) = hint {
        spans.push(Span::styled(format!("   {h}"), Style::default().fg(t.dim)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// A full-width bordered input box holding the editor's value (masked for the
/// paper key). The border lights up (accent) when focused, else recedes.
fn render_input(
    frame: &mut Frame,
    area: Rect,
    editor: &LineEditor,
    focused: bool,
    masked: bool,
    t: &Theme,
) {
    let border = if focused { t.accent } else { t.inactive };
    let blk = rounded_block(Style::default().fg(border));
    let text_area = blk.inner(area);
    frame.render_widget(blk, area);

    let mut spans = vec![Span::raw(" ")];
    if masked {
        spans.extend(editor_spans_masked(editor, focused, t));
    } else {
        spans.extend(editor_spans(editor, focused, t));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), text_area);
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
