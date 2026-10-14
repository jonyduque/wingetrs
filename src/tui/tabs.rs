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

/// Whether the boxes do not fit `area` and the titles are drawn as one plain line.
fn is_narrow(area: Rect, labels: &[TabLabel]) -> bool {
    let pad = 2 * usize::from(TAB_PAD_X);
    let total: usize = labels
        .iter()
        .map(|l| Span::raw(l.text.as_str()).width() + pad + 2)
        .sum();
    total + 1 > usize::from(area.width)
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
    let at = |x: usize, w: usize, y: u16, h: u16| {
        let x = area.x.saturating_add(u16::try_from(x).unwrap_or(u16::MAX));
        let w = u16::try_from(w).unwrap_or(u16::MAX);
        Rect::new(x, y, w, h).intersection(area)
    };
    let mut x = 0;
    let mut out = Vec::new();
    if is_narrow(area, labels) {
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

/// Draws the three header lines into `area` (needs height 3 and width >= 2 to draw fully);
/// every title is painted at the rectangle `tab_rects` gives it.
pub fn draw_tabs(frame: &mut Frame, area: Rect, labels: &[TabLabel], active: usize, theme: &Theme) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let width = usize::from(area.width);
    let pad = " ".repeat(usize::from(TAB_PAD_X));
    let rects = tab_rects(area, labels);
    let edge = theme.accent();
    let buf = frame.buffer_mut();
    let line_at = |buf: &mut ratatui::buffer::Buffer, x: u16, dy: u16, spans: Vec<Span>| {
        if dy < area.height {
            let room = area.right().saturating_sub(x);
            buf.set_line(x, area.y + dy, &Line::from(spans), room);
        }
    };
    if is_narrow(area, labels) {
        // Narrow: one line of labels, closed by a rounded top edge of the window.
        for (i, (l, r)) in labels.iter().zip(&rects).enumerate() {
            if i > 0 {
                line_at(
                    buf,
                    rects[i - 1].right(),
                    1,
                    vec![Span::styled(" · ", theme.fg_dim())],
                );
            }
            line_at(
                buf,
                r.x,
                1,
                vec![Span::styled(
                    l.text.clone(),
                    label_style(labels, i, active, theme),
                )],
            );
        }
        let top = format!("╭{}╮", "─".repeat(width - 2));
        line_at(buf, area.x, 2, vec![Span::styled(top, edge)]);
        return;
    }
    for (i, (l, r)) in labels.iter().zip(&rects).enumerate() {
        let w = usize::from(r.width) - 2;
        let interior = format!("{pad}{}{pad}", l.text);
        let (left, mid, right) = if i == active {
            (if i == 0 { "│" } else { "┘" }, " ".repeat(w), "└")
        } else {
            (if i == 0 { "├" } else { "┴" }, "─".repeat(w), "┴")
        };
        line_at(
            buf,
            r.x,
            0,
            vec![Span::styled(format!("╭{}╮", "─".repeat(w)), edge)],
        );
        line_at(
            buf,
            r.x,
            1,
            vec![
                Span::styled("│", edge),
                Span::styled(interior, label_style(labels, i, active, theme)),
                Span::styled("│", edge),
            ],
        );
        line_at(
            buf,
            r.x,
            2,
            vec![Span::styled(format!("{left}{mid}{right}"), edge)],
        );
    }
    let used = rects.last().map_or(0, |r| usize::from(r.right() - area.x));
    let rest = width - used;
    let rest_x = area.x + u16::try_from(used).unwrap_or(0);
    line_at(buf, rest_x, 0, vec![Span::raw(" ".repeat(rest))]);
    let badge_w = Span::raw(BADGE).width();
    if rest > badge_w {
        line_at(
            buf,
            rest_x,
            1,
            vec![
                Span::raw(" ".repeat(rest - badge_w - 1)),
                Span::styled(BADGE, theme.badge()),
                Span::raw(" "),
            ],
        );
    }
    line_at(
        buf,
        rest_x,
        2,
        vec![Span::styled(format!("{}╮", "─".repeat(rest - 1)), edge)],
    );
}
