//! The only styles of the CLI output and help (roles follow `tui/theme.rs`: accent, ok, warn,
//! error, muted). Basic 16 ANSI colors so they follow the terminal theme.
use anstream::ColorChoice;
pub use anstyle::Style;
use anstyle::{AnsiColor, Effects};
use std::sync::atomic::{AtomicBool, Ordering};

const fn fg(c: AnsiColor) -> Style {
    c.on_default()
}

pub const HEADER: Style = fg(AnsiColor::Magenta).effects(Effects::BOLD);
pub const ID: Style = fg(AnsiColor::Cyan);
pub const NAME: Style = Style::new().effects(Effects::BOLD);
pub const VERSION: Style = Style::new();
pub const NEW_VERSION: Style = fg(AnsiColor::Green).effects(Effects::BOLD);
pub const SOURCE: Style = fg(AnsiColor::Blue);
pub const OK: Style = fg(AnsiColor::Green);
pub const WARN: Style = fg(AnsiColor::Yellow);
pub const ERROR: Style = fg(AnsiColor::Red).effects(Effects::BOLD);
pub const DIM: Style = fg(AnsiColor::BrightBlack);
/// File and folder paths.
pub const PATH: Style = fg(AnsiColor::Cyan).effects(Effects::ITALIC);
/// Keys and flags in the help.
pub const KEY: Style = fg(AnsiColor::Cyan).effects(Effects::BOLD);
pub const PLACEHOLDER: Style = fg(AnsiColor::BrightBlack);

