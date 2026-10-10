//! Inline (no alternate screen) arrow-key picker drawn with crossterm.
use crate::tui::picker::{PickOutcome, Picker, PickerItem, window};
use crate::tui::theme::{Glyphs, Theme};
use crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use crossterm::event::{Event, read};
use crossterm::style::{Attribute, SetAttribute, SetStyle};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use ratatui::backend::IntoCrossterm;
use ratatui::style::Style;
use std::io::{self, IsTerminal, Write};
use std::ops::Range;

/// True when both stdin and stdout are terminals (the picker must not be called otherwise).
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Restores raw mode and the cursor on every exit path, including errors.
struct Guard {
    drawn: u16,
}

impl Guard {
    fn enter() -> io::Result<Guard> {
        terminal::enable_raw_mode()?;
        let g = Guard { drawn: 0 };
        execute!(io::stdout(), Hide)?;
        Ok(g)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.drawn > 0 {
            // Best effort: erase the lines left behind (also on I/O errors).
            let _ = execute!(
                io::stdout(),
                MoveUp(self.drawn),
                MoveToColumn(0),
                Clear(ClearType::FromCursorDown)
            );
        }
        let _ = execute!(io::stdout(), Show, SetAttribute(Attribute::Reset));
        let _ = terminal::disable_raw_mode();
    }
}

fn apply(out: &mut impl Write, s: Style) -> io::Result<()> {
    queue!(out, SetStyle(s.into_crossterm()))
}

fn draw(
    out: &mut impl Write,
    p: &Picker,
    range: &Range<usize>,
    theme: &Theme,
    glyphs: &Glyphs,
    redraw: bool,
) -> io::Result<()> {
    if redraw && !range.is_empty() {
        queue!(out, MoveUp(range.len() as u16))?;
    }
    for i in range.clone() {
        let it: &PickerItem = &p.items()[i];
        queue!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        let here = i == p.selected();
        let marker = if here { glyphs.selected } else { " " };
        let boxed = match (p.is_multi(), p.checked()[i]) {
            (false, _) => "",
            (true, true) => "[x] ",
            (true, false) => "[ ] ",
        };
        let style = if !it.selectable {
            theme.muted()
        } else if here {
            theme.accent()
        } else {
            Style::new()
        };
        apply(out, style)?;
        write!(out, "{marker} {boxed}{}", it.label)?;
        if !it.detail.is_empty() {
            write!(out, "  {}", it.detail)?;
        }
        if let Some(n) = &it.note {
            write!(out, "  ({n})")?;
        }
        queue!(out, SetAttribute(Attribute::Reset))?;
        write!(out, "\r\n")?;
    }
    out.flush()
}

/// Single-choice picker; returns the picked index or `None` when cancelled.
/// Not unit-testable without a TTY: the pure state lives in `tui::picker`.
pub fn pick_inline(
    items: Vec<PickerItem>,
    theme: &Theme,
    glyphs: &Glyphs,
) -> io::Result<Option<usize>> {
    let picked = run(Picker::single(items), theme, glyphs)?;
    Ok(picked.and_then(|v| v.first().copied()))
}

/// Multiple-choice picker with `initial` rows pre-checked; `None` when cancelled.
pub fn pick_inline_multi(
    items: Vec<PickerItem>,
    initial: &[bool],
    theme: &Theme,
    glyphs: &Glyphs,
) -> io::Result<Option<Vec<usize>>> {
    run(Picker::multi(items, initial), theme, glyphs)
}

fn run(mut p: Picker, theme: &Theme, glyphs: &Glyphs) -> io::Result<Option<Vec<usize>>> {
    if !is_interactive() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "seletor exige terminal interativo",
        ));
    }
    let mut guard = Guard::enter()?;
    let (_, rows) = terminal::size()?;
    let height = usize::from(rows.saturating_sub(2)).clamp(1, 15);
    let mut out = io::stdout();
    let range = window(p.selected(), p.items().len(), height);
    draw(&mut out, &p, &range, theme, glyphs, false)?;
    guard.drawn = range.len() as u16;
    let result = loop {
        let Event::Key(k) = read()? else { continue };
        match p.key(k) {
            PickOutcome::Pending => {}
            PickOutcome::Cancelled => break None,
            PickOutcome::Picked(v) => break Some(v),
        }
        // The window height is constant, but its start follows the selection.
        let range = window(p.selected(), p.items().len(), height);
        draw(&mut out, &p, &range, theme, glyphs, true)?;
    };
    drop(guard);
    Ok(result)
}
