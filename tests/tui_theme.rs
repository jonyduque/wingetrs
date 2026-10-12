use ratatui::style::{Color, Modifier};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Glyphs, Theme};

#[test]
fn dark_truecolor_uses_the_charm_palette() {
    let t = Theme::new(true, ColorDepth::TrueColor);
    assert_eq!(t.accent().fg, Some(Color::Rgb(0x7D, 0x56, 0xF4)));
    assert_eq!(t.sel_title().fg, Some(Color::Rgb(0xEE, 0x6F, 0xF8)));
    assert_eq!(t.help_key().fg, Some(Color::Rgb(0x62, 0x62, 0x62)));
    assert_eq!(t.badge().bg, Some(Color::Rgb(0x5F, 0x5F, 0xD7)));
}

#[test]
fn light_truecolor_accent_is_darker_purple() {
    let t = Theme::new(false, ColorDepth::TrueColor);
    assert_eq!(t.accent().fg, Some(Color::Rgb(0x87, 0x4B, 0xFD)));
}

#[test]
fn ansi256_uses_the_listed_indexes() {
    let t = Theme::new(true, ColorDepth::Ansi256);
    assert_eq!(t.accent().fg, Some(Color::Indexed(99)));
    assert_eq!(t.sel_title().fg, Some(Color::Indexed(213)));
    assert_eq!(t.pill_msg().bg, Some(Color::Indexed(204)));
    assert_eq!(t.ok().fg, Some(Color::Indexed(85)));
}

#[test]
fn no_color_keeps_pills_visible() {
    let t = Theme::new(true, ColorDepth::NoColor);
    assert_eq!(t.pill_msg().bg, None);
    assert!(t.pill_msg().add_modifier.contains(Modifier::REVERSED));
    assert!(t.badge().add_modifier.contains(Modifier::REVERSED));
    assert!(t.badge().add_modifier.contains(Modifier::BOLD));
}

#[test]
fn nerd_and_unicode_spin_with_dot_frames() {
    let n = Glyphs::for_set(GlyphSet::Nerd);
    assert_eq!(n.spinner.len(), 8);
    assert_eq!(n.spinner[0], "⣾");
    assert_eq!(Glyphs::for_set(GlyphSet::Unicode).spinner[0], "⣾");
    assert_eq!(
        Glyphs::for_set(GlyphSet::Ascii).spinner,
        &["|", "/", "-", "\\"]
    );
}

fn rgb(h: u32) -> Option<Color> {
    Some(Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8))
}

#[test]
fn theme_table_matches_the_brief() {
    for dark in [true, false] {
        let t = Theme::new(dark, ColorDepth::TrueColor);
        let p = |d: u32, l: u32| rgb(if dark { d } else { l });
        let fgs = [
            ("accent", t.accent().fg, p(0x7D56F4, 0x874BFD)),
            ("border_focus", t.border_focus().fg, p(0x7D56F4, 0x874BFD)),
            ("sel_bar", t.sel_bar().fg, p(0xAD58B4, 0xF793FF)),
            ("sel_desc", t.sel_desc().fg, p(0xAD58B4, 0xF793FF)),
            ("sel_title", t.sel_title().fg, p(0xEE6FF8, 0xEE6FF8)),
            ("fg", t.fg().fg, p(0xDDDDDD, 0x1A1A1A)),
            ("fg_dim", t.fg_dim().fg, p(0x777777, 0xA49FA5)),
            ("border", t.border().fg, p(0x383838, 0xD9DCCF)),
            ("ok", t.ok().fg, p(0x73F59F, 0x43BF6D)),
            ("help_key", t.help_key().fg, p(0x626262, 0x909090)),
            ("help_desc", t.help_desc().fg, p(0x6C6C6C, 0xB2B2B2)),
            ("help_sep", t.help_sep().fg, p(0x3C3C3C, 0xDADADA)),
            ("badge.fg", t.badge().fg, rgb(0xFFFFD7)),
            ("badge.bg", t.badge().bg, rgb(0x5F5FD7)),
            ("pill_msg.fg", t.pill_msg().fg, rgb(0xFFFDF5)),
            ("pill_msg.bg", t.pill_msg().bg, rgb(0xFF5F87)),
            ("pill_a.fg", t.pill_a().fg, rgb(0xFFFDF5)),
            ("pill_a.bg", t.pill_a().bg, rgb(0xA550DF)),
            ("pill_b.fg", t.pill_b().fg, rgb(0xFFFDF5)),
            ("pill_b.bg", t.pill_b().bg, rgb(0x6124DF)),
            ("spinner", t.spinner().fg, p(0x747373, 0x8E8E8E)),
        ];
        for (name, got, want) in fgs {
            assert_eq!(got, want, "{name} dark={dark}");
        }
    }
    let n = Theme::new(true, ColorDepth::NoColor);
    for s in [
        n.accent(),
        n.border_focus(),
        n.sel_bar(),
        n.sel_desc(),
        n.sel_title(),
        n.fg(),
        n.fg_dim(),
        n.border(),
        n.ok(),
        n.help_key(),
        n.help_desc(),
        n.help_sep(),
        n.badge(),
        n.pill_msg(),
        n.pill_a(),
        n.pill_b(),
        n.spinner(),
    ] {
        assert_eq!((s.fg, s.bg), (None, None));
    }
}
