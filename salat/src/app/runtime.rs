//! `Runtime` — builds and owns every service; the ONLY wiring site
//! (§19, §15.1's separation).
//!
//! Boot follows §15.7's fatality ladder exactly: paths → configuration →
//! cache → first prayer window. A `Runtime` that exists always holds a
//! window — afterwards, nothing is fatal: §18.3's rule is that a
//! non-fatal failure (unreachable network, a failed revalidation) sets
//! `Component` health and the app keeps serving the last good window,
//! because prayer times are immutable and yesterday's answer to "when is
//! Maghrib" does not rot.
//!
//! The split §15.1 mandates: this struct owns `Arc` service handles and
//! the mutable service state; [`crate::ui::model::build_model`] folds it
//! into an immutable `UiModel` per tick; rendering is a pure function of
//! that model. Nothing under `ui` or `cli` may reach a service except
//! through this struct's accessors.
//!
//! The tokio runtime lives here too, so [`Runtime::build`] stays
//! synchronous exactly as the §23 interface list specifies: the async
//! boot steps run under `block_on` inside it, and the command layer
//! later calls [`Runtime::block_on`] for its event loops.

use std::future::Future;
use std::sync::{Arc, PoisonError, RwLock, RwLockWriteGuard};

use chrono::{DateTime, NaiveDate, Offset as _};
use chrono_tz::Tz;
use tokio::sync::{Mutex, watch};

use crate::api::HttpClient;
use crate::api::params::Params;
use crate::app::clock::{Clock, SystemClock};
use crate::cache::{CacheError, Store, cache_key};
use crate::cli::Cli;
use crate::config::{Config, ConfigStore, resolve};
use crate::domain::{EventKind, Notifiable, PrayerWindow, day_summary, plan_events};
use crate::error::SalatError;
use crate::service::{PrayerTimeService, ServiceError};
use crate::status::{Component, ComponentId, Health, NextEvent, SystemStatus};

/// Read a lock's value, recovering from poisoning the way the codebase
/// does everywhere: a panic elsewhere mid-write leaves a coherent value
/// (whole-struct writes, never partial), so it is served rather than
/// propagated — a prayer app does not stop telling the time because a
/// background task panicked.
fn recover<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read()
        .map(|value| value.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
}

/// The write-side counterpart of [`recover`].
fn recover_mut<'a, T>(lock: &'a RwLock<T>) -> RwLockWriteGuard<'a, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

/// Every service handle, wired and warm (§15.1). Constructed only by
/// [`Runtime::build`] (or the test fixture below); every field is
/// reachable through an accessor, never directly.
pub struct Runtime {
    /// The tokio runtime everything async runs on. Dropped last, when
    /// the `Runtime` is — the process is on its way out by then.
    rt: tokio::runtime::Runtime,
    clock: Arc<dyn Clock>,
    config_store: Arc<ConfigStore>,
    config: RwLock<Arc<Config>>,
    /// The one subscription the event loop's config arm consumes
    /// (§8.5); behind an async `Mutex` because `changed()` needs the
    /// receiver by `mut` across an await.
    cfg_rx: Mutex<watch::Receiver<Arc<Config>>>,
    /// The zone "today" is defined in (§10.2) — the config's, parsed
    /// once; a config change replaces it.
    tz: RwLock<Tz>,
    cache: Arc<Store>,
    service: Arc<PrayerTimeService>,
    /// The served window. Never absent: boot is fatal without one
    /// (§15.7), and a failed refresh keeps the old value (§18.3).
    window: RwLock<Arc<PrayerWindow>>,
    completions: RwLock<Vec<Notifiable>>,
    /// The window's "today" — the rollover check compares against it.
    day: RwLock<NaiveDate>,
    /// Cached `(health, detail)` for the components whose story changes
    /// only on refresh/config events, so `status()` stays cheap.
    svc_health: RwLock<(Health, String)>,
    /// `Unknown` until the first refresh's HTTP attempt — no probe has
    /// run at boot, and §6.7 forbids a false `Ok`.
    network_health: RwLock<(Health, String)>,
    config_health: RwLock<(Health, String)>,
}

