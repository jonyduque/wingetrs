//! The one screen geometry: drawing and mouse hit-testing both read it from here.
use crate::tui::app::{
    ActionId, App, CONFIRM_BUTTONS, ConfirmButton, HELP_BAR_MIN_HEIGHT, ITEM_ROWS, Modal,
    ScriptLine, ScriptLineKind, Tab, buttons, items_in_rows,
};
use crate::tui::layout::{
    BORDER, DIALOG_PAD_X, DIALOG_PAD_Y, FILTER_GAP, ITEM_GAP, PAD_X, inset, split_with_gutter,
};
use crate::tui::settings::ORDER;
use crate::tui::tabs::{icon_lead, tab_rects_at};
use crate::tui::view::{MIN_HEIGHT_FOR_CHROME, MIN_WIDTH_FOR_DETAILS, list_window, tab_labels};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, BorderType, Borders};

/// Lines taken by the tab header.
pub const TABS_ROWS: u16 = 3;

/// Rectangles of the screen parts; a part that is not on screen is `Rect::default()`.
#[derive(Debug, Default, Clone)]
pub struct Regions {
    /// Header area (the tab boxes live inside it).
    pub tabs_area: Rect,
    /// Title rectangle of each tab.
    pub tabs: Vec<(Rect, Tab)>,
    /// The bordered window below the tabs.
    pub window: Rect,
    pub search_line: Rect,
    pub list: Rect,
    /// Column of the list/details divider, when the details are shown.
    pub divider: Option<u16>,
    /// The details text, above the buttons.
    pub details: Rect,
    /// Action buttons at the foot of the details pane, in drawing order.
    pub buttons: Vec<(Rect, ActionId)>,
    /// Area `draw_settings` paints into (Settings tab only).
    pub settings: Rect,
    /// One rectangle per setting of `ORDER` (those that fit), Settings tab only.
    pub setting_rows: Vec<Rect>,
    pub status: Rect,
    pub help: Rect,
    /// The confirmation dialog, its item list and its buttons (when it is open).
    pub confirm: Rect,
    pub confirm_list: Rect,
    pub confirm_scripts: Rect,
    pub confirm_buttons: Vec<(Rect, ConfirmButton)>,
}

const TAB_ORDER: [Tab; 4] = [Tab::Search, Tab::Installed, Tab::Updates, Tab::Settings];

/// Line of the settings body (0 = first) that shows setting `i`: a heading, the first four
/// settings, a blank line, a heading, then the rest.
pub fn setting_line(i: usize) -> usize {
    if i < 4 { 1 + i } else { 3 + i }
}

/// Number of lines of the settings body: the settings plus the blank line, heading and
/// buckets line of the sources section.
pub fn settings_body_lines() -> usize {
    setting_line(ORDER.len() - 1) + 4
}

/// First body line shown when `body_h` lines fit and setting `selected` must stay visible;
/// the last setting also pulls the sources section below it into view.
pub fn settings_scroll(selected: usize, body_h: usize) -> usize {
    let last = ORDER.len() - 1;
    let target = if selected >= last {
        settings_body_lines() - 1
    } else {
        setting_line(selected)
    };
    (target + 1).saturating_sub(body_h)
}

