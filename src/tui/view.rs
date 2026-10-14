//! Pure rendering of `App` (spec §7.1). Colors come only from `Theme` tokens.
use crate::core::{PackageDetails, SourceKind};
use crate::install::residue::OTHER_INSTALL_NOTE;
use crate::installed::correlate::{Correlation, ListRow, UpdateState};
use crate::tui::app::{App, ITEM_ROWS, Modal, RunProgress, Tab, queue_rows};
use crate::tui::help::{help_line, hints};
use crate::tui::icons::IconProtocol;
use crate::tui::layout::{
    BAR_PAD_X, BORDER, DETAIL_ICON_COLS, DETAIL_ICON_GAP, DETAIL_ICON_ROWS, DIALOG_PAD_X,
    DIALOG_PAD_Y, ITEM_GAP, LIST_ICON_COLS, LIST_ICON_GAP, PICKER_FOOTER_SLACK, inset, split_icon,
};
use crate::tui::picker::{Picker, window};
use crate::tui::regions::{list_visible, regions};
use crate::tui::settings::{ORDER, SettingId};
use crate::tui::tabs::{TabLabel, draw_tabs};
use crate::tui::theme::{Glyphs, Theme};
use crate::tui::toast::draw_toasts;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui_image::protocol::Protocol;

/// Below this width the details panel is hidden.
pub(crate) const MIN_WIDTH_FOR_DETAILS: u16 = 60;
/// Below this height only the list is drawn.
pub const MIN_HEIGHT_FOR_CHROME: u16 = 8;

pub fn view(app: &App, theme: &Theme, frame: &mut Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (window, icons) = draw_screen(app, theme, frame, area);
    if let Some(modal) = &app.modal {
        draw_modal(app, modal, theme, frame, area);
    }
    // Toast boxes never cover a dialog (the residue picker decides what gets deleted);
    // with a dialog open, or on a small terminal, the newest toast goes to the status bar.
    let mut toasts = Vec::new();
    if let Some(window) = window
        && !toasts_in_status_bar(app, area)
    {
        let g = Glyphs::for_set(app.glyphs);
        toasts = draw_toasts(frame, window, &app.toasts, theme, g);
    }
    // Drawn last: a terminal image paints its whole area, so one crossing a toast is skipped.
    for (rect, proto) in icons {
        if !toasts.iter().any(|t| t.intersects(rect)) {
            frame.render_widget(ratatui_image::Image::new(proto), rect);
        }
    }
}

/// Whether the package icon may be shown: never under a dialog or the help, nor when off.
pub fn icon_visible(app: &App) -> bool {
    app.modal.is_none() && !app.show_help && app.icon_protocol != IconProtocol::Off
}

/// The encoded icon of the selected package, when one may be shown.
fn selected_icon(app: &App) -> Option<&Protocol> {
    if !icon_visible(app) {
        return None;
    }
    let (source, id) = app.selected_key()?;
    app.icons.get(
        &format!("{source}/{id}"),
        (DETAIL_ICON_COLS, DETAIL_ICON_ROWS),
    )
}

/// On a small terminal or with a dialog open toasts are not boxes: the newest shows in the
/// status bar.
fn toasts_in_status_bar(app: &App, area: Rect) -> bool {
    app.modal.is_some() || area.width < TOAST_MIN_WIDTH || area.height < TOAST_MIN_HEIGHT
}

/// Smallest terminal that draws toast boxes.
const TOAST_MIN_WIDTH: u16 = 40;
const TOAST_MIN_HEIGHT: u16 = 12;

/// Where each icon goes and what it is; drawn by `view` after the toasts.
type IconPlaces<'a> = Vec<(Rect, &'a Protocol)>;

/// Draws the screen; returns the main window's rectangle when there is one, and the
/// icon still to be drawn.
fn draw_screen<'a>(
    app: &'a App,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) -> (Option<Rect>, IconPlaces<'a>) {
    let g = Glyphs::for_set(app.glyphs);
    let r = regions(app, area);
    if area.height < MIN_HEIGHT_FOR_CHROME {
        if app.tab == Tab::Settings {
            draw_settings(app, theme, g, frame, r.settings);
        } else {
            return (None, draw_list(app, theme, g, frame, r.list));
        }
        return (None, Vec::new());
    }
    draw_tabs(
        frame,
        r.tabs_area,
        &tab_labels(app, g),
        tab_index(app.tab),
        theme,
    );
    let window = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(theme.accent());
    frame.render_widget(window, r.window);
    let mut icons = Vec::new();
    if app.tab == Tab::Settings {
        draw_settings(app, theme, g, frame, r.settings);
    } else if let Some(divider) = r.divider {
        draw_search_line(app, theme, g, frame, r.search_line);
        icons = draw_list(app, theme, g, frame, r.list);
        let divider_style = theme.border();
        for y in r.list.y..r.list.bottom() {
            frame
                .buffer_mut()
                .set_string(divider, y, "│", divider_style);
        }
        icons.extend(draw_details(app, theme, g, frame, r.details));
    } else {
        draw_search_line(app, theme, g, frame, r.search_line);
        icons = draw_list(app, theme, g, frame, r.list);
    }
    let help_rows = r.help.height;
    draw_status(app, theme, g, frame, r.status);
    if help_rows > 0 {
        let bar = bar_area(r.help);
        let marked = app.marked_count();
        let lead = if marked > 0 {
            format!("{marked} marcados • ")
        } else {
            String::new()
        };
        let room = usize::from(bar.width).saturating_sub(lead.chars().count());
        let mut line = help_line(&hints(app), room, theme);
        if !lead.is_empty() {
            line.spans.insert(
                0,
                Span::styled(lead, theme.accent().add_modifier(Modifier::BOLD)),
            );
        }
        frame.render_widget(Paragraph::new(line), bar);
    }
    if app.show_help {
        draw_help(theme, frame, area);
    }
    (Some(r.window), icons)
}

