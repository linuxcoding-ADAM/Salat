//! Command entry points — one module per §14.1 surface entry (§19).
//!
//! This module also owns the shared [`TerminalGuard`] — raw mode plus
//! cursor state, entered by both interactive surfaces and restored on
//! every exit path (§15.3): `Drop` for normal returns and `?`
//! propagation, the installed panic hook for unwinding, and the `main`
//! error path reuses the same idempotent [`restore_terminal`].

use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal as cterm;

use crate::error::SalatError;

pub mod config;
pub mod daemon;
pub mod full;
pub mod quick;
pub mod status;
pub mod test_adhan;

// ─────────────────────────   terminal guard       ───────────────────────

/// Whether raw mode is currently on — the idempotence flag behind
/// [`restore_terminal`], so the guard's `Drop`, a `?` propagation and
/// the panic hook can all call it and exactly one restoration happens.
static RAW_MODE: AtomicBool = AtomicBool::new(false);

/// Whether the alternate screen is currently entered — only the
/// fullscreen guard sets it, so restoration leaves the alternate screen
/// exactly when it was entered (§15.3: the full UI restores the user's
/// previous screen contents).
static ALT_SCREEN: AtomicBool = AtomicBool::new(false);

static HOOK_INSTALLED: Once = Once::new();

/// Restore the terminal: show the cursor, leave the alternate screen if
/// it was entered, and leave raw mode. Idempotent.
fn restore_terminal() {
    if RAW_MODE.swap(false, Ordering::SeqCst) {
        let _ = execute!(std::io::stdout(), Show);
        if ALT_SCREEN.swap(false, Ordering::SeqCst) {
            let _ = execute!(std::io::stdout(), cterm::LeaveAlternateScreen);
        }
        let _ = cterm::disable_raw_mode();
    }
}

/// Raw mode, the hidden cursor and — for the fullscreen variant — the
/// alternate screen, restored however the command ends: `Drop` covers
/// normal exit and `?` propagation, and the panic hook installed here
/// covers the third path (§15.3) — it restores first, then delegates to
/// the previous hook (color-eyre's, installed in `main`), so the panic
/// report itself prints on a cooked terminal.
pub(crate) struct TerminalGuard;

impl TerminalGuard {
    /// The quick surface's guard (§16.14): raw mode and a hidden cursor
    /// in the inline viewport. No alternate screen — quick mode renders
    /// into the user's scrollback.
    pub(crate) fn new_inline() -> Result<Self, SalatError> {
        Self::enter(false)
    }

    /// The full surface's guard (§15.3): raw mode, a hidden cursor and
    /// the alternate screen, so the app owns every cell and the user's
    /// previous screen returns on exit.
    pub(crate) fn new_fullscreen() -> Result<Self, SalatError> {
        Self::enter(true)
    }

    fn enter(alt_screen: bool) -> Result<Self, SalatError> {
        cterm::enable_raw_mode().map_err(term_io)?;
        if alt_screen && let Err(error) = execute!(std::io::stdout(), cterm::EnterAlternateScreen) {
            let _ = cterm::disable_raw_mode();
            return Err(term_io(error));
        }
        if let Err(error) = execute!(std::io::stdout(), Hide) {
            if alt_screen {
                let _ = execute!(std::io::stdout(), cterm::LeaveAlternateScreen);
            }
            let _ = cterm::disable_raw_mode();
            return Err(term_io(error));
        }
        RAW_MODE.store(true, Ordering::SeqCst);
        if alt_screen {
            ALT_SCREEN.store(true, Ordering::SeqCst);
        }

        HOOK_INSTALLED.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                restore_terminal();
                previous(info);
            }));
        });
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// The shared terminal-I/O failure: E902.
pub(crate) fn term_io(error: std::io::Error) -> SalatError {
    SalatError::TerminalIo {
        cause: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §15.3's third restoration path: the panic hook. Raw mode needs
    /// a real terminal, so this runs under a pty —
    /// `/tmp/salat-pty/ptycheck.py` or
    /// `script -qec 'cargo test --lib -- --ignored panic_hook --exact'`
    /// — and is ignored by the plain suite, where
    /// `TerminalGuard::new_inline` has no terminal to enter.
    #[test]
    #[ignore = "needs a pty (run under script(1))"]
    fn panic_hook_restores_the_terminal() {
        let _guard = TerminalGuard::new_inline().expect("a terminal to enter raw mode on");
        assert!(RAW_MODE.load(Ordering::SeqCst));

        // The panic runs the installed hook — which must flip raw mode
        // off — before unwinding to here; the guard's `Drop` then finds
        // nothing left to do.
        let result = std::panic::catch_unwind(|| {
            panic!("terminal restoration check");
        });
        assert!(result.is_err(), "the panic was caught");
        assert!(
            !RAW_MODE.load(Ordering::SeqCst),
            "the hook restored the terminal before unwinding"
        );
    }
}
