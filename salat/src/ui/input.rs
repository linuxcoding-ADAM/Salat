//! Key resolution, §15.4's exact order:
//!
//! 1. Global, always: `Ctrl-C` quits, `Ctrl-L` forces a redraw.
//! 2. The overlay, if one is up: `Enter` dismisses and stops the
//!    audio (§13.6), `s` snoozes — stops the audio, re-arms the
//!    alarm (§11.7) — `Esc` dismisses without stopping the audio —
//!    and it swallows *everything* else, including `1`–`4` and `q`:
//!    a call to prayer must not be dismissed by an accidental tab
//!    switch.
//! 3. The focused screen's own keys.
//! 4. The global fallback: `q` quits, `1`–`4` switch the main tab,
//!    `?` opens the state reference, `Esc` pops the stack (A6).
//!
//! Only `KeyEventKind::Press` reaches a resolver — the filter lives in
//! [`crate::ui::shell::Shell::handle`], one place, before any order
//! applies.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::runtime::Runtime;
use crate::domain::Notifiable;

use super::shell::{Screen, Shell};

/// What the event loop should do after one event (§15.2's dirty flag,
/// made three-valued so quit/redraw/clean are distinct).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Leave the loop (`Ctrl-C`, `q`).
    Quit,
    /// Clear the terminal and force a full repaint (`Ctrl-L`).
    Redraw,
    /// Shell state changed — mark the model dirty and draw.
    Changed,
    /// Nothing consumed the event — no redraw needed.
    Ignored,
}

/// Resolve one `Press` key against the shell, §15.4's order. Async
/// because the dashboard's `c` completes a prayer through the store
/// (A3) — one SQLite write awaited inline, then the loop rebuilds the
/// model from the runtime's served state.
pub async fn resolve(key: KeyEvent, shell: &mut Shell, runtime: &Runtime) -> Outcome {
    // 1. Global, always — before the overlay, so even a call to prayer
    //    yields to Ctrl-C.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => Outcome::Quit,
            KeyCode::Char('l') | KeyCode::Char('L') => Outcome::Redraw,
            _ => Outcome::Ignored,
        };
    }

    // 2. The overlay swallows everything but its own three keys — the
    //    fallthrough below never runs while one is up. §13.6's
    //    asymmetric audio lifetime: `Enter` silences (DISMISS), `s`
    //    silences *and* re-arms, `Esc` removes the overlay but lets
    //    the audio play to its end.
    if let Some(overlay) = shell.overlay {
        return match key.code {
            KeyCode::Enter => {
                shell.set_overlay(None);
                stop_audio(runtime).await;
                Outcome::Changed
            }
            KeyCode::Char('s') => {
                let source = snooze_source(overlay);
                shell.set_overlay(None);
                stop_audio(runtime).await;
                // §11.7: the source alarm re-fires as its snooze event
                // (+3 min Adhan, +5 min mosque) — latest per alarm
                // wins, never persisted.
                if let Some(source) = source {
                    runtime.snooze(source).await;
                }
                Outcome::Changed
            }
            KeyCode::Esc => {
                shell.set_overlay(None);
                Outcome::Changed
            }
            _ => Outcome::Ignored,
        };
    }

    // 3. The focused screen's own keys.
    if let Some(outcome) = screen_keys(key, shell, runtime).await {
        return outcome;
    }

    // 4. The global fallback.
    match key.code {
        KeyCode::Char('q') => Outcome::Quit,
        KeyCode::Char('1') => {
            shell.go_main(Screen::Dashboard);
            Outcome::Changed
        }
        KeyCode::Char('2') => {
            shell.go_main(Screen::Settings);
            Outcome::Changed
        }
        KeyCode::Char('3') => {
            shell.go_main(Screen::PrayerConfig);
            Outcome::Changed
        }
        KeyCode::Char('4') => {
            shell.go_main(Screen::Status);
            Outcome::Changed
        }
        KeyCode::Char('?') => {
            shell.push(Screen::States);
            Outcome::Changed
        }
        // A6: pop, never empty.
        KeyCode::Esc => {
            shell.pop();
            Outcome::Changed
        }
        _ => Outcome::Ignored,
    }
}