fn tab_index(tab: Tab) -> usize {
    match tab {
        Tab::Search => 0,
        Tab::Installed => 1,
        Tab::Updates => 2,
        Tab::Settings => 3,
    }
}

fn block<'a>(theme: &Theme, focused: bool) -> Block<'a> {
    let border = if focused {
        theme.border_focus()
    } else {
        theme.border()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
}

fn italic_muted(theme: &Theme) -> ratatui::style::Style {
    theme.muted().add_modifier(Modifier::ITALIC)
}

fn updates_count(app: &App) -> usize {
    app.installed
        .iter()
        .filter(|r| matches!(r.state, UpdateState::Available { .. }))
        .count()
}

pub(crate) fn tab_labels(app: &App, g: &Glyphs) -> Vec<TabLabel> {
    let count = updates_count(app);
    let with_glyph = |glyph: &str, name: &str| {
        if glyph.is_empty() {
            name.to_string()
        } else {
            format!("{glyph} {name}")
        }
    };
    let plain = |text: String| TabLabel { text, warn: false };
    vec![
        plain(with_glyph(g.tab_search, "Pesquisar")),
        plain(with_glyph(g.tab_installed, "Instalados")),
        TabLabel {
            text: if count > 0 {
                with_glyph(g.tab_updates, &format!("Atualizações {count}"))
            } else {
                with_glyph(g.tab_updates, "Atualizações")
            },
            warn: count > 0,
        },
        plain(with_glyph(g.tab_settings, "Ajustes")),
    ]
}

/// Settings tab: sections, one row per setting and the selected one's description last.
fn draw_settings(app: &App, theme: &Theme, g: &Glyphs, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let st = &app.settings;
    let label_w = ORDER
        .iter()
        .map(|id| Span::raw(id.label()).width())
        .max()
        .unwrap_or(0);
    let heading = |text: &str| Line::from(Span::styled(text.to_string(), theme.accent()));
    let row = |i: usize, id: SettingId| {
        let sel = i == st.selected;
        let (bar, label_style) = if sel {
            (
                Span::styled(format!("{} ", g.bar), theme.sel_bar()),
                theme.sel_title().add_modifier(Modifier::BOLD),
            )
        } else {
            (Span::raw("  "), theme.fg())
        };
        let label = id.label();
        let pad = " ".repeat(label_w + 2 - Span::raw(label).width());
        let mut spans = vec![
            bar,
            Span::styled(label.to_string(), label_style),
            Span::raw(pad),
        ];
        let current = st.value_index(id);
        for (n, opt) in id.options().iter().enumerate() {
            if n > 0 {
                spans.push(Span::raw("  "));
            }
            if n == current {
                spans.push(Span::styled(
                    format!("‹ {opt} ›"),
                    theme.accent().add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled((*opt).to_string(), theme.fg_dim()));
            }
        }
        if id == SettingId::Glyphs && st.glyphs_forced {
            spans.push(Span::styled(
                "  (substituído por --glyphs nesta sessão)",
                theme.fg_dim(),
            ));
        }
        Line::from(spans)
    };
    let mut lines = vec![heading("Operações")];
    for (i, id) in ORDER.iter().enumerate().take(4) {
        lines.push(row(i, *id));
    }
    lines.push(Line::raw(""));
    lines.push(heading("Aparência"));
    for (i, id) in ORDER.iter().enumerate().skip(4) {
        lines.push(row(i, *id));
    }
    lines.push(Line::raw(""));
    lines.push(heading("Fontes (somente leitura)"));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Buckets do scoop  ", theme.fg()),
        Span::styled(app.buckets_summary.clone(), theme.fg_dim()),
    ]));
    let body_h = area.height.saturating_sub(2);
    let body = Rect::new(area.x, area.y, area.width, body_h.min(area.height));
    frame.render_widget(Paragraph::new(lines), body);
    if area.height >= 2 {
        let desc = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                st.current_id().description(),
                theme.fg_dim().add_modifier(Modifier::ITALIC),
            )),
            desc,
        );
    }
}

