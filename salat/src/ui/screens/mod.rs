//! The screens of §16, one module each (§19). Phase 6 implemented the
//! [`loading`] and [`states`] screens fully and [`error`] minimally
//! (boot failures); every placeholder body has since been replaced by
//! its phase: dashboard and status (Phase 7/12), the overlays (Phase
//! 10), settings + notif + location + confirm (Phase 11).

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