/// Splits `area` as `draw_screen` paints it.
pub fn regions(app: &App, area: Rect) -> Regions {
    let mut r = Regions::default();
    if area.width == 0 || area.height == 0 {
        return r;
    }
    if area.height < MIN_HEIGHT_FOR_CHROME {
        if app.tab == Tab::Settings {
            r.settings = area;
        } else {
            r.list = area;
        }
    } else {
        let help_rows = u16::from(area.height >= HELP_BAR_MIN_HEIGHT);
        let rows = Layout::vertical([
            Constraint::Length(TABS_ROWS),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(help_rows),
        ])
        .split(area);
        r.tabs_area = rows[0];
        r.window = rows[1];
        r.status = rows[2];
        r.help = rows[3];
        let glyphs = crate::tui::theme::Glyphs::for_set(app.glyphs);
        let labels = tab_labels(app, glyphs);
        let lead = icon_lead(rows[0], &labels, glyphs.app);
        r.tabs = tab_rects_at(rows[0], &labels, lead)
            .into_iter()
            .zip(TAB_ORDER)
            .collect();
        let inner = inset(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_type(BorderType::Rounded)
                .inner(rows[1]),
            PAD_X,
            0,
        );
        let parts = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(FILTER_GAP),
            Constraint::Min(0),
        ])
        .split(inner);
        r.search_line = parts[0];
        if app.tab == Tab::Settings {
            r.settings = inner;
        } else if area.width < MIN_WIDTH_FOR_DETAILS {
            r.list = parts[2];
        } else {
            let (left, divider, right) = split_with_gutter(parts[2], 55);
            r.list = left;
            r.divider = Some(divider);
            r.details = right;
            place_buttons(app, &mut r);
        }
    }
    if r.settings.width > 0 && r.settings.height > 0 {
        let body_h = usize::from(r.settings.height.saturating_sub(2));
        let scroll = settings_scroll(app.settings.selected, body_h);
        // Index-aligned with `ORDER`; a setting scrolled off the top gets an empty rect.
        r.setting_rows = (0..ORDER.len())
            .map(setting_line)
            .take_while(|line| *line < body_h + scroll)
            .map(|line| {
                if line < scroll {
                    return Rect::default();
                }
                let y = r
                    .settings
                    .y
                    .saturating_add(u16::try_from(line - scroll).unwrap_or(0));
                Rect::new(r.settings.x, y, r.settings.width, 1)
            })
            .collect();
    }
    if let Some(Modal::Confirm {
        items,
        notes,
        scripts,
        ..
    }) = &app.modal
    {
        let g = confirm_geom(items.len(), notes, &scripts.lines, area);
        r.confirm = g.dialog;
        r.confirm_list = g.list;
        r.confirm_scripts = g.scripts;
        r.confirm_buttons = g.buttons;
    }
    r
}

/// Rows the scripts section keeps for itself (title included) while the notes yield.
const SCRIPT_MIN_ROWS: usize = 4;

/// Share of the screen the confirmation may take, per side.
const CONFIRM_SCREEN_PERCENT: u16 = 80;

/// Where the confirmation draws its parts.
#[derive(Debug, Default, Clone)]
pub struct ConfirmGeom {
    pub dialog: Rect,
    /// The notes, wrapped to the dialog width; only the lines that fit.
    pub notes: Vec<String>,
    pub notes_area: Rect,
    pub list: Rect,
    /// The third-party scripts section: its title row, then the scrolling lines (empty when
    /// the plan has no third-party code).
    pub scripts: Rect,
    pub buttons: Vec<(Rect, ConfirmButton)>,
}

