//! The CLI's one choice component (Task 200): an inline (no alternate screen) arrow-key
//! selector drawn with crossterm. Every question of the CLI (yes/no, one of N, several with
//! Space) goes through it. The terminal is a [`Term`], so tests inject keys and read the
//! drawing; [`StdTerm`] is the real one.
use crate::tui::picker::{PickOutcome, Picker, PickerItem, window};
use crate::tui::theme::{Glyphs, Theme};
use crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, SetAttribute, SetStyle};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use ratatui::backend::IntoCrossterm;
use ratatui::style::Style;
use ratatui::text::Span;
use std::io::{self, IsTerminal, Write};

/// Error text when a question is asked without a terminal (callers check first and name
/// the flag to use; this is the last line of defence).
pub const NO_TTY: &str = "seletor exige terminal interativo";

/// True when both stdin and stdout are terminals (the selector refuses otherwise).
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// The terminal the selector draws on and reads keys from.
pub trait Term {
    /// Both stdin and stdout are terminals.
    fn interactive(&self) -> bool;
    fn enable_raw(&mut self) -> io::Result<()>;
    fn disable_raw(&mut self) -> io::Result<()>;
    /// `(columns, rows)`.
    fn size(&self) -> io::Result<(u16, u16)>;
    /// The next input event (blocks).
    fn read(&mut self) -> io::Result<Event>;
    fn out(&mut self) -> &mut dyn Write;
}

/// The process's own terminal: crossterm raw mode, events and stdout.
pub struct StdTerm {
    out: io::Stdout,
}

impl StdTerm {
    pub fn new() -> StdTerm {
        StdTerm { out: io::stdout() }
    }
}

impl Default for StdTerm {
    fn default() -> StdTerm {
        StdTerm::new()
    }
}

impl Term for StdTerm {
    fn interactive(&self) -> bool {
        is_interactive()
    }
    fn enable_raw(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()
    }
    fn disable_raw(&mut self) -> io::Result<()> {
        terminal::disable_raw_mode()
    }
    fn size(&self) -> io::Result<(u16, u16)> {
        terminal::size()
    }
    fn read(&mut self) -> io::Result<Event> {
        crossterm::event::read()
    }
    fn out(&mut self) -> &mut dyn Write {
        &mut self.out
    }
}

/// What a drawn line is; the theme token is chosen from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Prompt,
    /// The row under the cursor.
    Current,
    Normal,
    /// A row that cannot be picked.
    Disabled,
    Hint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameLine {
    pub text: String,
    pub kind: LineKind,
}

/// Shortcut letter of each row: the first letter or digit of its label, lowercased, when it
/// is pickable, not `q` (cancel) and no other pickable row starts with it.
pub fn shortcuts(items: &[PickerItem]) -> Vec<Option<char>> {
    let first = |it: &PickerItem| {
        it.label
            .chars()
            .find(|c| c.is_alphanumeric())
            .and_then(|c| c.to_lowercase().next())
    };
    let firsts: Vec<Option<char>> = items
        .iter()
        .map(|it| if it.selectable { first(it) } else { None })
        .collect();
    firsts
        .iter()
        .map(|c| c.filter(|&c| c != 'q' && firsts.iter().filter(|o| **o == Some(c)).count() == 1))
        .collect()
}

/// What a shortcut letter does in a single choice. In several choices it always moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Letters {
    /// The letter picks its row at once (yes/no, the Kindle options).
    Pick,
    /// The letter only moves the cursor; Enter confirms (targets that share a name).
    Move,
}