fn format_age(secs: i64) -> String {
    match secs {
        s if s < 60 => "agora".to_string(),
        s if s < 3600 => format!("{}min", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

fn spinner(app: &App, g: &Glyphs) -> &'static str {
    g.spinner
        .get(app.spinner_frame % g.spinner.len().max(1))
        .copied()
        .unwrap_or("")
}

fn draw_search_line(app: &App, theme: &Theme, g: &Glyphs, frame: &mut Frame, area: Rect) {
    let prompt = Span::styled(format!("{} ", g.search), theme.accent());
    let query = if app.query.is_empty() {
        Line::from(vec![
            prompt,
            Span::styled("digite para buscar…", italic_muted(theme)),
        ])
    } else {
        Line::from(vec![
            prompt,
            Span::styled(app.query.clone(), theme.name()),
            Span::styled("▏", theme.accent()),
        ])
    };
    frame.render_widget(Paragraph::new(query), area);
}

fn source_glyph<'a>(theme: &Theme, g: &Glyphs, kind: SourceKind) -> Span<'a> {
    match kind {
        SourceKind::Winget => Span::styled(g.winget, theme.src_winget()),
        SourceKind::Scoop => Span::styled(g.scoop, theme.src_scoop()),
    }
}

/// Name in bold with the fuzzy-matched characters underlined.
fn name_spans<'a>(theme: &Theme, name: &str, matched: &[u32]) -> Vec<Span<'a>> {
    let mut out: Vec<Span<'a>> = Vec::new();
    let mut buf = String::new();
    let mut buf_matched = false;
    let style_for = |is_match: bool| {
        if is_match {
            theme.matched().add_modifier(Modifier::BOLD)
        } else {
            theme.name()
        }
    };
    for (i, ch) in name.chars().enumerate() {
        let m = u32::try_from(i).is_ok_and(|i| matched.contains(&i));
        if m != buf_matched && !buf.is_empty() {
            out.push(Span::styled(
                std::mem::take(&mut buf),
                style_for(buf_matched),
            ));
        }
        buf_matched = m;
        buf.push(ch);
    }
    if !buf.is_empty() {
        out.push(Span::styled(buf, style_for(buf_matched)));
    }
    out
}

/// One list line: left spans, then the right-aligned version spans.
fn compose<'a>(left: Vec<Span<'a>>, right: Vec<Span<'a>>, width: usize) -> Line<'a> {
    let lw: usize = left.iter().map(Span::width).sum();
    let rw: usize = right.iter().map(Span::width).sum();
    let mut spans = left;
    if lw + rw + 2 <= width {
        spans.push(Span::raw(" ".repeat(width - lw - rw)));
    } else {
        spans.push(Span::raw("  "));
    }
    spans.extend(right);
    Line::from(spans)
}

fn state_span<'a>(theme: &Theme, g: &Glyphs, row: &ListRow) -> Span<'a> {
    match (&row.correlation, &row.state) {
        (_, UpdateState::Available { .. }) => Span::styled(g.upgrade, theme.warn()),
        (Correlation::Ambiguous { .. }, _) => Span::styled(g.ambiguous, theme.alert()),
        (_, UpdateState::UpToDate) => Span::styled(g.ok, theme.ok()),
        _ => Span::styled(g.unknown, theme.muted()),
    }
}

fn row_source_glyph<'a>(theme: &Theme, g: &Glyphs, row: &ListRow) -> Span<'a> {
    match &row.correlation {
        Correlation::Matched { source, .. } if source.starts_with("scoop") => {
            source_glyph(theme, g, SourceKind::Scoop)
        }
        Correlation::Matched { .. } => source_glyph(theme, g, SourceKind::Winget),
        Correlation::Ambiguous { .. } => Span::styled(g.ambiguous, theme.alert()),
        Correlation::Unmatched => Span::styled(g.unknown, theme.muted()),
    }
}

/// Rows `[start, end)` of a list of `len` rows that fit in `height` lines while keeping
/// `selected` visible, scrolling as little as possible from `scroll`.
pub fn list_window(
    selected: usize,
    scroll: usize,
    len: usize,
    height: usize,
) -> std::ops::Range<usize> {
    if len == 0 {
        return 0..0;
    }
    let start = if selected < scroll {
        selected
    } else if selected >= scroll + height {
        selected + 1 - height
    } else {
        scroll
    }
    .min(len - 1);
    start..(start + height).min(len)
}

/// Cuts `text` to at most `max` terminal cells (wide chars count 2), ending in an
/// ellipsis when it was cut. Control whitespace becomes a plain space first.
pub(crate) fn clip(text: &str, max: usize) -> String {
    let cell_width = |c: char| Span::raw(c.encode_utf8(&mut [0; 4]).to_string()).width();
    let text: String = text
        .chars()
        .map(|c| {
            if matches!(c, '\r' | '\n' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    if text.chars().map(cell_width).sum::<usize>() <= max {
        return text;
    }
    let budget = max.saturating_sub(1);
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let w = cell_width(c);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// Builds only the items in `window` (indices into the full row list): `ITEM_ROWS`
/// lines each, `ITEM_GAP` blank lines between items.
fn list_lines<'a>(
    app: &'a App,
    theme: &Theme,
    g: &Glyphs,
    width: usize,
    window: std::ops::Range<usize>,
) -> Vec<Line<'a>> {
    let bar = |sel: bool| {
        if sel {
            Span::styled(format!("{} ", g.bar), theme.sel_bar())
        } else {
            Span::raw("  ")
        }
    };
    let title_style = |sel: bool| {
        if sel {
            theme.sel_title().add_modifier(Modifier::BOLD)
        } else {
            theme.name()
        }
    };
    let desc_style = |sel: bool| {
        if sel {
            theme.sel_desc()
        } else {
            theme.fg_dim()
        }
    };
    // Name spans restyled for the selected row (matched letters keep underline).
    let name_for = |sel: bool, name: &str, matched: &[u32]| -> Vec<Span<'a>> {
        let mut spans = name_spans(theme, name, matched);
        if sel {
            for sp in &mut spans {
                if sp.style == theme.name() {
                    sp.style = title_style(true);
                }
            }
        }
        spans
    };
    // With a visible mark in the tab every row gets the column (on both lines), so the
    // names stay aligned.
    let mark_col = app.has_visible_mark();
    let mark_w = Span::raw(g.mark).width() + 1;
    let second = |sel: bool, text: String| -> Line<'a> {
        let indent = if mark_col { mark_w } else { 0 };
        let room = width.saturating_sub(4 + indent);
        Line::from(vec![
            bar(sel),
            Span::raw(" ".repeat(2 + indent)),
            Span::styled(clip(&text, room), desc_style(sel)),
        ])
    };
    let mark = |key: (String, String)| -> Option<Span<'a>> {
        if !mark_col {
            return None;
        }
        Some(if app.is_marked(&key) {
            Span::styled(format!("{} ", g.mark), theme.ok())
        } else {
            Span::raw(" ".repeat(mark_w))
        })
    };
    let mut items: Vec<[Line<'a>; ITEM_ROWS]> = Vec::new();
    match app.tab {
        Tab::Search => {
            for (i, h) in app
                .hits
                .iter()
                .enumerate()
                .skip(window.start)
                .take(window.len())
            {
                let Some(e) = app.search.entries().get(h.index) else {
                    continue;
                };
                let sel = i == app.selected;
                let mut left = vec![bar(sel), source_glyph(theme, g, e.source), Span::raw(" ")];
                left.extend(mark((e.source_name.clone(), e.id.clone())));
                left.extend(name_for(sel, &e.name, &h.name_indices));
                let right = vec![Span::styled(e.version.clone(), theme.fg_dim())];
                let detail = match e.description.as_deref().filter(|d| !d.is_empty()) {
                    Some(d) => format!("{} · {d}", e.id),
                    None => format!("{} · {}", e.id, e.source_name),
                };
                items.push([compose(left, right, width), second(sel, detail)]);
            }
        }
        Tab::Installed | Tab::Updates => {
            for (i, r) in app
                .installed_view()
                .into_iter()
                .enumerate()
                .skip(window.start)
                .take(window.len())
            {
                let sel = i == app.selected;
                let mut left = vec![bar(sel), row_source_glyph(theme, g, r), Span::raw(" ")];
                left.extend(mark((r.origin.clone(), r.name.clone())));
                left.extend(name_for(sel, &r.name, &[]));
                let old = r.installed_version.clone().unwrap_or_default();
                let mut right = match (&r.state, app.tab) {
                    (UpdateState::Available { latest }, Tab::Updates) => vec![
                        Span::styled(old, theme.dim()),
                        Span::styled(" → ", theme.fg_dim()),
                        Span::styled(latest.clone(), theme.ok().add_modifier(Modifier::BOLD)),
                    ],
                    _ => vec![
                        Span::styled(old, theme.fg_dim()),
                        Span::raw(" "),
                        state_span(theme, g, r),
                    ],
                };
                if !r.others.is_empty() {
                    right.push(Span::styled(format!(" +{}", r.others.len()), theme.muted()));
                }
                if crate::pins::is_pinned(&app.pins, &r.origin, &r.name) {
                    right.push(Span::raw(" "));
                    right.push(Span::styled(g.pin, theme.warn()));
                }
                let detail = match &r.correlation {
                    Correlation::Matched {
                        source, package_id, ..
                    } => format!("{package_id} · {source}"),
                    Correlation::Unmatched => "sem correspondência no catálogo".to_string(),
                    Correlation::Ambiguous { .. } => "correspondência ambígua".to_string(),
                };
                items.push([compose(left, right, width), second(sel, detail)]);
            }
        }
        Tab::Settings => {}
    }
    let mut lines = Vec::new();
    for (n, item) in items.into_iter().enumerate() {
        if n > 0 {
            lines.extend((0..ITEM_GAP).map(|_| Line::raw("")));
        }
        lines.extend(item);
    }
    lines
}

/// Draws the list; returns the icons of its visible rows, drawn by `view` after the toasts.
/// With icons on every item is indented, with or without an icon, so the text stays aligned.
fn draw_list<'a>(
    app: &'a App,
    theme: &Theme,
    g: &Glyphs,
    frame: &mut Frame,
    area: Rect,
) -> IconPlaces<'a> {
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return Vec::new();
    }
    if app.tab == Tab::Search && app.search.entries().is_empty() {
        let msg = Line::from(vec![
            Span::styled(format!("{} ", spinner(app, g)), theme.accent()),
            Span::styled("baixando catálogo…", italic_muted(theme)),
        ]);
        frame.render_widget(Paragraph::new(msg), inner);
        return Vec::new();
    }
    let per_item = ITEM_ROWS + usize::from(ITEM_GAP);
    let window = list_visible(app, inner);
    let indent = LIST_ICON_COLS + LIST_ICON_GAP;
    let icons_on = app.icon_protocol != IconProtocol::Off && inner.width >= indent + 20;
    let text_area = if icons_on {
        inset_left(inner, indent)
    } else {
        inner
    };
    let lines = list_lines(app, theme, g, usize::from(text_area.width), window.clone());
    if lines.is_empty() {
        let text = match app.tab {
            Tab::Search => "nenhum resultado",
            Tab::Installed => "nenhum pacote instalado encontrado",
            Tab::Updates => "tudo atualizado",
            Tab::Settings => "",
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text, italic_muted(theme))),
            inner,
        );
        return Vec::new();
    }
    frame.render_widget(Paragraph::new(lines), text_area);
    if !icons_on || !icon_visible(app) {
        return Vec::new();
    }
    let size = (LIST_ICON_COLS, ITEM_ROWS as u16);
    let mut places = Vec::new();
    let keys = app.list_keys(window.clone());
    for (n, (key, placeholder)) in keys
        .into_iter()
        .zip(app.list_placeholders(window))
        .enumerate()
    {
        let y = usize::from(inner.y) + n * per_item;
        let rect = Rect::new(inner.x, y as u16, size.0, size.1);
        if rect.bottom() > inner.bottom() {
            continue;
        }
        let proto = key.and_then(|(source, id)| app.icons.get(&format!("{source}/{id}"), size));
        match proto {
            Some(proto) => places.push((rect, proto)),
            None if placeholder => draw_no_icon(theme, g, frame, rect),
            None => {}
        }
    }
    places
}

