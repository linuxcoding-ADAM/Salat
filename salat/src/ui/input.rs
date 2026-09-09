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
use crate::config::{Config, LocationConfig, LocationSource, TimeFormat};
use crate::domain::Notifiable;
use crate::location::cities::{self, CityMatch};
use crate::location::system_timezone;
use crate::ui::screens::{notif_settings, prayer_config, settings};

use super::shell::{CoordFocus, LocationMode, NotifCol, Screen, Shell};

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
        Screen::Settings => settings_keys(key, shell, runtime).await,
        Screen::NotifSettings => notif_keys(key, shell, runtime).await,
        Screen::PrayerConfig => prayer_config_keys(key, shell, runtime).await,
        Screen::Location => location_keys(key, shell, runtime),
        // §16.9 item 7: Enter and Esc pop back; everything else is
        // ignored — the save already happened.
        Screen::Confirm { .. } => match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                shell.pop();
                Some(Outcome::Changed)
            }
            _ => Some(Outcome::Ignored),
        },
        // §16.10 item 4: the ERROR screen's actions are the error's own
        // — `r` retry, `c` coordinates, `d` defaults, Esc back.
        Screen::Error(error) => error_keys(key, shell, error.clone()),
        // §16.4 item 7: `↑↓`/`PgUp`/`PgDn` scroll the event log, `r`
        // re-probes. The tab digits, `q` and Esc fall through to the
        // global vocabulary.
        Screen::Status => status_keys(key, shell),
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

/// The STATUS screen's keys (§16.4 item 7): `↑↓` scroll the event log
/// one event at a time, `PgUp`/`PgDn` a page, `r` forces the refresh —
/// the flag paints `REFRESHING…` into the crumb, and the event loop's
/// tick arm (the only place that may await the re-probe) performs it
/// and clears the flag. Everything else returns `None` so the global
/// vocabulary (`q`, `1`–`4`, Esc) still applies.
fn status_keys(key: KeyEvent, shell: &mut Shell) -> Option<Outcome> {
    match key.code {
        KeyCode::Up => {
            shell.status.scroll = shell.status.scroll.saturating_sub(1);
            Some(Outcome::Changed)
        }
        KeyCode::Down => {
            shell.status.scroll = shell.status.scroll.saturating_add(1);
            Some(Outcome::Changed)
        }
        KeyCode::PageUp => {
            shell.status.scroll = shell.status.scroll.saturating_sub(PAGE);
            Some(Outcome::Changed)
        }
        KeyCode::PageDown => {
            shell.status.scroll = shell.status.scroll.saturating_add(PAGE);
            Some(Outcome::Changed)
        }
        KeyCode::Char('r') => {
            // Only set the flag; the re-probe itself is the tick arm's
            // (§16.4 item 8 — the repaint below shows REFRESHING…
            // while it runs). A repeat press is a no-op, not a queue.
            shell.status.refreshing = true;
            Some(Outcome::Changed)
        }
        _ => None,
    }
}

/// The ERROR screen's keys (§16.10 item 4): the buttons the error's
/// own `actions()` advertise — `r` retry, `c` coordinates, `d` use
/// defaults, Esc back. Keys whose action the error does not offer are
/// ignored (the screen renders only the offered buttons; a press the
/// design did not wire is not one).
fn error_keys(
    key: KeyEvent,
    shell: &mut Shell,
    error: crate::error::SalatError,
) -> Option<Outcome> {
    use crate::error::ErrorAction;
    match key.code {
        KeyCode::Char('r') if error.actions().contains(&ErrorAction::Retry) => {
            // Retry is the boot ladder's re-run — this screen's own
            // loop (full.rs `error_phase`) owns that key while the
            // stack is only a boot failure. On the post-boot stack the
            // honest answer is a redraw: the error stays until its
            // producer resolves it.
            Some(Outcome::Ignored)
        }
        KeyCode::Char('c') if error.actions().contains(&ErrorAction::Coordinates) => {
            // §16.10 item 5: jump straight to LOCATION COORDS mode —
            // the manual fix for a failed lookup.
            shell.pop();
            shell.location.mode = LocationMode::Coords;
            shell.push(Screen::Location);
            Some(Outcome::Changed)
        }
        KeyCode::Char('d') if error.actions().contains(&ErrorAction::UseDefaults) => {
            // §16.10 item 5: restore the default configuration, then
            // back — the draft the save flow keeps open is the
            // reverted one.
            shell.draft = None;
            shell.pop();
            Some(Outcome::Changed)
        }
        KeyCode::Esc if error.actions().contains(&ErrorAction::Back) => {
            shell.pop();
            Some(Outcome::Changed)
        }
        _ => Some(Outcome::Ignored),
    }
}

/// One page of the status log — the visible rows, by event.
const PAGE: u16 = 6;

