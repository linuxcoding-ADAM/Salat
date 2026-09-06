//! Bare `salat` — the quick live view in an inline viewport (§14.1, §16.14).
//!
//! Two entries, matching §14.3's dispatch:
//! - [`run_live`] — a TTY stdout and no `--once`: the inline viewport that
//!   renders the §16.14 layout at the §15.2 cadence until the user quits.
//! - [`run_once`] — `--once`, or any non-TTY stdout: one frame through the
//!   same render path, printed as trimmed lines (§14.6).
//!
//! Both funnel through [`render_quick`], a pure function of the `UiModel`,
//! so the ladder, the goldens and the live loop all exercise one code path.
//! §16.14's own rules are the contract here: the pinned `Small` breakpoint,
//! the `min(14, h − 1)` viewport, the 13-row content budget and its
//! degradation ladder, and the shared floor (`h < 10` or `w < 80`) below
//! which no viewport is opened at all and the §14.6 one-line summary prints.

use std::io::{IsTerminal as _, Stdout, Write as _};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Timelike as _;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal as cterm;
use futures_util::StreamExt as _;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::{Terminal, TerminalOptions, Viewport};
use unicode_width::UnicodeWidthStr as _;

use crate::app::runtime::Runtime;
use crate::app::signals;
use crate::cli::Cli;
use crate::config::resolve;
use crate::domain::Notifiable;
use crate::error::{ExitCode, SalatError};
use crate::status::{ComponentId, Health};
use crate::ui::model::{UiModel, build_model};
use crate::ui::theme::{Theme, hhmm, quick_name};
use crate::ui::widgets::bar;
use crate::ui::widgets::hr;
use crate::ui::widgets::panel::Panel;
use crate::ui::widgets::prayer_table::{self, PrayerRow};
use crate::ui::widgets::status_dot;

/// The strip's four groups (§16.14 item 10): component, label.
const STRIP: [(ComponentId, &str); 4] = [
    (ComponentId::PrayerService, "svc"),
    (ComponentId::Notifications, "notif"),
    (ComponentId::AdhanSound, "adhan"),
    (ComponentId::AudioSystem, "audio"),
];

// ─────────────────────────       geometry       ─────────────────────────

/// `v = min(14, h − 1)` (§15.3) — the only dimension quick mode takes
/// from that section; everything else is §16.14's.
pub fn viewport_height(term_height: u16) -> u16 {
    14.min(term_height.saturating_sub(1))
}

/// Which frame elements survive at a given content-row count (§16.14
/// item 9). The full frame is 13 content rows; each step below that
/// drops one more element in the section's fixed order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuickPlan {
    /// The one-row service strip.
    pub strip: bool,
    /// The `TODAY` head row.
    pub today_head: bool,
    /// The full-width rule under it.
    pub today_rule: bool,
    /// The prayer-row indices (`0..5`, list order) that were dropped.
    pub dropped_rows: Vec<usize>,
}

/// The ladder: `content_rows` is `c` (viewport height minus the header).
/// Drops, cumulatively: strip → TODAY's rule → TODAY's head → one prayer
/// row → a second prayer row. Which prayer row goes first is
/// `argmax |i − n|` with ties to the lower index, where `n` is the next
/// prayer's row — the next prayer is never the row that is dropped.
/// `next_index: None` is the exhausted-window sliver (tomorrow's Fajr is
/// next, outside today's rows); distance is then measured from `−1`, so
/// today's Isha goes first — the farthest row from tomorrow's Fajr.
pub(crate) fn quick_plan(content_rows: u16, next_index: Option<usize>) -> QuickPlan {
    let next_row = next_index.map(|index| index as i32).unwrap_or(-1);
    let excess = 13usize.saturating_sub(content_rows as usize);

    let mut plan = QuickPlan {
        strip: excess < 1,
        today_head: excess < 3,
        today_rule: excess < 2,
        dropped_rows: Vec::new(),
    };

    // The fourth and fifth drops are prayer rows, chosen by distance
    // from the next prayer's row; `Reverse(i)` breaks distance ties
    // towards the lower index.
    for _ in 0..excess.saturating_sub(3).min(2) {
        let farthest = (0..5)
            .filter(|index| !plan.dropped_rows.contains(index))
            .max_by_key(|index| {
                (
                    (*index as i32 - next_row).unsigned_abs(),
                    std::cmp::Reverse(*index),
                )
            });
        if let Some(index) = farthest {
            plan.dropped_rows.push(index);
        }
    }
    plan
}

/// The next prayer's row index, if it is one of today's rows.
fn next_index(model: &UiModel) -> Option<usize> {
    model.next.and_then(|next| {
        Notifiable::ALL
            .iter()
            .position(|prayer| *prayer == next.prayer)
    })
}

/// Whether the day is being served from cache after a failed refresh —
/// the `· cached` marker's condition (§16.14 item 8).
fn service_degraded(model: &UiModel) -> bool {
    model
        .status
        .component(ComponentId::PrayerService)
        .is_some_and(|component| component.health == Health::Degraded)
}

// ─────────────────────────       rendering       ────────────────────────

/// Write one clipped, styled string at `(x, y)` and return the next
/// column — the prayer table's truncation story (§16.14 item 10),
/// local to this renderer.
fn put(buf: &mut Buffer, x: u16, y: u16, limit: u16, text: &str, style: Style) -> u16 {
    if limit == 0 {
        return x;
    }
    let (next, _) = buf.set_stringn(x, y, text, limit as usize, style);
    next
}