/// The no-icon placeholder, centred in the icon's rectangle: text, never an encoded image.
fn draw_no_icon(theme: &Theme, g: &Glyphs, frame: &mut Frame, rect: Rect) {
    if rect.height == 0 {
        return;
    }
    let row = Rect::new(rect.x, rect.y + (rect.height - 1) / 2, rect.width, 1);
    let glyph = Paragraph::new(Span::styled(g.no_icon, theme.muted())).alignment(Alignment::Center);
    frame.render_widget(glyph, row);
}

/// `area` without its `cols` leftmost columns.
fn inset_left(area: Rect, cols: u16) -> Rect {
    let cols = cols.min(area.width);
    Rect::new(area.x + cols, area.y, area.width - cols, area.height)
}

/// Fields of the selected row, shown before `PackageDetails` arrives.
struct Basics {
    name: String,
    id: Option<String>,
    version: Option<String>,
    publisher: Option<String>,
    description: Option<String>,
    install: Option<Line<'static>>,
}

fn install_status(theme: &Theme, g: &Glyphs, r: &ListRow) -> Line<'static> {
    let installed = r.installed_version.as_deref().unwrap_or("?");
    match &r.state {
        UpdateState::Available { latest } => Line::from(vec![
            Span::styled(format!("{}{}", g.upgrade, g.gap), theme.warn()),
            Span::styled(
                format!("atualização disponível: {installed} → {latest}"),
                theme.warn(),
            ),
        ]),
        UpdateState::UpToDate => Line::from(vec![
            Span::styled(format!("{}{}", g.ok, g.gap), theme.ok()),
            Span::styled(format!("instalado {installed} (atualizado)"), theme.ok()),
        ]),
        UpdateState::Broken => Line::from(vec![
            Span::styled(format!("{}{}", g.unknown, g.gap), theme.alert()),
            Span::styled("instalação quebrada (rode repair)", theme.alert()),
        ]),
        UpdateState::Unknown => {
            let text = match &r.correlation {
                Correlation::Ambiguous { .. } => "correspondência ambígua no catálogo",
                _ => "sem correspondência no catálogo",
            };
            Line::from(vec![
                Span::styled(format!("{}{}", g.unknown, g.gap), theme.muted()),
                Span::styled(text, theme.muted()),
            ])
        }
    }
}