/// The lines of one frame: the prompt (when any), the visible rows, the key hint. In a single
/// choice each row shows its shortcut letter (`s) Sim`) when any row has one.
pub fn frame(
    p: &Picker,
    height: usize,
    prompt: Option<&str>,
    letters: Letters,
    glyphs: &Glyphs,
) -> Vec<FrameLine> {
    let mut lines = Vec::new();
    if let Some(q) = prompt {
        lines.push(FrameLine {
            text: q.to_string(),
            kind: LineKind::Prompt,
        });
    }
    let keys = shortcuts(p.items());
    let show_keys = !p.is_multi() && keys.iter().any(Option::is_some);
    for i in window(p.selected(), p.items().len(), height) {
        let it = &p.items()[i];
        let here = i == p.selected();
        let marker = if here { glyphs.selected } else { " " };
        let boxed = match (p.is_multi(), p.checked()[i]) {
            (false, _) => "",
            (true, true) => "[x] ",
            (true, false) => "[ ] ",
        };
        let key = match (show_keys, keys[i]) {
            (false, _) => String::new(),
            (true, Some(c)) => format!("{c}) "),
            (true, None) => "   ".to_string(),
        };
        let mut text = format!("{marker} {boxed}{key}{}", it.label);
        if !it.detail.is_empty() {
            text.push_str(&format!("  {}", it.detail));
        }
        if let Some(n) = &it.note {
            text.push_str(&format!("  ({n})"));
        }
        let kind = if !it.selectable {
            LineKind::Disabled
        } else if here {
            LineKind::Current
        } else {
            LineKind::Normal
        };
        lines.push(FrameLine { text, kind });
    }
    let hint = match (p.is_multi(), letters) {
        (true, _) => "setas movem · Espaço marca · Enter confirma · Esc cancela",
        (false, Letters::Pick) => "setas movem · letra ou Enter escolhe · Esc cancela",
        (false, Letters::Move) => "setas ou letra movem · Enter escolhe · Esc cancela",
    };
    lines.push(FrameLine {
        text: hint.to_string(),
        kind: LineKind::Hint,
    });
    lines
}

/// The one line left on screen once the choice is made.
pub fn summary(
    items: &[PickerItem],
    picked: &[usize],
    prompt: Option<&str>,
    glyphs: &Glyphs,
) -> String {
    let labels: Vec<&str> = picked
        .iter()
        .filter_map(|&i| items.get(i))
        .map(|it| it.label.as_str())
        .collect();
    let chosen = if labels.is_empty() {
        "nenhum".to_string()
    } else {
        labels.join(", ")
    };
    match prompt {
        Some(q) => format!("{q} {chosen}"),
        None => format!("{} {chosen}", glyphs.selected),
    }
}

/// Raw mode, hidden cursor and the drawn lines, undone on every exit path (error, panic).
struct Session<'t> {
    term: &'t mut dyn Term,
    drawn: u16,
}