/// Lays out a confirmation of `items` packages, `notes` and the `script_lines` of
/// third-party scripts (wrapped rows count): at most 80% of `area` per side; buttons at the foot, the notes on top,
/// then the list and the scripts section share the rows in between (the list keeps at least
/// one item, half of the rows when it has more).
pub fn confirm_geom(
    items: usize,
    notes: &[String],
    script_lines: &[ScriptLine],
    area: Rect,
) -> ConfirmGeom {
    let mut g = ConfirmGeom::default();
    let chrome_w = 2 * (BORDER + DIALOG_PAD_X);
    let chrome_h = 2 * (BORDER + DIALOG_PAD_Y);
    let pct = |n: u16| u16::try_from(u32::from(n) * u32::from(CONFIRM_SCREEN_PERCENT) / 100);
    let w = pct(area.width).unwrap_or(area.width);
    let text_w = usize::from(w.saturating_sub(chrome_w));
    let wrapped: Vec<String> = notes.iter().flat_map(|n| wrap(n, text_w)).collect();
    let list_rows = items.saturating_mul(ITEM_ROWS);
    let note_rows = if wrapped.is_empty() {
        0
    } else {
        wrapped.len() + 1
    };
    // The section: a title row, its lines and a gap.
    let script_total: usize = script_lines
        .iter()
        .map(|l| script_line_rows(l, text_w))
        .sum();
    let script_rows = if script_lines.is_empty() {
        0
    } else {
        script_total + 2
    };
    // Notes (and a gap), the list, the scripts, a gap and the button row.
    let wanted = chrome_h as usize + note_rows + list_rows + script_rows + 2;
    let h = u16::try_from(wanted)
        .unwrap_or(u16::MAX)
        .min(pct(area.height).unwrap_or(area.height));
    g.dialog = Rect::new(
        area.x + (area.width - w.min(area.width)) / 2,
        area.y + (area.height - h.min(area.height)) / 2,
        w.min(area.width),
        h.min(area.height),
    );
    let inner = inset(g.dialog, BORDER + DIALOG_PAD_X, BORDER + DIALOG_PAD_Y);
    if inner.width == 0 || inner.height == 0 {
        return g;
    }
    let body = inner.height.saturating_sub(2);
    // The notes yield to at least one item (and a few rows of scripts).
    let script_min = if script_lines.is_empty() {
        0
    } else {
        SCRIPT_MIN_ROWS + 1
    };
    let room = usize::from(body).saturating_sub(ITEM_ROWS + 1 + script_min);
    g.notes = wrapped.into_iter().take(room).collect();
    let notes_h = u16::try_from(g.notes.len()).unwrap_or(0);
    g.notes_area = Rect::new(inner.x, inner.y, inner.width, notes_h);
    let gap = u16::from(notes_h > 0);
    let free = body.saturating_sub(notes_h + gap);
    let mut list_h = free;
    if !script_lines.is_empty() {
        // Half of the rows for the list (at least one item), the rest for the scripts.
        let half = (free.saturating_sub(1) / 2).max(ITEM_ROWS as u16);
        list_h = u16::try_from(list_rows)
            .unwrap_or(u16::MAX)
            .min(half)
            .min(free);
        let left = free.saturating_sub(list_h + 1);
        let want = u16::try_from(script_total + 1).unwrap_or(u16::MAX);
        let h = want.min(left);
        if h > 0 {
            g.scripts = Rect::new(
                inner.x,
                inner.y + notes_h + gap + list_h + 1,
                inner.width,
                h,
            );
        }
    }
    g.list = Rect::new(inner.x, inner.y + notes_h + gap, inner.width, list_h);
    let y = inner.bottom() - 1;
    let mut x = inner.x;
    for (label, button) in CONFIRM_BUTTONS {
        let bw = u16::try_from(label.chars().count() + 2).unwrap_or(u16::MAX);
        let bw = bw.min(inner.right().saturating_sub(x));
        if bw == 0 {
            break;
        }
        g.buttons.push((Rect::new(x, y, bw, 1), button));
        x = x.saturating_add(bw + 2);
    }
    g
}

/// Items `[start, end)` of a confirmation of `len` items shown from `scroll` in `rows` items.
pub fn confirm_window(scroll: usize, len: usize, rows: usize) -> std::ops::Range<usize> {
    let start = scroll.min(len.saturating_sub(rows));
    start..(start + rows).min(len)
}

/// Rows a line of the scripts section takes at `width` cells: code is cut (one row), the
/// rest (headers, analysis, flags) wraps.
pub fn script_line_rows(line: &ScriptLine, width: usize) -> usize {
    if line.kind == ScriptLineKind::Code {
        return 1;
    }
    wrap(&line.text, width).len().max(1)
}

