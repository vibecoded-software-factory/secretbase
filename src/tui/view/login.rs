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
    widgets::{Clear, Paragraph},
};

use crate::domain::LineEditor;
use crate::tui::app::{App, LoginField};
use crate::tui::theme::Theme;
use crate::tui::view::widgets::{editor_spans, editor_spans_masked, rounded_block};
use crate::tui::view::{action, logo};

/// Rows the content-sized `Login` block occupies (borders + top pad + three
/// label/bordered-input pairs + spacer + buttons + hint).
const FORM_ROWS: u16 = 18;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let t = app.theme.clone();

    let outer = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);
    let body = outer[0];
    let bar = outer[1];

    // Starfield + figlet backdrop across the whole body; the form sits over its
    // lower portion (cleared, opaque), leaving the wordmark up top.
    logo::render(frame, app, body);

    let focus = app.login_focus;
    let reveal = app.login_reveal;

    // The Login block is **content-sized** and placed in the lower ~58% of the
    // body, top-aligned so it starts just below the wordmark (the reference
    // layout) — never stretched to fill, so there's no dead space inside.
    let block_w = (body.width * 82 / 100).clamp(40, body.width.saturating_sub(4));
    let zones = Layout::vertical([Constraint::Percentage(42), Constraint::Fill(1)]).split(body);
    let fz = zones[1];
    let block_h = FORM_ROWS.min(fz.height);
    let form = Rect {
        x: fz.x + fz.width.saturating_sub(block_w) / 2,
        y: fz.y,
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

    let rows = Layout::vertical([
        Constraint::Length(1), // top padding
        Constraint::Length(1), // Username label
        Constraint::Length(3), // Username input
        Constraint::Length(1), // Device label
        Constraint::Length(3), // Device input
        Constraint::Length(1), // Paper key label
        Constraint::Length(3), // Paper key input
        Constraint::Length(1), // spacer
        Constraint::Length(1), // buttons
        Constraint::Length(1), // hint
    ])
    .horizontal_margin(2)
    .split(inner);

    render_label(
        frame,
        rows[1],
        "Username",
        None,
        focus == LoginField::Username,
        &t,
    );
    render_input(
        frame,
        rows[2],
        &app.login_username,
        focus == LoginField::Username,
        false,
        &t,
    );
    render_label(
        frame,
        rows[3],
        "Device name",
        None,
        focus == LoginField::Device,
        &t,
    );
    render_input(
        frame,
        rows[4],
        &app.login_device,
        focus == LoginField::Device,
        false,
        &t,
    );
    render_label(
        frame,
        rows[5],
        "Paper key",
        Some(if reveal { "(F2: hide)" } else { "(F2: reveal)" }),
        focus == LoginField::PaperKey,
        &t,
    );
    render_input(
        frame,
        rows[6],
        &app.login_paperkey,
        focus == LoginField::PaperKey,
        !reveal,
        &t,
    );

    // Buttons, centered.
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            button("Log in", focus == LoginField::SubmitPaperkey, &t),
            Span::raw("    "),
            button("Log in in terminal", focus == LoginField::SubmitNative, &t),
        ]))
        .alignment(Alignment::Center),
        rows[8],
    );

    // A short, centered hint that always fits — which button to pick.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "new device → paper key · already logged out → terminal",
            Style::default().fg(t.dim),
        )))
        .alignment(Alignment::Center),
        rows[9],
    );

    render_footer(frame, bar, &t);

    // Spinner / toast (e.g. "Logging in…", errors) over the body.
    action::render(frame, app, body);
}

/// Bottom hint strip, split left (navigation) / right (`F1 help`) like the
/// reference — so neither half runs off the edge.
fn render_footer(frame: &mut Frame, bar: Rect, t: &Theme) {
    let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Length(9)]).split(bar);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Tab field · Enter login · F2 reveal · F5 retry · Esc quit",
            Style::default().fg(t.dim),
        ))),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "F1 help ",
            Style::default().fg(t.dim),
        )))
        .alignment(Alignment::Right),
        cols[1],
    );
}

/// A field label row: accent+bold when its field is focused, else dim. `hint`
/// (e.g. the reveal toggle) is appended dim after the label.
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