/// Stop the Adhan playback (§13.6's DISMISS semantics). Best-effort:
/// a stop failure is logged by the player, never surfaced as an
/// error — the overlay is already gone and the loop has nothing to
/// tell the user that the log does not. Shared by the full TUI and
/// quick mode's inline overlays: one audio lifetime rule.
pub(crate) async fn stop_audio(runtime: &Runtime) {
    use crate::audio::AudioPlayer as _;
    if let Err(error) = runtime.audio().stop().await {
        tracing::warn!(target: "salat::ui", ?error, "stopping adhan playback failed");
    }
}

/// The §11.7 source alarm behind an overlay — the event as it was
/// fired, reconstructed from the payload, so the scheduler maps it to
/// the right snooze event (`Adhan` -> `AdhanSnooze` +3 min,
/// `MosqueReminder` -> `ReminderSnooze` +5 min). The date rides the
/// fired alarm's civil date, not "today" — a snooze pressed just
/// after midnight re-arms the alarm that actually fired. Shared by
/// the full TUI and quick mode's inline overlays.
pub(crate) fn snooze_source(
    overlay: super::shell::Overlay,
) -> Option<crate::domain::ScheduledEvent> {
    use crate::domain::{EventKind, ScheduledEvent};
    match overlay {
        super::shell::Overlay::AdhanAlert { prayer, at, .. } => Some(ScheduledEvent {
            kind: EventKind::Adhan,
            prayer: Some(prayer),
            date: at.date_naive(),
            fire_at: at,
        }),
        super::shell::Overlay::MosqueReminder {
            prayer,
            reminder_at,
            ..
        } => Some(ScheduledEvent {
            kind: EventKind::MosqueReminder,
            prayer: Some(prayer),
            date: reminder_at.date_naive(),
            fire_at: reminder_at,
        }),
    }
}

/// Step 3: the focused screen's keys. LOADING consumes everything
/// (nothing is navigable mid-boot); the DASHBOARD owns the §16.1 item 7
/// trio; every other screen's vocabulary arrives with its phase and
/// falls through to the global vocabulary meanwhile.
async fn screen_keys(key: KeyEvent, shell: &mut Shell, runtime: &Runtime) -> Option<Outcome> {
    match shell.screen() {
        // Ctrl-C was already handled at step 1; the loading screen
        // offers nothing else to press.
        Screen::Loading => Some(Outcome::Ignored),
        Screen::Dashboard => dashboard_keys(key, shell, runtime).await,
        _ => None,
    }
}

