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
    /// Before the name of a row marked with Space.
    pub mark: &'static str,
    /// Next to the state of a row whose version is pinned.
    pub pin: &'static str,
    pub spinner: &'static [&'static str],
    /// Between an old and a new value (`1.0 -> 2.0`).
    pub arrow: &'static str,
    /// The app icon left of the tabs when no image is drawn.
    pub app: &'static str,
    pub tab_search: &'static str,
    pub tab_installed: &'static str,
    pub tab_updates: &'static str,
    pub tab_settings: &'static str,
    pub bar: &'static str,
    /// Separator after a glyph in the details; nerd glyphs may draw two columns wide.
    pub gap: &'static str,
    pub toast_ok: &'static str,
    pub toast_info: &'static str,
    pub toast_warn: &'static str,
    pub toast_err: &'static str,
    /// Drawn in the icon's place for a package known to have none.
    pub no_icon: &'static str,
    /// Download bar: a done cell, a missing cell and the bar's ends.
    pub bar_full: &'static str,
    pub bar_empty: &'static str,
    pub bar_open: &'static str,
    pub bar_close: &'static str,
}

const BRAILLE: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
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
    mark: "\u{f00c}",
    pin: "\u{f08d}",
    spinner: BRAILLE,
    arrow: "\u{f061}",
    app: "\u{f018d}",
    tab_search: "\u{f002}",
    tab_installed: "\u{f0a0}",
    tab_updates: "\u{f06b0}",
    tab_settings: "\u{f013}",
    bar: "┃",
    gap: "  ",
    toast_ok: "\u{f00c}",
    toast_info: "\u{f05a}",
    toast_warn: "\u{f071}",
    toast_err: "\u{f057}",
    no_icon: "\u{f03d7}",
    bar_full: "█",
    bar_empty: "░",
    bar_open: "",
    bar_close: "",
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
    mark: "✓",
    pin: "⚲",
    spinner: BRAILLE,
    arrow: "→",
    app: "▣",
    tab_search: "⌕",
    tab_installed: "▤",
    tab_updates: "↑",
    tab_settings: "⚙",
    bar: "┃",
    gap: " ",
    toast_ok: "✓",
    toast_info: "ℹ",
    toast_warn: "⚠",
    toast_err: "✗",
    no_icon: "▢",
    bar_full: "█",
    bar_empty: "░",
    bar_open: "",
    bar_close: "",
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
    mark: "*",
    pin: "!",
    spinner: ASCII_SPINNER,
    arrow: "->",
    app: "[w]",
    tab_search: "",
    tab_installed: "",
    tab_updates: "",
    tab_settings: "",
    bar: "|",
    gap: " ",
    toast_ok: "+",
    toast_info: "i",
    toast_warn: "!",
    toast_err: "x",
    no_icon: "[ ]",
    bar_full: "#",
    bar_empty: "-",
    bar_open: "[",
    bar_close: "]",
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
    sel_bar: Style,
    sel_desc: Style,
    sel_title: Style,
    help_key: Style,
    help_desc: Style,
    help_sep: Style,
    badge: Style,
    pill_msg: Style,
    pill_a: Style,
    pill_b: Style,
    spinner: Style,
}

/// (rgb, indexed-256 approximation)
type Pair = ((u8, u8, u8), u8);

