//! `salat --full` — the complete interactive TUI on the alternate
//! screen (§14.1): the shell, the navigation stack and every screen of
//! §16, driven by §15.2's event loop.
//!
//! Two phases. First the boot phase: [`app::boot`]'s ladder runs on its
//! own thread (it owns its tokio runtime and must complete
//! synchronously) while this thread animates the LOADING screen with
//! plain `poll`/`read` — there is no `Runtime` yet, so there is
//! nothing for an async loop to drive. Ctrl-C aborts, a failure shows
//! the red `✕` beat and then the ERROR screen with `[R] retry`. Then
//! the main loop: §15.2's `select!` over events, the 250 ms tick, the
//! configuration watch and the signals channel, drawing through
//! [`crate::ui::render`] at most every 50 ms.

use std::io::Stdout;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chrono_tz::Tz;
use crossterm::event;
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt as _;
use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Style;
use tokio::sync::broadcast;

use crate::app::boot::{self, BootState};
use crate::app::clock::{Clock, SystemClock};
use crate::app::runtime::Runtime;
use crate::app::signals;
use crate::cli::Cli;
use crate::config::Config;
use crate::error::{ExitCode, SalatError};
use crate::ui::input::Outcome;
use crate::ui::layout::Breakpoint;
use crate::ui::model::build_model;
use crate::ui::render;
use crate::ui::screens::{error as error_screen, loading};
use crate::ui::shell::{Shell, overlay_from_transition};
use crate::ui::theme::Theme;
use crate::ui::widgets::too_small;

use super::{TerminalGuard, term_io};

/// The blink cadence the boot animation draws at, and the input poll
/// that enforces it.
const BOOT_POLL: Duration = Duration::from_millis(50);

/// The red-`✕` beat §16.11 item 5 holds before the ERROR screen.
const ERROR_BEAT: Duration = Duration::from_millis(400);

/// What the boot phase ended with.
enum BootOutcome {
    /// The `Runtime`, wired and warm — behind the `Arc` the scheduler's
    /// runner will hold a `Weak` to (§5.1).
    Runtime(Arc<Runtime>),
    /// Ctrl-C during boot — a clean, quiet exit.
    UserAbort,
    /// A fatal boot stage; the ERROR screen takes over.
    Failed(SalatError),
}

/// One message from the boot thread.
enum BootMsg {
    State(BootState),
    Done(Box<Result<Arc<Runtime>, SalatError>>),
}

/// Run the full TUI (§15.3: alternate screen, raw mode, restore on
/// every exit path — the guard's `Drop` and panic hook).
pub fn run(cli: &Cli) -> Result<ExitCode, SalatError> {
    let _guard = TerminalGuard::new_fullscreen()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend).map_err(term_io)?;
    // The boot screen's theme: the configuration is stage 0's work, so
    // the defaults hold for the seconds the ladder takes — the real
    // theme re-renders every frame once the loop starts.
    let theme = Theme::from(&Config::default().display);

    let runtime = loop {
        match boot_phase(cli, &mut terminal, &theme)? {
            BootOutcome::Runtime(runtime) => break runtime,
            BootOutcome::UserAbort => return Ok(ExitCode::Ok),
            BootOutcome::Failed(error) => match error_phase(&error, &mut terminal, &theme)? {
                ErrorChoice::Retry => continue,
                ErrorChoice::Quit => return Err(error),
            },
        }
    };

    runtime.block_on(run_loop(&runtime, &mut terminal))?;
    // The guard's Drop leaves the alternate screen and raw mode; the
    // user's previous terminal contents return (§15.3).
    Ok(ExitCode::Ok)
}