/// The wired parts [`Runtime::build`]'s async boot hands back to the
/// synchronous constructor — the runtime itself cannot move into the
/// future it is driving.
struct Boot {
    clock: Arc<dyn Clock>,
    config_store: Arc<ConfigStore>,
    config: Arc<Config>,
    cfg_rx: watch::Receiver<Arc<Config>>,
    tz: Tz,
    cache: Arc<Store>,
    service: Arc<PrayerTimeService>,
    window: Arc<PrayerWindow>,
    completions: Vec<Notifiable>,
    today: NaiveDate,
}

impl Runtime {
    /// Wire every service (§15.7's boot ladder, §19's single wiring
    /// site). Fatal, in order: the runtime itself (`E901`), the XDG
    /// paths, the configuration (`E201`/`E202`), the cache (`E601`), and
    /// the first prayer window (`E301`–`E304`) — a `Runtime` that
    /// returns from this call can render a frame.
    ///
    /// The boot vacuum and today's completions are non-fatal (§17.3): a
    /// warning and a degraded detail, never an error.
    pub fn build(cli: &Cli) -> Result<Self, SalatError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| SalatError::RuntimeStartFailed {
                cause: error.to_string(),
            })?;
        // The async boot steps run under `block_on` and hand back the
        // wired parts; the runtime itself cannot move into the future
        // it is driving.
        let boot = rt.block_on(async {
            let paths = resolve(cli.config.as_deref())?;

            let config_store = Arc::new(ConfigStore::new(paths.config.clone()));
            let config = config_store
                .load()
                .map_err(|error| error.into_salat_error(&paths.config))?;
            let cfg_rx = config_store.subscribe();
            let tz: Tz = config
                .location
                .timezone
                .parse()
                .map_err(|_| SalatError::TimezoneUndetermined)?;

            let clock: Arc<dyn Clock> = Arc::new(SystemClock);
            let cache = Arc::new(
                Store::open(&paths.database)
                    .await
                    .map_err(|error: CacheError| error.to_salat_error())?,
            );

            let params = Params::from_parts(&config.location, &config.calculation);
            // §17.3's boot vacuum: prune rows no instance will read.
            if let Err(error) = cache.vacuum(clock.now_in(tz), &cache_key(&params)).await {
                tracing::warn!(target: "salat::app", "boot vacuum failed: {error}");
            }

            let client = HttpClient::new().map_err(|error| error.to_salat_error())?;
            let service = Arc::new(PrayerTimeService::new(
                Arc::new(client),
                Arc::clone(&cache),
                params,
                Arc::clone(&clock),
            ));

            // Boot step 3: a window, or nothing (§15.7).
            let today = clock.now_in(tz).date_naive();
            let window = service
                .window(today)
                .await
                .map_err(|error| error.to_salat_error())?;
            let completions = match cache.completions_for(today).await {
                Ok(completions) => completions,
                Err(error) => {
                    tracing::warn!(
                        target: "salat::app",
                        "could not load today's completions: {error}"
                    );
                    Vec::new()
                }
            };

            Ok::<_, SalatError>(Boot {
                clock,
                config_store,
                config,
                cfg_rx,
                tz,
                cache,
                service,
                window,
                completions,
                today,
            })
        })?;
        Ok(Self {
            rt,
            clock: boot.clock,
            config_store: boot.config_store,
            config: RwLock::new(boot.config),
            cfg_rx: Mutex::new(boot.cfg_rx),
            tz: RwLock::new(boot.tz),
            cache: boot.cache,
            service: boot.service,
            window: RwLock::new(boot.window),
            completions: RwLock::new(boot.completions),
            day: RwLock::new(boot.today),
            svc_health: RwLock::new((Health::Ok, "RUNNING".to_owned())),
            network_health: RwLock::new((Health::Unknown, String::new())),
            config_health: RwLock::new((Health::Ok, "OK".to_owned())),
        })
    }

    /// The time source (§6.6) — the one callers may read "now" from.
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock)
    }

    /// The effective configuration (§8.5's publication).
    pub fn config(&self) -> Arc<Config> {
        recover(&self.config)
    }

    /// The served window (§7.6).
    pub fn window(&self) -> Arc<PrayerWindow> {
        recover(&self.window)
    }

    /// Today's completed prayers (A3).
    pub fn completions(&self) -> Vec<Notifiable> {
        recover(&self.completions)
    }

    /// A second subscription to the configuration watch — the command
    /// layer's event loop selects on its own `cfg_rx.changed()` arm
    /// (§15.2), while [`Runtime::on_config_change`] consumes the one the
    /// runtime holds. A publication marks both receivers; the loop's arm
    /// fires, hands the change to `on_config_change`, and that call's
    /// `changed()` completes immediately on the same pending value.
    pub fn subscribe_config(&self) -> watch::Receiver<Arc<Config>> {
        self.config_store.subscribe()
    }

    /// Run a future on the runtime this `Runtime` owns — what the
    /// command layer's event loops enter through.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.rt.block_on(future)
    }

    fn tz(&self) -> Tz {
        recover(&self.tz)
    }

    fn day(&self) -> NaiveDate {
        recover(&self.day)
    }

    /// The §7.6 refresh trigger: revalidate, re-serve. Success restores
    /// the service and network components and swaps the window in whole;
    /// failure is never fatal (§18.3) — the last good window keeps
    /// serving, the strip's `● svc` goes yellow, and `Network` reads the
    /// design's `OFFLINE (using cache)` (§17.3).
    ///
    /// A cache-layer failure (not a network one) leaves `Network`
    /// untouched — the network is not implicated, and saying otherwise
    /// would lie in the other direction.
    pub async fn refresh(&self) {
        match self.service.refresh().await {
            Ok(refreshed) => {
                let today = refreshed.window.today.date;
                let rolled = today != self.day();
                if refreshed.revalidated {
                    *recover_mut(&self.svc_health) = (Health::Ok, "RUNNING".to_owned());
                    *recover_mut(&self.network_health) = (Health::Ok, "ONLINE".to_owned());
                } else {
                    // §17.3's cached case: the window serves, but the
                    // last network attempt failed — say so instead of
                    // guessing freshness.
                    *recover_mut(&self.svc_health) = (Health::Degraded, "CACHED".to_owned());
                    *recover_mut(&self.network_health) =
                        (Health::Degraded, "OFFLINE (using cache)".to_owned());
                }
                *recover_mut(&self.day) = today;
                *recover_mut(&self.window) = refreshed.window;
                if rolled {
                    // Completions are per civil date (A3); the new day
                    // starts with none.
                    self.reload_completions(today).await;
                }
            }
            Err(error) => {
                tracing::warn!(
                    target: "salat::app",
                    "refresh failed, serving the cached window: {error}"
                );
                *recover_mut(&self.svc_health) = (Health::Degraded, "CACHED".to_owned());
                if !matches!(error, ServiceError::Cache(_)) {
                    *recover_mut(&self.network_health) =
                        (Health::Degraded, "OFFLINE (using cache)".to_owned());
                }
            }
        }
    }

    /// The §7.6 rollover trigger: compare the clock's local date against
    /// the window's, refresh only when they disagree. The render loop
    /// calls this every tick — it must be free when the day has not
    /// rolled.
    pub async fn refresh_on_rollover(&self) {
        let today = self.clock().now_in(self.tz()).date_naive();
        if today != self.day() {
            self.refresh().await;
        }
    }

    /// The §8.5 config-change arm: wait for the store's next
    /// publication, then apply it — new zone, new `Params` for the
    /// window service (which drops its memo), and the §7.6 refresh so
    /// the frame shows the new location's times. If that refresh cannot
    /// reach the network the *old* window keeps serving — the honest
    /// degraded state, not a guess at the new location's times.
    pub async fn on_config_change(&self) {
        let config = {
            let mut receiver = self.cfg_rx.lock().await;
            if receiver.changed().await.is_err() {
                return; // the store is gone; nothing to apply
            }
            receiver.borrow_and_update().clone()
        };
        self.apply_config(config).await;
    }

    /// The §8.5 SIGHUP path: re-read the file. On success the store's
    /// own watch publication carries the change to
    /// [`Runtime::on_config_change`] — one path applies configs, whether
    /// it was a save, a `--config` swap or a hangup. On failure the old
    /// configuration keeps running and `Config file` reports the
    /// problem.
    pub async fn reload_config(&self) {
        match self.config_store.load() {
            Ok(_) => {
                *recover_mut(&self.config_health) = (Health::Ok, "OK".to_owned());
            }
            Err(error) => {
                tracing::warn!(
                    target: "salat::app",
                    "configuration reload failed, keeping the running configuration: {error}"
                );
                *recover_mut(&self.config_health) = (Health::Down, "INVALID".to_owned());
            }
        }
    }

    /// Apply a freshly published configuration: zone, parameters,
    /// window. Shared by the config-change arm; every configuration
    /// change in the process funnels through here.
    async fn apply_config(&self, config: Arc<Config>) {
        match config.location.timezone.parse() {
            Ok(tz) => *recover_mut(&self.tz) = tz,
            Err(_) => tracing::error!(
                target: "salat::app",
                "configured timezone {:?} did not parse; keeping {}",
                config.location.timezone,
                self.tz().name()
            ),
        }
        // The window service takes the new parameters whole; its memo is
        // dropped so the next window is computed under them (§7.6).
        self.service
            .set_params(Params::from_parts(&config.location, &config.calculation));
        *recover_mut(&self.config) = config;
        *recover_mut(&self.config_health) = (Health::Ok, "OK".to_owned());
        self.refresh().await;
    }

    async fn reload_completions(&self, date: NaiveDate) {
        match self.cache.completions_for(date).await {
            Ok(completions) => *recover_mut(&self.completions) = completions,
            Err(error) => tracing::warn!(
                target: "salat::app",
                "could not load completions for {date}: {error}"
            ),
        }
    }

    /// A fresh [`SystemStatus`] (§6.7), rebuilt on every call so the
    /// counters and next event never go stale between refreshes — it is
    /// cheap: a summary and a plan over ten instants, from cached
    /// health tuples.
    ///
    /// The component details are the design's Status-screen vocabulary
    /// verbatim: `RUNNING`, `OK`, `SÉTIF, ALGERIA` (the label
    /// uppercased), `UTC+1`, `ONLINE`, `LOADED`. Notifications, Adhan
    /// sound and Audio system are `Unknown` until Phases 10–12 own them
    /// — §6.7's honest report, never a false `Ok`.
    pub fn status(&self) -> Arc<SystemStatus> {
        let config = self.config();
        let window = self.window();
        let now = self.clock().now_in(self.tz());

        let (svc_health, svc_detail) = recover(&self.svc_health);
        let (network_health, network_detail) = recover(&self.network_health);
        let (config_health, config_detail) = recover(&self.config_health);

        let components = ComponentId::ALL
            .map(|id| {
                let (health, detail) = match id {
                    ComponentId::PrayerService => (svc_health, svc_detail.clone()),
                    ComponentId::PrayerCalculation => (Health::Ok, "OK".to_owned()),
                    ComponentId::Location => (Health::Ok, config.location.label.to_uppercase()),
                    ComponentId::Timezone => (Health::Ok, utc_offset_label(now)),
                    ComponentId::Notifications
                    | ComponentId::AdhanSound
                    | ComponentId::AudioSystem => (Health::Unknown, String::new()),
                    ComponentId::Network => (network_health, network_detail.clone()),
                    ComponentId::ConfigFile => (config_health, config_detail.clone()),
                    ComponentId::PrayerDatabase => (Health::Ok, "LOADED".to_owned()),
                };
                Component::new(id, health, detail)
            })
            .to_vec();

        let next_event = plan_events(&window, &config, now)
            .into_iter()
            .find(|event| event.kind != EventKind::DayRollover)
            .map(|event| NextEvent {
                kind: event.kind,
                prayer: event.prayer,
                fire_at: event.fire_at,
            });

        Arc::new(SystemStatus {
            components,
            next_event,
            today: day_summary(&window, &config, now),
            // The event log is the scheduler's (Phase 8+); quick mode
            // has nothing to show yet.
            recent: Vec::new(),
        })
    }
}