/// The LOCATION screen's keys (§16.6 item 6): `←→` switch mode (the
/// AUTO proposal computed on entering AUTO), `Tab` switches mode in
/// SEARCH and AUTO but toggles LAT/LNG focus in COORDINATES, SEARCH
/// takes printables and `↑↓`/Enter over its results, COORDINATES
/// takes digits/`.`/`-` and Enter, AUTO takes Enter. Esc falls
/// through to the global pop — back to SETTINGS with the draft
/// intact (§8.5).
fn location_keys(key: KeyEvent, shell: &mut Shell, runtime: &Runtime) -> Option<Outcome> {
    // Printables are consumed by the text modes — `q` and the tab
    // digits must not fire while the user is typing a query. Ctrl was
    // step 1; Alt-modified chars are not text either.
    let printable = matches!(key.code, KeyCode::Char(_))
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

    match (shell.location.mode, key.code) {
        // ←→ and (outside COORDINATES) Tab cycle the modes.
        (_, KeyCode::Left | KeyCode::Right) => {
            let forward = key.code == KeyCode::Right;
            shell.location.mode = match shell.location.mode {
                LocationMode::Search if forward => LocationMode::Coords,
                LocationMode::Search => LocationMode::Auto,
                LocationMode::Coords if forward => LocationMode::Auto,
                LocationMode::Coords => LocationMode::Search,
                LocationMode::Auto if forward => LocationMode::Search,
                LocationMode::Auto => LocationMode::Coords,
            };
            enter_auto(shell);
            Some(Outcome::Changed)
        }
        // In COORDINATES, Tab (and BackTab) toggles the field.
        (LocationMode::Coords, KeyCode::Tab | KeyCode::BackTab) => {
            shell.location.focus = match shell.location.focus {
                CoordFocus::Lat => CoordFocus::Lng,
                CoordFocus::Lng => CoordFocus::Lat,
            };
            Some(Outcome::Changed)
        }
        (_, KeyCode::Tab) => {
            cycle_mode(shell, true);
            Some(Outcome::Changed)
        }
        (_, KeyCode::BackTab) => {
            cycle_mode(shell, false);
            Some(Outcome::Changed)
        }

        // SEARCH: type, delete, move, commit.
        (LocationMode::Search, KeyCode::Backspace) => {
            shell.location.query.pop();
            research(shell);
            Some(Outcome::Changed)
        }
        (LocationMode::Search, KeyCode::Up) => {
            shell.location.sel = shell.location.sel.saturating_sub(1);
            Some(Outcome::Changed)
        }
        (LocationMode::Search, KeyCode::Down) => {
            let last = shell.location.results.len().saturating_sub(1);
            shell.location.sel = (shell.location.sel + 1).min(last);
            Some(Outcome::Changed)
        }
        (LocationMode::Search, KeyCode::Enter) => {
            let city = shell.location.results.get(shell.location.sel).cloned()?;
            commit_location(shell, runtime, city_to_config(&city, LocationSource::City));
            Some(Outcome::Changed)
        }

        // COORDINATES: digits, dot, minus; Enter validates and
        // commits — the inline error stays the render's derivation.
        (LocationMode::Coords, KeyCode::Backspace) => {
            coord_buffer(shell).pop();
            Some(Outcome::Changed)
        }
        (LocationMode::Coords, KeyCode::Enter) => {
            let (lat, lng) = parse_coords(&shell.location.lat_buf, &shell.location.lng_buf)?;
            commit_location(shell, runtime, coords_to_config(lat, lng));
            Some(Outcome::Changed)
        }

        // AUTO: Enter accepts the proposal, if there is one.
        (LocationMode::Auto, KeyCode::Enter) => {
            let city = shell.location.auto.clone().and_then(Result::ok)?;
            commit_location(shell, runtime, city_to_config(&city, LocationSource::Auto));
            Some(Outcome::Changed)
        }
        _ => None,
    }
    .or_else(|| {
        // The text-entry arms: after the structural keys, a printable
        // goes into whichever buffer the mode is editing.
        if !printable {
            return None;
        }
        let KeyCode::Char(ch) = key.code else {
            return None;
        };
        // A local copy: matching on `shell.location.mode` directly would
        // hold the borrow through the arms' guards.
        let mode = shell.location.mode;
        match mode {
            LocationMode::Search if shell.location.query.chars().count() < 40 => {
                shell.location.query.push(ch);
                research(shell);
                Some(Outcome::Changed)
            }
            LocationMode::Coords
                if (ch.is_ascii_digit() || ch == '.' || ch == '-')
                    && coord_buffer(shell).chars().count() < 12 =>
            {
                coord_buffer(shell).push(ch);
                Some(Outcome::Changed)
            }
            _ => None,
        }
    })
}

/// Cycle the location mode forward or back, computing AUTO's proposal
/// on entering it.
fn cycle_mode(shell: &mut Shell, forward: bool) {
    shell.location.mode = match shell.location.mode {
        LocationMode::Search if forward => LocationMode::Coords,
        LocationMode::Search => LocationMode::Auto,
        LocationMode::Coords if forward => LocationMode::Auto,
        LocationMode::Coords => LocationMode::Search,
        LocationMode::Auto if forward => LocationMode::Search,
        LocationMode::Auto => LocationMode::Coords,
    };
    enter_auto(shell);
}

/// Entering AUTO computes its proposal once (§9.3): the system zone's
/// largest city, or E103's inline wording when the chain fails.
fn enter_auto(shell: &mut Shell) {
    if shell.location.mode == LocationMode::Auto && shell.location.auto.is_none() {
        shell.location.auto = Some(auto_proposal());
    }
}

/// AUTO-DETECT's proposal — the §9.3 chain and the zone's largest
/// city, with the screen's inline error wording as the `Err`.
fn auto_proposal() -> Result<CityMatch, String> {
    let tz = system_timezone().map_err(|_| "timezone could not be detected".to_owned())?;
    let index = cities::load().map_err(|_| "city index unavailable".to_owned())?;
    index
        .largest_in_zone(tz.name())
        .ok_or_else(|| "no city in the detected zone".to_owned())
}

/// Re-run the search for the current query (§9.2's top three), the
/// selection back to the first result.
fn research(shell: &mut Shell) {
    let results = cities::load()
        .map(|index| index.search(&shell.location.query))
        .unwrap_or_default();
    shell.location.results = results;
    shell.location.sel = 0;
}

/// The coordinate buffer keyboard entry targets.
fn coord_buffer(shell: &mut Shell) -> &mut String {
    match shell.location.focus {
        CoordFocus::Lat => &mut shell.location.lat_buf,
        CoordFocus::Lng => &mut shell.location.lng_buf,
    }
}

/// Parse and range-check the two buffers (§9.3's E101) — `None` when
/// either fails, in which case Enter is inert and the inline error
/// the render derives says why.
fn parse_coords(lat_buf: &str, lng_buf: &str) -> Option<(f64, f64)> {
    let lat = lat_buf.parse::<f64>().ok()?;
    let lng = lng_buf.parse::<f64>().ok()?;
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng) {
        Some((lat, lng))
    } else {
        None
    }
}

/// A chosen city as a `LocationConfig` (§16.6 item 4's six fields).
fn city_to_config(city: &CityMatch, source: LocationSource) -> LocationConfig {
    LocationConfig {
        label: city.name.clone(),
        country: city.country.clone(),
        latitude: city.lat,
        longitude: city.lng,
        timezone: city.tz.clone(),
        source,
    }
}

