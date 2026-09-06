//! Key resolution, §15.4's exact order:
//!
//! 1. Global, always: `Ctrl-C` quits, `Ctrl-L` forces a redraw.
//! 2. The overlay, if one is up: `Enter` dismisses (and stops the
//!    audio, Phase 10), `s` snoozes (Phase 10), `Esc` dismisses — and
//!    it swallows *everything* else, including `1`–`4` and `q`: a call
//!    to prayer must not be dismissed by an accidental tab switch.
//! 3. The focused screen's own keys.
//! 4. The global fallback: `q` quits, `1`–`4` switch the main tab,
//!    `?` opens the state reference, `Esc` pops the stack (A6).
//!
//! Only `KeyEventKind::Press` reaches a resolver — the filter lives in
//! [`crate::ui::shell::Shell::handle`], one place, before any order
//! applies.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

/// Resolve one `Press` key against the shell, §15.4's order.
pub fn resolve(key: KeyEvent, shell: &mut Shell) -> Outcome {
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
    //    fallthrough below never runs while one is up.
    if shell.overlay.is_some() {
        return match key.code {
            KeyCode::Enter => {
                // Dismiss (and stop the audio — Phase 10 wires the
                // stop call into this arm).
                shell.set_overlay(None);
                Outcome::Changed
            }
            KeyCode::Char('s') => Outcome::Ignored, // snooze — Phase 10
            KeyCode::Esc => {
                shell.set_overlay(None);
                Outcome::Changed
            }
            _ => Outcome::Ignored,
        };
    }

    // 3. The focused screen's own keys.
    if let Some(outcome) = screen_keys(key, shell) {
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

/// Step 3: the focused screen's keys. Phase 6's screens are shells —
/// their vocabularies arrive with their phases (the dashboard's `c`,
/// the matrix's Tab, …). The one screen with Phase 6 keys of its own
/// is LOADING: Ctrl-C only, so it consumes everything else rather than
/// letting the fallback navigate mid-boot.
fn screen_keys(_key: KeyEvent, shell: &Shell) -> Option<Outcome> {
    match shell.screen() {
        // Ctrl-C was already handled at step 1; the loading screen
        // offers nothing else to press.
        Screen::Loading => Some(Outcome::Ignored),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
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
        shell.set_overlay(Some(super::super::shell::Overlay::AdhanAlert));
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

        // Enter and Esc dismiss; `s` stays for Phase 10.
        assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Changed);
        assert!(shell.overlay.is_none());
    }

    /// Ctrl-C outranks the overlay (§15.4 step 1).
    #[test]
    fn ctrl_c_beats_the_overlay() {
        let mut shell = Shell::new();
        shell.set_overlay(Some(super::super::shell::Overlay::MosqueReminder));
        assert_eq!(resolve(ctrl(KeyCode::Char('c')), &mut shell), Outcome::Quit);
        assert!(shell.overlay.is_some(), "quitting does not mutate state");
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
}