fn basics(app: &App, theme: &Theme, g: &Glyphs) -> Option<Basics> {
    match app.tab {
        Tab::Settings => None,
        Tab::Search => {
            let h = app.hits.get(app.selected)?;
            let e = app.search.entries().get(h.index)?;
            Some(Basics {
                name: e.name.clone(),
                id: Some(e.id.clone()),
                version: Some(e.version.clone()),
                publisher: e.publisher.clone(),
                description: e.description.clone(),
                install: None,
            })
        }
        Tab::Installed | Tab::Updates => {
            let rows = app.installed_view();
            let r = rows.get(app.selected)?;
            let (entry, id) = match &r.correlation {
                Correlation::Matched {
                    source, package_id, ..
                } => (
                    app.search
                        .entries()
                        .iter()
                        .find(|e| &e.source_name == source && &e.id == package_id),
                    Some(package_id.clone()),
                ),
                _ => (None, None),
            };
            Some(Basics {
                name: r.name.clone(),
                id,
                version: r.installed_version.clone(),
                publisher: entry.and_then(|e| e.publisher.clone()),
                description: entry.and_then(|e| e.description.clone()),
                install: Some(install_status(theme, g, r)),
            })
        }
    }
}

fn labeled<'a>(theme: &Theme, glyph: &str, gap: &str, text: String) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{glyph}{gap}"), theme.accent()),
        Span::styled(text, theme.fg_dim()),
    ])
}

fn heading<'a>(theme: &Theme, text: &'a str) -> Line<'a> {
    Line::from(Span::styled(
        text,
        theme.accent().add_modifier(Modifier::BOLD),
    ))
}

