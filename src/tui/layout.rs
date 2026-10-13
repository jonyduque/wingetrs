//! The one spacing scale for every box, dialog and bar (tui-design §2.8).
//! Drawing code uses these names; no loose spacing numbers elsewhere.
use ratatui::layout::Rect;

/// Columns between a box border and its text.
pub const PAD_X: u16 = 1;
/// Blank lines between the search line and the list.
pub const FILTER_GAP: u16 = 1;
/// Blank columns on each side of the list/details divider.
pub const GUTTER: u16 = 1;
/// Blank lines between list items (D10: none).
pub const ITEM_GAP: u16 = 0;
/// Columns between a dialog border and its text.
pub const DIALOG_PAD_X: u16 = 2;
/// Lines between a dialog border and its text.
pub const DIALOG_PAD_Y: u16 = 1;
/// Columns between a toast border and its text.
pub const TOAST_PAD_X: u16 = 1;
/// Columns between a toast and the screen edge.
pub const TOAST_MARGIN_X: u16 = 2;
/// Lines between a toast and the screen edge.
pub const TOAST_MARGIN_Y: u16 = 1;
/// Columns of padding on each side of a tab label.
pub const TAB_PAD_X: u16 = 1;
/// Columns before the status bar text.
pub const BAR_PAD_X: u16 = 1;
/// Border thickness per side, in cells.
pub const BORDER: u16 = 1;

/// Shrinks `area` by `x` columns on each side and `y` lines top and bottom;
/// an area smaller than the inset collapses to width/height 0.
pub fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(x.min(area.width)),
        area.y.saturating_add(y.min(area.height)),
        area.width.saturating_sub(x.saturating_mul(2)),
        area.height.saturating_sub(y.saturating_mul(2)),
    )
}

/// Splits `area` into `(left, divider_column, right)` with `GUTTER` blank columns
/// on each side of the divider. `left` is `width * left_percent / 100` columns
/// wide (before the gaps); too-narrow areas saturate to 0.
pub fn split_with_gutter(area: Rect, left_percent: u16) -> (Rect, u16, Rect) {
    let left_w = u16::try_from(u32::from(area.width) * u32::from(left_percent.min(100)) / 100)
        .unwrap_or(area.width);
    let left = Rect::new(area.x, area.y, left_w, area.height);
    let divider = area
        .x
        .saturating_add(left_w)
        .saturating_add(GUTTER)
        .min(area.right());
    let right_x = divider.saturating_add(1 + GUTTER).min(area.right());
    let right = Rect::new(right_x, area.y, area.right() - right_x, area.height);
    (left, divider, right)
}
/// Lines of the details pane beside the package icon.
pub const DETAIL_ICON_ROWS: u16 = 4;
/// Columns of the package icon.
pub const DETAIL_ICON_COLS: u16 = 8;
/// Blank columns between the icon and the text beside it.
pub const DETAIL_ICON_GAP: u16 = 2;

/// Splits the top `rows` lines of `area` into `(icon, text)`: the icon takes `cols` columns
/// at the left and the text starts `gap` columns after it; too-narrow areas saturate to 0.
pub fn split_icon(area: Rect, rows: u16, cols: u16, gap: u16) -> (Rect, Rect) {
    let h = rows.min(area.height);
    let icon = Rect::new(area.x, area.y, cols.min(area.width), h);
    let text_x = area
        .x
        .saturating_add(cols)
        .saturating_add(gap)
        .min(area.right());
    let text = Rect::new(text_x, area.y, area.right() - text_x, h);
    (icon, text)
}

/// Lines a picker dialog reserves beyond border and padding: the footer line plus one spare.
pub const PICKER_FOOTER_SLACK: u16 = 2;

/// Columns of a package icon in the list (its height is `ITEM_ROWS`).
pub const LIST_ICON_COLS: u16 = 4;
/// Blank columns between a list icon and the item text.
pub const LIST_ICON_GAP: u16 = 1;
