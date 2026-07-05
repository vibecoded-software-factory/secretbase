//! Terminal User Interface — the *driving* adapter of the application.
//!
//! The TUI is responsible for:
//!
//! 1. Holding the [`App`] state container,
//! 2. Reading terminal events and dispatching them to per-screen handlers,
//! 3. Rendering the current state to the screen via Ratatui,
//! 4. Draining responses from the worker thread that owns the
//!    [`KeybasePort`] and applying them to `App` via
//!    [`flows::apply_response`].
//!
//! The bottom-level domain ports ([`crate::ports`]) are injected at
//! construction time; the keybase port is moved into a dedicated
//! worker thread on boot so the render thread never blocks on a
//! subprocess call.

pub mod action;
pub mod app;
pub mod channel_browser_state;
pub mod cmdlog_state;
pub mod debug_log;
pub mod emoji_catalog;
pub mod file_picker;
pub mod flows;
pub mod icons;
pub mod image;
pub mod image_pipeline;
pub mod input;
pub mod members_state;
pub mod mouse_areas;
pub mod pin_state;
pub mod screens;
pub mod settings_model;
pub mod syntax;
pub mod teams_state;
pub mod theme;
pub mod view;
pub mod worker;

pub use app::App;

use color_eyre::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use std::time::Duration;

use crate::domain::ChatEvent;
use crate::ports::{ClipboardPort, KeybasePort, OpenerPort, SettingsPort};
use action::ActionState;
use std::sync::mpsc::Receiver;
use worker::WorkerHandle;

/// Number of poll ticks (~80 ms each) a Done/Error message stays on
/// screen before reverting to Idle. ~1.5 s total.
const FEEDBACK_TICKS: u8 = 19;

/// Polling interval while idle (controls how quickly resize events are
/// detected when no other event arrives).
const POLL_IDLE_MS: u64 = 500;

/// Polling interval during a Running/Done/Error state (drives spinner
/// animation and feedback expiry).
const POLL_BUSY_MS: u64 = 80;

/// Max input events processed per frame. Coalesces held-key repeats so
/// they don't pile up faster than we redraw, while still bounding the
/// work done between frames.
const EVENT_BATCH_CAP: u16 = 256;

/// Composition entry point — installs a terminal, spawns the worker
/// thread, runs the event loop, and tears down on exit.
pub fn run(
    keybase: Box<dyn KeybasePort + Send>,
    keybase_bg: Box<dyn KeybasePort + Send>,
    clipboard: Box<dyn ClipboardPort>,
    opener: Box<dyn OpenerPort>,
    settings: Box<dyn SettingsPort>,
    chat_rx: Option<Receiver<ChatEvent>>,
) -> Result<()> {
    ratatui::run(|terminal| {
        // The worker thread owns the keybase port. The handle stays
        // alive in this scope so `WorkerHandle::drop` reaps the
        // thread (Shutdown + join) when we return — even on error.
        let mut worker = WorkerHandle::spawn(keybase);
        let worker_tx = worker.tx();
        // Background lane for the idle inbox auto-refresh.
        let bg_worker_tx = worker.spawn_extra(keybase_bg);
        let worker_rx = worker.take_rx();

        let mut app = App::new(
            worker_tx,
            bg_worker_tx,
            worker_rx,
            chat_rx,
            clipboard,
            opener,
            settings,
        );

        execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;

        // Kitty keyboard protocol (where the terminal supports it): makes
        // the modifier chords this app leans on — Alt+Shift ranges,
        // Alt+Enter, Shift+arrows — reliably distinguishable, which classic
        // termios delivery often mangles over SSH. Push only the
        // disambiguation flag (no release/repeat reporting — the input loop
        // is Press-only anyway); unsupported terminals are detected and
        // skipped, so behaviour degrades to exactly today's.
        let kitty_keys = matches!(
            crossterm::terminal::supports_keyboard_enhancement(),
            Ok(true)
        );
        if kitty_keys {
            let _ = execute!(
                std::io::stdout(),
                event::PushKeyboardEnhancementFlags(
                    event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                )
            );
        }

        // Belt-and-suspenders terminal restore on a panic. `ratatui::run`
        // already installed a hook that leaves the alternate screen + raw mode,
        // but it doesn't know we enabled mouse capture (or pushed keyboard
        // flags) — so chain a hook that undoes those first (while still on
        // the alt screen), then defers to the previous hook. The normal and
        // `?`-error exits restore below; this only covers an unwinding panic,
        // which otherwise leaves the terminal spewing mouse escape sequences.
        // (A hard SIGSEGV can't be intercepted here under
        // `#![forbid(unsafe_code)]`; the answer to that is to not crash —
        // run `reset` if one ever slips through.)
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if kitty_keys {
                let _ = execute!(std::io::stdout(), event::PopKeyboardEnhancementFlags);
            }
            let _ = execute!(
                std::io::stdout(),
                DisableMouseCapture,
                DisableBracketedPaste
            );
            prev_hook(info);
        }));

        // Kick off the boot status check. The run loop picks it up
        // immediately, draws a spinner, and applies the response when
        // the worker comes back.
        flows::auth::request_status(&mut app);

        let result = run_loop(terminal, &mut app);
        if kitty_keys {
            let _ = execute!(std::io::stdout(), event::PopKeyboardEnhancementFlags);
        }
        let _ = execute!(
            std::io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste
        );
        drain_pending_events();
        // Drop order: app first (closes its `worker_tx`), then
        // `worker` triggers Shutdown + join.
        drop(app);
        drop(worker);
        result
    })
}

