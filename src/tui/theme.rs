//! Color tokens, glyph sets and color-depth detection (spec §7.3).
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSet {
    Nerd,
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    NoColor,
}

pub struct Glyphs {
    pub winget: &'static str,
    pub scoop: &'static str,
    pub search: &'static str,
    pub ok: &'static str,
    pub upgrade: &'static str,
    pub unknown: &'static str,
    pub ambiguous: &'static str,
    pub refresh: &'static str,
    pub offline: &'static str,
    pub license: &'static str,
    pub publisher: &'static str,
    pub id: &'static str,
    pub selected: &'static str,
    pub spinner: &'static [&'static str],
}

const BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: &[&str] = &["|", "/", "-", "\\"];

static NERD: Glyphs = Glyphs {
    winget: "\u{f17a}",
    scoop: "\u{f1b3}",
    search: "\u{f002}",
    ok: "\u{f00c}",
    upgrade: "\u{f062}",
    unknown: "\u{f128}",
    ambiguous: "\u{f071}",
    refresh: "\u{f021}",
    offline: "\u{f127}",
    license: "\u{f24e}",
    publisher: "\u{f007}",
    id: "\u{f1b2}",
    selected: "▶",
    spinner: BRAILLE,
};

static UNICODE: Glyphs = Glyphs {
    winget: "●",
    scoop: "◆",
    search: "⌕",
    ok: "✓",
    upgrade: "↑",
    unknown: "?",
    ambiguous: "⚠",
    refresh: "⟳",
    offline: "⊘",
    license: "§",
    publisher: "@",
    id: "#",
    selected: "▶",
    spinner: BRAILLE,
};

static ASCII: Glyphs = Glyphs {
    winget: "W",
    scoop: "S",
    search: "/",
    ok: "+",
    upgrade: "^",
    unknown: "?",
    ambiguous: "!",
    refresh: "~",
    offline: "x",
    license: "L",
    publisher: "@",
    id: "#",
    selected: ">",
    spinner: ASCII_SPINNER,
};

impl Glyphs {
    pub fn for_set(set: GlyphSet) -> &'static Glyphs {
        match set {
            GlyphSet::Nerd => &NERD,
            GlyphSet::Unicode => &UNICODE,
            GlyphSet::Ascii => &ASCII,
        }
    }
}

/// `COLORTERM` truecolor/24bit selects RGB; a non-empty `NO_COLOR` disables color.
pub fn detect_depth(colorterm: Option<&str>, no_color: Option<&str>) -> ColorDepth {
    if no_color.is_some_and(|v| !v.is_empty()) {
        return ColorDepth::NoColor;
    }
    match colorterm {
        Some("truecolor" | "24bit") => ColorDepth::TrueColor,
        _ => ColorDepth::Ansi256,
    }
}

pub struct Theme {
    depth: ColorDepth,
    accent: Style,
    ok: Style,
    warn: Style,
    muted: Style,
    alert: Style,
    err: Style,
    src_winget: Style,
    src_scoop: Style,
    fg: Style,
    fg_dim: Style,
    bg_selected: Style,
    border: Style,
    border_focus: Style,
}

/// (rgb, indexed-256 approximation)
type Pair = ((u8, u8, u8), u8);

impl Theme {
    pub fn new(dark: bool, depth: ColorDepth) -> Theme {
        let fg = |p: Pair| Self::fg_style(depth, p);
        let bg = |p: Pair| Self::bg_style(depth, p);
        // dark / light palettes
        let (accent, ok, warn, muted, alert, err, winget, scoop, text, dim, sel, border, focus) =
            if dark {
                (
                    ((122, 162, 247), 111),
                    ((158, 206, 106), 114),
                    ((224, 175, 104), 179),
                    ((86, 95, 137), 60),
                    ((255, 158, 100), 209),
                    ((247, 118, 142), 204),
                    ((42, 195, 222), 45),
                    ((187, 154, 247), 141),
                    ((192, 202, 245), 189),
                    ((130, 139, 184), 103),
                    ((41, 46, 66), 236),
                    ((59, 66, 97), 238),
                    ((122, 162, 247), 111),
                )
            } else {
                (
                    ((52, 84, 138), 24),
                    ((72, 120, 20), 64),
                    ((143, 94, 21), 94),
                    ((120, 125, 150), 102),
                    ((177, 85, 20), 130),
                    ((186, 40, 70), 125),
                    ((15, 130, 150), 30),
                    ((100, 60, 180), 97),
                    ((52, 59, 88), 238),
                    ((100, 108, 140), 60),
                    ((210, 214, 230), 253),
                    ((170, 176, 200), 249),
                    ((52, 84, 138), 24),
                )
            };
        Theme {
            depth,
            accent: fg(accent),
            ok: fg(ok),
            warn: fg(warn),
            muted: fg(muted),
            alert: fg(alert),
            err: fg(err),
            src_winget: fg(winget),
            src_scoop: fg(scoop),
            fg: fg(text),
            fg_dim: fg(dim),
            bg_selected: bg(sel),
            border: fg(border),
            border_focus: fg(focus),
        }
    }

    fn color(depth: ColorDepth, (rgb, idx): Pair) -> Option<Color> {
        match depth {
            ColorDepth::TrueColor => Some(Color::Rgb(rgb.0, rgb.1, rgb.2)),
            ColorDepth::Ansi256 => Some(Color::Indexed(idx)),
            ColorDepth::NoColor => None,
        }
    }

    fn fg_style(depth: ColorDepth, p: Pair) -> Style {
        Self::color(depth, p).map_or_else(Style::new, |c| Style::new().fg(c))
    }

    fn bg_style(depth: ColorDepth, p: Pair) -> Style {
        Self::color(depth, p).map_or_else(Style::new, |c| Style::new().bg(c))
    }

    pub fn depth(&self) -> ColorDepth {
        self.depth
    }
    pub fn accent(&self) -> Style {
        self.accent
    }
    pub fn ok(&self) -> Style {
        self.ok
    }
    pub fn warn(&self) -> Style {
        self.warn
    }
    pub fn muted(&self) -> Style {
        self.muted
    }
    pub fn alert(&self) -> Style {
        self.alert
    }
    pub fn err(&self) -> Style {
        self.err
    }
    pub fn src_winget(&self) -> Style {
        self.src_winget
    }
    pub fn src_scoop(&self) -> Style {
        self.src_scoop
    }
    pub fn fg(&self) -> Style {
        self.fg
    }
    pub fn fg_dim(&self) -> Style {
        self.fg_dim
    }
    pub fn bg_selected(&self) -> Style {
        self.bg_selected
    }
    pub fn border(&self) -> Style {
        self.border
    }
    pub fn border_focus(&self) -> Style {
        self.border_focus
    }
    pub fn name(&self) -> Style {
        self.fg.add_modifier(Modifier::BOLD)
    }
    pub fn description(&self) -> Style {
        self.fg.add_modifier(Modifier::ITALIC)
    }
    pub fn dim(&self) -> Style {
        self.fg.add_modifier(Modifier::DIM)
    }
    pub fn matched(&self) -> Style {
        self.accent.add_modifier(Modifier::UNDERLINED)
    }
}