/// The whole quick frame (§16.14 item 4), a pure function of the model.
/// The caller guarantees `width >= 80` and `height >= 1`; the guard is
/// the belt to that brace.
pub(crate) fn render_quick(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.width < 80 || area.height == 0 {
        return;
    }
    let plan = quick_plan(area.height.saturating_sub(1), next_index(model));

    render_header(model, area, buf, theme);

    let mut y = area.y + 1;

    // The panel always keeps its five rows (item 9); the clamp only
    // bites on degenerate areas the run paths never produce.
    let panel_height = 5.min(area.bottom().saturating_sub(y));
    if panel_height > 0 {
        render_next_prayer(
            model,
            Rect::new(area.x, y, area.width, panel_height),
            buf,
            theme,
        );
        y += panel_height;
    }

    if plan.today_head && y < area.bottom() {
        render_today_head(model, Rect::new(area.x, y, area.width, 1), buf, theme);
        y += 1;
    }
    if plan.today_rule && y < area.bottom() {
        hr::render(Rect::new(area.x, y, area.width, 1), buf, theme, None);
        y += 1;
    }

    let rows = prayer_rows(model, &plan);
    prayer_table::render(
        Rect::new(area.x, y, area.width, rows.len() as u16),
        buf,
        theme,
        &rows,
    );
    y += rows.len() as u16;

    if plan.strip && y < area.bottom() {
        render_strip(model, Rect::new(area.x, y, area.width, 1), buf, theme);
    }
}

/// `SALAT` green bold at the left, the 24-hour clock right-aligned to
/// the frame's edge — the Figma small header, `HH:MM:SS` via `pad()`.
fn render_header(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let style = Style::new().fg(theme.green).add_modifier(Modifier::BOLD);
    put(buf, area.x, area.y, area.width, "SALAT", style);
    let clock = format!(
        "{:02}:{:02}:{:02}",
        model.now.hour(),
        model.now.minute(),
        model.now.second()
    );
    let clock_x = area.right().saturating_sub(8);
    put(
        buf,
        clock_x,
        area.y,
        area.right().saturating_sub(clock_x),
        &clock,
        Style::new().fg(theme.green),
    );
}

/// The compressed NEXT PRAYER panel (§16.14): border, name row,
/// countdown row with `REMAINING` folded on (item 4's −1), gauge row,
/// border — five rows, the gauge fixed at 34 cells.
fn render_next_prayer(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let inner = Panel::new()
        .title("NEXT PRAYER")
        .active(true)
        .render(area, buf, theme);
    // One column of indent inside the border, as the specimen shows.
    let x = inner.x + 1;
    let width = inner.width.saturating_sub(1);

    match model.next {
        Some(next) => {
            // One green-bold span: `● MAGHRIB · 19:21` (the Figma small
            // panel's name row, verbatim).
            let name_row = format!(
                "● {} · {}",
                quick_name(next.prayer).to_uppercase(),
                hhmm(next.at, &model.config.display.time_format)
            );
            put(
                buf,
                x,
                inner.y,
                width,
                &name_row,
                Style::new().fg(theme.green).add_modifier(Modifier::BOLD),
            );
            let after = put(
                buf,
                x,
                inner.y + 1,
                width,
                &model.countdown.format(),
                Style::new()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD),
            );
            put(
                buf,
                after,
                inner.y + 1,
                x + width.saturating_sub(after - x),
                "  REMAINING",
                Style::new().fg(theme.text_sub),
            );
            bar::render(
                Rect::new(x, inner.y + 2, width, 1),
                buf,
                theme,
                model.progress,
                34,
                None,
            );
        }
        // The exhausted-window sliver (the tick after tomorrow's Isha):
        // honest placeholders, never a panic (§6.5) — and the gauge is
        // `progress`'s own degenerate `0.0`.
        None => {
            let dim = Style::new().fg(theme.text_dim);
            put(buf, x, inner.y, width, "● -- · --:--", dim);
            put(buf, x, inner.y + 1, width, "--:--:--  REMAINING", dim);
            bar::render(
                Rect::new(x, inner.y + 2, width, 1),
                buf,
                theme,
                0.0,
                34,
                None,
            );
        }
    }
}

/// The borderless TODAY head — `TODAY` in the panel-title colour, plus
/// the `· cached` marker when the day is served from cache (§16.14
/// item 8).
fn render_today_head(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let after = put(
        buf,
        area.x,
        area.y,
        area.width,
        "TODAY",
        Style::new().fg(theme.text_sub),
    );
    if service_degraded(model) {
        put(
            buf,
            after,
            area.y,
            area.right().saturating_sub(after),
            " · cached",
            Style::new().fg(theme.yellow),
        );
    }
}

/// Today's rows, minus the ladder's drops, with the next prayer's note.
fn prayer_rows(model: &UiModel, plan: &QuickPlan) -> Vec<PrayerRow> {
    Notifiable::ALL
        .iter()
        .enumerate()
        .filter(|(index, _)| !plan.dropped_rows.contains(index))
        .map(|(index, prayer)| {
            let is_next = model.next.is_some_and(|next| next.prayer == *prayer);
            PrayerRow {
                name: quick_name(*prayer).to_owned(),
                time: hhmm(
                    model.window.today.time(prayer.as_prayer()),
                    &model.config.display.time_format,
                ),
                state: model.states[index],
                note: is_next.then(|| format!("· in {}", model.countdown.format())),
            }
        })
        .collect()
}