impl<'t> Session<'t> {
    fn enter(term: &'t mut dyn Term) -> io::Result<Session<'t>> {
        term.enable_raw()?;
        let s = Session { term, drawn: 0 };
        // `&mut &mut dyn Write`: the crossterm macros need a sized writer.
        execute!(&mut s.term.out(), Hide)?;
        Ok(s)
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let out = &mut self.term.out();
        if self.drawn > 0 {
            // Best effort: erase the lines left behind (also on I/O errors and panics).
            let _ = execute!(
                out,
                MoveUp(self.drawn),
                MoveToColumn(0),
                Clear(ClearType::FromCursorDown)
            );
        }
        let _ = execute!(out, Show, SetAttribute(Attribute::Reset));
        let _ = self.term.disable_raw();
    }
}

fn style_of(kind: LineKind, theme: &Theme) -> Style {
    match kind {
        LineKind::Prompt | LineKind::Normal => Style::new(),
        LineKind::Current => theme.accent(),
        LineKind::Disabled | LineKind::Hint => theme.muted(),
    }
}

/// The longest prefix of `text` that fits in `width` terminal columns (wide CJK and emoji
/// characters count two).
pub fn fit(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let mut buf = [0u8; 4];
        let w = Span::raw(&*c.encode_utf8(&mut buf)).width();
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

/// Draws `lines`, first erasing the `drawn` lines of the previous frame (its height may
/// differ after a resize).
fn draw(
    out: &mut dyn Write,
    lines: &[FrameLine],
    theme: &Theme,
    cols: u16,
    drawn: u16,
) -> io::Result<()> {
    let out = &mut &mut *out;
    if drawn > 0 {
        queue!(
            out,
            MoveUp(drawn),
            MoveToColumn(0),
            Clear(ClearType::FromCursorDown)
        )?;
    }
    // One column short so a full line never wraps (the move-up count would be wrong).
    let width = usize::from(cols.saturating_sub(1)).max(1);
    for l in lines {
        queue!(
            out,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            SetStyle(style_of(l.kind, theme).into_crossterm())
        )?;
        write!(out, "{}", fit(&l.text, width))?;
        queue!(out, SetAttribute(Attribute::Reset))?;
        write!(out, "\r\n")?;
    }
    out.flush()
}

/// What a key does before the picker sees it: `q` cancels, a shortcut letter picks its row
/// (one choice with [`Letters::Pick`]) or moves the cursor to it. `None` = the picker decides.
fn pre_key(
    p: &mut Picker,
    keys: &[Option<char>],
    letters: Letters,
    k: KeyEvent,
) -> Option<PickOutcome> {
    if k.kind == KeyEventKind::Release
        || k.modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let KeyCode::Char(c) = k.code else {
        return None;
    };
    let c = c.to_lowercase().next()?;
    if c == 'q' {
        return Some(PickOutcome::Cancelled);
    }
    let i = keys.iter().position(|s| *s == Some(c))?;
    if !p.is_multi() && letters == Letters::Pick {
        return Some(PickOutcome::Picked(vec![i]));
    }
    // Move the cursor through the picker's own keys: Home, then Down over pickable rows.
    let press = |code| KeyEvent::new(code, KeyModifiers::NONE);
    p.key(press(KeyCode::Home));
    while p.selected() < i {
        let before = p.selected();
        p.key(press(KeyCode::Down));
        if p.selected() == before {
            break;
        }
    }
    Some(PickOutcome::Pending)
}

/// Rows of the window for a terminal of `rows` lines (prompt and hint take the rest).
fn window_height(rows: u16, prompt: Option<&str>) -> usize {
    let fixed = 1 + usize::from(prompt.is_some());
    usize::from(rows).saturating_sub(fixed + 1).clamp(1, 15)
}

/// Runs `p` on `term` until a pick or a cancel (Esc, `q`, Ctrl+C). The drawing is erased at
/// the end and, after a pick, one line with the choice is left. Without a terminal nothing is
/// drawn nor read: an `Unsupported` error.
pub fn run(
    term: &mut dyn Term,
    mut p: Picker,
    prompt: Option<&str>,
    letters: Letters,
    theme: &Theme,
    glyphs: &Glyphs,
) -> io::Result<Option<Vec<usize>>> {
    if !term.interactive() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, NO_TTY));
    }
    let keys = shortcuts(p.items());
    let (mut cols, rows) = term.size()?;
    let mut height = window_height(rows, prompt);
    let mut s = Session::enter(term)?;
    let lines = frame(&p, height, prompt, letters, glyphs);
    draw(s.term.out(), &lines, theme, cols, 0)?;
    s.drawn = lines.len() as u16;
    let result = loop {
        match s.term.read()? {
            Event::Key(k) => {
                let outcome = match pre_key(&mut p, &keys, letters, k) {
                    Some(o) => o,
                    None => p.key(k),
                };
                match outcome {
                    PickOutcome::Pending => {}
                    PickOutcome::Cancelled => break None,
                    PickOutcome::Picked(v) => break Some(v),
                }
            }
            Event::Resize(c, r) => {
                cols = c;
                height = window_height(r, prompt);
            }
            _ => continue,
        }
        // The window follows the selection (and the terminal size after a resize).
        let lines = frame(&p, height, prompt, letters, glyphs);
        let drawn = s.drawn;
        draw(s.term.out(), &lines, theme, cols, drawn)?;
        s.drawn = lines.len() as u16;
    };
    drop(s);
    if let Some(v) = &result {
        let line = summary(p.items(), v, prompt, glyphs);
        let out = term.out();
        write!(out, "{line}\r\n")?;
        out.flush()?;
    }
    Ok(result)
}