/// The details text, and how many of its first lines identify the package (name, ID,
/// publisher, version): those go beside the icon.
fn details_lines(app: &App, theme: &Theme, g: &Glyphs) -> (Vec<Line<'static>>, usize) {
    let Some(b) = basics(app, theme, g) else {
        return (
            vec![Line::from(Span::styled(
                "nenhum pacote selecionado",
                italic_muted(theme),
            ))],
            0,
        );
    };
    let mut out = vec![Line::from(Span::styled(b.name.clone(), theme.name()))];
    if let Some(id) = &b.id {
        out.push(labeled(theme, g.id, g.gap, id.clone()));
    }
    if let Some(p) = &b.publisher {
        out.push(labeled(theme, g.publisher, g.gap, p.clone()));
    }
    if let Some(v) = &b.version {
        out.push(Line::from(Span::styled(format!("versão {v}"), theme.fg())));
    }
    let identity = out.len();
    if let Some(i) = b.install.clone() {
        out.push(i);
    }
    let key = app.selected_key();
    let loaded: Option<&Result<PackageDetails, String>> = key
        .as_ref()
        .and_then(|(s, id)| app.details.get(&format!("{s}/{id}")));
    if let Some(Ok(d)) = loaded
        && let Some(l) = &d.license
    {
        out.push(labeled(theme, g.license, g.gap, l.clone()));
    }
    let desc = match loaded {
        Some(Ok(d)) => d.description.clone().or(b.description.clone()),
        _ => b.description.clone(),
    };
    if let Some(d) = desc {
        out.push(Line::from(Span::styled(d, theme.description())));
    }
    if matches!(app.tab, Tab::Installed | Tab::Updates)
        && let Some(r) = app.installed_view().get(app.selected)
        && !r.others.is_empty()
    {
        out.push(Line::raw(""));
        out.push(heading(theme, "Registros"));
        let version = |v: &Option<String>| v.clone().unwrap_or_else(|| "?".to_string());
        out.push(Line::from(Span::styled(
            format!(
                "  {} · {} · {}",
                r.location,
                version(&r.installed_version),
                r.origin
            ),
            theme.fg(),
        )));
        for o in &r.others {
            out.push(Line::from(Span::styled(
                format!(
                    "  {} · {} · {}",
                    o.location,
                    version(&o.installed_version),
                    o.origin
                ),
                theme.fg(),
            )));
        }
        out.push(Line::from(Span::styled(
            "O Windows tem mais de um registro deste app. Pode ser uma instalação antiga esquecida ou o mesmo app instalado de dois jeitos.",
            italic_muted(theme),
        )));
    }
    match loaded {
        Some(Ok(d)) => {
            if !d.installers.is_empty() {
                out.push(Line::raw(""));
                out.push(heading(theme, "Instaladores"));
                for i in &d.installers {
                    let scope = i.scope.as_deref().unwrap_or("-");
                    out.push(Line::from(Span::styled(
                        format!("  {} · {} · {scope}", i.architecture, i.installer_type),
                        theme.fg(),
                    )));
                }
            }
            if !d.available_versions.is_empty() {
                out.push(Line::raw(""));
                out.push(heading(theme, "Versões"));
                let vs: Vec<&str> = d
                    .available_versions
                    .iter()
                    .take(8)
                    .map(String::as_str)
                    .collect();
                out.push(Line::from(Span::styled(
                    format!("  {}", vs.join(", ")),
                    theme.fg_dim(),
                )));
            }
        }
        Some(Err(e)) => {
            out.push(Line::raw(""));
            out.push(Line::from(Span::styled(
                format!("{}{}{e}", g.ambiguous, g.gap),
                theme.err(),
            )));
        }
        None => {
            if key.is_some() {
                out.push(Line::raw(""));
                out.push(Line::from(vec![
                    Span::styled(format!("{}{}", spinner(app, g), g.gap), theme.accent()),
                    Span::styled("carregando…", italic_muted(theme)),
                ]));
            }
        }
    }
    (out, identity)
}

/// Rows `line` takes in `width` columns, wrapped exactly as the details text is drawn.
fn wrapped_rows(line: &Line<'static>, width: u16) -> u16 {
    const MAX_ROWS: u16 = 64;
    if width == 0 {
        return u16::MAX;
    }
    let area = Rect::new(0, 0, width, MAX_ROWS);
    let mut buf = Buffer::empty(area);
    Paragraph::new(line.clone())
        .wrap(Wrap { trim: true })
        .render(area, &mut buf);
    (0..MAX_ROWS)
        .rev()
        .find(|&y| (0..width).any(|x| buf[(x, y)].symbol() != " "))
        .map_or(1, |y| y + 1)
}

/// Draws the details; with an icon, its first lines sit beside it and the rest below.
/// Returns where the icon goes (drawn by `view`, after the toasts).
fn draw_details<'a>(
    app: &'a App,
    theme: &Theme,
    g: &Glyphs,
    frame: &mut Frame,
    area: Rect,
) -> Option<(Rect, &'a Protocol)> {
    let (mut lines, identity) = details_lines(app, theme, g);
    // Guard: the narrowest details pane drawn today (60-column terminal) is 23 columns.
    let fits =
        area.width >= DETAIL_ICON_COLS + DETAIL_ICON_GAP + 10 && area.height >= DETAIL_ICON_ROWS;
    // `None` inside: the no-icon placeholder goes in the icon's place.
    let icon = match selected_icon(app) {
        _ if !fits => None,
        Some(proto) => Some(Some(proto)),
        None if icon_visible(app) && app.selected_placeholder() => Some(None),
        None => None,
    };
    let Some(proto) = icon else {
        let p = Paragraph::new(lines).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
        return None;
    };
    let (icon_rect, text) = split_icon(area, DETAIL_ICON_ROWS, DETAIL_ICON_COLS, DETAIL_ICON_GAP);
    // Identity lines go beside the icon while they fit wrapped in its rows; from the first
    // that does not, everything goes below at full width.
    let mut used = 0u16;
    let beside = lines[..identity]
        .iter()
        .take_while(|l| {
            used = used.saturating_add(wrapped_rows(l, text.width));
            used <= DETAIL_ICON_ROWS
        })
        .count();
    let rest = lines.split_off(beside);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text);
    let below = Rect::new(
        area.x,
        area.y + DETAIL_ICON_ROWS,
        area.width,
        area.height - DETAIL_ICON_ROWS,
    );
    frame.render_widget(Paragraph::new(rest).wrap(Wrap { trim: true }), below);
    match proto {
        Some(proto) => Some((icon_rect, proto)),
        None => {
            draw_no_icon(theme, g, frame, icon_rect);
            None
        }
    }
}

