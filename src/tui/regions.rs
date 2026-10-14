//! The one screen geometry: drawing and mouse hit-testing both read it from here.
use crate::tui::app::{ActionId, App, HELP_BAR_MIN_HEIGHT, ITEM_ROWS, Tab, buttons, items_in_rows};
use crate::tui::layout::{FILTER_GAP, ITEM_GAP, PAD_X, inset, split_with_gutter};
use crate::tui::settings::ORDER;
use crate::tui::tabs::tab_rects;
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
}

const TAB_ORDER: [Tab; 4] = [Tab::Search, Tab::Installed, Tab::Updates, Tab::Settings];

/// Line of the settings body (0 = first) that shows setting `i`: a heading, the first four
/// settings, a blank line, a heading, then the rest.
pub fn setting_line(i: usize) -> usize {
    if i < 4 { 1 + i } else { 3 + i }
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
        r.tabs = tab_rects(rows[0], &tab_labels(app, glyphs))
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
        r.setting_rows = (0..ORDER.len())
            .map(setting_line)
            .take_while(|line| *line < body_h)
            .map(|line| {
                let y = r
                    .settings
                    .y
                    .saturating_add(u16::try_from(line).unwrap_or(0));
                Rect::new(r.settings.x, y, r.settings.width, 1)
            })
            .collect();
    }
    r
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
