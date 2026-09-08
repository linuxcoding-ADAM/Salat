//! The audio subsystem (§13): an external player subprocess, never an
//! embedded engine (§13.1 — ALSA linkage would forbid the static musl
//! binary), driven through the [`AudioPlayer`] contract of §13.3.
//!
//! * [`players`] — the §13.2 probe chain and per-player argv.
//! * [`file`] — the §13.4 file/preset resolution and validation.
//! * [`Player`] (this module) — the production player: one playback
//!   at a time, fire-and-forget `play`, a bounded `stop`, and the
//!   Status screen's `Audio system` component.
//!
//! The player is owned by the `Runtime`, never by a screen (§13.6):
//! playback survives screen changes and resizes, and stops only on
//! `Enter` (dismiss = silence), `s` (snooze = stop + re-arm), or the
//! process's own exit — the child is `kill_on_drop`, so a crashing or
//! interrupted Salat never leaves an Adhan playing.

pub mod file;
pub mod players;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::watch;

use crate::config::Config;
use crate::domain::{EventKind, ScheduledEvent};
use crate::scheduler::EventSink;
use crate::status::model::{Component, ComponentId, Health};
use file::AudioError;
use players::PlayerKind;

pub use file::{resolve_path, system_data_dir, validate};
pub use players::probe_path;

/// §13.3: a wedged player must not block `Enter` on the Adhan overlay —
/// `stop` SIGTERMs, waits this long, then SIGKILLs.
const STOP_GRACE: Duration = Duration::from_millis(500);

/// How often the fire-and-forget watcher checks whether the child has
/// exited. Sync `try_wait` only — no lock is ever held across an await.
const WATCH_INTERVAL: Duration = Duration::from_millis(200);

/// The §13.3 contract. One playback at a time; `play` is
/// fire-and-forget (returns once the child is spawned).
#[async_trait]
pub trait AudioPlayer: Send + Sync {
    /// Start `file` at `volume`, stopping any current playback first.
    async fn play(&self, file: &Path, volume: u8) -> Result<(), AudioError>;
    /// Stop the current playback: SIGTERM, 500 ms, SIGKILL. A no-op
    /// (not an error) when nothing is playing.
    async fn stop(&self) -> Result<(), AudioError>;
    /// Whether a child is running right now.
    fn is_playing(&self) -> bool;
    /// The `Audio system` Status component.
    fn report(&self) -> Component;
}

/// One live playback: the child, shared between the player's slot (the
/// `stop` surface) and the watcher task (the exit logger). The inner
/// `Option` being empty means "stopped elsewhere" — the watcher breaks
/// on it rather than reporting a duration it can no longer observe.
struct Playback {
    child: Mutex<Option<tokio::process::Child>>,
    started: Instant,
}

impl Playback {
    /// SIGTERM the child in place, without waiting — `Drop`'s only
    /// move, because a destructor cannot await. The ladder in
    /// [`stop_child`] is the full §13.3 contract.
    fn signal(&self) {
        if let Ok(mut slot) = self.child.lock()
            && let Some(child) = slot.as_mut()
        {
            terminate(child);
        }
    }

    /// Take the child out of the slot — `stop` and the replacement
    /// path both funnel through here. The empty slot tells the watcher
    /// its observation is over.
    fn take_child(&self) -> Result<Option<tokio::process::Child>, AudioError> {
        let mut slot = self
            .child
            .lock()
            .map_err(|poisoned| AudioError::PlaybackFailed {
                detail: format!("the child slot is poisoned: {poisoned}"),
            })?;
        Ok(slot.take())
    }
}