/// The service strip (§16.14 item 10): `● svc  ● notif  ● adhan  ● audio`
/// at the left — dot colours from each component's health, labels in
/// `textDim` — and `{label} · {hijri}` right-aligned, dropped whole when
/// it would collide with the left group.
fn render_strip(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let end = area.x + area.width;
    let label_style = Style::new().fg(theme.text_dim);
    let mut x = area.x;
    for (index, (id, label)) in STRIP.iter().enumerate() {
        if index > 0 {
            x = put(buf, x, area.y, end - x, "  ", label_style);
        }
        let health = model
            .status
            .component(*id)
            .map(|component| component.health)
            .unwrap_or(Health::Unknown);
        let dot = status_dot::span(health, theme);
        x = put(buf, x, area.y, end - x, dot.content.as_ref(), dot.style);
        x = put(buf, x, area.y, end - x, " ", label_style);
        x = put(buf, x, area.y, end - x, label, label_style);
    }
    let left_end = x;

    let hijri = &model.window.today.hijri;
    let right = format!(
        "{} · {} {} {} AH",
        model.config.location.label, hijri.day, hijri.month_en, hijri.year
    );
    let right_x = end.saturating_sub(right.width() as u16);
    // Dropped whole — never half-drawn, and never touching the left
    // group's last column (§16.14 item 10).
    if right_x > left_end {
        put(buf, right_x, area.y, end - right_x, &right, label_style);
    }
}

// ─────────────────────────   frame → stdout       ───────────────────────

/// The §14.6 one-line summary: `{NAME} · {HH:MM} · in {HH:MM:SS}`, plus
/// ` · cached` when the day was served from cache. Four fields, fixed
/// order, nothing else — no location, no Hijri, no dots. `None` is the
/// exhausted window: stdout then carries nothing (the frame-or-nothing
/// rule's honest empty case).
pub(crate) fn one_line_summary(model: &UiModel) -> Option<String> {
    let next = model.next?;
    let mut line = format!(
        "{} · {} · in {}",
        quick_name(next.prayer).to_uppercase(),
        hhmm(next.at, &model.config.display.time_format),
        model.countdown.format()
    );
    if service_degraded(model) {
        line.push_str(" · cached");
    }
    Some(line)
}

/// The style state one emitted cell run carries: enough to decide when
/// an SGR transition is needed and to encode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CellStyle {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
}

impl CellStyle {
    fn of(cell: &ratatui::buffer::Cell) -> Self {
        let style = cell.style();
        let normalise = |colour: Option<Color>| match colour {
            Some(Color::Reset) | None => None,
            other => other,
        };
        Self {
            fg: normalise(style.fg),
            bg: normalise(style.bg),
            bold: style.add_modifier.contains(Modifier::BOLD),
        }
    }

    fn is_default(self) -> bool {
        self == Self::default()
    }
}

/// The named colours' foreground SGR codes — `30..37` for the base
/// eight, `90..97` for the bright eight; the background side is the
/// same table `+10`. `None` for the colours that carry their own
/// encoding (`Rgb`, `Indexed`) or none (`Reset`).
fn named_code(colour: Color) -> Option<u8> {
    match colour {
        Color::Black => Some(30),
        Color::Red => Some(31),
        Color::Green => Some(32),
        Color::Yellow => Some(33),
        Color::Blue => Some(34),
        Color::Magenta => Some(35),
        Color::Cyan => Some(36),
        Color::Gray => Some(37),
        Color::DarkGray => Some(90),
        Color::LightRed => Some(91),
        Color::LightGreen => Some(92),
        Color::LightYellow => Some(93),
        Color::LightBlue => Some(94),
        Color::LightMagenta => Some(95),
        Color::LightCyan => Some(96),
        Color::White => Some(97),
        Color::Reset | Color::Rgb(..) | Color::Indexed(..) => None,
    }
}

/// Encode one SGR sequence for the style — the params of §14.6's "only
/// SGR colour and attribute sequences": `38;2;r;g;b` / `38;5;n` /
/// 30–37·90–97 for the foreground, `48;…` / 40–47·100–107 for the
/// ground, `1` for bold.
fn sgr(style: CellStyle) -> String {
    let colour = |colour: Color, ground: bool| -> String {
        let base = if ground { 48 } else { 38 };
        match colour {
            Color::Rgb(red, green, blue) => format!("{base};2;{red};{green};{blue}"),
            Color::Indexed(index) => format!("{base};5;{index}"),
            named => named_code(named).map_or_else(String::new, |code| {
                (code + if ground { 10 } else { 0 }).to_string()
            }),
        }
    };

    let mut params: Vec<String> = Vec::new();
    if let Some(fg) = style.fg {
        params.push(colour(fg, false));
    }
    if let Some(bg) = style.bg {
        params.push(colour(bg, true));
    }
    if style.bold {
        params.push("1".to_owned());
    }
    params.retain(|param| !param.is_empty());
    format!("\x1b[{}m", params.join(";"))
}