/// A bar row with `BAR_PAD_X` of margin on both sides.
fn bar_area(area: Rect) -> Rect {
    inset(area, BAR_PAD_X, 0)
}

fn draw_status(app: &App, theme: &Theme, g: &Glyphs, frame: &mut Frame, area: Rect) {
    let mut right: Vec<Span> = Vec::new();
    let mut push = |span: Span<'static>| {
        if !right.is_empty() {
            right.push(Span::raw(" "));
        }
        right.push(span);
    };
    if app.refreshing {
        push(Span::styled(
            format!(" {} atualizando ", spinner(app, g)),
            theme.pill_a(),
        ));
    }
    if app.offline {
        push(Span::styled(
            " offline ",
            theme.alert().add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(age) = app.index_age_secs {
        push(Span::styled(
            format!(" {} índice {} ", g.refresh, format_age(age)),
            theme.muted(),
        ));
    }
    let bar = bar_area(area);
    let running = running_text(app, g);
    // The operation in progress wins over the right pills when both do not fit.
    if let Some(t) = &running {
        let rw: usize = right.iter().map(Span::width).sum();
        if Span::raw(t.as_str()).width() + 2 + rw + 2 > usize::from(bar.width) {
            right.clear();
        }
    }
    // Otherwise the right pills win: the left text is cut to what is left, minus a 2-cell gap.
    let rw: usize = right.iter().map(Span::width).sum();
    let room = usize::from(bar.width).saturating_sub(rw + 2);
    // Without toast boxes the newest toast takes the message pill when there is no status.
    let message = running.as_deref().or(app.status.as_deref()).or_else(|| {
        toasts_in_status_bar(app, frame.area())
            .then(|| app.toasts.first().map(|t| t.text.as_str()))
            .flatten()
    });
    let left = match message {
        Some(s) => vec![Span::styled(
            format!(" {} ", clip(s, room.saturating_sub(2))),
            theme.pill_msg(),
        )],
        None => {
            let len = app.visible_len();
            let mut text = format!("{} de {len}", if len == 0 { 0 } else { app.selected + 1 });
            if app.tab == Tab::Search {
                text.push_str(&format!(" · {} no catálogo", app.search.entries().len()));
            }
            vec![Span::styled(clip(&text, room), theme.fg_dim())]
        }
    };
    frame.render_widget(
        Paragraph::new(compose(left, right, usize::from(bar.width))),
        bar,
    );
}

fn draw_help(theme: &Theme, frame: &mut Frame, area: Rect) {
    let rect = centered(area, 52 + 2 * DIALOG_PAD_X, 17 + 2 * DIALOG_PAD_Y);
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(
                format!("{k:<12}"),
                theme.accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(d.to_string(), theme.fg()),
        ])
    };
    let lines = vec![
        key("↑↓ j k", "mover seleção"),
        key("PgUp PgDn", "mover de 10 em 10"),
        key("←→ / Tab", "trocar aba"),
        key("Enter", "foco nos detalhes"),
        key("Espaço", "marcar (busca vazia)"),
        key("Ins Ctrl+Esp", "marcar (sempre)"),
        key("i", "instalar (aba Busca, scoop)"),
        key("u", "remover (aba Instalados)"),
        key("U", "atualizar (aba Atualizações)"),
        key("f", "fila de operações"),
        key("Ctrl+S", "filtrar fonte"),
        key("Ctrl+R", "atualizar catálogo"),
        key("Esc", "voltar / limpar / sair"),
        key("q Ctrl+C", "sair"),
    ];
    frame.render_widget(Clear, rect);
    let blk = block(theme, true).title(Span::styled(
        " Ajuda ",
        theme.accent().add_modifier(Modifier::BOLD),
    ));
    let inner = inset(blk.inner(rect), DIALOG_PAD_X, DIALOG_PAD_Y);
    frame.render_widget(blk, rect);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws a bordered dialog and its padded text.
fn render_dialog(frame: &mut Frame, rect: Rect, blk: Block, body: Paragraph) {
    let inner = inset(blk.inner(rect), DIALOG_PAD_X, DIALOG_PAD_Y);
    frame.render_widget(Clear, rect);
    frame.render_widget(blk, rect);
    frame.render_widget(body, inner);
}

/// Outer size of a dialog that holds `text_w` x `text_h` of text.
fn dialog_size(area: Rect, text_w: u16, text_h: u16) -> Rect {
    centered(
        area,
        text_w.saturating_add(2 * (BORDER + DIALOG_PAD_X)),
        text_h.saturating_add(2 * (BORDER + DIALOG_PAD_Y)),
    )
}

/// Centered rectangle of at most `w` x `h` inside `area`.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn modal_block<'a>(theme: &Theme, title: &'a str) -> Block<'a> {
    block(theme, true).title(Span::styled(
        format!(" {title} "),
        theme.accent().add_modifier(Modifier::BOLD),
    ))
}

