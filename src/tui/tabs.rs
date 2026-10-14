//! Bubble Tea `tabs` style header: rounded boxes, the active one open into the window below.
use crate::tui::layout::TAB_PAD_X;
use crate::tui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

const BADGE: &str = " wingetrs ";

/// One tab label; `warn` colors it as a warning while the tab is inactive.
pub struct TabLabel {
    pub text: String,
    pub warn: bool,
}

/// Label style: the active tab is bold accent; an inactive Updates tab with a count is `warn`.
fn label_style(labels: &[TabLabel], i: usize, active: usize, theme: &Theme) -> Style {
    if i == active {
        theme.accent().add_modifier(Modifier::BOLD)
    } else if labels[i].warn {
        theme.warn()
    } else {
        theme.fg_dim()
    }
}

/// Screen rectangle of each tab title inside `area`, the same geometry `draw_tabs` paints:
/// the whole box (3 lines) in the wide form, the label text on the middle line when narrow.
pub fn tab_rects(area: Rect, labels: &[TabLabel]) -> Vec<Rect> {
    if area.width < 2 || area.height == 0 {
        return Vec::new();
    }
    let pad = 2 * usize::from(TAB_PAD_X);
    let widths: Vec<usize> = labels
        .iter()
        .map(|l| Span::raw(l.text.as_str()).width())
        .collect();
    let total: usize = widths.iter().map(|w| w + pad + 2).sum();
    let at = |x: usize, w: usize, y: u16, h: u16| {
        let x = area.x.saturating_add(u16::try_from(x).unwrap_or(u16::MAX));
        let w = u16::try_from(w).unwrap_or(u16::MAX);
        Rect::new(x, y, w, h).intersection(area)
    };
    let mut x = 0;
    let mut out = Vec::new();
    if total + 1 > usize::from(area.width) {
        x += usize::from(TAB_PAD_X);
        for w in &widths {
            out.push(at(x, *w, area.y.saturating_add(1), 1));
            x += w + 3;
        }
    } else {
        for w in &widths {
            out.push(at(x, w + pad + 2, area.y, area.height.min(3)));
            x += w + pad + 2;
        }
    }
    out
}

/// Draws the three header lines into `area` (needs height 3 and width >= 2 to draw fully).
pub fn draw_tabs(frame: &mut Frame, area: Rect, labels: &[TabLabel], active: usize, theme: &Theme) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let width = usize::from(area.width);
    let pad = " ".repeat(usize::from(TAB_PAD_X));
    let interiors: Vec<String> = labels
        .iter()
        .map(|l| format!("{pad}{}{pad}", l.text))
        .collect();
    let widths: Vec<usize> = interiors
        .iter()
        .map(|s| Span::raw(s.as_str()).width())
        .collect();
    let total: usize = widths.iter().map(|w| w + 2).sum();
    let edge = theme.accent();
    let mut rows: [Vec<Span>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    if total + 1 > width {
        // Narrow: one line of labels, closed by a rounded top edge of the window.
        let mut spans = vec![Span::raw(pad.clone())];
        for (i, l) in labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", theme.fg_dim()));
            }
            spans.push(Span::styled(
                l.text.clone(),
                label_style(labels, i, active, theme),
            ));
        }
        rows[1] = spans;
        rows[2] = vec![Span::styled(format!("╭{}╮", "─".repeat(width - 2)), edge)];
    } else {
        for (i, interior) in interiors.iter().enumerate() {
            let w = widths[i];
            rows[0].push(Span::styled(format!("╭{}╮", "─".repeat(w)), edge));
            rows[1].push(Span::styled("│", edge));
            rows[1].push(Span::styled(
                interior.clone(),
                label_style(labels, i, active, theme),
            ));
            rows[1].push(Span::styled("│", edge));
            let (left, mid, right) = if i == active {
                (if i == 0 { "│" } else { "┘" }, " ".repeat(w), "└")
            } else {
                (if i == 0 { "├" } else { "┴" }, "─".repeat(w), "┴")
            };
            rows[2].push(Span::styled(format!("{left}{mid}{right}"), edge));
        }
        let rest = width - total;
        rows[0].push(Span::raw(" ".repeat(rest)));
        let badge_w = Span::raw(BADGE).width();
        if rest > badge_w {
            rows[1].push(Span::raw(" ".repeat(rest - badge_w - 1)));
            rows[1].push(Span::styled(BADGE, theme.badge()));
            rows[1].push(Span::raw(" "));
        }
        rows[2].push(Span::styled(format!("{}╮", "─".repeat(rest - 1)), edge));
    }
    for (y, spans) in rows.into_iter().enumerate() {
        let Ok(dy) = u16::try_from(y) else { continue };
        if dy >= area.height {
            break;
        }
        frame
            .buffer_mut()
            .set_line(area.x, area.y + dy, &Line::from(spans), area.width);
    }
}