/// Rows of `line` as drawn (see [`script_line_rows`]).
pub fn script_line_chunks(line: &ScriptLine, width: usize) -> Vec<String> {
    if line.kind == ScriptLineKind::Code {
        return vec![line.text.clone()];
    }
    let chunks = wrap(&line.text, width);
    if chunks.is_empty() {
        vec![String::new()]
    } else {
        chunks
    }
}

/// How many lines from `start` fit whole in `rows` rows (at least one when there is any).
pub fn script_fit(lines: &[ScriptLine], start: usize, rows: usize, width: usize) -> usize {
    let mut used = 0;
    let mut n = 0;
    for l in lines.iter().skip(start) {
        used += script_line_rows(l, width);
        if used > rows && n > 0 {
            break;
        }
        n += 1;
        if used >= rows {
            break;
        }
    }
    n
}

/// How many lines ending just before `end` fit whole in `rows` rows (at least one when there is any).
pub fn script_fit_back(lines: &[ScriptLine], end: usize, rows: usize, width: usize) -> usize {
    let mut used = 0;
    let mut n = 0;
    for l in lines[..end.min(lines.len())].iter().rev() {
        used += script_line_rows(l, width);
        if used > rows && n > 0 {
            break;
        }
        n += 1;
        if used >= rows {
            break;
        }
    }
    n
}

/// Largest first line of the section that still fills the window.
pub fn script_max_scroll(lines: &[ScriptLine], rows: usize, width: usize) -> usize {
    lines.len() - script_fit_back(lines, lines.len(), rows, width)
}

/// Greedy word wrap of `text` to `width` cells; a word longer than a line is cut when drawn.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let cells = |s: &str| ratatui::text::Span::raw(s).width();
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && cells(&line) + 1 + cells(word) > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Details text rows kept above the buttons; with less room the buttons are not shown.
const MIN_DETAIL_ROWS: u16 = 3;

/// Flows the buttons along the foot of `r.details`, wrapping when they do not fit, and takes
/// their rows (and one blank line) from the details text.
fn place_buttons(app: &App, r: &mut Regions) {
    let labels = buttons(app);
    let area = r.details;
    if labels.is_empty() || area.width == 0 {
        return;
    }
    let mut placed: Vec<(u16, u16, u16, ActionId)> = Vec::new();
    let (mut x, mut row) = (0u16, 0u16);
    for (label, id) in labels {
        let w = u16::try_from(label.chars().count() + 2)
            .unwrap_or(u16::MAX)
            .min(area.width);
        if x > 0 && x + w > area.width {
            x = 0;
            row += 1;
        }
        placed.push((x, row, w, id));
        x += w + 1;
    }
    let rows = row + 1;
    if area.height < rows + 1 + MIN_DETAIL_ROWS {
        return;
    }
    let top = area.bottom() - rows;
    r.buttons = placed
        .into_iter()
        .map(|(x, row, w, id)| (Rect::new(area.x + x, top + row, w, 1), id))
        .collect();
    r.details.height = area.height - rows - 1;
}

/// Rows `[start, end)` of the list that `list` shows, as `draw_list` windows them.
pub fn list_visible(app: &App, list: Rect) -> std::ops::Range<usize> {
    let len = app.visible_len();
    let sel = app.selected.min(len.saturating_sub(1));
    list_window(
        sel,
        app.scroll,
        len,
        items_in_rows(usize::from(list.height)),
    )
}

/// Index of the list item under cell `(x, y)`; the gap between items and the space below the
/// last one hit nothing.
pub fn item_at(app: &App, list: Rect, x: u16, y: u16) -> Option<usize> {
    if x < list.x || x >= list.right() || y < list.y || y >= list.bottom() {
        return None;
    }
    let per_item = ITEM_ROWS + usize::from(ITEM_GAP);
    let off = usize::from(y - list.y);
    if off % per_item >= ITEM_ROWS {
        return None;
    }
    let window = list_visible(app, list);
    let index = window.start + off / per_item;
    (index < window.end).then_some(index)
}