/// The boot phase: run the ladder on its own thread while animating
/// the LOADING screen. Returns when boot completes, fails, or the user
/// aborts — never before one of those.
fn boot_phase(
    cli: &Cli,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    theme: &Theme,
) -> Result<BootOutcome, SalatError> {
    let (tx, rx) = mpsc::channel::<BootMsg>();
    let boot_cli = cli.clone();
    let boot_thread = thread::spawn(move || {
        let result = boot::run(&boot_cli, |state| {
            // The receiver dies with the main thread's return — a send
            // failure then just means nobody is watching.
            let _ = tx.send(BootMsg::State(state));
        });
        let _ = tx.send(BootMsg::Done(Box::new(result)));
    });

    let mut state = BootState::starting();
    let clock = SystemClock;
    loop {
        // 1. Drain every pending transition.
        let mut done = None;
        loop {
            match rx.try_recv() {
                Ok(BootMsg::State(next)) => state = next,
                Ok(BootMsg::Done(result)) => done = Some(result),
                Err(_) => break,
            }
        }

        // 2. Non-blocking input sweep: Ctrl-C is the one key boot
        //    answers (§16.11 item 7).
        while event::poll(Duration::ZERO).map_err(term_io)? {
            if is_ctrl_c(event::read().map_err(term_io)?) {
                // Detach, never join: the ladder may be mid-fetch with
                // its whole retry budget ahead of it, and the abort must
                // not wait. The process exits here; the thread owns
                // everything it touches and dies with the process.
                drop(boot_thread);
                return Ok(BootOutcome::UserAbort);
            }
        }

        // 3. Draw the current state.
        draw_boot(terminal, theme, &state, &clock)?;

        // 4. A finished ladder decides the outcome; a failure holds
        //    its §16.11 beat first.
        if let Some(result) = done {
            let _ = boot_thread.join();
            return Ok(match *result {
                Ok(runtime) => BootOutcome::Runtime(runtime),
                Err(error) => {
                    // The error state was the last thing drawn; hold
                    // the red `✕` beat before the ERROR screen.
                    thread::sleep(ERROR_BEAT);
                    BootOutcome::Failed(error)
                }
            });
        }

        // 5. Block briefly — the animation's cadence, and the latest
        //    a finished ladder can wait.
        if event::poll(BOOT_POLL).map_err(term_io)? && is_ctrl_c(event::read().map_err(term_io)?) {
            // Same detach-as-abort as the sweep above.
            drop(boot_thread);
            return Ok(BootOutcome::UserAbort);
        }
    }
}

/// What the user chose on the boot-failure ERROR screen.
enum ErrorChoice {
    Retry,
    Quit,
}

/// The boot-failure ERROR screen: `[R] retry` re-runs the ladder,
/// `q`/Esc/Ctrl-C quits with the error. The `c`/`d` actions belong to
/// Phases 11–12, so they wait here.
fn error_phase(
    error: &SalatError,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    theme: &Theme,
) -> Result<ErrorChoice, SalatError> {
    loop {
        terminal
            .draw(|frame| {
                let area = frame.area();
                let buf = frame.buffer_mut();
                buf.set_style(area, Style::new().bg(theme.term_bg));
                let breakpoint = Breakpoint::of(area);
                if breakpoint == Breakpoint::TooSmall {
                    too_small::render(area, buf);
                } else {
                    error_screen::render(error, area, buf, theme, breakpoint);
                }
            })
            .map_err(term_io)?;

        if event::poll(Duration::from_millis(250)).map_err(term_io)?
            && let Event::Key(key) = event::read().map_err(term_io)?
            && key.kind == KeyEventKind::Press
        {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
            {
                return Ok(ErrorChoice::Quit);
            }
            match key.code {
                KeyCode::Char('r') => return Ok(ErrorChoice::Retry),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(ErrorChoice::Quit),
                _ => {}
            }
        }
    }
}

/// One boot frame: the guard below 80×24 (§16.13 — the DoD holds
/// during boot too), the LOADING screen otherwise.
fn draw_boot(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    theme: &Theme,
    state: &BootState,
    clock: &SystemClock,
) -> Result<(), SalatError> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            let buf = frame.buffer_mut();
            buf.set_style(area, Style::new().bg(theme.term_bg));
            let breakpoint = Breakpoint::of(area);
            if breakpoint == Breakpoint::TooSmall {
                too_small::render(area, buf);
            } else {
                // Blinking needs "now"; the configured zone is stage
                // 1's work, so the beat runs on UTC — only the phase
                // matters, never the zone.
                let now = clock.now_in(Tz::UTC);
                loading::render(area, buf, theme, state, now, breakpoint);
            }
        })
        .map(|_| ())
        .map_err(term_io)
}