/// The buffer as printed lines (§14.6): trailing whitespace trimmed —
/// a space with a ground colour is not whitespace to a colour reader,
/// so only default-ground spaces trim — and, when `colored`, SGR
/// transitions exactly where the style changes: a reset then the new
/// style's combined params, a bare reset at each line's end.
pub(crate) fn frame_to_string(buf: &Buffer, colored: bool) -> String {
    let width = buf.area.width;
    let mut out = String::new();
    for y in 0..buf.area.height {
        // The last column that prints: a non-space, or (colored) a
        // space whose ground is visible.
        let last = (0..width)
            .rev()
            .find(|x| {
                let cell = &buf[(*x, y)];
                cell.symbol() != " "
                    || (colored && matches!(cell.style().bg, Some(bg) if bg != Color::Reset))
            })
            .map_or(0, |x| x + 1);

        let mut current = CellStyle::default();
        for x in 0..last {
            let cell = &buf[(x, y)];
            let style = CellStyle::of(cell);
            if colored && style != current {
                out.push_str("\x1b[0m");
                if !style.is_default() {
                    out.push_str(&sgr(style));
                }
                current = style;
            }
            out.push_str(cell.symbol());
        }
        if colored && !current.is_default() {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }
    out
}

// ─────────────────────────   terminal guard       ───────────────────────

/// Whether raw mode is currently on — the idempotence flag behind
/// [`restore_terminal`], so the guard's `Drop`, a `?` propagation and
/// the panic hook can all call it and exactly one restoration happens.
static RAW_MODE: AtomicBool = AtomicBool::new(false);
static HOOK_INSTALLED: Once = Once::new();

/// Restore the terminal: show the cursor, leave raw mode. Idempotent.
fn restore_terminal() {
    if RAW_MODE.swap(false, Ordering::SeqCst) {
        let _ = execute!(std::io::stdout(), Show);
        let _ = cterm::disable_raw_mode();
    }
}

/// Raw mode and the hidden cursor, restored however the command ends:
/// `Drop` covers normal exit and `?` propagation, and the panic hook
/// installed here covers the third path (§15.3) — it restores first,
/// then delegates to the previous hook (color-eyre's, installed in
/// `main`), so the panic report itself prints on a cooked terminal.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self, SalatError> {
        cterm::enable_raw_mode().map_err(term_io)?;
        if let Err(error) = execute!(std::io::stdout(), Hide) {
            let _ = cterm::disable_raw_mode();
            return Err(term_io(error));
        }
        RAW_MODE.store(true, Ordering::SeqCst);

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

fn term_io(error: std::io::Error) -> SalatError {
    SalatError::TerminalIo {
        cause: error.to_string(),
    }
}

// ─────────────────────────     live view        ─────────────────────────

/// What a key event means to the quick loop (§16.14 item 7).
enum KeyAction {
    Quit,
    Redraw,
    Ignore,
}

fn key_action(event: &Event) -> KeyAction {
    let Event::Key(key) = event else {
        return KeyAction::Ignore;
    };
    if key.kind != KeyEventKind::Press {
        return KeyAction::Ignore; // §15.4: Press only
    }
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyAction::Quit,
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => KeyAction::Redraw,
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => KeyAction::Quit,
        _ => KeyAction::Ignore,
    }
}

/// The live inline view (§15.2's loop, quick mode's arms). The boot
/// ladder's fatal cases have already propagated; from here nothing is
/// fatal but the terminal itself.
pub fn run_live(cli: &Cli) -> Result<ExitCode, SalatError> {
    let runtime = Runtime::build(cli)?;

    // §16.14 item 9: the floor is evaluated against the *terminal*
    // size, before any mode change — below it no viewport is opened and
    // the §14.6 summary is the frame.
    let (width, height) = cterm::size().map_err(term_io)?;
    if width < 80 || height < 10 {
        let model = build_model(&runtime);
        if let Some(line) = one_line_summary(&model) {
            println!("{line}");
        }
        return Ok(ExitCode::Ok);
    }

    let theme = Theme::from(&runtime.config().display);
    let _guard = TerminalGuard::new()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_height(height)),
        },
    )
    .map_err(term_io)?;

    runtime.block_on(run_loop(&runtime, &mut terminal, &theme))?;
    // The last frame stays in scrollback (§15.3): no clear, no
    // alternate screen to leave.
    Ok(ExitCode::Ok)
}

/// §15.2's loop with Phase 5's arms: events, the 250 ms tick (with the
/// rollover check), the configuration watch, and the signals channel.
/// `svc_rx` arrives with the scheduler's phase. Drawing is on demand,
/// capped at 20 Hz.
async fn run_loop(
    runtime: &Runtime,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    theme: &Theme,
) -> Result<(), SalatError> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut cfg_rx = runtime.subscribe_config();
    let mut signal_rx = signals::forward();

    let mut model = build_model(runtime);
    let mut theme = *theme;
    let mut last_draw = Instant::now();
    let mut dirty = true;

    loop {
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => match key_action(&event) {
                    KeyAction::Quit => break,
                    KeyAction::Redraw => {
                        terminal.clear().map_err(term_io)?;
                        dirty = true;
                    }
                    KeyAction::Ignore => {
                        // A resize needs no handler (§15.2 decision 3);
                        // its only job is marking dirty.
                        if matches!(event, Event::Resize(..)) {
                            dirty = true;
                        }
                    }
                },
                // A read error is not fatal to the loop — the next
                // event may well arrive.
                Some(Err(_)) => {}
                None => break,
            },
            _ = tick.tick() => {
                runtime.refresh_on_rollover().await;
                model = build_model(runtime);
                dirty = true;
            }
            changed = cfg_rx.changed() => {
                if changed.is_ok() {
                    // The runtime's own receiver holds the same pending
                    // publication, so this applies rather than waits.
                    runtime.on_config_change().await;
                    model = build_model(runtime);
                    theme = Theme::from(&model.config.display);
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
        }

        if dirty && last_draw.elapsed() >= Duration::from_millis(50) {
            terminal
                .draw(|frame| {
                    render_quick(&model, frame.area(), frame.buffer_mut(), &theme);
                })
                .map_err(term_io)?;
            last_draw = Instant::now();
            dirty = false;
        }
    }
    Ok(())
}

