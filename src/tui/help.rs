//! Contextual key hints for the bottom help bar.
use crate::tui::app::{App, Focus, Tab, ToastKind};
use crate::tui::theme::Theme;
use ratatui::text::{Line, Span};

const SEP: &str = " • ";
const ELLIPSIS: &str = " …";

/// Key hints for the current tab and focus, most useful first, `?` last.
pub fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    if app.focus == Focus::Details {
        out.push(("esc", "voltar"));
    }
    out.push(("↑/↓", "mover"));
    out.push(("←/→", "aba"));
    out.push(("enter", "detalhes"));
    match app.tab {
        Tab::Search => out.push(("i", "instalar")),
        Tab::Installed => out.push(("u", "remover")),
        Tab::Updates => out.push(("U", "atualizar")),
    }
    out.push(("ctrl+s", "fonte"));
    if app.toasts.iter().any(|t| t.kind == ToastKind::Error) {
        out.push(("esc", "fecha erro"));
    }
    out.push(("?", "ajuda"));
    out
}

/// `key desc` hints joined by a dim bullet; whole hints only, ending in `…` when some are left out.
pub fn help_line(hints: &[(&str, &str)], width: usize, theme: &Theme) -> Line<'static> {
    let sep = Span::styled(SEP, theme.help_sep());
    let ellipsis = Span::styled(ELLIPSIS, theme.help_sep());
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let mut cut = false;
    for (i, (k, d)) in hints.iter().enumerate() {
        let key = Span::styled((*k).to_string(), theme.help_key());
        let gap = Span::styled(" ", theme.help_desc());
        let desc = Span::styled((*d).to_string(), theme.help_desc());
        let hint_w = key.width() + gap.width() + desc.width();
        let lead = if i > 0 { sep.width() } else { 0 };
        let reserve = if i + 1 < hints.len() {
            ellipsis.width()
        } else {
            0
        };
        if used + lead + hint_w + reserve > width {
            cut = true;
            break;
        }
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.extend([key, gap, desc]);
        used += lead + hint_w;
    }
    if cut && used + ellipsis.width() <= width {
        spans.push(ellipsis);
    }
    Line::from(spans)
}