/// A manual coordinate as a `LocationConfig`, with §9.3's zone
/// policy: the nearest city's zone within 200 km, the system zone
/// beyond — a mid-ocean point still needs a zone, and the machine's
/// own is the honest guess. The label is the coordinate itself; the
/// nearest city stays a hint, never a silent substitution.
fn coords_to_config(lat: f64, lng: f64) -> LocationConfig {
    let zone = match cities::load().ok().and_then(|index| {
        index
            .nearest(lat, lng)
            .filter(|&(_, km)| km <= 200.0)
            .map(|(city, _)| city.tz.clone())
    }) {
        Some(zone) => zone,
        None => system_timezone()
            .map(|tz| tz.name().to_owned())
            .unwrap_or_else(|_| "UTC".to_owned()),
    };
    LocationConfig {
        label: format!("{lat:.2}, {lng:.2}"),
        country: String::new(),
        latitude: lat,
        longitude: lng,
        timezone: zone,
        source: LocationSource::Coordinates,
    }
}

/// Commit a chosen location into the settings draft (§8.5, §16.6):
/// the draft opens from the live config if none is in flight, the
/// whole `location` block is replaced, and the screen pops back to
/// SETTINGS. The prayer-time refetch is the save flow's business —
/// §8.5 keeps a draft local until save, so §16.6 item 8's
/// "committing triggers a refetch" lands at `s`, not at Enter.
fn commit_location(shell: &mut Shell, runtime: &Runtime, location: LocationConfig) {
    let config = runtime.config();
    shell.open_draft(&config);
    if let Some(draft) = shell.draft.as_mut() {
        draft.working().location = location;
    }
    shell.pop();
}

/// The SETTINGS screen's keys (§16.2 item 7): `↑↓` move the flat
/// cursor across all four panels, `←→` cycle the focused row's value
/// in place — the method table, the time format, the booleans, the
/// volume by ±5 — and `Enter` opens the row's sub-screen or drives
/// the test playback. Esc with a draft in flight discards it and
/// leaves (the CANCEL button); without one it falls through to the
/// global pop. The `s` save key is the save flow's, not this
/// handler's — it falls through until the CONFIRM screen exists.
async fn settings_keys(key: KeyEvent, shell: &mut Shell, runtime: &Runtime) -> Option<Outcome> {
    let row = settings::Row::at(shell.settings.row)?;
    match key.code {
        KeyCode::Up => {
            shell.settings.row = shell.settings.row.saturating_sub(1);
            Some(Outcome::Changed)
        }
        KeyCode::Down => {
            shell.settings.row = (shell.settings.row + 1).min(settings::Row::ALL.len() - 1);
            Some(Outcome::Changed)
        }
        KeyCode::Left | KeyCode::Right => {
            let forward = key.code == KeyCode::Right;
            // Only rows with an enumerable value cycle. The preset is
            // a free-form file name, the theme a single variant, the
            // language one supported code — §16.2 item 7's "presets"
            // has no set to walk on this row, and an inert arrow must
            // not open an editor session either.
            if !matches!(
                row,
                settings::Row::Method
                    | settings::Row::TimeFormat
                    | settings::Row::EnableNotifications
                    | settings::Row::MosqueReminder
                    | settings::Row::EnableAdhan
                    | settings::Row::Volume
                    | settings::Row::CompactMode
                    | settings::Row::ShowSeconds
                    | settings::Row::CountdownProgress
            ) {
                return None;
            }
            edit_draft(shell, runtime, |config| match row {
                settings::Row::Method => {
                    if let Some(next) = settings::cycle_method(config.calculation.method, forward) {
                        config.calculation.method = next;
                    }
                }
                settings::Row::TimeFormat => {
                    config.display.time_format = match config.display.time_format {
                        TimeFormat::TwentyFourHour => TimeFormat::TwelveHour,
                        TimeFormat::TwelveHour => TimeFormat::TwentyFourHour,
                    };
                }
                settings::Row::EnableNotifications => {
                    config.notifications.enabled = !config.notifications.enabled;
                }
                settings::Row::MosqueReminder => {
                    config.notifications.mosque_reminder = !config.notifications.mosque_reminder;
                }
                settings::Row::EnableAdhan => {
                    config.adhan.enabled = !config.adhan.enabled;
                }
                settings::Row::Volume => {
                    // §16.2 item 7's ±5, clamped to the field's range.
                    let delta = if forward { 5 } else { -5 };
                    let volume = config.adhan.volume as i16 + delta;
                    config.adhan.volume = volume.clamp(0, 100) as u8;
                }
                settings::Row::CompactMode => {
                    config.display.compact_mode = !config.display.compact_mode;
                }
                settings::Row::ShowSeconds => {
                    config.display.show_seconds = !config.display.show_seconds;
                }
                settings::Row::CountdownProgress => {
                    config.display.countdown_progress = !config.display.countdown_progress;
                }
                _ => {}
            });
            Some(Outcome::Changed)
        }
        KeyCode::Enter => match row {
            settings::Row::Location => {
                shell.push(Screen::Location);
                Some(Outcome::Changed)
            }
            settings::Row::EnableNotifications | settings::Row::PerPrayerTiming => {
                shell.push(Screen::NotifSettings);
                Some(Outcome::Changed)
            }
            settings::Row::TestAdhan => test_adhan(shell, runtime).await,
            _ => None,
        },
        // The CANCEL button: leave, discarding the draft (§8.5 — the
        // store never saw it). Without a draft there is nothing to
        // discard, so the global pop runs instead.
        KeyCode::Esc => {
            if shell.draft.take().is_some() {
                shell.pop();
                Some(Outcome::Changed)
            } else {
                None
            }
        }
        // The SAVE button (§8.5, §16.2 item 7).
        KeyCode::Char('s') => save_draft(shell, runtime).await,
        _ => None,
    }
}