impl Theme {
    pub fn new(dark: bool, depth: ColorDepth) -> Theme {
        let fg = |p: Pair| Self::fg_style(depth, p);
        let bg = |p: Pair| Self::bg_style(depth, p);
        // dark / light palettes (Charm Lip Gloss / Bubbles)
        let pick = |d: Pair, l: Pair| if dark { d } else { l };
        let accent = pick(((0x7D, 0x56, 0xF4), 99), ((0x87, 0x4B, 0xFD), 99));
        let sel_bar = pick(((0xAD, 0x58, 0xB4), 133), ((0xF7, 0x93, 0xFF), 219));
        let sel_title = ((0xEE, 0x6F, 0xF8), 213);
        let pill_fg: Pair = ((0xFF, 0xFD, 0xF5), 230);
        let (ok, warn, muted, alert, err, winget, scoop, sel) = if dark {
            (
                ((0x73, 0xF5, 0x9F), 85),
                ((224, 175, 104), 179),
                ((86, 95, 137), 60),
                ((255, 158, 100), 209),
                ((247, 118, 142), 204),
                ((42, 195, 222), 45),
                ((187, 154, 247), 141),
                ((41, 46, 66), 236),
            )
        } else {
            (
                ((0x43, 0xBF, 0x6D), 71),
                ((143, 94, 21), 94),
                ((120, 125, 150), 102),
                ((177, 85, 20), 130),
                ((186, 40, 70), 125),
                ((15, 130, 150), 30),
                ((100, 60, 180), 97),
                ((210, 214, 230), 253),
            )
        };
        let text = pick(((0xDD, 0xDD, 0xDD), 253), ((0x1A, 0x1A, 0x1A), 234));
        let dim = pick(((0x77, 0x77, 0x77), 243), ((0xA4, 0x9F, 0xA5), 247));
        let border = pick(((0x38, 0x38, 0x38), 237), ((0xD9, 0xDC, 0xCF), 253));
        let help_key = pick(((0x8A, 0x8A, 0x8A), 245), ((0x70, 0x70, 0x70), 242));
        let help_desc = pick(((0x6C, 0x6C, 0x6C), 242), ((0xB2, 0xB2, 0xB2), 249));
        let help_sep = pick(((0x3C, 0x3C, 0x3C), 237), ((0xDA, 0xDA, 0xDA), 253));
        let spinner = pick(((0x74, 0x73, 0x73), 243), ((0x8E, 0x8E, 0x8E), 245));
        let both = |f: Pair, b: Pair, extra: Modifier| {
            let mut st = Style::new().add_modifier(extra);
            if let Some(c) = Self::color(depth, f) {
                st = st.fg(c);
            }
            if let Some(c) = Self::color(depth, b) {
                st = st.bg(c);
            }
            if depth == ColorDepth::NoColor {
                st = st.add_modifier(Modifier::REVERSED);
            }
            st
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
            border_focus: fg(accent),
            sel_bar: fg(sel_bar),
            sel_desc: fg(sel_bar),
            sel_title: fg(sel_title),
            help_key: fg(help_key).add_modifier(Modifier::BOLD),
            help_desc: fg(help_desc),
            help_sep: fg(help_sep),
            badge: both(
                ((0xFF, 0xFF, 0xD7), 230),
                ((0x5F, 0x5F, 0xD7), 62),
                Modifier::BOLD,
            ),
            pill_msg: both(pill_fg, ((0xFF, 0x5F, 0x87), 204), Modifier::empty()),
            pill_a: both(pill_fg, ((0xA5, 0x50, 0xDF), 134), Modifier::empty()),
            pill_b: both(pill_fg, ((0x61, 0x24, 0xDF), 56), Modifier::empty()),
            spinner: fg(spinner),
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
    pub fn sel_bar(&self) -> Style {
        self.sel_bar
    }
    pub fn sel_desc(&self) -> Style {
        self.sel_desc
    }
    pub fn sel_title(&self) -> Style {
        self.sel_title
    }
    pub fn help_key(&self) -> Style {
        self.help_key
    }
    pub fn help_desc(&self) -> Style {
        self.help_desc
    }
    pub fn help_sep(&self) -> Style {
        self.help_sep
    }
    pub fn badge(&self) -> Style {
        self.badge
    }
    pub fn pill_msg(&self) -> Style {
        self.pill_msg
    }
    pub fn pill_a(&self) -> Style {
        self.pill_a
    }
    pub fn pill_b(&self) -> Style {
        self.pill_b
    }
    pub fn spinner(&self) -> Style {
        self.spinner
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