/// Whether an event is the Ctrl-C abort.
fn is_ctrl_c(event: Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    key.kind == KeyEventKind::Press
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
}

/// §15.2's loop with Phase 8's arms: events, the 250 ms tick (with the
/// rollover check), the configuration watch, the signals channel, and
/// the scheduler's `svc_rx`. The owner path raises its overlays on the
/// broadcast; an observer, which receives nothing, raises them from
/// the state machine's own transitions (§16.7 item 3's second entry
/// path). Drawing is on demand, capped at 20 Hz.
async fn run_loop(
    runtime: &Runtime,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), SalatError> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut cfg_rx = runtime.subscribe_config();
    let mut svc_rx = runtime.subscribe_service();
    let mut signal_rx = signals::forward();

    let mut shell = Shell::new();
    let mut model = build_model(runtime);
    let mut previous_states = model.states;
    let mut last_draw = Instant::now();
    let mut dirty = true;

    loop {
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => match shell.handle(event, runtime).await? {
                    Outcome::Quit => break,
                    Outcome::Redraw => {
                        terminal.clear().map_err(term_io)?;
                        dirty = true;
                    }
                    // The shell's state changed — the cursor moved or a
                    // completion landed in the store. Rebuild the model
                    // now rather than waiting for the next tick: the
                    // draw below could otherwise paint the stale one.
                    Outcome::Changed => {
                        let (width, height) =
                            crossterm::terminal::size().map_err(term_io)?;
                        model = build_model(runtime);
                        model.breakpoint = Breakpoint::of_size(width, height);
                        dirty = true;
                    }
                    // A resize needs no handler (§15.2 decision 3) —
                    // layout derives from the frame area each draw, so
                    // only the redraw matters.
                    Outcome::Ignored => {}
                },
                // A read error is not fatal to the loop — the next
                // event may well arrive.
                Some(Err(_)) => {}
                None => break,
            },
            _ = tick.tick() => {
                runtime.refresh_on_rollover().await;
                // The frame's breakpoint is a per-frame fact (§4.5).
                let (width, height) = crossterm::terminal::size().map_err(term_io)?;
                let rebuilt = build_model(runtime);
                // §16.7 item 3's transition path. In observer mode it
                // is the only alarm (the daemon owns the fire); in
                // owner mode it is the §10.1 backstop — `adhan = false`
                // mutes the event, not the visual alarm — subordinated
                // to the broadcast through `raise_transition` so a
                // late fire keeps its `(delayed)` wording.
                if let Some(raised) =
                    overlay_from_transition(&previous_states, &rebuilt.states, &rebuilt)
                {
                    let merged =
                        crate::ui::shell::raise_transition(shell.overlay, raised);
                    shell.set_overlay(merged);
                }
                previous_states = rebuilt.states;
                model = rebuilt;
                model.breakpoint = Breakpoint::of_size(width, height);
                dirty = true;
            }
            changed = cfg_rx.changed() => {
                if changed.is_ok() {
                    // The runtime's own receiver holds the same pending
                    // publication, so this applies rather than waits.
                    runtime.on_config_change().await;
                    model = build_model(runtime);
                    previous_states = model.states;
                    dirty = true;
                }
            }
            signal = signal_rx.recv() => match signal {
                Some(signal) if signal.is_shutdown() => break,
                // SIGHUP (§8.5): re-read the file; the store's own
                // publication wakes the config arm above.
                Some(_) => runtime.reload_config().await,
                None => {}
            },
            service = svc_rx.recv() => match service {
                Ok(event) => {
                    if shell.on_service_event(event) {
                        model = build_model(runtime);
                        previous_states = model.states;
                        dirty = true;
                    }
                }
                // A slow draw skipped publications — the rebuild catches
                // the frame up to the newest one.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    model = build_model(runtime);
                    previous_states = model.states;
                    dirty = true;
                }
                // Never closed: the runtime holds the sender.
                Err(broadcast::error::RecvError::Closed) => {}
            },
        }

        if dirty && last_draw.elapsed() >= BOOT_POLL {
            terminal
                .draw(|frame| {
                    render(frame, &model, &shell);
                })
                .map_err(term_io)?;
            last_draw = Instant::now();
            dirty = false;
        }
    }
    Ok(())
}