/// The NOTIF SETTINGS screen's keys (§16.5 item 7): `↑↓` move the row
/// (the design's `Math.min(v+1, len-1)` clamp, no wrap), `Tab`/
/// `Shift-Tab` cycle the column, `←→` adjust the focused minutes by
/// ±1 clamped `0..=60` (the design's exact bounds), `Enter`/`Space`
/// toggle the ADHAN/NOTIF booleans. Esc falls through to the global
/// pop — back to SETTINGS with the draft intact (§8.5, the same rule
/// LOCATION uses), because the `s` save flow owns the discard.
async fn notif_keys(key: KeyEvent, shell: &mut Shell, runtime: &Runtime) -> Option<Outcome> {
    let prayers = Notifiable::ALL;
    let row = shell.notif.row;
    match key.code {
        KeyCode::Up => {
            shell.notif.row = shell.notif.row.saturating_sub(1);
            Some(Outcome::Changed)
        }
        KeyCode::Down => {
            shell.notif.row = (shell.notif.row + 1).min(prayers.len() - 1);
            Some(Outcome::Changed)
        }
        KeyCode::Tab => {
            shell.notif.col = notif_settings::cycle_col(shell.notif.col, true);
            Some(Outcome::Changed)
        }
        KeyCode::BackTab => {
            shell.notif.col = notif_settings::cycle_col(shell.notif.col, false);
            Some(Outcome::Changed)
        }
        KeyCode::Left | KeyCode::Right => {
            // The design's exact clamp: `Math.max(0, Math.min(60,
            // value + d))`.
            let delta: i16 = if key.code == KeyCode::Right { 1 } else { -1 };
            let which = match shell.notif.col {
                NotifCol::Before => 0,
                NotifCol::After => 1,
                NotifCol::Adhan | NotifCol::Notif => return None,
            };
            edit_draft(shell, runtime, |config| {
                let prayer = prayers[row];
                let settings = prayer.settings_mut(&mut config.prayers);
                let minutes = match which {
                    0 => &mut settings.before_minutes,
                    _ => &mut settings.after_minutes,
                };
                let stepped = *minutes as i16 + delta;
                *minutes = stepped.clamp(0, 60) as u8;
            });
            Some(Outcome::Changed)
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            // The toggle the focused column owns — read before the
            // edit: the closure borrows `shell` mutably.
            let toggle_adhan = match shell.notif.col {
                NotifCol::Adhan => true,
                NotifCol::Notif => false,
                NotifCol::Before | NotifCol::After => return None,
            };
            edit_draft(shell, runtime, |config| {
                let prayer = prayers[row];
                let settings = prayer.settings_mut(&mut config.prayers);
                if toggle_adhan {
                    settings.adhan = !settings.adhan;
                } else {
                    settings.notify = !settings.notify;
                }
            });
            Some(Outcome::Changed)
        }
        // The SAVE button (§16.5 item 7: `s` save → `confirm`).
        KeyCode::Char('s') => save_draft(shell, runtime).await,
        _ => None,
    }
}

