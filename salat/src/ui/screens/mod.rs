//! The screens of §16, one module each (§19). Phase 6 implements the
//! [`loading`] and [`states`] screens fully, [`error`] minimally (boot
//! failures — Phase 12 completes it), and every other module as a
//! title-only placeholder body whose phase replaces it: dashboard and
//! status (Phase 7), the overlays (Phase 10), settings + notif +
//! location + confirm (Phase 11).

pub mod adhan_alert;
pub mod confirm;
pub mod dashboard;
pub mod error;
pub mod loading;
pub mod location;
pub mod mosque_reminder;
pub mod notif_settings;
pub mod prayer_config;
pub mod settings;
pub mod states;
pub mod status;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use super::shell::{Overlay, Screen, Shell};

/// Render the focused screen's body into `area` (the chrome is already
/// drawn around it).
pub fn render(
    screen: &Screen,
    model: &UiModel,
    shell: &Shell,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    match screen {
        Screen::Dashboard => {
            dashboard::render(model, &shell.dashboard, area, buf, theme, breakpoint)
        }
        Screen::Settings => settings::render(model, &shell.settings, area, buf, theme, breakpoint),
        Screen::PrayerConfig => {
            prayer_config::render(model, &shell.prayer_cfg, area, buf, theme, breakpoint)
        }
        Screen::Status => status::render(model, &shell.status, area, buf, theme, breakpoint),
        Screen::NotifSettings => {
            notif_settings::render(model, &shell.notif, area, buf, theme, breakpoint)
        }
        Screen::Location => location::render(model, &shell.location, area, buf, theme, breakpoint),
        Screen::Confirm { path, recalculated } => {
            confirm::render(path, *recalculated, area, buf, theme)
        }
        Screen::Error(error) => error::render(error, area, buf, theme, breakpoint),
        Screen::Loading => loading::render(area, buf, theme, &shell.boot, model.now, breakpoint),
        Screen::States => states::render(model, area, buf, theme, breakpoint),
    }
}

/// Render an overlay over the whole frame (§3.3: no header, no
/// footer). §16.7's full-screen Adhan wash and §16.8's centered
/// mosque panel — the shell raised one from a scheduler fire and the
/// frame is rebuilt from the payload alone.
pub fn render_overlay(
    overlay: Overlay,
    model: &UiModel,
    _shell: &Shell,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    match overlay {
        Overlay::AdhanAlert { .. } => adhan_alert::render(overlay, model, area, buf, theme),
        Overlay::MosqueReminder { .. } => mosque_reminder::render(overlay, model, area, buf, theme),
    }
}

/// A placeholder body: the screen's design title, centered — the shape
/// every unimplemented screen shares while its phase is pending.
pub(crate) fn title_only(title: &str, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.width < 4 || area.height < 1 {
        return;
    }
    let line = Line::from(Span::styled(title, Style::new().fg(theme.text_sub)));
    let y = area.y + area.height / 2;
    let width = title.chars().count() as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    line.render(Rect::new(x, y, area.width, 1), buf);
}