/// One of `items`, the cursor starting on `default` (else the first pickable row).
/// `None` = cancelled.
pub fn select_one(
    term: &mut dyn Term,
    prompt: Option<&str>,
    items: Vec<PickerItem>,
    default: Option<usize>,
    letters: Letters,
    theme: &Theme,
    glyphs: &Glyphs,
) -> io::Result<Option<usize>> {
    let p = match default {
        Some(at) => Picker::single_at(items, at),
        None => Picker::single(items),
    };
    let picked = run(term, p, prompt, letters, theme, glyphs)?;
    Ok(picked.and_then(|v| v.first().copied()))
}

/// Several of `items` (Space marks), `initial` rows pre-marked. `None` = cancelled.
pub fn select_many(
    term: &mut dyn Term,
    prompt: Option<&str>,
    items: Vec<PickerItem>,
    initial: &[bool],
    theme: &Theme,
    glyphs: &Glyphs,
) -> io::Result<Option<Vec<usize>>> {
    run(
        term,
        Picker::multi(items, initial),
        prompt,
        Letters::Move,
        theme,
        glyphs,
    )
}

const YES_NO_HINT: &str = "←/→ escolher · Enter confirmar · s/n · Esc = Não";

/// Draws the question with its two buttons (the focused one highlighted) and the hint line,
/// first erasing the `drawn` lines of the previous frame.
fn draw_buttons(
    out: &mut dyn Write,
    question: &str,
    yes_focus: bool,
    theme: &Theme,
    cols: u16,
    drawn: u16,
) -> io::Result<()> {
    let out = &mut &mut *out;
    if drawn > 0 {
        queue!(
            out,
            MoveUp(drawn),
            MoveToColumn(0),
            Clear(ClearType::FromCursorDown)
        )?;
    }
    let width = usize::from(cols.saturating_sub(1)).max(1);
    // The buttons keep their room; a long question is the one that gets cut.
    let buttons = "  [ Sim ]  [ Não ]".chars().count();
    let q = fit(question, width.saturating_sub(buttons).max(1));
    queue!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    write!(out, "{q}")?;
    let focus = theme
        .accent()
        .add_modifier(ratatui::style::Modifier::REVERSED);
    for (label, focused) in [("Sim", yes_focus), ("Não", !yes_focus)] {
        let style = if focused { focus } else { theme.muted() };
        write!(out, "  ")?;
        queue!(out, SetStyle(style.into_crossterm()))?;
        write!(out, "[ {label} ]")?;
        queue!(out, SetAttribute(Attribute::Reset))?;
    }
    write!(out, "\r\n")?;
    queue!(
        out,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        SetStyle(theme.muted().into_crossterm())
    )?;
    write!(out, "{}", fit(YES_NO_HINT, width))?;
    queue!(out, SetAttribute(Attribute::Reset))?;
    write!(out, "\r\n")?;
    out.flush()
}