/// Reads and discards every event currently buffered in stdin. Bounded
/// by both an event count and a wall-clock budget so a misbehaving
/// terminal can't trap us here.
fn drain_pending_events() {
    let deadline = std::time::Instant::now() + Duration::from_millis(40);
    let mut drained: u32 = 0;
    while std::time::Instant::now() < deadline && drained < 256 {
        match event::poll(Duration::from_millis(0)) {
            Ok(true) => {
                let _ = event::read();
                drained += 1;
            }
            _ => break,
        }
    }
}

/// Inner event loop — separated from [`run`] so the terminal restore
/// runs even on early returns.
fn run_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut done_ticks: u8 = 0;
    let mut last_size = terminal.size()?;
    let mut prev_overlay = false;
    // Image rects painted last frame — repaint only when the visible set or
    // their positions change (scroll, download, resize) so the graphics don't
    // flicker on every idle redraw.
    let mut last_img: Vec<(ratatui::layout::Rect, String)> = Vec::new();
    // Monotonic clock driving GIF frame selection (stamped into `anim_ms`).
    let anim_clock = std::time::Instant::now();

    loop {
        app.images.anim_ms = anim_clock.elapsed().as_millis() as u64;
        let size = terminal.size()?;
        if size != last_size {
            last_size = size;
            terminal.clear()?;
        }
        // Closing a modal overlay forces a full repaint: wide glyphs (emoji in
        // the reaction picker) can leave residual cells the diff misses.
        let overlay = app.has_overlay();
        if prev_overlay && !overlay {
            terminal.clear()?;
        }
        prev_overlay = overlay;
        // Record live terminal size so the mouse layer can detect
        // stale rects (mouse_areas computed for a frame size that
        // no longer matches the terminal).
        app.last_terminal_size = (size.width, size.height);

        terminal.draw(|frame| view::draw(frame, app))?;

        // Inline image thumbnails. Symbols render in-buffer (handled entirely
        // by the view), so only the true-graphics protocols need the run loop
        // to paint over the reserved region; either way, enqueue downloads.
        flows::chat::ensure_visible_images(app);
        flows::chat::ensure_pending_gif_decodes(app);
        let graphics = matches!(app.images.proto, Some(p) if p != image::ImgProto::Symbols);
        if let Some(proto) = app.images.proto.filter(|_| graphics) {
            // Don't paint while an overlay covers the chat (no bleed over popups).
            let target: Vec<(ratatui::layout::Rect, String)> = if app.has_overlay() {
                Vec::new()
            } else {
                app.images.areas.clone()
            };
            if target != last_img || app.images.dirty {
                if proto == image::ImgProto::Kitty {
                    // Kitty graphics are a deletable layer.
                    let _ = image::clear();
                } else {
                    // Sixel / iTerm are written outside Ratatui's buffer, so a
                    // changed or cleared set (e.g. an overlay opening) would
                    // ghost — the diff won't wipe cells it never tracked. Force
                    // a full text repaint first.
                    let _ = terminal.clear();
                    terminal.draw(|frame| view::draw(frame, app))?;
                }
                for (rect, path) in &target {
                    let _ = image::render_into(&mut app.images.render_cache, proto, *rect, path);
                }
                last_img = target;
            }
        }
        app.images.dirty = false;

        // Drain any worker response that arrived since the last
        // tick — non-blocking. The match in `apply_response` handles
        // multiple chained requests (e.g. boot Status → LoadInbox)
        // by re-queuing inside the response handler; the next
        // iteration of this loop picks them up.
        loop {
            match app.worker_rx.try_recv() {
                Ok(resp) => {
                    flows::apply_response(app, resp);
                    done_ticks = 0;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // Every worker thread is gone — no response will ever come.
                // Unwedge the UI instead of spinning "busy" forever.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.on_worker_dead();
                    break;
                }
            }
        }
        // Belt-and-suspenders for a lost ticket (worker died mid-call, or a
        // response was dropped): release a slot that outlived every per-op
        // timeout so `busy_blocks` can't lock input permanently.
        app.watchdog_release_stuck_request();

        // Drain push events from the `keybase chat api-listen` stream —
        // non-blocking. Collected first so the dispatch can take `&mut
        // app` without holding the `chat_rx` borrow.
        let chat_evs: Vec<ChatEvent> = match app.chat_rx.as_ref() {
            Some(rx) => {
                let mut v = Vec::new();
                while let Ok(ev) = rx.try_recv() {
                    v.push(ev);
                    if v.len() >= 128 {
                        break;
                    }
                }
                v
            }
            None => Vec::new(),
        };
        for ev in chat_evs {
            flows::apply_chat_event(app, ev);
            done_ticks = 0;
        }

        // Loading images / decoding GIFs → poll fast so the skeleton gives way
        // to the picture promptly (the worker response is drained next tick).
        let loading_images = !app.images.pending.is_empty() || !app.images.decoding.is_empty();
        if event::poll(poll_timeout(
            &app.action_state,
            app.is_busy(),
            app.images.animating || loading_images,
        ))? {
            // Drain ALL buffered events before redrawing. Holding a key
            // floods key-repeat events; processing one per frame lets
            // them pile up faster than we draw, so a change of direction
            // would lag while stale events drain. A capped batch keeps
            // the draw cadence honest.
            let mut processed = 0u16;
            loop {
                input::handle_events(app, event::read()?);
                processed += 1;
                if app.should_quit || processed >= EVENT_BATCH_CAP {
                    break;
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            // An input handler may have installed feedback synchronously
            // (e.g. a copy → Done, "Message is empty" → Error). Restart
            // the expiry timer so it gets its full on-screen duration
            // (`set_action` can't reach this run-loop-local counter).
            done_ticks = 0;
        } else {
            tick_state(app, &mut done_ticks);
            maybe_auto_refresh(app);
        }

        // Interactive login: the handler asked us to cede the terminal to
        // `keybase login` (the passphrase path can't be scripted). Do it
        // between frames — suspend the TUI, run it, restore, re-check status.
        if let Some(username) = app.pending_native_login.take() {
            run_native_login(terminal, &username);
            drain_pending_events();
            terminal.clear()?;
            flows::auth::request_status(app);
            done_ticks = 0;
        }

        // Compose in $EDITOR: cede the terminal between frames, hand the
        // draft over, read it back. Same suspend/restore as the login.
        if app.pending_editor_compose {
            app.pending_editor_compose = false;
            run_editor_compose(app);
            drain_pending_events();
            terminal.clear()?;
            done_ticks = 0;
        }

        // Terminal-window title: conversation + attention badges — a free
        // status surface in tmux / SSH window lists. Only re-emitted when
        // it changes.
        let title = app.desired_term_title();
        if title != app.last_term_title {
            let _ = execute!(std::io::stdout(), crossterm::terminal::SetTitle(&title));
            app.last_term_title = title;
        }

        if app.should_quit {
            break;
        }
    }
    // Leave the window title clean — terminals show their default when
    // the title is empty.
    let _ = execute!(std::io::stdout(), crossterm::terminal::SetTitle(""));
    Ok(())
}

/// Cedes the terminal to `$VISUAL`/`$EDITOR` (fallback `vi`) with the
/// current draft in a **0600** temp file; on return the file's content
/// replaces the draft and the file is overwritten with spaces, then
/// removed — chat text touches disk only for the editor round-trip, as
/// short-lived as we can make it. A missing editor or I/O error leaves the
/// draft untouched and surfaces on the feedback strip.
fn run_editor_compose(app: &mut app::App) {
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use std::io::Write;

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let Some(program) = parts.next() else {
        app.set_action(crate::tui::action::ActionState::Error(
            "$EDITOR is empty".into(),
        ));
        return;
    };
    let args: Vec<&str> = parts.collect();

    // Hand-rolled unique path (tempfile is a dev-dependency only); 0600
    // from the first byte via create_new + mode.
    let path = std::env::temp_dir().join(format!(
        "secretbase-draft-{}-{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let draft = app.compose.text().to_string();
    let write = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path)?;
        f.write_all(draft.as_bytes())
    })();
    if let Err(e) = write {
        app.set_action(crate::tui::action::ActionState::Error(format!(
            "draft file: {e}"
        )));
        return;
    }

    let kitty_keys = matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    );
    if kitty_keys {
        let _ = execute!(std::io::stdout(), event::PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();

    let status = std::process::Command::new(program)
        .args(&args)
        .arg(&path)
        .status();

    let _ = enable_raw_mode();
    let _ = execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    );
    if kitty_keys {
        let _ = execute!(
            std::io::stdout(),
            event::PushKeyboardEnhancementFlags(
                event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        );
    }

    match status {
        Ok(s) if s.success() => {
            if let Ok(mut text) = std::fs::read_to_string(&path) {
                while text.ends_with('\n') {
                    text.pop();
                }
                app.compose.clear();
                app.compose.insert_str(&text);
                app.set_action(crate::tui::action::ActionState::Done(
                    "Draft updated from editor".into(),
                ));
            }
        }
        Ok(_) => app.set_action(crate::tui::action::ActionState::Done(
            "Editor exited without saving — draft unchanged".into(),
        )),
        Err(e) => app.set_action(crate::tui::action::ActionState::Error(format!(
            "{program}: {e}"
        ))),
    }
    // Best-effort wipe: the draft was chat content on disk.
    if let Ok(meta) = std::fs::metadata(&path) {
        let _ = std::fs::write(&path, " ".repeat(meta.len() as usize));
    }
    let _ = std::fs::remove_file(&path);
}

/// Cedes the terminal to interactive `keybase login [username]` — the only
/// login path once the device is already provisioned (keybase collects the
/// passphrase through its own pinentry/terminal prompt, which can't be fed
/// over stdin). Leaves the alternate screen + raw mode + mouse capture so
/// keybase owns the real TTY, runs it to completion, then restores the TUI.
/// Best-effort: a spawn/terminal error is swallowed and reflected by the
/// follow-up `status` re-check.
fn run_native_login(terminal: &mut ratatui::DefaultTerminal, username: &str) {
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };

    // Suspend the TUI: hand the real terminal to keybase. Pop the keyboard
    // enhancement flags too — keybase's own prompts expect classic delivery.
    let kitty_keys = matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    );
    if kitty_keys {
        let _ = execute!(std::io::stdout(), event::PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();

    let suffix = if username.is_empty() {
        String::new()
    } else {
        format!(" {username}")
    };
    println!("\n  Running `keybase login{suffix}` — follow the prompts below.\n");
    let mut cmd = std::process::Command::new("keybase");
    cmd.arg("login");
    if !username.is_empty() {
        cmd.arg(username);
    }
    let _ = cmd.status(); // inherits stdio; the user interacts directly
    println!("\n  Returning to secretbase…");

    // Restore the TUI (re-pushing the keyboard flags — the alternate
    // screen's flag stack isn't guaranteed to survive the round-trip).
    let _ = enable_raw_mode();
    let _ = execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    );
    if kitty_keys {
        let _ = execute!(
            std::io::stdout(),
            event::PushKeyboardEnhancementFlags(
                event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        );
    }
    let _ = terminal.clear();
}

/// Returns the next event-poll timeout — fast during animation or
/// while a worker call is in flight, longer when idle.
fn poll_timeout(state: &ActionState, busy: bool, gif_animating: bool) -> Duration {
    // An animated GIF on screen drives the redraw cadence (~12.5 fps) so frames
    // advance even while otherwise idle.
    if gif_animating {
        return Duration::from_millis(80);
    }
    if busy {
        return Duration::from_millis(POLL_BUSY_MS);
    }
    match state {
        ActionState::Idle => Duration::from_millis(POLL_IDLE_MS),
        _ => Duration::from_millis(POLL_BUSY_MS),
    }
}

/// If the user is idle on the inbox screen and the configured refresh
/// cadence has elapsed since the last load, queues a silent
/// `keybase chat api list`. Quietly no-ops outside the inbox, while
/// the user is composing, or when the cadence is set to `0` (disabled).
///
/// "Silent" applies only to the success path — see
/// [`flows::chat::request_load_inbox_silent`] for the exact policy
/// (errors still surface so the user notices a wedged refresh).
fn maybe_auto_refresh(app: &mut App) {
    let interval = app.settings_cache.inbox_refresh_secs;
    if interval == 0 {
        return;
    }
    if app.screen != screens::Screen::Inbox {
        return;
    }
    // Runs on the background lane, so it does NOT gate on the user's
    // in_flight/action state — only on whether a background refresh is
    // already running.
    if app.bg_inflight {
        return;
    }
    if app.last_inbox_load.elapsed() < std::time::Duration::from_secs(interval) {
        return;
    }
    flows::chat::request_load_inbox_silent(app);
}

/// Advances the spinner or expires Done/Error feedback after
/// [`FEEDBACK_TICKS`] ticks (~1.5 s).
fn tick_state(app: &mut App, done_ticks: &mut u8) {
    match &app.action_state {
        ActionState::Running(_) => app.tick_action(),
        ActionState::Done(_) => {
            *done_ticks += 1;
            if *done_ticks >= FEEDBACK_TICKS {
                app.set_action(ActionState::Idle);
                *done_ticks = 0;
            }
        }
        // Errors are **sticky** (mutt/lazygit): a failure is a condition the
        // user must read, not a 1.5 s event. The next keypress clears it
        // (input::handle_key), success toasts keep the short fuse.
        ActionState::Error(_) => {}
        ActionState::Idle => {}
    }
}