// ─────────────────────────      --once           ────────────────────────

/// One frame, printed, exit (§14.6). Renders through the same
/// [`render_quick`] path into an offscreen buffer; opens no viewport,
/// enters no raw mode, subscribes to nothing. Stdout carries the frame
/// or nothing — every message goes to stderr.
pub fn run_once(cli: &Cli) -> Result<ExitCode, SalatError> {
    // §14.6's "configuration missing" row: the note goes to stderr and
    // the frame still prints, so the file is checked before `build`
    // (which is what writes the default) and the note printed after it
    // succeeds.
    let paths = resolve(cli.config.as_deref())?;
    let config_existed = paths.config.exists();

    let runtime = Runtime::build(cli)?;
    if !config_existed {
        eprintln!(
            "salat: created default configuration at {}",
            paths.config.display()
        );
    }

    // One revalidation, so §14.6's "API failure, cache warm" row is
    // reachable: cache-first boot serves the window with zero HTTP, and
    // only a refresh attempt can discover the API is down while the
    // cache keeps serving (§7.5's ETag revalidation keeps it cheap).
    runtime.block_on(runtime.refresh());
    let model = build_model(&runtime);

    // E302's warning, exactly when the design says it: the day was
    // served from cache *because* the API could not be reached.
    let api_unreachable = service_degraded(&model)
        && model
            .status
            .component(ComponentId::Network)
            .is_some_and(|component| component.health == Health::Degraded);
    if api_unreachable {
        eprintln!("salat: warning[E302]: api.aladhan.com unreachable, using cached times");
    }

    // Width: the terminal's when stdout is a TTY, else the 80×24
    // fallback that fixes piped output at 80 columns (§14.6).
    let stdout_is_tty = std::io::stdout().is_terminal();
    let (width, height) = if stdout_is_tty {
        cterm::size().unwrap_or((80, 24))
    } else {
        (80, 24)
    };

    // The shared floor (§16.14 item 9): the summary line *is* the
    // frame for these cases, so the exit table applies to it verbatim.
    if width < 80 || height < 10 {
        if let Some(line) = one_line_summary(&model) {
            println!("{line}");
        }
        return Ok(ExitCode::Ok);
    }

    let theme = Theme::from(&model.config.display);
    let area = Rect::new(0, 0, width, viewport_height(height));
    let mut buf = Buffer::empty(area);
    render_quick(&model, area, &mut buf, &theme);

    // §14.6's four conditions: stdout a TTY, no `--no-color`, `NO_COLOR`
    // unset — the resolved `color_mode` has no `none` variant left to
    // check (`--no-color`/`NO_COLOR` are that rule's carriers).
    let colored = stdout_is_tty && !cli.no_color && std::env::var_os("NO_COLOR").is_none();

    let mut stdout = std::io::stdout();
    write!(stdout, "{}", frame_to_string(&buf, colored))
        .and_then(|()| stdout.flush())
        .map_err(term_io)?;
    Ok(ExitCode::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crate::domain::testing as dt;
    use crate::ui::layout::Breakpoint;

    /// The §20.3 snapshot model: next Maghrib 19:21, countdown
    /// 01:42:37, states [Passed, Passed, Passed, Next, Upcoming].
    fn snapshot_model() -> UiModel {
        build_model(&testing::snapshot_runtime().runtime)
    }

    /// A row as the string of its cell symbols — the readable form the
    /// ladder and specimen assertions read.
    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect()
    }

    /// `v = min(14, h − 1)` (§15.3) at every height §16.14 item 9
    /// names, from the roomy default down to the floor's edge.
    #[test]
    fn viewport_height_follows_the_formula() {
        for (height, viewport) in [
            (24, 14),
            (15, 14),
            (14, 13),
            (13, 12),
            (12, 11),
            (11, 10),
            (10, 9),
            (9, 8),
        ] {
            assert_eq!(
                viewport_height(height),
                viewport,
                "v at terminal height {height}"
            );
        }
    }

    /// Every rung drops exactly the element §16.14 item 9 names —
    /// cumulatively: strip, then TODAY's rule, then its head, then one
    /// prayer row, then a second. At the snapshot the next prayer is
    /// Maghrib (row 3), so the prayer rows go Fajr first, then Dhuhr.
    #[test]
    fn ladder_drops_one_element_per_rung() {
        let rungs = [
            // (c, strip, head, rule, dropped)
            (13, true, true, true, vec![]),
            (12, false, true, true, vec![]),
            (11, false, true, false, vec![]),
            (10, false, false, false, vec![]),
            (9, false, false, false, vec![0]),
            (8, false, false, false, vec![0, 1]),
        ];
        for (rows, strip, head, rule, dropped) in rungs {
            let plan = quick_plan(rows, Some(3));
            assert_eq!(
                plan,
                QuickPlan {
                    strip,
                    today_head: head,
                    today_rule: rule,
                    dropped_rows: dropped,
                },
                "the plan at {rows} content rows"
            );
        }
    }

    /// The ladder's header, panel and at least three prayer rows are
    /// inviolable (§16.14 item 9): the drops stop at two rows even if
    /// a narrower area were ever rendered.
    #[test]
    fn ladder_never_drops_more_than_two_rows() {
        let plan = quick_plan(0, Some(3));
        assert!(plan.dropped_rows.len() <= 2);
    }

    /// `argmax |i − n|` with ties to the lower index (§16.14 item 9).
    /// With Asr next (row 2) the farthest rows are Fajr (0) and Isha
    /// (4) at distance 2 — the tie goes to Fajr, and Isha follows.
    #[test]
    fn prayer_row_ties_drop_the_lower_index() {
        assert_eq!(quick_plan(8, Some(2)).dropped_rows, vec![0, 4]);
    }

    /// The exhausted window (`next: None` — tomorrow's Fajr is next,
    /// outside today's rows): distance is measured from `−1`, so
    /// today's Isha goes first, then Maghrib.
    #[test]
    fn exhausted_window_drops_from_the_far_end() {
        assert_eq!(quick_plan(8, None).dropped_rows, vec![4, 3]);
    }

    /// The full 80×14 frame at the snapshot, against §14.6's specimen:
    /// header, panel, TODAY head, rule, five rows, strip.
    #[test]
    fn full_frame_renders_the_specimen_shape() {
        let model = snapshot_model();
        assert_eq!(model.breakpoint, Breakpoint::Small, "quick pins Small");
        let theme = Theme::from(&model.config.display);
        let area = Rect::new(0, 0, 80, 14);
        let mut buf = Buffer::empty(area);
        render_quick(&model, area, &mut buf, &theme);

        let header = row(&buf, 0);
        assert!(header.starts_with("SALAT"), "header: {header:?}");
        assert!(header.ends_with("17:38:23"), "clock: {header:?}");
        assert!(row(&buf, 1).starts_with("╭─ NEXT PRAYER"), "panel top");
        assert!(row(&buf, 2).contains("● MAGHRIB · 19:21"), "name row");
        assert!(row(&buf, 3).contains("01:42:37  REMAINING"), "countdown");
        let gauge = row(&buf, 4);
        assert!(
            gauge.contains('█') && gauge.contains('░'),
            "gauge: {gauge:?}"
        );
        assert!(row(&buf, 5).starts_with('╰'), "panel bottom");
        assert_eq!(row(&buf, 6).trim_end(), "TODAY", "head");
        assert!(row(&buf, 7).trim_end().chars().all(|c| c == '─'), "rule");
        for (y, name) in [
            (8, "Fajr"),
            (9, "Dhuhr"),
            (10, "Asr"),
            (11, "Maghrib"),
            (12, "Isha"),
        ] {
            let line = row(&buf, y);
            assert!(line.starts_with(name), "row {y}: {line:?}");
        }
        assert!(row(&buf, 11).contains("● next · in 01:42:37"), "note");
        let strip = row(&buf, 13);
        assert!(strip.starts_with("● svc"), "strip: {strip:?}");
        assert!(strip.contains("1448 AH"), "hijri group: {strip:?}");
        assert!(strip.contains("Sétif, Algeria · 23 Rabīʿ al-awwal 1448 AH"));
    }

    /// A narrower-than-80 area renders nothing — the belt to the run
    /// paths' floor brace (§16.14 item 9).
    #[test]
    fn narrower_than_eighty_renders_nothing() {
        let model = snapshot_model();
        let theme = Theme::from(&model.config.display);
        let area = Rect::new(0, 0, 79, 14);
        let mut buf = Buffer::empty(area);
        render_quick(&model, area, &mut buf, &theme);
        assert!(buf.content.iter().all(|cell| cell.symbol() == " "));
    }

    /// The degraded rungs (§16.14 item 9), by what sits where: the
    /// strip is the bottom row only while it survives (h ≥ 15); below
    /// that the bottom is the last prayer row, and the first dropped
    /// prayer row (Fajr, then Dhuhr) moves the survivors up.
    #[test]
    fn degraded_rungs_render_shorter_frames() {
        let model = snapshot_model();
        let theme = Theme::from(&model.config.display);
        // (terminal height, row 6, bottom row)
        let rungs = [
            (15, "TODAY", "● svc"),
            (14, "TODAY", "Isha"),
            (13, "TODAY", "Isha"),
            (12, "Fajr", "Isha"),
            (11, "Dhuhr", "Isha"),
            (10, "Asr", "Isha"),
        ];
        for (height, row6, bottom) in rungs {
            let viewport = viewport_height(height);
            let area = Rect::new(0, 0, 80, viewport);
            let mut buf = Buffer::empty(area);
            render_quick(&model, area, &mut buf, &theme);
            assert!(
                row(&buf, 6).starts_with(row6),
                "row 6 at h={height}: {:?}",
                row(&buf, 6)
            );
            assert!(
                row(&buf, viewport - 1).starts_with(bottom),
                "bottom at h={height}: {:?}",
                row(&buf, viewport - 1)
            );
            // The header and the panel survive every rung.
            assert!(row(&buf, 0).starts_with("SALAT"));
            assert!(row(&buf, 1).starts_with("╭─ NEXT PRAYER"));
        }
    }

    /// The §14.6 one-line summary at the snapshot: four fields, fixed
    /// order, nothing else.
    #[test]
    fn one_line_summary_is_four_fields() {
        let model = snapshot_model();
        assert_eq!(
            one_line_summary(&model).as_deref(),
            Some("MAGHRIB · 19:21 · in 01:42:37")
        );
    }

    /// After Isha the next prayer is tomorrow's Fajr (05:10) and the
    /// summary names it; the ladder's row drops then run from the far
    /// end of the day (Isha, Maghrib), never the next row.
    #[test]
    fn after_isha_names_tomorrows_fajr() {
        let fixture = testing::snapshot_runtime();
        fixture.clock.set(dt::utc(dt::TODAY, 21, 38, 23));
        let model = build_model(&fixture.runtime);
        assert_eq!(
            one_line_summary(&model).as_deref(),
            Some("FAJR · 05:10 · in 07:31:37")
        );
        assert_eq!(next_index(&model), Some(0));
        assert_eq!(quick_plan(8, next_index(&model)).dropped_rows, vec![4, 3]);
    }

    /// Plain output (§14.6): every line at most 80 display columns, no
    /// trailing whitespace, and not one escape byte.
    #[test]
    fn plain_frame_is_trimmed_and_escape_free() {
        let model = snapshot_model();
        let theme = Theme::from(&model.config.display);
        let area = Rect::new(0, 0, 80, 14);
        let mut buf = Buffer::empty(area);
        render_quick(&model, area, &mut buf, &theme);

        let plain = frame_to_string(&buf, false);
        assert!(!plain.contains('\x1b'), "escape byte in plain output");
        for line in plain.lines() {
            assert!(line.width() <= 80, "line wider than 80: {line:?}");
            assert_eq!(line.trim_end(), line, "trailing whitespace: {line:?}");
        }
        assert_eq!(plain.lines().count(), 14, "one line per viewport row");
    }

    /// Colored output carries SGR sequences — and resets, so the
    /// terminal is never left mid-style at a line's end.
    #[test]
    fn colored_frame_carries_sgr_and_resets() {
        let model = snapshot_model();
        let theme = Theme::from(&model.config.display);
        let area = Rect::new(0, 0, 80, 14);
        let mut buf = Buffer::empty(area);
        render_quick(&model, area, &mut buf, &theme);

        let colored = frame_to_string(&buf, true);
        assert!(colored.contains("\x1b["), "no SGR in colored output");
        for line in colored.lines() {
            if line.contains("\x1b[") {
                assert!(line.ends_with("\x1b[0m"), "unterminated style: {line:?}");
            }
        }
    }

    /// The encoder's SGR parameter tables: named colours map to the
    /// 3x/4x/9x/10x ranges, indexed to `38;5;n`, truecolour to
    /// `38;2;r;g;b`, and bold rides along.
    #[test]
    fn sgr_encodes_the_ansi_tables() {
        assert_eq!(
            sgr(CellStyle {
                fg: Some(Color::Green),
                ..Default::default()
            }),
            "\x1b[32m"
        );
        assert_eq!(
            sgr(CellStyle {
                bg: Some(Color::Green),
                ..Default::default()
            }),
            "\x1b[42m"
        );
        assert_eq!(
            sgr(CellStyle {
                fg: Some(Color::LightGreen),
                ..Default::default()
            }),
            "\x1b[92m"
        );
        assert_eq!(
            sgr(CellStyle {
                bg: Some(Color::LightGreen),
                ..Default::default()
            }),
            "\x1b[102m"
        );
        assert_eq!(
            sgr(CellStyle {
                fg: Some(Color::Indexed(208)),
                ..Default::default()
            }),
            "\x1b[38;5;208m"
        );
        assert_eq!(
            sgr(CellStyle {
                fg: Some(Color::Rgb(1, 2, 3)),
                bold: true,
                ..Default::default()
            }),
            "\x1b[38;2;1;2;3;1m"
        );
        assert_eq!(sgr(CellStyle::default()), "\x1b[m");
    }

    /// §16.14 item 7's keys: q, Esc and Ctrl-C quit, Ctrl-L redraws,
    /// and only `Press` events count (§15.4).
    #[test]
    fn key_actions_follow_the_design() {
        let key = |code: KeyCode, modifiers: KeyModifiers| {
            Event::Key(crossterm::event::KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            })
        };
        assert!(matches!(
            key_action(&key(KeyCode::Char('q'), KeyModifiers::NONE)),
            KeyAction::Quit
        ));
        assert!(matches!(
            key_action(&key(KeyCode::Esc, KeyModifiers::NONE)),
            KeyAction::Quit
        ));
        assert!(matches!(
            key_action(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            KeyAction::Quit
        ));
        assert!(matches!(
            key_action(&key(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            KeyAction::Redraw
        ));
        assert!(matches!(
            key_action(&key(KeyCode::Char('x'), KeyModifiers::NONE)),
            KeyAction::Ignore
        ));

        let mut release = key(KeyCode::Char('q'), KeyModifiers::NONE);
        if let Event::Key(ref mut event) = release {
            event.kind = KeyEventKind::Release;
        }
        assert!(matches!(key_action(&release), KeyAction::Ignore));
    }

    /// §15.3's third restoration path: the panic hook. Raw mode needs
    /// a real terminal, so this runs under a pty —
    /// `script -qec 'cargo test --lib -- --ignored panic_hook --exact'`
    /// — and is ignored by the plain suite, where `TerminalGuard::new`
    /// has no terminal to enter.
    #[test]
    #[ignore = "needs a pty (run under script(1))"]
    fn panic_hook_restores_the_terminal() {
        let _guard = TerminalGuard::new().expect("a terminal to enter raw mode on");
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

    /// §14.6's five golden files and §20's three snapshots — the
    /// `--once` surface byte-for-byte. The goldens live in
    /// `tests/golden/`, the snapshots in `tests/snapshots/`; regenerate
    /// the goldens with `UPDATE_GOLDEN=1 cargo test golden`, and accept
    /// snapshots with `INSTA_UPDATE=always` (or `cargo insta accept`).
    mod golden {
        use super::*;
        use std::path::PathBuf;

        /// The `--once` frame at a terminal size: the same render path
        /// [`run_once`] walks, offscreen.
        fn once_frame(model: &UiModel, width: u16, height: u16, colored: bool) -> String {
            let theme = Theme::from(&model.config.display);
            let area = Rect::new(0, 0, width, viewport_height(height));
            let mut buf = Buffer::empty(area);
            render_quick(model, area, &mut buf, &theme);
            frame_to_string(&buf, colored)
        }

        /// A line with its SGR sequences stripped, so the colour
        /// golden's *visible* width can be measured.
        fn strip_sgr(line: &str) -> String {
            let mut out = String::new();
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    for c in chars.by_ref() {
                        if c == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }

        /// §14.6's two width assertions, on every golden: no line
        /// wider than the frame it was rendered into, and no trailing
        /// whitespace (measured after escapes are stripped, so a
        /// coloured trailing space with a visible ground is still
        /// content, not whitespace).
        fn assert_widths(rendered: &str, limit: usize) {
            for line in rendered.lines() {
                let visible = strip_sgr(line);
                assert!(
                    visible.width() <= limit,
                    "line wider than {limit}: {visible:?}"
                );
                assert_eq!(
                    visible.trim_end(),
                    visible,
                    "trailing whitespace: {visible:?}"
                );
            }
        }

        /// Compare against (or, under `UPDATE_GOLDEN=1`, write) a
        /// golden file, byte for byte.
        fn golden(name: &str, rendered: &str) {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/golden")
                .join(name);
            if std::env::var_os("UPDATE_GOLDEN").is_some() {
                std::fs::write(&path, rendered).expect("the golden directory exists");
            }
            let expected = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("golden {name}: {error} — run UPDATE_GOLDEN=1"));
            assert_eq!(rendered, expected, "golden {name} differs");
        }

        /// An insta snapshot filed under `tests/snapshots/` — §20's
        /// reviewable form of the same frames.
        fn snap(name: &str, rendered: &str) {
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(name, rendered));
        }

        fn golden_frame_case(name: &str, width: u16, height: u16, colored: bool) {
            let model = snapshot_model();
            let rendered = once_frame(&model, width, height, colored);
            assert_widths(&rendered, width as usize);
            golden(name, &rendered);
        }

        /// The default invocation's frame: a common 80×24 terminal.
        #[test]
        fn golden_quick_once_80x24() {
            golden_frame_case("quick_once_80x24.txt", 80, 24, false);
        }

        /// Roomier terminals widen the frame, not the layout.
        #[test]
        fn golden_quick_once_100x30() {
            golden_frame_case("quick_once_100x30.txt", 100, 30, false);
        }

        #[test]
        fn golden_quick_once_120x35() {
            golden_frame_case("quick_once_120x35.txt", 120, 35, false);
        }

        /// The coloured form: same frame, SGR sequences where the
        /// styles change.
        #[test]
        fn golden_quick_once_80x24_color() {
            golden_frame_case("quick_once_80x24_color.txt", 80, 24, true);
        }

        /// The one-line summary — the floor's frame, ≤ 38 columns
        /// (29 plain, ` · cached` at the widest).
        #[test]
        fn golden_quick_once_oneline() {
            let model = snapshot_model();
            let rendered = one_line_summary(&model).expect("the snapshot has a next prayer");
            assert_widths(&rendered, 38);
            golden("quick_once_oneline.txt", &format!("{rendered}\n"));
        }

        /// §20's first snapshot: the full frame at 80 columns.
        #[test]
        fn snap_quick_full_80() {
            let model = snapshot_model();
            snap("quick_full_80", &once_frame(&model, 80, 24, false));
        }

        /// §20's second: a ladder-degraded frame — terminal height 11
        /// drops the strip, the rule, the head and Fajr's row.
        #[test]
        fn snap_quick_ladder_h11() {
            let model = snapshot_model();
            snap("quick_ladder_h11", &once_frame(&model, 80, 11, false));
        }

        /// §20's third: after Isha — every row passed, the panel and
        /// summary naming tomorrow's Fajr.
        #[test]
        fn snap_quick_after_isha() {
            let fixture = testing::snapshot_runtime();
            fixture.clock.set(dt::utc(dt::TODAY, 21, 38, 23));
            let model = build_model(&fixture.runtime);
            snap("quick_after_isha", &once_frame(&model, 80, 24, false));
        }
    }
}