fn draw_modal(app: &App, modal: &Modal, theme: &Theme, frame: &mut Frame, area: Rect) {
    match modal {
        Modal::Confirm { plan_text, .. } => {
            let mut lines: Vec<Line> = plan_text
                .lines()
                .map(|l| Line::from(Span::styled(l.to_string(), theme.fg())))
                .collect();
            lines.push(Line::from(Span::styled(
                "Enter confirma · Esc cancela",
                theme.accent(),
            )));
            let h = u16::try_from(lines.len()).unwrap_or(u16::MAX);
            let rect = dialog_size(area, 88, h);
            render_dialog(
                frame,
                rect,
                modal_block(theme, "Confirmar"),
                Paragraph::new(lines).wrap(Wrap { trim: false }),
            );
        }
        Modal::Picking { picker, .. } => {
            draw_picker(
                picker,
                "Escolha o pacote",
                "Enter escolhe · Esc cancela",
                theme,
                frame,
                area,
            );
        }
        Modal::Actions { picker, app, .. } => {
            draw_picker(
                picker,
                app,
                "Enter escolhe · Esc cancela",
                theme,
                frame,
                area,
            );
        }
        Modal::Choosing { picker, .. } => {
            draw_picker(
                picker,
                "Escolha a correspondência",
                "Enter escolhe · Esc cancela",
                theme,
                frame,
                area,
            );
        }
        Modal::Residues { picker, app, .. } => {
            let title = format!("Resíduos de {app}");
            draw_picker(
                picker,
                &title,
                "Espaço marca · Enter remove os marcados · Esc mantém",
                theme,
                frame,
                area,
            );
        }
        Modal::Queue { selected } => {
            let (ids, items): (Vec<u64>, Vec<_>) = queue_rows(app).into_iter().unzip();
            let at = ids.iter().position(|id| id == selected).unwrap_or(0);
            draw_picker(
                &Picker::single_at(items, at),
                "Fila",
                "x/Delete tira da fila · Esc fecha",
                theme,
                frame,
                area,
            );
        }
    }
}

/// Status bar text of the operation in progress (or of the queue waiting for the lock).
fn running_text(app: &App, g: &Glyphs) -> Option<String> {
    let mut text = match &app.running {
        Some(r) => {
            let mut t = format!("{} {}", spinner(app, g), r.label);
            if let Some(p) = &r.progress {
                t.push(' ');
                t.push_str(&progress_text(p));
            }
            t
        }
        None if app.waiting_lock && !app.queue.is_empty() => {
            "aguardando outra instalação".to_string()
        }
        None => return None,
    };
    if !app.queue.is_empty() {
        text.push_str(&format!(" · fila: {}", app.queue.len()));
    }
    if app.quit_after_running {
        text.push_str(" · saindo após terminar…");
    }
    Some(text)
}

fn progress_text(p: &RunProgress) -> String {
    match p.total.filter(|t| *t > 0) {
        Some(total) => {
            let pct = (p.done.saturating_mul(100) / total).min(100);
            let filled = usize::try_from(pct / 5).unwrap_or(20);
            format!(
                "{} [{}{}] {pct}%",
                p.label,
                "#".repeat(filled),
                "-".repeat(20 - filled.min(20))
            )
        }
        None => format!("{} {} bytes", p.label, p.done),
    }
}

fn draw_picker(
    picker: &Picker,
    title: &str,
    footer: &str,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let items = picker.items();
    // Rows left after borders, vertical padding, the footer line and one spare line.
    let spare = 2 * (BORDER + DIALOG_PAD_Y) + PICKER_FOOTER_SLACK;
    let rows = usize::from(area.height.saturating_sub(spare)).clamp(1, 12);
    let range = window(picker.selected(), items.len(), rows);
    let mut lines: Vec<Line> = Vec::new();
    for i in range {
        let it = &items[i];
        let cursor = if i == picker.selected() { ">" } else { " " };
        let mark = if !picker.is_multi() {
            ""
        } else if picker.checked().get(i).copied().unwrap_or(false) {
            "[x] "
        } else {
            "[ ] "
        };
        // The warning goes before the label/path: long paths get cut at the end.
        let note = it
            .note
            .as_deref()
            .map(|n| {
                let n = if n == OTHER_INSTALL_NOTE {
                    "outra instalação?"
                } else {
                    n
                };
                format!("({n}) ")
            })
            .unwrap_or_default();
        let style = if !it.selectable {
            theme.muted()
        } else if i == picker.selected() {
            theme.bg_selected()
        } else {
            theme.fg()
        };
        lines.push(Line::from(Span::styled(
            format!("{cursor} {mark}{note}{}  {}", it.label, it.detail),
            style,
        )));
    }
    lines.push(Line::from(Span::styled(footer.to_string(), theme.accent())));
    let h = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let rect = dialog_size(area, 98, h);
    let blk = block(theme, true).title(Span::styled(
        format!(" {title} "),
        theme.accent().add_modifier(Modifier::BOLD),
    ));
    render_dialog(frame, rect, blk, Paragraph::new(lines));
}
