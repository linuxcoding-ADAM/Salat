//! The status domain (§6.7): the decoupled `SystemStatus` data model
//! in [`model`], and the `Collector` (§15.8) that assembles it.
//!
//! Neither status surface — the Status screen nor `salat status` —
//! may reach into any service; they consume `SystemStatus` only. The
//! [`Collector`] is the one place that may: it calls each service's
//! `report()` and folds the runtime's own cached health into the ten
//! components. Neither consumer knows that "Network" health comes from
//! the last HTTP attempt's outcome or that "Audio system" comes from a
//! PATH probe. Adding a component is: add a `ComponentId` variant,
//! implement `report()`, done.

pub mod model;

pub use model::{
    Component, ComponentId, EventCategory, Health, LoggedEvent, NextEvent, SystemStatus,
};

use crate::app::runtime::Runtime;
use crate::cache::prayer_from_key;
use crate::domain::day_summary;

/// Assemble a [`SystemStatus`] from a [`Runtime`] (§15.8): every
/// service reports, the runtime's cached health fills the components
/// whose story changes only on refresh and config events, and today's
/// event log (which the runtime caches as `recent`) rides along.
pub struct Collector;

impl Collector {
    /// §15.8's one assembly point. Synchronous by design — the render
    /// loop calls it per frame, so everything slow (the HTTP probe, the
    /// event-log read) happens elsewhere and lands in the runtime's
    /// cached slots before this runs.
    pub fn collect(runtime: &Runtime) -> SystemStatus {
        let config = runtime.config();
        let window = runtime.window();
        let now = runtime.clock().now_in(runtime.tz());

        let components = ComponentId::ALL
            .map(|id| match id {
                ComponentId::PrayerService => runtime.prayer_service_component(),
                ComponentId::PrayerCalculation => Component::new(id, Health::Ok, "OK".to_owned()),
                ComponentId::Location => {
                    Component::new(id, Health::Ok, config.location.label.to_uppercase())
                }
                ComponentId::Timezone => Component::new(id, Health::Ok, utc_offset_label(now)),
                ComponentId::Notifications => runtime.notifier_component(),
                ComponentId::AdhanSound => runtime.adhan_sound_component(),
                ComponentId::AudioSystem => runtime.audio_system_component(),
                ComponentId::Network => runtime.network_component(),
                ComponentId::ConfigFile => runtime.config_file_component(),
                ComponentId::PrayerDatabase => Component::new(id, Health::Ok, "LOADED".to_owned()),
            })
            .to_vec();

        let next_event = crate::domain::plan_events(&window, &config, now)
            .into_iter()
            .find(|event| event.kind != crate::domain::EventKind::DayRollover)
            .map(|event| NextEvent {
                kind: event.kind,
                prayer: event.prayer,
                fire_at: event.fire_at,
            });

        let mut today = day_summary(&window, &config, now);
        let recent = runtime.recent_events();
        // §16.4 item 6's log-sourced counters: the domain cannot know
        // them, but the rows that recorded the events do.
        today.notifications_sent = recent
            .iter()
            .filter(|event| event.category == EventCategory::Notification)
            .count() as u32;
        today.adhan_plays = recent
            .iter()
            .filter(|event| event.category == EventCategory::Adhan)
            .count() as u32;

        SystemStatus {
            components,
            next_event,
            today,
            recent,
        }
    }

    /// Map a cache `event_log` row (§17.2) into the status model's
    /// [`LoggedEvent`] — the one translation between the storage and
    /// the render vocabulary. A row whose category or prayer column
    /// holds an unrecognised spelling is corruption: it is dropped
    /// with a warning rather than rendered as a row that claims a
    /// category it does not have.
    pub(crate) fn logged_event(row: &crate::cache::LoggedEvent) -> Option<LoggedEvent> {
        let Some(category) = EventCategory::from_store(&row.category) else {
            tracing::warn!(
                target: "salat::status",
                "dropping event_log row with unknown category {:?}",
                row.category
            );
            return None;
        };
        Some(LoggedEvent {
            at: row.at,
            category,
            label: row.label.clone(),
            detail: row.detail.clone(),
            prayer: row.prayer.as_deref().and_then(prayer_from_key),
        })
    }
}

