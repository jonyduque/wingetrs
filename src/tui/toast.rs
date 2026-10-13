//! Toast boxes: stacked at the top right of the main window, newest first.
use crate::tui::app::{Toast, ToastKind};
use crate::tui::layout::{BORDER, TOAST_MARGIN_X, TOAST_MARGIN_Y, TOAST_PAD_X, inset};
use crate::tui::theme::{Glyphs, Theme};
use crate::tui::view::clip;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

/// Widest a toast gets, borders included.
const TOAST_MAX_WIDTH: u16 = 44;
/// Lines of message text in a toast; the rest is cut with `…`.
const TOAST_TEXT_LINES: usize = 3;

fn kind_style(theme: &Theme, kind: ToastKind) -> Style {
    match kind {
        ToastKind::Success => theme.ok(),
        ToastKind::Info => theme.accent(),
        ToastKind::Warning => theme.warn(),
        ToastKind::Error => theme.err(),
    }
}

fn kind_label(g: &Glyphs, kind: ToastKind) -> (&'static str, &'static str) {
    match kind {
        ToastKind::Success => (g.toast_ok, "Sucesso"),
        ToastKind::Info => (g.toast_info, "Info"),
        ToastKind::Warning => (g.toast_warn, "Aviso"),
        ToastKind::Error => (g.toast_err, "Erro"),
    }
}

fn cells(s: &str) -> usize {
    Span::raw(s).width()
}

/// Greedy word wrap measured in terminal cells; words wider than `width` are split.
fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut used = 0usize;
    for word in text.split_whitespace() {
        let w = cells(word);
        let gap = usize::from(!line.is_empty());
        if used + gap + w <= width {
            if gap == 1 {
                line.push(' ');
            }
            line.push_str(word);
            used += gap + w;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        for c in word.chars() {
            let cw = cells(c.encode_utf8(&mut [0; 4]));
            if used + cw > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(c);
            used += cw;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// At most `TOAST_TEXT_LINES` wrapped lines; the last one ends in `…` when text was left out.
fn text_lines(text: &str, width: usize) -> Vec<String> {
    let mut lines = wrap_cells(text, width);
    if lines.len() > TOAST_TEXT_LINES {
        let rest = lines.split_off(TOAST_TEXT_LINES - 1).join(" ");
        lines.push(clip(&rest, width));
    }
    lines
}

fn toast_lines(toast: &Toast, width: usize, theme: &Theme, g: &Glyphs) -> Vec<Line<'static>> {
    let style = kind_style(theme, toast.kind);
    let (glyph, title) = kind_label(g, toast.kind);
    let mut head = vec![Span::styled(
        format!("{glyph} {title}"),
        style.add_modifier(Modifier::BOLD),
    )];
    if toast.count > 1 {
        head.push(Span::styled(format!(" ×{}", toast.count), style));
    }
    let mut lines = vec![Line::from(head)];
    lines.extend(
        text_lines(&toast.text, width)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, theme.fg()))),
    );
    if toast.kind == ToastKind::Error {
        lines.push(Line::from(Span::styled("esc fecha", theme.help_desc())));
    }
    lines
}

/// Draws `toasts` (newest first) down from the top right of `window`; what does not fit
/// in the window's height is left out. Returns the rectangles drawn.
pub fn draw_toasts(
    frame: &mut Frame,
    window: Rect,
    toasts: &[Toast],
    theme: &Theme,
    glyphs: &Glyphs,
) -> Vec<Rect> {
    let mut drawn = Vec::new();
    let w = TOAST_MAX_WIDTH.min(window.width.saturating_sub(2 * TOAST_MARGIN_X));
    let text_w = w.saturating_sub(2 * (BORDER + TOAST_PAD_X));
    if text_w == 0 {
        return drawn;
    }
    let x = window.right().saturating_sub(TOAST_MARGIN_X + w);
    let bottom = window.bottom().saturating_sub(BORDER);
    let mut y = window.y.saturating_add(TOAST_MARGIN_Y);
    for toast in toasts {
        let lines = toast_lines(toast, usize::from(text_w), theme, glyphs);
        let h = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2 * BORDER);
        if y.saturating_add(h) > bottom {
            break;
        }
        let rect = Rect::new(x, y, w, h);
        let blk = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(kind_style(theme, toast.kind));
        let inner = inset(blk.inner(rect), TOAST_PAD_X, 0);
        frame.render_widget(Clear, rect);
        frame.render_widget(blk, rect);
        frame.render_widget(Paragraph::new(lines), inner);
        drawn.push(rect);
        y = y.saturating_add(h);
    }
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_counts_cells_and_cuts_after_three_lines() {
        assert_eq!(wrap_cells("ab cd ef", 5), vec!["ab cd", "ef"]);
        // Wide chars take two cells each.
        assert_eq!(wrap_cells("日本語", 4), vec!["日本", "語"]);
        let lines = text_lines(&"palavra ".repeat(20), 10);
        assert_eq!(lines.len(), 3);
        assert!(lines[2].ends_with('…'), "{lines:?}");
        assert!(lines.iter().all(|l| cells(l) <= 10), "{lines:?}");
    }
}
