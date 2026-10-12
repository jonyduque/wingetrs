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