/// The Timezone component's detail (§16.4): the zone's current UTC
/// offset as `UTC+1`, `UTC-4` — or `UTC+5:30` where a zone runs on a
/// half hour. Pure, so the format is pinned by unit test.
fn utc_offset_label(now: chrono::DateTime<chrono_tz::Tz>) -> String {
    use chrono::Offset as _;
    let seconds = now.offset().fix().local_minus_utc();
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.unsigned_abs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}

/// `salat status`'s text rendering (§14.1, §23 Phase 12 item 4):
/// `status::render_text(&SystemStatus, colored) -> String`.
///
/// The same [`SystemStatus`] the STATUS screen renders, through the
/// plain-text formatter: the ten components with their health marks,
/// the next event, today's counters, today's event log. The `colored`
/// bool is §14.6's rule — when it is false the output carries zero
/// escape bytes, so a pipe (`salat status | cat`) sees a stable,
/// ANSI-free block. Colour, when on, is SGR-only: the health mark's
/// colour and the event log's category colours, nothing else.
pub fn render_text(status: &SystemStatus, colored: bool) -> String {
    let mut out = String::new();
    for component in &status.components {
        let (mark, color) = mark_and_colour(component.health);
        let line = format!("{mark} {} — {}", component.label, component.detail);
        if colored {
            out.push_str(&format!("\x1b[{}m{line}\x1b[0m\n", sgr_code(color)));
        } else {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push('\n');

    if let Some(next) = &status.next_event {
        let what = match next.prayer {
            Some(prayer) => format!("{} · {}", event_label(next.kind), prayer.display()),
            None => event_label(next.kind).to_owned(),
        };
        out.push_str(&format!(
            "next event: {what} at {}\n",
            next.fire_at.format("%H:%M")
        ));
    } else {
        out.push_str("next event: —\n");
    }
    out.push('\n');

    let today = &status.today;
    let next_notify = today
        .next_notify
        .map(|at| at.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".to_owned());
    for (label, value) in [
        ("total prayers", today.total.to_string()),
        ("passed", today.passed.to_string()),
        ("remaining", today.remaining.to_string()),
        ("notifications sent", today.notifications_sent.to_string()),
        ("adhan plays", today.adhan_plays.to_string()),
        ("next notify", next_notify),
    ] {
        out.push_str(&format!("{label}: {value}\n"));
    }
    out.push('\n');

    if status.recent.is_empty() {
        out.push_str("recent events: no events yet today\n");
    } else {
        out.push_str("recent events:\n");
        for event in &status.recent {
            let line = format!(
                "  {} │ {}",
                event.at.format("%H:%M"),
                match event.prayer {
                    Some(prayer) => format!("{} {}", prayer.display(), event.label),
                    None => event.label.clone(),
                }
            );
            if colored {
                let color = match event.category {
                    EventCategory::Notification => "33", // yellow
                    EventCategory::Adhan => "34",        // blue
                    EventCategory::State | EventCategory::System | EventCategory::Error => "2", // dim
                };
                out.push_str(&format!("\x1b[{color}m{line}\x1b[0m\n"));
            } else {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

/// The health mark for the text surface: the same `●`/`✕`/`○`
/// vocabulary the screen's `status_dot` renders, with the SGR colour
/// code it would carry.
fn mark_and_colour(health: crate::status::Health) -> (&'static str, &'static str) {
    match health {
        crate::status::Health::Ok => ("●", "32"),
        crate::status::Health::Degraded => ("●", "33"),
        crate::status::Health::Down => ("✕", "31"),
        crate::status::Health::Unknown => ("○", "2"),
    }
}

/// The SGR foreground code for one of [`mark_and_colour`]'s colours.
fn sgr_code(code: &str) -> &str {
    code
}

/// The event kinds' one-line names — shared with the screen's NEXT
/// EVENT panel; the text formatter prints the same vocabulary.
fn event_label(kind: crate::domain::EventKind) -> &'static str {
    use crate::domain::EventKind;
    match kind {
        EventKind::PreNotify => "Pre-Adhan notification",
        EventKind::Adhan => "Adhan",
        EventKind::MosqueReminder => "Mosque reminder",
        EventKind::DayRollover => "Day rollover",
        EventKind::AdhanSnooze => "Adhan (snoozed)",
        EventKind::ReminderSnooze => "Reminder (snoozed)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing as rt;
    use crate::domain::testing as dt;
    use chrono::TimeZone as _;

    /// The offset label's formats: whole hours, the negative sign, and
    /// the half-hour zone.
    #[test]
    fn utc_offset_label_formats_the_three_shapes() {
        let algiers = chrono::Utc
            .with_ymd_and_hms(2026, 9, 9, 12, 0, 0)
            .unwrap()
            .with_timezone(&dt::TZ);
        assert_eq!(utc_offset_label(algiers), "UTC+1");
        let new_york = chrono::Utc
            .with_ymd_and_hms(2026, 9, 9, 12, 0, 0)
            .unwrap()
            .with_timezone(&chrono_tz::America::New_York);
        assert_eq!(utc_offset_label(new_york), "UTC-4");
        let kolkata = chrono::Utc
            .with_ymd_and_hms(2026, 9, 9, 12, 0, 0)
            .unwrap()
            .with_timezone(&chrono_tz::Asia::Kolkata);
        assert_eq!(utc_offset_label(kolkata), "UTC+5:30");
    }

    /// The §15.8 assembly over the §20.3 fixture: ten components in
    /// the design's order, the next event, and — after the runtime
    /// reloads its `recent` cache — today's rows with the counters
    /// they imply.
    #[test]
    fn collect_assembles_the_snapshot_status() {
        let fixture = rt::snapshot_runtime();
        let status = Collector::collect(&fixture.runtime);

        assert_eq!(status.components.len(), 10);
        for (index, id) in ComponentId::ALL.iter().enumerate() {
            assert_eq!(status.components[index].id, *id, "order is the design's");
        }
        assert_eq!(
            status
                .component(ComponentId::Timezone)
                .map(|c| c.detail.clone()),
            Some("UTC+1".to_owned())
        );

        // Nothing has fired and nothing is logged: the counters are
        // zero and the log is empty — honestly, not as a guess.
        assert_eq!(status.today.notifications_sent, 0);
        assert_eq!(status.today.adhan_plays, 0);
        assert!(status.recent.is_empty());

        // A logged notification and a logged Adhan, then a reload:
        // both counters count, the rows map, newest first.
        let now = dt::snapshot_now();
        fixture.runtime.block_on(async {
            fixture
                .store
                .append_event(
                    now,
                    "notification",
                    "Pre-Adhan notification",
                    Some("MAGHRIB · 5 min window"),
                    Some("maghrib"),
                )
                .await
                .expect("append");
            fixture
                .store
                .append_event(now, "adhan", "Maghrib Adhan fired", None, Some("maghrib"))
                .await
                .expect("append");
        });
        fixture
            .runtime
            .block_on(fixture.runtime.reload_recent_events());
        let status = Collector::collect(&fixture.runtime);
        assert_eq!(status.today.notifications_sent, 1);
        assert_eq!(status.today.adhan_plays, 1);
        assert_eq!(status.recent.len(), 2);
        // Newest first: both rows share `at`, so insertion order breaks
        // the tie — the Adhan (appended last) leads.
        assert_eq!(status.recent[0].label, "Maghrib Adhan fired");
        assert_eq!(
            status.recent[0].prayer,
            Some(crate::domain::Notifiable::Maghrib)
        );
        assert_eq!(
            status.recent[1].detail.as_deref(),
            Some("MAGHRIB · 5 min window")
        );
    }

    /// §14.6 / §23 item 8: piped output is stable and carries zero
    /// escape bytes; the ten components, the counters and the empty
    /// placeholder all appear.
    #[test]
    fn render_text_is_stable_and_ansi_free_when_piped() {
        let fixture = rt::snapshot_runtime();
        let status = Collector::collect(&fixture.runtime);

        let plain = render_text(&status, false);
        assert!(!plain.contains('\x1b'), "no escape bytes when piped");
        for component in &status.components {
            assert!(plain.contains(component.label));
        }
        assert!(plain.contains("total prayers: 5"));
        assert!(plain.contains("recent events: no events yet today"));

        // Stability: two renders over the same status agree exactly.
        assert_eq!(plain, render_text(&status, false));
    }

    /// The coloured path is SGR-only — every escape is an SGR sequence
    /// around a whole line, never cursor movement.
    #[test]
    fn render_text_colour_is_sgr_only() {
        let fixture = rt::snapshot_runtime();
        let status = Collector::collect(&fixture.runtime);
        let colored = render_text(&status, true);
        assert!(colored.contains('\x1b'));
        for (i, _) in colored.match_indices('\x1b') {
            // A byte-slice of fixed width would land inside the
            // multibyte `○`; the remainder's prefix is boundary-safe.
            assert!(colored[i..].starts_with("\x1b["), "only SGR escapes");
        }
    }
}