pub const CLAP_STYLES: clap::builder::Styles = clap::builder::Styles::styled()
    .header(HEADER)
    .usage(HEADER)
    .literal(KEY)
    .placeholder(PLACEHOLDER)
    .error(ERROR)
    .valid(OK)
    .invalid(WARN);

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Glyph set of the status lines on stdout and on stderr: 0 = unicode (default, and always
/// off a terminal), 1 = nerd, 2 = ascii.
static GLYPHS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static GLYPHS_ERR: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

std::thread_local! {
    /// Set while a stderr line is built ([`on_stderr`]).
    static ON_STDERR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The glyphs of the status lines (`✓`, `⚠`, `✗`, `→` in the unicode set), of stderr's set
/// inside [`on_stderr`], else of stdout's.
pub fn glyphs() -> &'static crate::tui::theme::Glyphs {
    use crate::tui::theme::{GlyphSet, Glyphs};
    let slot = if ON_STDERR.with(std::cell::Cell::get) {
        &GLYPHS_ERR
    } else {
        &GLYPHS
    };
    Glyphs::for_set(match slot.load(Ordering::Relaxed) {
        1 => GlyphSet::Nerd,
        2 => GlyphSet::Ascii,
        _ => GlyphSet::Unicode,
    })
}

/// Builds a stderr line: the glyphs inside `f` follow stderr's terminal, not stdout's.
pub fn on_stderr<T>(f: impl FnOnce() -> T) -> T {
    let prev = ON_STDERR.with(|o| o.replace(true));
    let out = f();
    ON_STDERR.with(|o| o.set(prev));
    out
}

/// The glyph code of one stream: `set` on a terminal, unicode off it.
fn glyph_code(set: crate::tui::theme::GlyphSet, tty: bool) -> u8 {
    use crate::tui::theme::GlyphSet;
    match set {
        _ if !tty => 0,
        GlyphSet::Nerd => 1,
        GlyphSet::Ascii => 2,
        GlyphSet::Unicode => 0,
    }
}

/// Uses `set` for the status lines of each stream that is a terminal; a stream off a terminal
/// stays unicode (a log or a pipe is read without the Nerd font).
pub fn init_glyphs(set: crate::tui::theme::GlyphSet) {
    let out = glyph_code(set, super::emit::stdout_is_terminal());
    let err = glyph_code(set, super::emit::stderr_is_terminal());
    GLYPHS.store(out, Ordering::Relaxed);
    GLYPHS_ERR.store(err, Ordering::Relaxed);
}

std::thread_local! {
    static OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

fn enabled() -> bool {
    OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or_else(|| ENABLED.load(Ordering::Relaxed))
}

/// Columns of the terminal stdout writes to; 0 = not a terminal (tables keep full cells).
static TABLE_COLUMNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

std::thread_local! {
    static COLUMNS_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The width tables must fit in (stdout's terminal), or `None` off a terminal.
pub fn table_columns() -> Option<usize> {
    let n = COLUMNS_OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or_else(|| TABLE_COLUMNS.load(Ordering::Relaxed));
    (n > 0).then_some(n)
}

/// Runs `f` as if stdout were a terminal of `cols` columns (0 = not a terminal) (tests).
pub fn with_columns<T>(cols: usize, f: impl FnOnce() -> T) -> T {
    let prev = COLUMNS_OVERRIDE.with(|o| o.replace(Some(cols)));
    let out = f();
    COLUMNS_OVERRIDE.with(|o| o.set(prev));
    out
}

/// Runs `f` with painting forced on or off on this thread (tests).
pub fn with_enabled<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let prev = OVERRIDE.with(|o| o.replace(Some(on)));
    let out = f();
    OVERRIDE.with(|o| o.set(prev));
    out
}

/// `text` in `style`, always (help text is built before the color choice exists).
pub fn paint_raw(style: Style, text: &str) -> String {
    format!("{style}{text}{style:#}")
}

/// `text` in `style` when the output is colored, else the text itself.
pub fn paint(style: Style, text: &str) -> String {
    if text.is_empty() || !enabled() {
        text.to_string()
    } else {
        paint_raw(style, text)
    }
}

/// A color of `tui/theme.rs` as the CLI writes it (`Reset` = the terminal's own).
fn color_of(c: ratatui::style::Color) -> Option<anstyle::Color> {
    use ratatui::style::Color as C;
    let ansi = match c {
        C::Reset => return None,
        C::Rgb(r, g, b) => return Some(anstyle::RgbColor(r, g, b).into()),
        C::Indexed(i) => return Some(anstyle::Ansi256Color(i).into()),
        C::Black => AnsiColor::Black,
        C::Red => AnsiColor::Red,
        C::Green => AnsiColor::Green,
        C::Yellow => AnsiColor::Yellow,
        C::Blue => AnsiColor::Blue,
        C::Magenta => AnsiColor::Magenta,
        C::Cyan => AnsiColor::Cyan,
        C::Gray => AnsiColor::White,
        C::DarkGray => AnsiColor::BrightBlack,
        C::LightRed => AnsiColor::BrightRed,
        C::LightGreen => AnsiColor::BrightGreen,
        C::LightYellow => AnsiColor::BrightYellow,
        C::LightBlue => AnsiColor::BrightBlue,
        C::LightMagenta => AnsiColor::BrightMagenta,
        C::LightCyan => AnsiColor::BrightCyan,
        C::White => AnsiColor::BrightWhite,
    };
    Some(ansi.into())
}

/// A token of `tui/theme.rs` as a CLI style: the same color at the theme's depth (RGB,
/// 256 colors or none) and its bold, dim, italic and underline.
pub fn from_theme(s: ratatui::style::Style) -> Style {
    use ratatui::style::Modifier;
    let mut out = Style::new()
        .fg_color(s.fg.and_then(color_of))
        .bg_color(s.bg.and_then(color_of));
    for (m, e) in [
        (Modifier::BOLD, Effects::BOLD),
        (Modifier::DIM, Effects::DIMMED),
        (Modifier::ITALIC, Effects::ITALIC),
        (Modifier::UNDERLINED, Effects::UNDERLINE),
    ] {
        if s.add_modifier.contains(m) {
            out = out.effects(out.get_effects() | e);
        }
    }
    out
}

/// Colors of the CLI progress line, from the TUI theme (Task 216).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProgressStyles {
    pub spinner: Style,
    /// The done cells of the download bar.
    pub fill: Style,
    /// The missing cells.
    pub empty: Style,
}

impl ProgressStyles {
    /// Spinner and bar in `accent`, the missing cells in `muted`.
    pub fn of(theme: &crate::tui::theme::Theme) -> ProgressStyles {
        ProgressStyles {
            spinner: from_theme(theme.accent()),
            fill: from_theme(theme.accent()),
            empty: from_theme(theme.muted()),
        }
    }
}

/// The color decision: `--color` beats the variables, `NO_COLOR` beats `auto`, then
/// `CLICOLOR_FORCE`, then whether stdout is a terminal. Never returns `Auto`.
pub fn choose(flag: ColorArg, is_tty: bool, no_color: bool, clicolor_force: bool) -> ColorChoice {
    match flag {
        ColorArg::Always => ColorChoice::Always,
        ColorArg::Never => ColorChoice::Never,
        ColorArg::Auto if no_color => ColorChoice::Never,
        ColorArg::Auto if clicolor_force || is_tty => ColorChoice::Always,
        ColorArg::Auto => ColorChoice::Never,
    }
}

/// `CLICOLOR_FORCE` forces color unless unset, empty or `0`.
fn force_from(v: Option<&std::ffi::OsStr>) -> bool {
    v.is_some_and(|v| !v.is_empty() && v != "0")
}

/// Decides from the environment and installs the choice for painting and for `anstream`.
pub fn init(flag: ColorArg) {
    let var = |k: &str| std::env::var_os(k);
    let is_tty = super::emit::stdout_is_terminal();
    let no_color = var("NO_COLOR").is_some_and(|v| !v.is_empty());
    let force = force_from(var("CLICOLOR_FORCE").as_deref());
    let chosen = choose(flag, is_tty, no_color, force);
    ENABLED.store(chosen == ColorChoice::Always, Ordering::Relaxed);
    if is_tty && let Ok((cols, _)) = crossterm::terminal::size() {
        TABLE_COLUMNS.store(usize::from(cols), Ordering::Relaxed);
    }
    // On a terminal let anstream enable VT (or convert) itself.
    let global = if flag == ColorArg::Auto && chosen == ColorChoice::Always && is_tty && !force {
        ColorChoice::Auto
    } else {
        chosen
    };
    global.write_global();
}

/// Root help examples with the command words in `KEY` (text unchanged).
pub fn root_after_help() -> String {
    let mut out = String::from("Exemplos:\n");
    for (cmd, note) in [
        ("wingetrs", "abre a TUI"),
        ("wingetrs update", "atualiza o catálogo"),
        ("wingetrs search powertoys", "busca pacotes"),
        ("wingetrs show Microsoft.PowerToys", ""),
        (
            "wingetrs list --upgradable",
            "lista pacotes com atualização",
        ),
        ("wingetrs install ripgrep", "instala um app do Scoop"),
        (
            "wingetrs upgrade --all",
            "atualiza tudo que o wingetrs instalou",
        ),
    ] {
        out.push_str("  ");
        out.push_str(&paint_raw(KEY, cmd));
        if !note.is_empty() {
            let pad = 31usize.saturating_sub(cmd.chars().count());
            out.push_str(&" ".repeat(pad.max(1)));
            out.push_str(note);
        }
        out.push('\n');
    }
    out.pop();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_choice_table() {
        use ColorArg::*;
        use ColorChoice::{Always as Y, Never as N};
        // (flag, tty, NO_COLOR, CLICOLOR_FORCE) -> choice
        let table = [
            (Auto, false, false, false, N),
            (Auto, true, false, false, Y),
            (Auto, true, true, false, N),
            (Auto, false, false, true, Y),
            (Auto, true, true, true, N),
            (Always, false, true, false, Y),
            (Never, true, false, true, N),
            (Always, false, false, false, Y),
        ];
        for (flag, tty, nc, force, want) in table {
            assert_eq!(
                choose(flag, tty, nc, force),
                want,
                "{flag:?} {tty} {nc} {force}"
            );
        }
    }

    #[test]
    fn clicolor_force_zero_and_empty_do_not_force() {
        use std::ffi::OsStr;
        assert!(!force_from(None));
        assert!(!force_from(Some(OsStr::new(""))));
        assert!(!force_from(Some(OsStr::new("0"))));
        assert!(force_from(Some(OsStr::new("1"))));
    }

    #[test]
    fn glyphs_follow_each_stream() {
        use crate::tui::theme::GlyphSet::{Ascii, Nerd, Unicode};
        assert_eq!(glyph_code(Nerd, true), 1);
        assert_eq!(glyph_code(Nerd, false), 0);
        assert_eq!(glyph_code(Ascii, true), 2);
        assert_eq!(glyph_code(Unicode, true), 0);
        // Stderr's slot inside `on_stderr`, stdout's outside (both unicode by default).
        assert!(on_stderr(|| ON_STDERR.with(std::cell::Cell::get)));
        assert!(!ON_STDERR.with(std::cell::Cell::get));
    }

    #[test]
    fn paint_is_plain_when_disabled() {
        assert_eq!(with_enabled(false, || paint(ID, "x")), "x");
        assert!(with_enabled(true, || paint(ID, "x")).contains('\x1b'));
    }
}
