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
pub mod debug_log;
pub mod file_picker;
pub mod flows;
pub mod input;
pub mod mouse_areas;
pub mod screens;
pub mod theme;
pub mod view;
pub mod worker;

pub use app::App;

use color_eyre::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
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
/// work done between frames (mirrors jewel).
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

        execute!(std::io::stdout(), EnableMouseCapture)?;

        // Kick off the boot status check. The run loop picks it up
        // immediately, draws a spinner, and applies the response when
        // the worker comes back.
        flows::auth::request_boot_status(&mut app);

        let result = run_loop(terminal, &mut app);
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
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

    loop {
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

        // Drain any worker response that arrived since the last
        // tick — non-blocking. The match in `apply_response` handles
        // multiple chained requests (e.g. boot Status → LoadInbox)
        // by re-queuing inside the response handler; the next
        // iteration of this loop picks them up.
        while let Ok(resp) = app.worker_rx.try_recv() {
            flows::apply_response(app, resp);
            done_ticks = 0;
        }

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

        if event::poll(poll_timeout(&app.action_state, app.is_busy()))? {
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

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Returns the next event-poll timeout — fast during animation or
/// while a worker call is in flight, longer when idle.
fn poll_timeout(state: &ActionState, busy: bool) -> Duration {
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
        ActionState::Done(_) | ActionState::Error(_) => {
            *done_ticks += 1;
            if *done_ticks >= FEEDBACK_TICKS {
                app.set_action(ActionState::Idle);
                *done_ticks = 0;
            }
        }
        ActionState::Idle => {}
    }
}
