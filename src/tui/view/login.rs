//! Login screen (signed-out) — a form over the figlet/starfield backdrop.
//! Each field is a labelled, full-width bordered input (Username / Device name
//! / Paper key, the last masked unless F2-revealed); below them two action
//! buttons: "Log in" (non-interactive paper-key login) and "Log in in terminal"
//! (cede the terminal to interactive `keybase login` for a provisioned device).

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
};

use crate::domain::LineEditor;
use crate::tui::action::ActionState;
use crate::tui::app::{App, LoginField};
use crate::tui::theme::Theme;
use crate::tui::view::starfield::fill_stars;
use crate::tui::view::widgets::{
    button, draw_hint_bar, editor_spans, editor_spans_masked, rounded_block,
};
use crate::tui::view::{logo, splash};

/// Fixed form-block height: padding(1) + three label(1)+input(3) pairs +
/// spacer(1) + buttons(1) + hint(1) + feedback strip(2) + borders(2).
const FORM_ROWS: u16 = 20;

pub fn draw(frame: &mut Frame, app: &mut App) {
    // While a login / status check is in flight the form has nothing
    // actionable — show the same centered logo + spinner as the boot splash.
    if matches!(app.action_state, ActionState::Running(_)) {
        splash::draw(frame, app);
        return;
    }

    let t = app.theme.clone();
    let area = frame.area();

    // Stars above the form (2/3) and below (1/3); the hint strip at the bottom.
    let c = Layout::vertical([
        Constraint::Fill(2),
        Constraint::Length(FORM_ROWS),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .split(area);
    let (logo_chunk, form_chunk, lower_chunk, bar_chunk) = (c[0], c[1], c[2], c[3]);

    if logo_chunk.height >= 6 {
        logo::render(frame, app, logo_chunk);
    } else {
        fill_stars(frame, logo_chunk, &t);
    }
    fill_stars(frame, lower_chunk, &t);

    // Center the form; fill the gutters either side with starfield so the
    // backdrop is continuous without the panel losing readability.
    let form_w = area.width.saturating_sub(8).clamp(44, 72);
    let fr = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(form_w),
        Constraint::Fill(1),
    ])
    .split(form_chunk);
    fill_stars(frame, fr[0], &t);
    fill_stars(frame, fr[2], &t);
    let form_area = fr[1];

    let border = if matches!(app.action_state, ActionState::Error(_)) {
        t.error
    } else {
        t.accent
    };
    let block = Block::default()
        .title(Span::styled(
            " Login ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .padding(Padding::horizontal(2));
    let inner = block.inner(form_area);
    frame.render_widget(block, form_area);

    let focus = app.login_focus;
    let reveal = app.login_reveal;
    let f = Layout::vertical([
        Constraint::Length(1), // [0] padding
        Constraint::Length(1), // [1] username label
        Constraint::Length(3), // [2] username input
        Constraint::Length(1), // [3] device label
        Constraint::Length(3), // [4] device input
        Constraint::Length(1), // [5] paper key label
        Constraint::Length(3), // [6] paper key input
        Constraint::Length(1), // [7] spacer
        Constraint::Length(1), // [8] buttons
        Constraint::Length(1), // [9] hint
        Constraint::Length(2), // [10] feedback strip
    ])
    .split(inner);

    render_label(
        frame,
        f[1],
        "Username",
        None,
        focus == LoginField::Username,
        &t,
    );
    render_input(
        frame,
        f[2],
        &app.login_username,
        focus == LoginField::Username,
        false,
        &t,
    );
    render_label(
        frame,
        f[3],
        "Device name",
        None,
        focus == LoginField::Device,
        &t,
    );
    render_input(
        frame,
        f[4],
        &app.login_device,
        focus == LoginField::Device,
        false,
        &t,
    );
    render_label(
        frame,
        f[5],
        "Paper key",
        Some(if reveal { "(F2: hide)" } else { "(F2: reveal)" }),
        focus == LoginField::PaperKey,
        &t,
    );
    render_input(
        frame,
        f[6],
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
        f[8],
    );

    // Short, centered hint — which button to pick.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "new device → paper key · already logged out → terminal",
            Style::default().fg(t.dim),
        )))
        .alignment(Alignment::Center),
        f[9],
    );

    // Feedback strip: a top-bordered separator carrying the last error / result.
    render_strip(frame, f[10], app, &t);

    // Bottom hint bar — the shared, width-fitting strip (no mode badge here).
    draw_hint_bar(
        frame,
        bar_chunk,
        "Tab field · Enter login · F2 reveal · F5 retry · Ctrl+C quit",
        &t,
    );
}

/// The in-form feedback row: a `─` separator with the last error/result (blank
/// when idle), matching the rest of the form's chrome.
fn render_strip(frame: &mut Frame, area: Rect, app: &App, t: &Theme) {
    let sep = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(t.muted));
    let line = if let Some(msg) = &app.boot_error {
        // A missing/unrunnable keybase binary makes the whole form moot —
        // this notice persists (unlike the expiring toast) until a retry
        // succeeds.
        Line::from(vec![
            Span::styled(
                " ✕ ",
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg.clone(), Style::default().fg(t.error)),
        ])
    } else {
        match &app.action_state {
            ActionState::Error(msg) => Line::from(vec![
                Span::styled(
                    " ✕ ",
                    Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                ),
                Span::styled(msg.clone(), Style::default().fg(t.error)),
            ]),
            ActionState::Done(msg) => Line::from(vec![
                Span::styled(
                    " ✓ ",
                    Style::default().fg(t.success).add_modifier(Modifier::BOLD),
                ),
                Span::styled(msg.clone(), Style::default().fg(t.success)),
            ]),
            _ => Line::from(""),
        }
    };
    frame.render_widget(Paragraph::new(line).block(sep), area);
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