/// §13.3's ladder over one child: SIGTERM, `STOP_GRACE` of reaping,
/// then SIGKILL. Whoever calls this owns the child — it is reaped by
/// the final `wait`, so no Adhan outlives the command that stopped it.
async fn stop_child(mut child: tokio::process::Child) {
    terminate(&child);
    if tokio::time::timeout(STOP_GRACE, child.wait())
        .await
        .is_err()
    {
        // The grace expired: the KILL rung, then a reaping wait that
        // cannot hang — SIGKILL is not ignorable.
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

/// SIGTERM one child — the first rung of §13.3's ladder, and the
/// replacement path's best effort.
///
/// tokio's `Child` can only KILL (`start_kill` sends SIGKILL), and the
/// mandated stack (§5.1) has no signal-sending crate, so `kill(2)` is
/// declared directly: `std` already links the C library on every
/// Linux target, and `SIGTERM` (15) is kernel ABI, not a libc constant
/// that varies.
///
/// # Safety
///
/// The `pid` belongs to a child Salat spawned and has not reaped, so
/// the signal cannot land on a recycled pid. A child that already
/// exited answers `-1`/`ESRCH` — best effort by design; the ladder's
/// KILL rung is the guarantee.
fn terminate(child: &tokio::process::Child) {
    // POSIX `kill(2)` — see `terminate`'s commentary.
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGTERM: i32 = 15;
    if let Some(pid) = child.id() {
        // Best effort — see the ladder.
        unsafe { kill(pid as i32, SIGTERM) };
    }
}

/// The production player over the four §13.2 subprocesses.
pub struct Player {
    /// The probed player, when one was found (§13.2). `None` with
    /// `probed: true` is the `NO AUDIO PLAYER` status row — the
    /// overlay still shows (§23 Phase 10 item 8: the visual
    /// announcement is independent of playback).
    kind: Option<PlayerKind>,
    /// Whether the §13.2 probe ran. An unprobed player reports
    /// `Unknown`, never a false `Ok` (§16.4 item 8's honesty rule).
    probed: bool,
    /// The current playback, if any.
    current: Mutex<Option<Arc<Playback>>>,
    /// The most recent playback failure's text — `report` prefers it
    /// over the probe, like the notifier's `last_error` (§12.3).
    last_error: Mutex<Option<String>>,
}

impl Player {
    /// The unprobed player — `report` says `NOT PROBED`. Production
    /// uses [`Player::probed`].
    pub fn new() -> Self {
        Self {
            kind: None,
            probed: false,
            current: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    /// Boot stage 4's one probe (§13.2): keep the first player on
    /// `$PATH`, or record the absence.
    pub fn probed() -> Self {
        let kind = probe_path(&players::SystemPathEnv);
        tracing::info!(target: "salat::audio", ?kind, "audio player probe");
        Self {
            kind,
            probed: true,
            current: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    /// The probed player kind — `test-adhan` (§13.5) prints it.
    pub fn kind(&self) -> Option<PlayerKind> {
        self.kind
    }

    /// A probed player with a scripted kind — the runtime fixture's
    /// healthy machine, without touching the real `$PATH` (a real
    /// probe would make the Status row — and every strip dot that
    /// feeds on it — depend on the host). Test-only: production
    /// always goes through [`Player::probed`].
    #[cfg(test)]
    pub(crate) fn scripted(kind: PlayerKind) -> Self {
        Self {
            kind: Some(kind),
            probed: true,
            current: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Player {
    /// §13.3's process cleanup: a dropping Salat (panic, runtime
    /// teardown, normal exit) never leaves an Adhan playing. The
    /// destructor's only move is the best-effort SIGTERM — it cannot
    /// await — and `kill_on_drop` on the child plus the runtime's
    /// teardown supply the SIGKILL backstop.
    fn drop(&mut self) {
        if let Ok(current) = self.current.lock()
            && let Some(playback) = current.as_ref()
        {
            playback.signal();
        }
    }
}

#[async_trait]
impl AudioPlayer for Player {
    async fn play(&self, file: &Path, volume: u8) -> Result<(), AudioError> {
        let kind = self.kind.ok_or(AudioError::NoPlayer)?;
        // One lock for the whole swap: stop the old child and drop the
        // player's reference, then put the new playback in the slot.
        // No await happens inside.
        let playback = {
            let mut current = self.lock_current()?;
            if let Some(old) = current.take() {
                // The replaced playback stops through the full §13.3
                // ladder, off the spawn path: `play` stays
                // fire-and-forget, and one playback at a time still
                // holds — the worst case is a stubborn child
                // overlapping its replacement for the grace window.
                match old.take_child() {
                    Ok(Some(child)) => {
                        tokio::spawn(stop_child(child));
                    }
                    // A poisoned or already-taken slot still gets the
                    // best-effort SIGTERM; kill_on_drop is the backstop.
                    _ => old.signal(),
                }
            }
            let child = players::spawn(kind, file, volume).map_err(|error| {
                self.record_failure(error.to_string());
                AudioError::PlaybackFailed {
                    detail: error.to_string(),
                }
            })?;
            let playback = Arc::new(Playback {
                child: Mutex::new(Some(child)),
                started: Instant::now(),
            });
            current.replace(playback.clone());
            playback
        };

        // The fire-and-forget watcher (§13.3): awaits the child's exit
        // and logs the duration. It never holds the child's lock across
        // an await — `try_wait` is sync, the sleep is between checks —
        // and an empty slot means `stop` took the child, not an exit.
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(WATCH_INTERVAL).await;
                let exited = {
                    let Ok(mut slot) = playback.child.lock() else {
                        break;
                    };
                    let Some(child) = slot.as_mut() else {
                        // Taken by `stop` — no duration to report.
                        return;
                    };
                    matches!(child.try_wait(), Ok(Some(_status)))
                };
                if exited {
                    tracing::info!(
                        target: "salat::audio",
                        "adhan playback ended — duration {:?}",
                        playback.started.elapsed()
                    );
                    return;
                }
            }
        });
        Ok(())
    }

    async fn stop(&self) -> Result<(), AudioError> {
        // Take the whole playback, then the child itself, so the guards
        // are released before the first await.
        let child = {
            let mut current = self.lock_current()?;
            let Some(playback) = current.take() else {
                // Nothing playing: a no-op, not an error — Enter on a
                // silent overlay must not fail (§13.3's bounded stop).
                return Ok(());
            };
            playback.take_child()?
        };
        let Some(child) = child else {
            return Ok(());
        };
        // §13.3's ladder, awaited: SIGTERM, 500 ms, SIGKILL — the
        // whole ladder is async (`tokio::time::sleep`, the reaping
        // `wait`), so a wedged child blocks neither the scheduler's
        // tick nor the render loop.
        stop_child(child).await;
        Ok(())
    }

    fn is_playing(&self) -> bool {
        let Ok(current) = self.current.lock() else {
            return false;
        };
        let Some(playback) = current.as_ref() else {
            return false;
        };
        let Ok(mut slot) = playback.child.lock() else {
            return false;
        };
        slot.as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
    }

    fn report(&self) -> Component {
        // A recorded failure outranks the probe (§12.3's rule, applied
        // to the audio twin): a player that was alive at boot and
        // failed since is news.
        if let Ok(last_error) = self.last_error.lock()
            && let Some(detail) = last_error.as_ref()
        {
            return Component::new(ComponentId::AudioSystem, Health::Degraded, detail.clone());
        }
        let (health, detail) = if !self.probed {
            (Health::Unknown, "NOT PROBED")
        } else {
            match self.kind {
                Some(PlayerKind::Aplay) => (Health::Ok, "READY (aplay · no volume control)"),
                Some(_) => (Health::Ok, "READY"),
                None => (Health::Degraded, "NO AUDIO PLAYER"),
            }
        };
        Component::new(ComponentId::AudioSystem, health, detail)
    }
}

impl Player {
    fn lock_current(&self) -> Result<std::sync::MutexGuard<'_, Option<Arc<Playback>>>, AudioError> {
        self.current
            .lock()
            .map_err(|poisoned| AudioError::PlaybackFailed {
                detail: format!("the player state is poisoned: {poisoned}"),
            })
    }

    fn record_failure(&self, detail: String) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(detail);
        }
    }

    /// The `Adhan sound` Status component (§6.7's row, distinct from
    /// `Audio system`): the file's verdict, checked against the live
    /// configuration — `READY` when the resolved file validates,
    /// `NO ADHAN FILE` when it does not, `DISABLED` when Adhan itself
    /// is off. Checked on every `status()` call, because the file can
    /// appear or vanish while the app runs.
    pub fn adhan_report(&self, config: &Config) -> Component {
        if !config.adhan.enabled {
            return Component::new(ComponentId::AdhanSound, Health::Ok, "DISABLED");
        }
        let data_dir = system_data_dir().unwrap_or_else(|_| Path::new("/").to_path_buf());
        let path = resolve_path(&config.adhan, &data_dir);
        match validate(&path) {
            Ok(_) => Component::new(ComponentId::AdhanSound, Health::Ok, "READY"),
            Err(_) => Component::new(ComponentId::AdhanSound, Health::Degraded, "NO ADHAN FILE"),
        }
    }
}

/// The scheduler's audio half (§13.6, §23 Phase 10 item 7): a fired
/// `Adhan` (or an `AdhanSnooze` re-fire) resolves the §13.4 file from
/// the live configuration and plays it. The failures are all
/// non-events for the scheduler: no player, no file, a failed spawn —
/// each is logged (the Status rows say the same thing) and swallowed,
/// because the overlay and the notification must still go out.
pub struct AudioSink {
    player: Arc<Player>,
    config: watch::Receiver<Arc<Config>>,
}

impl AudioSink {
    pub fn new(player: Arc<Player>, config: watch::Receiver<Arc<Config>>) -> Self {
        Self { player, config }
    }
}

#[async_trait]
impl EventSink for AudioSink {
    async fn on_event(&self, event: ScheduledEvent, _late: bool) {
        if event.kind != EventKind::Adhan && event.kind != EventKind::AdhanSnooze {
            return;
        }
        let cfg = self.config.borrow().clone();
        if !cfg.adhan.enabled {
            // The planner gates on this, so reaching here with it off
            // means the config changed between plan and fire — skip.
            return;
        }
        let data_dir = system_data_dir().unwrap_or_else(|_| Path::new("/").to_path_buf());
        let path = resolve_path(&cfg.adhan, &data_dir);
        if let Err(error) = validate(&path) {
            tracing::warn!(
                target: "salat::audio",
                %error,
                prayer = ?event.prayer,
                "adhan not played — no usable file"
            );
            return;
        }
        let volume = cfg.adhan.volume;
        match self.player.play(&path, volume).await {
            Ok(()) => tracing::info!(
                target: "salat::audio",
                prayer = ?event.prayer,
                %volume,
                file = %path.display(),
                "adhan playback started"
            ),
            Err(error) => tracing::warn!(
                target: "salat::audio",
                %error,
                prayer = ?event.prayer,
                "adhan playback failed — overlay still shown"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report rows for a probed machine: a normal player is
    /// `READY`, aplay is the honest partial capability, none is
    /// `NO AUDIO PLAYER` — and an unprobed player never claims `Ok`
    /// (§16.4 item 8's honesty rule).
    #[test]
    fn report_matches_the_probe_verdict() {
        let unprobed = Player::new();
        assert_eq!(unprobed.report().health, Health::Unknown);
        assert_eq!(unprobed.report().detail, "NOT PROBED");

        let mut aplay = Player::probed();
        aplay.kind = Some(PlayerKind::Aplay);
        assert_eq!(aplay.report().detail, "READY (aplay · no volume control)");
        assert_eq!(aplay.report().health, Health::Ok);

        let mut mpv = Player::probed();
        mpv.kind = Some(PlayerKind::Mpv);
        assert_eq!(mpv.report().detail, "READY");
        assert_eq!(mpv.report().health, Health::Ok);

        let mut none = Player::probed();
        none.kind = None;
        assert_eq!(none.report().detail, "NO AUDIO PLAYER");
        assert_eq!(none.report().health, Health::Degraded);
    }

    /// A recorded playback failure outranks the probe in `report`.
    #[test]
    fn a_playback_failure_is_reported() {
        let mut player = Player::probed();
        player.kind = Some(PlayerKind::Mpv);
        player.record_failure("No such file or directory (os error 2)".to_string());
        assert_eq!(player.report().health, Health::Degraded);
        assert!(player.report().detail.contains("os error 2"));
    }

    /// `stop` with nothing playing is `Ok(())`, and `is_playing` says
    /// the truth both ways.
    #[tokio::test]
    async fn stopping_silence_is_a_no_op() {
        let player = Player::probed();
        player.stop().await.expect("nothing to stop");
        assert!(!player.is_playing());
    }

    /// `play` with no probed player is `NoPlayer` (E401's family) —
    /// and it records nothing in the last-error slot (the probe is the
    /// verdict, not a failure).
    #[tokio::test]
    async fn playing_without_a_player_is_no_player() {
        let mut player = Player::probed();
        player.kind = None;
        let error = player
            .play(Path::new("/data/salat/adhan/x.mp3"), 80)
            .await
            .expect_err("no player was probed");
        assert_eq!(error, AudioError::NoPlayer);
        assert!(player.last_error.lock().expect("slot").is_none());
        assert!(!player.is_playing());
    }

    /// §23 Phase 10 item 8's audio half: `adhan = false` plays
    /// nothing. The sink checks the live configuration before the
    /// file — the gate the planner also applies, guarding the window
    /// where the config changed between plan and fire. The visual
    /// overlay is unaffected (§10.1); that independence is the shell
    /// module's test.
    #[tokio::test]
    async fn a_disabled_adhan_plays_nothing() {
        let mut player = Player::probed();
        player.kind = Some(PlayerKind::Mpv);
        let player = Arc::new(player);

        let mut config = crate::config::Config::default();
        config.adhan.enabled = false;
        // A file that would validate, so only the gate can be what
        // stops the playback.
        let adhan = tempfile::NamedTempFile::new().expect("a stub adhan file");
        std::fs::write(adhan.path(), b"ID3 stub adhan bytes").expect("writable");
        config.adhan.file = adhan.path().to_path_buf();

        let (_tx, rx) = watch::channel(Arc::new(config));
        let sink = AudioSink::new(Arc::clone(&player), rx);
        sink.on_event(
            crate::domain::ScheduledEvent {
                kind: EventKind::Adhan,
                prayer: Some(crate::domain::Notifiable::Maghrib),
                date: crate::domain::testing::TODAY,
                fire_at: crate::domain::testing::maghrib_instant().at,
            },
            false,
        )
        .await;
        assert!(!player.is_playing(), "the gate held");
    }

    /// The slot discipline `stop` relies on: taking the playback out
    /// of `current` empties the player's slot, so `is_playing` says
    /// false and a second `stop` is a no-op. The end-to-end child
    /// lifecycle (spawn → is_playing → stop → silence) lives in
    /// `tests/audio.rs`, which runs a real stub player binary.
    #[test]
    fn taking_the_playback_empties_the_slot() {
        let player = Player::probed();
        let playback = Arc::new(Playback {
            child: Mutex::new(None),
            started: Instant::now(),
        });
        player
            .lock_current()
            .expect("lock")
            .replace(playback.clone());
        assert!(player.current.lock().expect("slot").is_some());
        let taken = player.lock_current().expect("lock").take();
        assert!(taken.is_some());
        assert!(player.current.lock().expect("slot").is_none());
    }
}