/// The dashboard's three keys (§16.1 item 7): `↑↓` move the TODAY
/// cursor, `c` toggles the cursor row's completion. The cursor clamps
/// to the five rows — the section says nothing of wrapping, and a
/// clamp is the conservative reading. Everything else returns `None`
/// so the global vocabulary (`q`, `1`–`4`, `?`, Esc) still applies.
async fn dashboard_keys(key: KeyEvent, shell: &mut Shell, runtime: &Runtime) -> Option<Outcome> {
    match key.code {
        KeyCode::Up => {
            shell.dashboard.cursor = shell.dashboard.cursor.saturating_sub(1);
            Some(Outcome::Changed)
        }
        KeyCode::Down => {
            shell.dashboard.cursor = (shell.dashboard.cursor + 1).min(Notifiable::ALL.len() - 1);
            Some(Outcome::Changed)
        }
        KeyCode::Char('c') => {
            // A3: the completion round-trips through the store; the
            // `Changed` outcome makes the loop rebuild the model from
            // the runtime's served set, so the badge is the store's
            // answer, never a local guess.
            let prayer = Notifiable::ALL.get(shell.dashboard.cursor).copied()?;
            runtime.toggle_completion(prayer).await;
            Some(Outcome::Changed)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crossterm::event::KeyEvent;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    /// Resolve against the §20.3 fixture runtime — the `c` arm's
    /// store write needs a real one; every other key ignores it.
    fn resolve(key: KeyEvent, shell: &mut Shell) -> Outcome {
        let fixture = testing::snapshot_runtime();
        fixture
            .runtime
            .block_on(async { super::resolve(key, shell, &fixture.runtime).await })
    }

    /// §23 Phase 6's test 1: `1`–`4` set the expected screen — from
    /// any depth, replacing the root.
    #[test]
    fn digits_switch_the_main_screen() {
        let mut shell = Shell::new();
        for (key, screen) in [
            (KeyCode::Char('1'), Screen::Dashboard),
            (KeyCode::Char('2'), Screen::Settings),
            (KeyCode::Char('3'), Screen::PrayerConfig),
            (KeyCode::Char('4'), Screen::Status),
        ] {
            assert_eq!(resolve(press(key), &mut shell), Outcome::Changed);
            assert_eq!(*shell.screen(), screen);
            assert_eq!(shell.stack.len(), 1, "tab keys replace, not stack");
        }
    }

    /// Esc pops without ever emptying (A6).
    #[test]
    fn esc_pops_but_never_empties() {
        let mut shell = Shell::new();
        shell.push(Screen::States);
        assert_eq!(resolve(press(KeyCode::Esc), &mut shell), Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Dashboard);
        assert_eq!(resolve(press(KeyCode::Esc), &mut shell), Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Dashboard, "the root holds");
    }

    /// `?` pushes the state reference; Esc returns.
    #[test]
    fn question_pushes_states_and_esc_returns() {
        let mut shell = Shell::new();
        assert_eq!(
            resolve(press(KeyCode::Char('?')), &mut shell),
            Outcome::Changed
        );
        assert_eq!(*shell.screen(), Screen::States);
        assert_eq!(resolve(press(KeyCode::Esc), &mut shell), Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Dashboard);
    }

    /// The overlay swallows `1`–`4` (and `q`): the screen does not
    /// change and nothing falls through.
    #[test]
    fn an_overlay_swallows_the_tab_keys_and_quit() {
        let mut shell = Shell::new();
        shell.set_overlay(Some(test_adhan_overlay()));
        for key in [
            KeyCode::Char('1'),
            KeyCode::Char('4'),
            KeyCode::Char('q'),
            KeyCode::Char('?'),
        ] {
            assert_eq!(resolve(press(key), &mut shell), Outcome::Ignored);
        }
        assert_eq!(*shell.screen(), Screen::Dashboard);
        assert!(shell.overlay.is_some(), "the overlay held");

        // Enter and Esc dismiss.
        assert_eq!(resolve(press(KeyCode::Esc), &mut shell), Outcome::Changed);
        assert!(shell.overlay.is_none());
    }

    /// §16.7 item 7 / §16.8 item 7: `Enter` and `s` both dismiss —
    /// `s` through the §11.7 re-arm, which is safe against the fixture
    /// (no scheduler is running, so `Runtime::snooze` returns early)
    /// and through §13.6's stop (a no-op with nothing playing).
    #[test]
    fn enter_and_s_dismiss_the_overlay() {
        let mut shell = Shell::new();
        shell.set_overlay(Some(test_adhan_overlay()));
        assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Changed);
        assert!(shell.overlay.is_none(), "Enter dismisses");

        shell.set_overlay(Some(test_mosque_overlay()));
        assert_eq!(
            resolve(press(KeyCode::Char('s')), &mut shell),
            Outcome::Changed
        );
        assert!(shell.overlay.is_none(), "s dismisses while re-arming");
    }

    /// Ctrl-C outranks the overlay (§15.4 step 1).
    #[test]
    fn ctrl_c_beats_the_overlay() {
        let mut shell = Shell::new();
        shell.set_overlay(Some(test_mosque_overlay()));
        assert_eq!(resolve(ctrl(KeyCode::Char('c')), &mut shell), Outcome::Quit);
        assert!(shell.overlay.is_some(), "quitting does not mutate state");
    }

    /// A payloaded overlay for the tests — Maghrib at the snapshot's
    /// specimen instant.
    fn test_adhan_overlay() -> super::super::shell::Overlay {
        use chrono::TimeZone;
        let zone: chrono_tz::Tz = chrono_tz::Africa::Algiers;
        let at = zone
            .with_ymd_and_hms(2026, 9, 5, 19, 21, 0)
            .single()
            .expect("the specimen instant exists");
        super::super::shell::Overlay::AdhanAlert {
            prayer: Notifiable::Maghrib,
            at,
            late: false,
        }
    }

    /// A payloaded mosque reminder for the tests — Maghrib's, at the
    /// specimen reminder instant.
    fn test_mosque_overlay() -> super::super::shell::Overlay {
        use chrono::TimeZone;
        let zone: chrono_tz::Tz = chrono_tz::Africa::Algiers;
        let reminder_at = zone
            .with_ymd_and_hms(2026, 9, 5, 19, 36, 0)
            .single()
            .expect("the specimen reminder instant exists");
        super::super::shell::Overlay::MosqueReminder {
            prayer: Notifiable::Maghrib,
            reminder_at,
            late: false,
        }
    }

    /// Ctrl-L is the redraw escape hatch.
    #[test]
    fn ctrl_l_redraws() {
        let mut shell = Shell::new();
        assert_eq!(
            resolve(ctrl(KeyCode::Char('l')), &mut shell),
            Outcome::Redraw
        );
    }

    /// The loading screen consumes everything but Ctrl-C.
    #[test]
    fn loading_consumes_navigation_keys() {
        let mut shell = Shell::new();
        shell.replace(Screen::Loading);
        for key in [KeyCode::Char('1'), KeyCode::Char('q'), KeyCode::Char('?')] {
            assert_eq!(resolve(press(key), &mut shell), Outcome::Ignored);
        }
        assert_eq!(*shell.screen(), Screen::Loading);
    }

    /// §16.1 item 7: `↑↓` move the cursor and clamp at both ends —
    /// `↑` at the top stays on Fajr, `↓` past the bottom stays on
    /// Isha; the spec names no wrap, so there is none.
    #[test]
    fn the_cursor_moves_and_clamps() {
        let mut shell = Shell::new();
        assert_eq!(resolve(press(KeyCode::Down), &mut shell), Outcome::Changed);
        assert_eq!(shell.dashboard.cursor, 1);
        assert_eq!(resolve(press(KeyCode::Up), &mut shell), Outcome::Changed);
        assert_eq!(shell.dashboard.cursor, 0);
        assert_eq!(resolve(press(KeyCode::Up), &mut shell), Outcome::Changed);
        assert_eq!(shell.dashboard.cursor, 0, "clamped at the top");
        for _ in 0..7 {
            assert_eq!(resolve(press(KeyCode::Down), &mut shell), Outcome::Changed);
        }
        assert_eq!(shell.dashboard.cursor, 4, "clamped at the bottom");
    }

    /// A3's round trip: `c` on the cursor's row writes the completion
    /// through SQLite and serves it back — `c` again un-completes.
    /// The served set is the store's answer, read through a second
    /// runtime on the same database file.
    #[test]
    fn c_toggles_a_completion_through_the_store() {
        let fixture = testing::snapshot_runtime();
        let mut shell = Shell::new();
        shell.dashboard.cursor = 3; // Maghrib, the snapshot's next

        {
            let outcome = fixture.runtime.block_on(async {
                super::resolve(press(KeyCode::Char('c')), &mut shell, &fixture.runtime).await
            });
            assert_eq!(outcome, Outcome::Changed);
        }
        assert_eq!(
            fixture.runtime.completions(),
            vec![Notifiable::Maghrib],
            "the toggle landed in the served set"
        );
        // And the store row itself — the write, not the mirror.
        let window = fixture.runtime.window();
        let stored = fixture.runtime.block_on(async {
            fixture
                .store
                .completions_for(window.today.date)
                .await
                .expect("the store is the fixture's own")
        });
        assert_eq!(stored, vec![Notifiable::Maghrib]);

        // `c` again un-completes: the row is deleted, the set empties.
        {
            let outcome = fixture.runtime.block_on(async {
                super::resolve(press(KeyCode::Char('c')), &mut shell, &fixture.runtime).await
            });
            assert_eq!(outcome, Outcome::Changed);
        }
        assert!(fixture.runtime.completions().is_empty());
    }
}