/// Yes/no as two buttons on one line; focus starts on "Não" and cancelling is "Não".
/// `←`/`→`/Tab move the focus, Enter takes it, `s`/`n` answer at once.
pub fn yes_no(
    term: &mut dyn Term,
    question: &str,
    theme: &Theme,
    glyphs: &Glyphs,
) -> io::Result<bool> {
    let _ = glyphs;
    if !term.interactive() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, NO_TTY));
    }
    let (mut cols, _) = term.size()?;
    let mut s = Session::enter(term)?;
    let mut yes = false;
    draw_buttons(s.term.out(), question, yes, theme, cols, 0)?;
    s.drawn = 2;
    let answer = loop {
        match s.term.read()? {
            Event::Key(k) => {
                if k.kind == KeyEventKind::Release {
                    continue;
                }
                if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                    break false;
                }
                if k.modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    continue;
                }
                match k.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                        yes = !yes;
                    }
                    KeyCode::Enter => break yes,
                    KeyCode::Esc => break false,
                    KeyCode::Char(c) => match c.to_lowercase().next() {
                        Some('s') => break true,
                        Some('n' | 'q') => break false,
                        _ => continue,
                    },
                    _ => continue,
                }
            }
            Event::Resize(c, _) => cols = c,
            _ => continue,
        }
        let drawn = s.drawn;
        draw_buttons(s.term.out(), question, yes, theme, cols, drawn)?;
    };
    drop(s);
    let out = term.out();
    write!(out, "{question} {}\r\n", if answer { "Sim" } else { "Não" })?;
    out.flush()?;
    Ok(answer)
}

/// A scripted [`Term`] for tests: keys come from a list, the drawing goes to a buffer, and
/// the raw-mode state is shared so a test can check it after the selector returned.
pub mod fake {
    use super::Term;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::collections::VecDeque;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex, PoisonError};

    /// What the selector did to the terminal.
    #[derive(Debug, Default)]
    pub struct Screen {
        pub out: Vec<u8>,
        pub raw: bool,
        pub raw_enabled: usize,
        pub reads: usize,
    }

    /// Writes through to the shared [`Screen`].
    pub struct Out(Arc<Mutex<Screen>>);

    impl Write for Out {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            s.out.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub struct ScriptedTerm {
        pub interactive: bool,
        pub size: (u16, u16),
        pub keys: VecDeque<Event>,
        /// Panics on the read after the scripted keys ran out (the guard test).
        pub panic_when_empty: bool,
        screen: Arc<Mutex<Screen>>,
        out: Out,
    }

    impl ScriptedTerm {
        /// An interactive 80x24 terminal that answers with `keys`, then fails reading.
        pub fn new(keys: Vec<Event>) -> ScriptedTerm {
            let screen = Arc::new(Mutex::new(Screen::default()));
            ScriptedTerm {
                interactive: true,
                size: (80, 24),
                keys: keys.into(),
                panic_when_empty: false,
                out: Out(Arc::clone(&screen)),
                screen,
            }
        }

        pub fn screen(&self) -> Arc<Mutex<Screen>> {
            Arc::clone(&self.screen)
        }
    }

    /// A key press.
    pub fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Ctrl+C as the terminal sends it in raw mode.
    pub fn ctrl_c() -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
    }

    /// `bytes` without the escape sequences (CSI `ESC [ ... final`), as text.
    pub fn plain(bytes: &[u8]) -> String {
        let text = String::from_utf8_lossy(bytes);
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for f in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&f) {
                            break;
                        }
                    }
                }
                continue;
            }
            out.push(c);
        }
        out
    }

    impl Term for ScriptedTerm {
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn enable_raw(&mut self) -> io::Result<()> {
            let mut s = self.screen.lock().unwrap_or_else(PoisonError::into_inner);
            s.raw = true;
            s.raw_enabled += 1;
            Ok(())
        }
        fn disable_raw(&mut self) -> io::Result<()> {
            self.screen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .raw = false;
            Ok(())
        }
        fn size(&self) -> io::Result<(u16, u16)> {
            Ok(self.size)
        }
        fn read(&mut self) -> io::Result<Event> {
            self.screen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .reads += 1;
            match self.keys.pop_front() {
                Some(e) => Ok(e),
                None if self.panic_when_empty => panic!("scripted keys ran out"),
                None => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "scripted keys ran out",
                )),
            }
        }
        fn out(&mut self) -> &mut dyn Write {
            &mut self.out
        }
    }
}
