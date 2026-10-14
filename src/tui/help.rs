//! Contextual key hints for the bottom help bar.
use crate::tui::app::{App, ShowFilter, SortKey, SourceFilter, Tab, ToastKind};
use crate::tui::theme::Theme;
use ratatui::text::{Line, Span};

const SEP: &str = " • ";
const ELLIPSIS: &str = " …";

fn sort_hint(app: &App) -> &'static str {
    match app.sort_key() {
        SortKey::Name => "trocar ordem: nome",
        SortKey::Source => "trocar ordem: fonte",
        SortKey::State => "trocar ordem: estado",
    }
}

fn source_hint(app: &App) -> &'static str {
    match app.filter {
        SourceFilter::All => "gerenciador: todos",
        SourceFilter::Winget => "gerenciador: winget",
        SourceFilter::Scoop => "gerenciador: scoop",
    }
}

fn show_hint(app: &App) -> &'static str {
    match app.show {
        ShowFilter::All => "mostrar: todos",
        ShowFilter::Unmatched => "mostrar: sem correspondência",
        ShowFilter::Duplicates => "mostrar: duplicados",
    }
}

/// Key hints for the current tab and focus, most useful first, `?` last.
pub fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    let queue = app.running.is_some() || !app.queue.is_empty();
    if app.tab == Tab::Settings {
        let mut out = vec![("enter", "mudar")];
        if queue {
            out.push(("f", "fila"));
        }
        out.push(("?", "ajuda"));
        return out;
    }
    let mut out = Vec::new();
    match app.tab {
        Tab::Search => {
            out.push(("enter", "instalar"));
        }
        Tab::Installed => {
            out.push(("enter", "ações"));
            out.push(("u", "remover"));
            out.push(("o", sort_hint(app)));
        }
        Tab::Updates => {
            out.push(("enter", "atualizar"));
            out.push(("o", sort_hint(app)));
        }
        Tab::Settings => {}
    }
    if app.query.is_empty() {
        out.push(("espaço", "marcar"));
    } else {
        out.push(("ins", "marcar"));
    }
    out.push(("ctrl+s", source_hint(app)));
    if app.tab == Tab::Installed {
        out.push(("ctrl+e", show_hint(app)));
    }
    if queue {
        out.push(("f", "fila"));
    }
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
