//! The only styles of the CLI output and help (roles follow `tui/theme.rs`: accent, ok, warn,
//! error, muted). Basic 16 ANSI colors so they follow the terminal theme.
use anstream::ColorChoice;
pub use anstyle::Style;
use anstyle::{AnsiColor, Effects};
use std::io::IsTerminal;
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

std::thread_local! {
    static OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

fn enabled() -> bool {
    OVERRIDE
        .with(std::cell::Cell::get)
        .unwrap_or_else(|| ENABLED.load(Ordering::Relaxed))
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

/// Decides from the environment and installs the choice for painting and for `anstream`.
pub fn init(flag: ColorArg) {
    let var = |k: &str| std::env::var_os(k);
    let is_tty = std::io::stdout().is_terminal();
    let no_color = var("NO_COLOR").is_some_and(|v| !v.is_empty());
    let force = var("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0");
    let chosen = choose(flag, is_tty, no_color, force);
    ENABLED.store(chosen == ColorChoice::Always, Ordering::Relaxed);
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
    fn paint_is_plain_when_disabled() {
        assert_eq!(with_enabled(false, || paint(ID, "x")), "x");
        assert!(with_enabled(true, || paint(ID, "x")).contains('\x1b'));
    }
}
