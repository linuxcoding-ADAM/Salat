//! The previous→next timeline (§16.1 item 10): the gauge with its
//! three labels — the previous prayer left, the percentage centre in
//! green, the next prayer right, all `textDim` and tabular otherwise.
//!
//! One widget because the three labels are the gauge's legend, not
//! free text: they say what the `█` run measures. The bar itself is
//! [`super::bar`]'s; this module places it and annotates it.
//!
//! A one-row area renders the gauge alone — the 80×24 dashboard's
//! case (§4.4: the panel has no room for a legend row, and A1 already
//! spent its slack on the countdown).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::theme::Theme;

use super::bar;

/// What the labels say: the prayers the gauge runs between, each as
/// `(name, time)` — already formatted, `None` on the degenerate
/// intervals §6.5 answers with an empty gauge.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timeline<'a> {
    pub previous: Option<(&'a str, &'a str)>,
    pub next: Option<(&'a str, &'a str)>,
}

/// Draw the gauge (and, given a second row, its legend) into `area`,
/// centred, `width` cells wide (34/32/40 by breakpoint — the design's
/// `Bar` widths).
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    progress: f64,
    timeline: &Timeline<'_>,
    width: u16,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Centred: the design's Bar sits mid-panel, not flush left.
    let width = width.min(area.width);
    let gauge = Rect::new(area.x + (area.width - width) / 2, area.y, width, 1);
    bar::render(gauge, buf, theme, progress, width, None);

    if area.height < 2 {
        return;
    }

    let dim = Style::new().fg(theme.text_dim);
    let mut left = String::new();
    if let Some((name, time)) = timeline.previous {
        left.push_str(name);
        left.push(' ');
        left.push_str(time);
    }
    let mut right = String::new();
    if let Some((name, time)) = timeline.next {
        right.push_str(name);
        right.push(' ');
        right.push_str(time);
    }
    let pct = format!("{}%", (progress.clamp(0.0, 1.0) * 100.0).round() as u32);

    // The legend spans the area: previous at the left edge, the
    // percentage centred, next at the right — the design's
    // `justify-content: space-between`.
    let legend = Rect::new(area.x, area.y + 1, area.width, 1);
    let left_spans = Line::from(Span::styled(left, dim));
    let pct_width = pct.chars().count() as u16;
    let pct_x = legend.x + legend.width.saturating_sub(pct_width) / 2;
    left_spans.render(legend, buf);
    let right_width = right.chars().count() as u16;
    Line::from(Span::styled(right, dim)).render(
        Rect::new(
            legend.x + legend.width.saturating_sub(right_width),
            legend.y,
            right_width,
            1,
        ),
        buf,
    );
    Line::from(Span::styled(pct, Style::new().fg(theme.green)))
        .render(Rect::new(pct_x, legend.y, pct_width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect()
    }

    /// The specimen's timeline (§16.1 item 10): `Asr 16:23` left,
    /// `42%` green centre, `Maghrib 19:21` right, gauge centred above.
    #[test]
    fn the_legend_labels_the_gauge() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 44, 2));
        render(
            buf.area,
            &mut buf,
            &theme,
            4523.0 / 10_680.0,
            &Timeline {
                previous: Some(("Asr", "16:23")),
                next: Some(("Maghrib", "19:21")),
            },
            40,
        );
        let legend = row(&buf, 1);
        assert!(legend.starts_with("Asr 16:23"), "left: {legend:?}");
        assert!(legend.ends_with("Maghrib 19:21"), "right: {legend:?}");
        let pct_x = (44 - 3) / 2;
        assert_eq!(&legend[pct_x as usize..pct_x as usize + 3], "42%");
        assert_eq!(buf[(pct_x, 1)].style().fg, Some(theme.green));

        // The gauge row: 42 % of 40 cells is 17 `█`, centred.
        let gauge: Vec<char> = row(&buf, 0).chars().collect();
        let offset = (44 - 40) / 2;
        let filled = gauge[offset..offset + 40]
            .iter()
            .filter(|cell| **cell == '█')
            .count();
        assert_eq!(filled, 17, "gauge: {gauge:?}");
    }

    /// One row renders the gauge alone — the 80×24 case (§4.4).
    #[test]
    fn a_single_row_renders_the_gauge_only() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        render(
            buf.area,
            &mut buf,
            &theme,
            0.5,
            &Timeline {
                previous: Some(("Asr", "16:23")),
                next: Some(("Maghrib", "19:21")),
            },
            34,
        );
        let text = row(&buf, 0);
        assert!(text.contains('█'), "the gauge drew: {text:?}");
        assert!(!text.contains("Asr"), "no legend in one row: {text:?}");
    }
}