/// The PRAYER CONFIG screen's keys (§16.3 item 7, the resolved
/// conflict): `↑↓` walk the cursor — row 0 is the tab bar's own focus,
/// rows 1–5 the panel — · `←→` switch the prayer tab at row 0 and
/// adjust the focused stepper by the design's ±5 (clamped `0..=60`)
/// below it · `Tab`/`Shift-Tab` switch the tab from any row ·
/// `Enter`/`Space` toggle the ADHAN/NOTIF rows · the preset row is
/// display-only (its value is owned by SETTINGS → Sound preset).
/// Esc falls through to the global pop — the draft stays open (§8.5,
/// the rule every settings sub-screen shares), the `s` save flow owns
/// the discard.
async fn prayer_config_keys(
    key: KeyEvent,
    shell: &mut Shell,
    runtime: &Runtime,
) -> Option<Outcome> {
    let rows = prayer_config::Row::ALL.len();
    let tab = shell.prayer_cfg.tab;
    let row = shell.prayer_cfg.row;
    let focused = prayer_config::Row::at(row)?;
    match key.code {
        KeyCode::Up => {
            shell.prayer_cfg.row = shell.prayer_cfg.row.saturating_sub(1);
            Some(Outcome::Changed)
        }
        KeyCode::Down => {
            shell.prayer_cfg.row = (shell.prayer_cfg.row + 1).min(rows - 1);
            Some(Outcome::Changed)
        }
        // The escape hatch: tabs switch from any row (§16.3 item 7).
        KeyCode::Tab => {
            shell.prayer_cfg.tab = (tab + 1) % Notifiable::ALL.len();
            Some(Outcome::Changed)
        }
        KeyCode::BackTab => {
            shell.prayer_cfg.tab = (tab + Notifiable::ALL.len() - 1) % Notifiable::ALL.len();
            Some(Outcome::Changed)
        }
        KeyCode::Left | KeyCode::Right => {
            let forward = key.code == KeyCode::Right;
            if focused == prayer_config::Row::Tab {
                // Row 0: `←→` is the tab switch (§16.3 item 7's
                // dedicated first row).
                if forward {
                    shell.prayer_cfg.tab = (tab + 1) % Notifiable::ALL.len();
                } else {
                    shell.prayer_cfg.tab =
                        (tab + Notifiable::ALL.len() - 1) % Notifiable::ALL.len();
                }
                Some(Outcome::Changed)
            } else if matches!(
                focused,
                prayer_config::Row::Before | prayer_config::Row::After
            ) {
                // The design's `◂ 5` / `5 ▸` buttons, clamped 0..=60.
                let delta: i16 = if forward { 5 } else { -5 };
                let prayer = Notifiable::ALL.get(tab).copied()?;
                let which = focused == prayer_config::Row::Before;
                edit_draft(shell, runtime, |config| {
                    let settings = prayer.settings_mut(&mut config.prayers);
                    let minutes = if which {
                        &mut settings.before_minutes
                    } else {
                        &mut settings.after_minutes
                    };
                    let stepped = *minutes as i16 + delta;
                    *minutes = stepped.clamp(0, 60) as u8;
                });
                Some(Outcome::Changed)
            } else {
                // The toggle rows take Enter; the preset has no set to
                // walk (§16.2 item 7) — an inert arrow opens no editor.
                None
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let prayer = Notifiable::ALL.get(tab).copied()?;
            match focused {
                prayer_config::Row::Adhan | prayer_config::Row::Notif => {
                    let toggle_adhan = focused == prayer_config::Row::Adhan;
                    edit_draft(shell, runtime, |config| {
                        let settings = prayer.settings_mut(&mut config.prayers);
                        if toggle_adhan {
                            settings.adhan = !settings.adhan;
                        } else {
                            settings.notify = !settings.notify;
                        }
                    });
                    Some(Outcome::Changed)
                }
                _ => None,
            }
        }
        // The SAVE button (§16.3's footer, the same `[S] SAVE` the
        // other settings screens carry).
        KeyCode::Char('s') => save_draft(shell, runtime).await,
        _ => None,
    }
}

/// Open the draft on the first edit and apply one edit to the working
/// copy (§8.5) — the shared shape of every in-place value change,
/// the same open-then-mutate `commit_location` uses.
fn edit_draft(shell: &mut Shell, runtime: &Runtime, edit: impl FnOnce(&mut Config)) {
    let config = runtime.config();
    shell.open_draft(&config);
    if let Some(draft) = shell.draft.as_mut() {
        edit(draft.working());
    }
}

/// The `[S] SAVE` button (§8.5, §16.9): commit the draft through
/// `Runtime::save_draft` — validate, write atomically, publish — and
/// push the CONFIRM screen carrying the real path and the honest
/// verdict. The publication is the propagation: the event loop's
/// `cfg_rx` arm applies the new configuration, the scheduler re-arms,
/// no restart. A save failure shows `E202` and keeps the draft so the
/// user does not lose their edits (§8.5's exact rule).
///
/// A save with no draft in flight — the footer's button pressed
/// without an edit — is a no-op that still confirms: the store never
/// saw a change, so `recalculated` is false and the screen says
/// `Settings applied.`, which is also true.
async fn save_draft(shell: &mut Shell, runtime: &Runtime) -> Option<Outcome> {
    match shell.draft.as_ref() {
        Some(draft) => match runtime.save_draft(draft) {
            Ok(outcome) => {
                // §8.5: the draft is consumed — its values are now the
                // published configuration, and the screens below render
                // from the live config.
                shell.draft = None;
                shell.push(Screen::Confirm {
                    path: runtime.config_path().display().to_string(),
                    recalculated: outcome.recalculated,
                });
                Some(Outcome::Changed)
            }
            Err(error) => {
                // E202 — the draft stays open (§8.5).
                shell.push(Screen::Error(error));
                Some(Outcome::Changed)
            }
        },
        None => {
            shell.push(Screen::Confirm {
                path: runtime.config_path().display().to_string(),
                recalculated: false,
            });
            Some(Outcome::Changed)
        }
    }
}

/// The Test adhan row (§16.2 item 7): playing toggles to stopping,
/// and stopped resolves the *pending* preset — the draft's, if the
/// user just changed it — plays it at the pending volume, and logs
/// rather than errors when there is nothing playable (the row's own
/// rendering says why it would be).
async fn test_adhan(shell: &Shell, runtime: &Runtime) -> Option<Outcome> {
    use crate::audio::AudioPlayer as _;

    let audio = runtime.audio();
    if audio.is_playing() {
        stop_audio(runtime).await;
        return Some(Outcome::Changed);
    }

    let adhan = match shell.draft.as_ref() {
        Some(draft) => draft.working_ref().adhan.clone(),
        None => runtime.config().adhan.clone(),
    };
    let data_dir = crate::audio::file::system_data_dir().ok()?;
    let file = crate::audio::file::resolve_path(&adhan, &data_dir);
    if crate::audio::file::validate(&file).is_err() {
        tracing::warn!(target: "salat::ui", file = %file.display(), "test adhan: no playable file");
        return Some(Outcome::Ignored);
    }
    if let Err(error) = audio.play(&file, adhan.volume).await {
        tracing::warn!(target: "salat::ui", ?error, "test adhan playback failed");
    }
    Some(Outcome::Changed)
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

    /// §16.6 / §9.2: typing "Algiers" searches the embedded index;
    /// Enter commits it into the draft (§8.5) and pops back to
    /// SETTINGS — the live config (the fixture's Sétif) untouched
    /// until save.
    #[test]
    fn search_finds_algiers_and_enter_commits_the_draft() {
        let fixture = testing::snapshot_runtime();
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        shell.push(Screen::Location);
        for ch in "Algiers".chars() {
            let outcome = fixture.runtime.block_on(async {
                super::resolve(press(KeyCode::Char(ch)), &mut shell, &fixture.runtime).await
            });
            assert_eq!(outcome, Outcome::Changed);
        }
        assert!(
            shell
                .location
                .results
                .first()
                .is_some_and(|m| m.name == "Algiers"),
            "the index found Algiers"
        );
        assert!(shell.draft.is_none(), "typing alone opens no draft");

        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Enter), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Settings, "Enter pops to SETTINGS");
        let draft = shell.draft.as_ref().expect("the commit opened the draft");
        assert_eq!(draft.working_ref().location.label, "Algiers");
        assert_eq!(draft.working_ref().location.country, "Algeria");
        assert_eq!(draft.working_ref().location.timezone, "Africa/Algiers");
        assert_eq!(
            draft.working_ref().location.source,
            super::LocationSource::City
        );
        // §8.5: draft-local-until-save — the runtime's config is still
        // the fixture's Sétif, not Algiers.
        assert_eq!(fixture.runtime.config().location.label, "Sétif, Algeria");
    }

    /// §16.9 item 3: `s` on SETTINGS saves the draft, consumes it, and
    /// pushes the CONFIRM screen with the real path and the honest
    /// verdict — a location change did move the API cache key. Enter
    /// pops back to the screen beneath (item 7).
    #[test]
    fn s_saves_the_draft_and_confirms() {
        let fixture = testing::snapshot_runtime();
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        let mut draft = crate::config::ConfigDraft::from(fixture.runtime.config().as_ref());
        draft.working().location.label = "Mecca".to_owned();
        draft.working().location.latitude = 21.42;
        draft.working().location.longitude = 39.83;
        shell.draft = Some(draft);

        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Char('s')), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Changed);
        assert!(shell.draft.is_none(), "the save consumed the draft");
        match shell.screen() {
            Screen::Confirm { recalculated, .. } => assert!(recalculated),
            other => panic!("CONFIRM was pushed, got {other:?}"),
        }
        // The write landed: the file on disk holds the draft.
        let written = crate::config::ConfigStore::new(fixture.config_path())
            .load()
            .expect("the save wrote a parseable file");
        assert_eq!(written.location.label, "Mecca");

        // Item 7: Enter pops back to the screen beneath.
        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Enter), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Settings);
    }

    /// `s` with nothing pending still confirms — `Settings applied.`,
    /// not a recalculation that never happened (§16.9 item 6).
    #[test]
    fn s_with_no_draft_confirms_without_recalculating() {
        let fixture = testing::snapshot_runtime();
        let mut shell = Shell::new();
        shell.push(Screen::Settings);

        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Char('s')), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Changed);
        match shell.screen() {
            Screen::Confirm { recalculated, .. } => assert!(!recalculated),
            other => panic!("CONFIRM was pushed, got {other:?}"),
        }
    }

    /// §16.9 item 7's other half: Esc pops too, and every other key —
    /// including the global vocabulary — is swallowed by CONFIRM.
    #[test]
    fn confirm_only_answers_enter_and_esc() {
        let fixture = testing::snapshot_runtime();
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        shell.push(Screen::Confirm {
            path: "/x/config.toml".to_owned(),
            recalculated: false,
        });
        for key in [KeyCode::Char('q'), KeyCode::Char('1'), KeyCode::Char('?')] {
            let outcome = fixture
                .runtime
                .block_on(async { super::resolve(press(key), &mut shell, &fixture.runtime).await });
            assert_eq!(outcome, Outcome::Ignored, "{key:?} is ignored");
        }
        assert_eq!(
            *shell.screen(),
            Screen::Confirm {
                path: "/x/config.toml".to_owned(),
                recalculated: false
            },
            "the dialogue held"
        );
        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Esc), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Settings);
    }

    /// §8.5's failure row through the keyboard: a save that cannot
    /// write (the atomic-write temp path is blocked by a directory)
    /// pushes the ERROR screen with E202 and *keeps the draft* — the
    /// edit is not lost to a full disk.
    #[test]
    fn a_failed_save_shows_e202_and_keeps_the_draft() {
        let fixture = testing::snapshot_runtime();
        // `<config>.toml.tmp` as a directory: the atomic write cannot
        // create its temp file there — a genuine Write failure.
        let tmp = fixture.config_path().with_extension("toml.tmp");
        std::fs::create_dir_all(&tmp).expect("the blocking directory is creatable");
        let before = std::fs::read_to_string(fixture.config_path())
            .expect("the fixture wrote a config file");

        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        let mut draft = crate::config::ConfigDraft::from(fixture.runtime.config().as_ref());
        draft.working().location.label = "Mecca".to_owned();
        shell.draft = Some(draft);

        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Char('s')), &mut shell, &fixture.runtime).await
        });
        // The block is gone before any assert can fail and leak it.
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(outcome, Outcome::Changed);
        match shell.screen() {
            Screen::Error(error) => assert_eq!(error.code(), "E202"),
            other => panic!("ERROR was pushed, got {other:?}"),
        }
        assert!(
            shell.draft.as_ref().is_some_and(|d| d.is_dirty()),
            "§8.5: a failed save keeps the draft open"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.config_path()).expect("the file still reads"),
            before,
            "the atomic write never touched the old file"
        );
    }

    /// §16.6 item 6: `←→` cycle the modes (computing AUTO's proposal
    /// on entry), and in COORDINATES `Tab` toggles the LAT/LNG focus
    /// instead of cycling.
    #[test]
    fn arrows_and_tab_manage_the_modes_and_focus() {
        let mut shell = Shell::new();
        shell.push(Screen::Location);
        assert_eq!(resolve(press(KeyCode::Right), &mut shell), Outcome::Changed);
        assert_eq!(shell.location.mode, super::LocationMode::Coords);
        assert_eq!(resolve(press(KeyCode::Tab), &mut shell), Outcome::Changed);
        assert_eq!(shell.location.focus, super::CoordFocus::Lng, "Tab toggles");
        assert_eq!(
            shell.location.mode,
            super::LocationMode::Coords,
            "Tab in COORDINATES does not cycle"
        );
        assert_eq!(resolve(press(KeyCode::Left), &mut shell), Outcome::Changed);
        assert_eq!(shell.location.mode, super::LocationMode::Search);
        assert_eq!(resolve(press(KeyCode::Left), &mut shell), Outcome::Changed);
        assert_eq!(shell.location.mode, super::LocationMode::Auto);
        // The proposal was computed — this machine's chain resolves
        // (§9.3's own test-machine rule).
        assert!(shell.location.auto.is_some());
    }

    /// §9.3's E101 path: Enter with an out-of-range coordinate is
    /// inert — no commit, no pop, no draft — and a valid coordinate
    /// commits with the nearest city's zone.
    #[test]
    fn coords_enter_rejects_out_of_range_and_commits_valid() {
        let fixture = testing::snapshot_runtime();
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        shell.push(Screen::Location);
        resolve(press(KeyCode::Right), &mut shell); // SEARCH → COORDS
        for ch in "999".chars() {
            resolve(press(KeyCode::Char(ch)), &mut shell);
        }
        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Enter), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Ignored, "999 is out of range");
        assert_eq!(*shell.screen(), Screen::Location, "no pop");
        assert!(shell.draft.is_none(), "no draft");

        // Fix the latitude, enter the longitude, commit.
        for _ in 0..3 {
            resolve(press(KeyCode::Backspace), &mut shell);
        }
        for ch in "36.19".chars() {
            resolve(press(KeyCode::Char(ch)), &mut shell);
        }
        resolve(press(KeyCode::Tab), &mut shell); // → LNG
        for ch in "5.41".chars() {
            resolve(press(KeyCode::Char(ch)), &mut shell);
        }
        let outcome = fixture.runtime.block_on(async {
            super::resolve(press(KeyCode::Enter), &mut shell, &fixture.runtime).await
        });
        assert_eq!(outcome, Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Settings);
        let draft = shell.draft.as_ref().expect("the commit opened the draft");
        assert_eq!(draft.working_ref().location.label, "36.19, 5.41");
        assert_eq!(draft.working_ref().location.timezone, "Africa/Algiers");
        assert_eq!(
            draft.working_ref().location.source,
            super::LocationSource::Coordinates
        );
    }

    /// §9.3's zone policy: within 200 km the nearest city's zone,
    /// beyond it the system zone's (which parses — this machine's
    /// chain resolves).
    #[test]
    fn coords_zone_derivation_follows_the_200km_policy() {
        let near = super::coords_to_config(36.19, 5.41);
        assert_eq!(near.timezone, "Africa/Algiers");
        assert_eq!(near.source, super::LocationSource::Coordinates);

        let far = super::coords_to_config(0.0, -140.0);
        assert!(
            far.timezone.parse::<chrono_tz::Tz>().is_ok(),
            "the fallback zone is a real zone: {}",
            far.timezone
        );
    }

    /// Typing on the LOCATION screen swallows the global vocabulary —
    /// `q` must not quit and `2` must not switch tabs mid-query.
    #[test]
    fn typing_swallows_the_global_keys() {
        let mut shell = Shell::new();
        shell.push(Screen::Location);
        for key in [KeyCode::Char('q'), KeyCode::Char('2'), KeyCode::Char('?')] {
            assert_eq!(resolve(press(key), &mut shell), Outcome::Changed);
        }
        assert_eq!(shell.location.query, "q2?");
        assert_eq!(*shell.screen(), Screen::Location);
    }
    /// §16.2 item 7: `↑↓` move the flat cursor and clamp at both
    /// ends; the row identity the handler resolves matches the
    /// render's `Row::at`.
    #[test]
    fn settings_arrows_move_and_clamp() {
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        assert_eq!(resolve(press(KeyCode::Down), &mut shell), Outcome::Changed);
        assert_eq!(shell.settings.row, 1);
        assert_eq!(resolve(press(KeyCode::Up), &mut shell), Outcome::Changed);
        assert_eq!(shell.settings.row, 0, "clamps at the first row");
        assert_eq!(resolve(press(KeyCode::Up), &mut shell), Outcome::Changed);
        assert_eq!(shell.settings.row, 0, "saturating, not wrap");
        for _ in 0..20 {
            resolve(press(KeyCode::Down), &mut shell);
        }
        assert_eq!(shell.settings.row, settings::Row::ALL.len() - 1);
    }

    /// `←→` cycle the focused value in place through the draft — the
    /// boolean rows flip, the volume steps ±5 and clamps, the method
    /// walks the table — and the draft opens from the live config on
    /// the first one.
    #[test]
    fn settings_left_right_edit_the_draft() {
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        // Row 12: Compact mode (ON by default in the fixture).
        shell.settings.row = settings::Row::ALL
            .iter()
            .position(|row| *row == settings::Row::CompactMode)
            .unwrap();
        assert_eq!(resolve(press(KeyCode::Right), &mut shell), Outcome::Changed);
        let draft = shell
            .draft
            .as_ref()
            .expect("the first edit opens the draft");
        let live = testing::snapshot_runtime();
        assert_ne!(
            draft.working_ref().display.compact_mode,
            live.runtime.config().display.compact_mode,
            "the toggle lands in the working copy"
        );

        // Volume: row 9, ±5 with a clamp at both ends.
        shell.settings.row = 9;
        for _ in 0..30 {
            resolve(press(KeyCode::Right), &mut shell);
        }
        assert_eq!(
            shell.draft.as_ref().unwrap().working_ref().adhan.volume,
            100
        );
        for _ in 0..30 {
            resolve(press(KeyCode::Left), &mut shell);
        }
        assert_eq!(shell.draft.as_ref().unwrap().working_ref().adhan.volume, 0);

        // Method: row 1 walks the table; the fixture's method 12 is
        // not the table's last entry, so one step changes the value.
        shell.settings.row = 1;
        let before = shell
            .draft
            .as_ref()
            .unwrap()
            .working_ref()
            .calculation
            .method;
        resolve(press(KeyCode::Right), &mut shell);
        assert_ne!(
            shell
                .draft
                .as_ref()
                .unwrap()
                .working_ref()
                .calculation
                .method,
            before
        );
    }

    /// `←→` is inert on the rows with no enumerable value (§16.2 item
    /// 7's "presets" have no set here): the preset, the theme, the
    /// language — and no draft opens for an inert press.
    #[test]
    fn settings_left_right_inert_rows_open_no_draft() {
        for row in [
            settings::Row::Location,
            settings::Row::Language,
            settings::Row::SoundPreset,
            settings::Row::Theme,
        ] {
            let mut shell = Shell::new();
            shell.push(Screen::Settings);
            shell.settings.row = settings::Row::ALL.iter().position(|r| *r == row).unwrap();
            assert_eq!(
                resolve(press(KeyCode::Right), &mut shell),
                Outcome::Ignored,
                "{row:?}"
            );
            assert!(shell.draft.is_none(), "inert press, no editor session");
        }
    }

    /// `Enter` opens the three sub-screen rows; the value rows stay
    /// inert.
    #[test]
    fn settings_enter_opens_the_sub_screens() {
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        shell.settings.row = 0;
        assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::Location);

        shell.pop();
        shell.settings.row = 4; // Enable notifications
        assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::NotifSettings);

        shell.pop();
        shell.settings.row = 5; // Per-prayer timing
        assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Changed);
        assert_eq!(*shell.screen(), Screen::NotifSettings);

        shell.pop();
        shell.settings.row = 2; // Time format — a value row
        assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Ignored);
        assert_eq!(*shell.screen(), Screen::Settings);
    }

    /// Esc on SETTINGS with a draft discards it and leaves; without a
    /// draft it is the global pop (§16.2 item 7's CANCEL, §8.5's
    /// "the store never saw it").
    #[test]
    fn settings_esc_discards_the_draft() {
        let mut shell = Shell::new();
        shell.push(Screen::Settings);
        shell.settings.row = 12; // Compact mode
        resolve(press(KeyCode::Right), &mut shell);
        assert!(shell.draft.is_some());

        assert_eq!(resolve(press(KeyCode::Esc), &mut shell), Outcome::Changed);
        assert!(shell.draft.is_none(), "CANCEL discards");
        assert_eq!(*shell.screen(), Screen::Dashboard, "CANCEL leaves");
    }

    /// §16.3 item 7's resolved conflict, in the input: row 0's `←→`
    /// switches the tab, deeper rows' `←→` steps the focused stepper
    /// ±5 clamped `0..=60`, and `Tab` switches the tab from any row.
    #[test]
    fn prayer_config_left_right_is_tab_then_adjust() {
        let mut shell = Shell::new();
        shell.push(Screen::PrayerConfig);
        assert_eq!(shell.prayer_cfg.tab, 3, "opens on Maghrib");

        // Row 0: the tab bar's focus — `→` moves to Isha.
        assert_eq!(resolve(press(KeyCode::Right), &mut shell), Outcome::Changed);
        assert_eq!(shell.prayer_cfg.tab, 4);
        assert_eq!(resolve(press(KeyCode::Left), &mut shell), Outcome::Changed);
        assert_eq!(shell.prayer_cfg.tab, 3, "wraps back");
        assert!(shell.draft.is_none(), "a tab switch edits nothing");

        // Row 1 (Before): ±5 on the draft, clamped both ends.
        shell.prayer_cfg.row = 1;
        let live = testing::snapshot_runtime();
        let base = Notifiable::Maghrib
            .settings(&live.runtime.config().prayers)
            .before_minutes;
        assert_eq!(resolve(press(KeyCode::Right), &mut shell), Outcome::Changed);
        let draft = shell
            .draft
            .as_ref()
            .expect("the first edit opens the draft");
        assert_eq!(
            Notifiable::Maghrib
                .settings(&draft.working_ref().prayers)
                .before_minutes,
            (base + 5).min(60)
        );
        // The borrow ends here (NLL) — the clamp reads re-borrow.
        for _ in 0..30 {
            resolve(press(KeyCode::Left), &mut shell);
        }
        assert_eq!(
            Notifiable::Maghrib
                .settings(&shell.draft.as_ref().unwrap().working_ref().prayers)
                .before_minutes,
            0,
            "clamps at 0"
        );
        // Only Maghrib moved — the edit addresses the focused tab.
        assert_eq!(
            Notifiable::Fajr
                .settings(&shell.draft.as_ref().unwrap().working_ref().prayers)
                .before_minutes,
            Notifiable::Fajr
                .settings(&live.runtime.config().prayers)
                .before_minutes,
            "the other tabs are untouched"
        );

        // `Tab` switches from a deep row too — the escape hatch.
        assert_eq!(resolve(press(KeyCode::Tab), &mut shell), Outcome::Changed);
        assert_eq!(shell.prayer_cfg.tab, 4);
        assert_eq!(
            resolve(press(KeyCode::BackTab), &mut shell),
            Outcome::Changed
        );
        assert_eq!(shell.prayer_cfg.tab, 3);
    }

    /// `↑↓` walk the six-row cursor (tab bar first) and clamp.
    #[test]
    fn prayer_config_arrows_move_and_clamp() {
        let mut shell = Shell::new();
        shell.push(Screen::PrayerConfig);
        for step in 0..8 {
            assert_eq!(resolve(press(KeyCode::Down), &mut shell), Outcome::Changed);
            assert_eq!(
                shell.prayer_cfg.row,
                (step + 1).min(prayer_config::Row::ALL.len() - 1)
            );
        }
        for _ in 0..8 {
            resolve(press(KeyCode::Up), &mut shell);
        }
        assert_eq!(shell.prayer_cfg.row, 0, "saturates, no wrap");
    }

    /// `Enter`/`Space` toggle the ADHAN and NOTIF rows; the steppers
    /// and the display-only preset row stay inert.
    #[test]
    fn prayer_config_enter_toggles_the_rows() {
        let live = testing::snapshot_runtime();
        for (row, field) in [
            (prayer_config::Row::Adhan, 0),
            (prayer_config::Row::Notif, 1),
        ] {
            let mut shell = Shell::new();
            shell.push(Screen::PrayerConfig);
            shell.prayer_cfg.row = prayer_config::Row::ALL
                .iter()
                .position(|r| *r == row)
                .unwrap();
            let live_config = live.runtime.config();
            let live_settings = Notifiable::Maghrib.settings(&live_config.prayers);
            let (before, after) = (live_settings.adhan, live_settings.notify);
            let was = match field {
                0 => before,
                _ => after,
            };
            assert_eq!(resolve(press(KeyCode::Enter), &mut shell), Outcome::Changed);
            let draft = shell.draft.as_ref().expect("the toggle opens the draft");
            let settings = Notifiable::Maghrib.settings(&draft.working_ref().prayers);
            let now = match field {
                0 => settings.adhan,
                _ => settings.notify,
            };
            assert_ne!(now, was, "{row:?} toggled");

            // Space is the same key.
            let mut shell = Shell::new();
            shell.push(Screen::PrayerConfig);
            shell.prayer_cfg.row = prayer_config::Row::ALL
                .iter()
                .position(|r| *r == row)
                .unwrap();
            assert_eq!(
                resolve(press(KeyCode::Char(' ')), &mut shell),
                Outcome::Changed
            );
        }

        // The preset row: display-only, no key of its own.
        let mut shell = Shell::new();
        shell.push(Screen::PrayerConfig);
        shell.prayer_cfg.row = 5;
        assert_eq!(
            resolve(press(KeyCode::Enter), &mut shell),
            Outcome::Ignored,
            "the preset row takes no Enter"
        );
    }
}