/// The Timezone component's detail (§16.4): the zone's current UTC
/// offset as `UTC+1`, `UTC-4` — or `UTC+5:30` where a zone runs on a
/// half hour. Pure, so the format is pinned by unit test.
fn utc_offset_label(now: DateTime<Tz>) -> String {
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

#[cfg(test)]
pub(crate) mod testing {
    //! The §20.3 snapshot `Runtime`: the fixture window and clock, an
    //! in-memory cache, a `MockClient` whose queues the test scripts,
    //! and the configuration at a real temp file so the SIGHUP path is
    //! exercisable. Nothing here touches the network or the filesystem
    //! outside `tempfile`'s own file.

    use std::path::Path;

    use super::*;
    use crate::api::MockClient;
    use crate::app::clock::MockClock;
    use crate::domain::testing as dt;
    use chrono::Datelike as _;

    /// A test [`Runtime`] plus the handles its tests script.
    pub(crate) struct Fixture {
        pub runtime: Runtime,
        /// Push scripted responses here; an empty queue fails the call
        /// loudly (the mock's contract).
        pub client: Arc<MockClient>,
        pub store: Arc<Store>,
        pub clock: Arc<MockClock>,
        /// The `ConfigStore`'s file — write a new configuration here and
        /// call `reload_config` to exercise the SIGHUP path.
        pub config_file: tempfile::NamedTempFile,
    }

    impl Fixture {
        /// The store's file path.
        pub fn config_path(&self) -> &Path {
            self.config_file.path()
        }

        /// Set the served completions directly — the cache round-trip
        /// is the cache module's test; model tests only need the
        /// runtime to serve them.
        pub(crate) fn set_completions(&self, completions: Vec<Notifiable>) {
            *recover_mut(&self.runtime.completions) = completions;
        }
    }

    /// A `Runtime` for an arbitrary configuration, aligned with the
    /// §20.3 fixture window (whose zone is `Africa/Algiers` — set the
    /// config's zone to that or `status()`'s "today" will disagree with
    /// the clock).
    pub(crate) fn runtime(config: Config) -> Fixture {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime needs no resources");

        let (window, clock) = dt::fixture();
        let clock = Arc::new(clock);
        let client = Arc::new(MockClient::new());
        let store = Arc::new(
            rt.block_on(async { Store::open_in_memory().await })
                .expect("an in-memory store cannot fail to open"),
        );
        let params = Params::from_parts(&config.location, &config.calculation);
        let service = Arc::new(PrayerTimeService::new(
            Arc::clone(&client) as Arc<dyn crate::api::AlAdhanClient>,
            Arc::clone(&store),
            params,
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));

        let config_file = tempfile::NamedTempFile::new().expect("a temp file");
        std::fs::write(
            config_file.path(),
            toml::to_string_pretty(&config).expect("Config serializes — save() relies on it"),
        )
        .expect("the temp file is writable");

        let config_store = Arc::new(ConfigStore::new(config_file.path().to_path_buf()));
        // Subscribed before any load, exactly as `build` does — the
        // store's next publication (a reload_config) is what the
        // config-change arm observes.
        let cfg_rx = config_store.subscribe();
        let runtime = Runtime {
            rt,
            clock: Arc::clone(&clock) as Arc<dyn Clock>,
            config_store,
            config: RwLock::new(Arc::new(config)),
            cfg_rx: Mutex::new(cfg_rx),
            tz: RwLock::new(dt::TZ),
            cache: Arc::clone(&store),
            service,
            window: RwLock::new(Arc::new(window)),
            completions: RwLock::new(Vec::new()),
            day: RwLock::new(dt::TODAY),
            svc_health: RwLock::new((Health::Ok, "RUNNING".to_owned())),
            network_health: RwLock::new((Health::Unknown, String::new())),
            config_health: RwLock::new((Health::Ok, "OK".to_owned())),
        };
        Fixture {
            runtime,
            client,
            store,
            clock,
            config_file,
        }
    }

    /// The §20.3 snapshot runtime: the default configuration pointing at
    /// the fixture's Sétif location, so every derived value is the one
    /// the document's specimens show.
    pub(crate) fn snapshot_runtime() -> Fixture {
        let mut config = dt::default_config();
        config.location.label = "Sétif, Algeria".to_owned();
        config.location.country = "Algeria".to_owned();
        config.location.timezone = "Africa/Algiers".to_owned();
        runtime(config)
    }

    /// A complete September 2026 month in the fixture's shape — enough
    /// for the window service to serve 2026-09-05 from cache with zero
    /// calendar calls (§7.6).
    pub(crate) fn september() -> Vec<crate::domain::PrayerDay> {
        (1..=30)
            .map(|day| {
                let date =
                    NaiveDate::from_ymd_opt(2026, 9, day).expect("every September has this day");
                let next = date.succ_opt().expect("2026-10-01 follows 2026-09-30");
                crate::domain::PrayerDay {
                    date,
                    tz: dt::TZ,
                    times: [
                        dt::at(date, 5, 9, 0),
                        dt::at(date, 6, 44, 0),
                        dt::at(date, 12, 47, 0),
                        dt::at(date, 16, 23, 0),
                        dt::at(date, 19, 21, 0),
                        dt::at(date, 20, 47, 0),
                    ],
                    midnight: dt::at(next, 0, 37, 0),
                    last_third: dt::at(next, 2, 47, 0),
                    hijri: crate::domain::HijriDate {
                        // The recorded calendar's mapping for this month
                        // (§7.8): Sep 1 → 19 Rabīʿ al-awwal 1448, so
                        // the Hijri day is `civil + 18`.
                        day: date.day() + 18,
                        month_number: 3,
                        month_en: "Rabīʿ al-awwal".to_owned(),
                        month_ar: "رَبيع الأوّل".to_owned(),
                        year: 1448,
                    },
                    meta: crate::domain::DayMeta {
                        method_id: 12,
                        method_name: "Union Organization Islamic de France".to_owned(),
                        school: 0,
                        lat: 36.19,
                        lng: 5.41,
                        tz_name: "Africa/Algiers".to_owned(),
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Revalidated;
    use crate::domain::testing as dt;
    use testing::Fixture;

    /// The strip's and Status screen's vocabulary, design-verbatim, in
    /// the design's order — at the snapshot, before any refresh.
    #[test]
    fn status_reports_the_designs_ten_components() {
        let fixture = testing::snapshot_runtime();
        let status = fixture.runtime.status();

        assert_eq!(status.components.len(), 10);
        for (index, id) in ComponentId::ALL.iter().enumerate() {
            assert_eq!(status.components[index].id, *id, "order is the design's");
        }

        let reported = |id| {
            status
                .component(id)
                .map(|component| (component.health, component.detail.clone()))
        };
        assert_eq!(
            reported(ComponentId::PrayerService),
            Some((Health::Ok, "RUNNING".to_owned()))
        );
        assert_eq!(
            reported(ComponentId::PrayerCalculation),
            Some((Health::Ok, "OK".to_owned()))
        );
        assert_eq!(
            reported(ComponentId::Location),
            Some((Health::Ok, "SÉTIF, ALGERIA".to_owned()))
        );
        assert_eq!(
            reported(ComponentId::Timezone),
            Some((Health::Ok, "UTC+1".to_owned()))
        );
        // Phases 10–12 own these; Unknown is the honest report (§6.7).
        for id in [
            ComponentId::Notifications,
            ComponentId::AdhanSound,
            ComponentId::AudioSystem,
        ] {
            assert_eq!(reported(id), Some((Health::Unknown, String::new())));
        }
        // No HTTP attempt has run at boot — not "ONLINE", which would
        // be a guess.
        assert_eq!(
            reported(ComponentId::Network),
            Some((Health::Unknown, String::new()))
        );
        assert_eq!(
            reported(ComponentId::ConfigFile),
            Some((Health::Ok, "OK".to_owned()))
        );
        assert_eq!(
            reported(ComponentId::PrayerDatabase),
            Some((Health::Ok, "LOADED".to_owned()))
        );

        // The counters and next event at the snapshot (§20.3): three
        // prayers passed, two remaining, Maghrib's pre-notify next.
        assert_eq!(
            (
                status.today.total,
                status.today.passed,
                status.today.remaining
            ),
            (5, 3, 2)
        );
        let next = status
            .next_event
            .as_ref()
            .expect("the plan holds evening events at 17:38");
        assert_eq!(next.kind, EventKind::PreNotify);
        assert_eq!(next.prayer, Some(Notifiable::Maghrib));
        assert_eq!(next.fire_at, dt::at(dt::TODAY, 19, 16, 0));
    }

    /// `UTC+1` for whole hours, `UTC+5:30` for half ones, `UTC-4` west
    /// of Greenwich — the Timezone detail's format.
    #[test]
    fn utc_offset_label_formats_whole_and_half_hours() {
        let algiers = dt::at(dt::TODAY, 12, 0, 0);
        assert_eq!(utc_offset_label(algiers), "UTC+1");
        let kolkata: chrono::DateTime<Tz> =
            dt::at(dt::TODAY, 12, 0, 0).with_timezone(&chrono_tz::Asia::Kolkata);
        assert_eq!(utc_offset_label(kolkata), "UTC+5:30");
        let new_york = dt::at(dt::TODAY, 12, 0, 0).with_timezone(&chrono_tz::America::New_York);
        assert_eq!(utc_offset_label(new_york), "UTC-4");
        let london = dt::at(dt::TODAY, 12, 0, 0).with_timezone(&chrono_tz::Europe::London);
        assert_eq!(utc_offset_label(london), "UTC+1"); // September 2026, BST
    }

    /// §18.3: a refresh that cannot reach anything keeps the window and
    /// degrades the components — the mock's empty queues fail both the
    /// revalidation and the month fetch.
    #[test]
    fn a_failed_refresh_degrades_but_keeps_serving() {
        let fixture = testing::snapshot_runtime();
        let before = fixture.runtime.window();

        fixture.runtime.block_on(fixture.runtime.refresh());

        // Same window, still served.
        assert_eq!(*fixture.runtime.window(), *before);
        let status = fixture.runtime.status();
        assert_eq!(
            status
                .component(ComponentId::PrayerService)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Degraded, "CACHED".to_owned()))
        );
        assert_eq!(
            status
                .component(ComponentId::Network)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Degraded, "OFFLINE (using cache)".to_owned()))
        );
    }

    /// A refresh that revalidates clean (304) restores the components to
    /// the design's `RUNNING` / `ONLINE` — September is seeded in the
    /// cache, so the month is served with zero calendar calls and only
    /// the prefetch month's failure is logged.
    #[test]
    fn a_successful_refresh_restores_health() {
        let fixture = testing::snapshot_runtime();
        // First degrade (empty cache, nothing scripted), so the test
        // proves restoration, not inertia.
        fixture.runtime.block_on(fixture.runtime.refresh());
        assert_eq!(
            fixture
                .runtime
                .status()
                .component(ComponentId::PrayerService)
                .unwrap()
                .health,
            Health::Degraded
        );

        // Seed September and script a clean 304; the month now serves
        // from cache with zero calendar calls.
        let config = fixture.runtime.config();
        let key = cache_key(&Params::from_parts(&config.location, &config.calculation));
        fixture
            .runtime
            .block_on(fixture.store.upsert_month(
                &key,
                &testing::september(),
                Some("etag"),
                dt::snapshot_now(),
            ))
            .expect("the in-memory store accepts a month");
        fixture
            .client
            .push_revalidation(Ok(Revalidated::NotModified));
        fixture.runtime.block_on(fixture.runtime.refresh());

        let status = fixture.runtime.status();
        assert_eq!(
            status
                .component(ComponentId::PrayerService)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Ok, "RUNNING".to_owned()))
        );
        assert_eq!(
            status
                .component(ComponentId::Network)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Ok, "ONLINE".to_owned()))
        );
        // The window served from the seeded month is today's.
        assert_eq!(fixture.runtime.window().today.date, dt::TODAY);
    }

    /// §17.3's middle case: the revalidation cannot reach the API but
    /// the cache covers the day — the refresh still serves, degraded,
    /// so the frame carries `· cached` and Network reads
    /// `OFFLINE (using cache)`. Nothing scripted means the mock fails
    /// the revalidation; the seeded month means it never needs a
    /// calendar call.
    #[test]
    fn an_unreachable_api_with_a_warm_cache_serves_degraded() {
        let fixture = testing::snapshot_runtime();
        let config = fixture.runtime.config();
        let key = cache_key(&Params::from_parts(&config.location, &config.calculation));
        fixture
            .runtime
            .block_on(fixture.store.upsert_month(
                &key,
                &testing::september(),
                None,
                dt::snapshot_now(),
            ))
            .expect("the in-memory store accepts a month");

        fixture.runtime.block_on(fixture.runtime.refresh());

        // The cached month answers — today serves, with zero fetches.
        assert_eq!(fixture.runtime.window().today.date, dt::TODAY);
        let status = fixture.runtime.status();
        assert_eq!(
            status
                .component(ComponentId::PrayerService)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Degraded, "CACHED".to_owned()))
        );
        assert_eq!(
            status
                .component(ComponentId::Network)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Degraded, "OFFLINE (using cache)".to_owned()))
        );
    }

    /// The rollover arm is free until the clock crosses local midnight,
    /// then refreshes — observed through the mock's revalidation
    /// counter, which only a real refresh increments.
    #[test]
    fn rollover_only_refreshes_when_the_day_changes() {
        let fixture = testing::snapshot_runtime();
        fixture
            .runtime
            .block_on(fixture.runtime.refresh_on_rollover());
        assert_eq!(fixture.client.revalidation_calls(), 0);

        // 17:38 → 00:38 the next day, still Africa/Algiers.
        fixture.clock.advance(chrono::Duration::hours(7));
        fixture
            .runtime
            .block_on(fixture.runtime.refresh_on_rollover());
        assert_eq!(fixture.client.revalidation_calls(), 1);
    }

    /// §8.5's SIGHUP path: a valid file reloads, the watch carries it,
    /// and the applied configuration is the new one; the refresh inside
    /// the apply degrades (nothing scripted) but the window and the
    /// running configuration stay coherent.
    #[test]
    fn a_sighup_reloads_and_applies_the_configuration() {
        let fixture = testing::snapshot_runtime();
        let mut config = dt::default_config();
        config.location.label = "Mecca".to_owned();
        config.location.timezone = "Africa/Algiers".to_owned();
        std::fs::write(
            fixture.config_path(),
            toml::to_string_pretty(&config).expect("Config serializes — save() relies on it"),
        )
        .expect("the temp file is writable");

        fixture.runtime.block_on(fixture.runtime.reload_config());
        fixture.runtime.block_on(fixture.runtime.on_config_change());

        assert_eq!(fixture.runtime.config().location.label, "Mecca");
        let status = fixture.runtime.status();
        assert_eq!(
            status
                .component(ComponentId::Location)
                .map(|c| c.detail.clone()),
            Some("MECCA".to_owned())
        );
        assert_eq!(
            status
                .component(ComponentId::ConfigFile)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Ok, "OK".to_owned()))
        );
        // The refresh inside the apply had nothing scripted: degraded,
        // window intact.
        assert_eq!(
            status
                .component(ComponentId::PrayerService)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Degraded, "CACHED".to_owned()))
        );
        assert_eq!(fixture.runtime.window().today.date, dt::TODAY);
    }

    /// §8.5's other SIGHUP row: an invalid file is reported, and the
    /// running configuration is untouched. A failed `load` publishes
    /// nothing — there is no valid configuration to publish — so the
    /// config-change arm stays asleep; this test must not call
    /// `on_config_change`, whose `changed()` wait would (correctly)
    /// never complete.
    #[test]
    fn an_invalid_reload_is_reported_and_ignored() {
        let fixture = testing::snapshot_runtime();
        let label_before = fixture.runtime.config().location.label.clone();
        std::fs::write(
            fixture.config_path(),
            "version = 2\n[location]\nlabel = \"x\"\n",
        )
        .expect("the temp file is writable");

        fixture.runtime.block_on(fixture.runtime.reload_config());

        assert_eq!(fixture.runtime.config().location.label, label_before);
        assert_eq!(
            fixture
                .runtime
                .status()
                .component(ComponentId::ConfigFile)
                .map(|c| (c.health, c.detail.clone())),
            Some((Health::Down, "INVALID".to_owned()))
        );
    }

    /// `Fixture`'s own contract: the temp file exists and holds the
    /// configuration the runtime serves.
    #[test]
    fn the_fixture_wires_a_coherent_runtime() {
        let fixture: Fixture = testing::snapshot_runtime();
        assert!(fixture.config_path().exists());
        assert_eq!(fixture.runtime.window().today.date, dt::TODAY);
        assert_eq!(fixture.runtime.completions(), Vec::new());
    }
}
