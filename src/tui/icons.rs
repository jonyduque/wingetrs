//! Package icons in the details pane: protocol choice, picker and off-UI encoding.
//! The terminal is never queried (the picker's stdio query leaves a thread that is never
//! joined and hangs with stdin at EOF): the protocol comes from the setting and the environment.
use ratatui::layout::Size;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FontSize, Resize};
use std::collections::VecDeque;

/// The wingetrs icon shown left of the tabs, encoded once per protocol like a package icon.
// Generated once: magick -background none -density 384 docs/assets/icon.svg -resize 64x64 assets/icon.png
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");
/// Key and size (columns, rows) of the app icon in the `IconCache`.
pub const APP_ICON_KEY: &str = "wingetrs/app";
pub const APP_ICON_SIZE: (u16, u16) = (2, 1);

/// Graphics protocol used to draw icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconProtocol {
    Iterm2,
    Sixel,
    Off,
}

/// Settings value used when the configured one is unknown.
pub const DEFAULT_SETTING: &str = "auto";
/// Valid values of `icons` in `config.toml`.
pub const SETTINGS: [&str; 4] = ["auto", "iterm2", "sixel", "off"];

/// Protocol for `setting` (`auto`, `iterm2`, `sixel`, `off`); `auto` (and any unknown value)
/// picks iTerm2 in WezTerm, Sixel in Windows Terminal and nothing elsewhere.
pub fn choose(setting: &str, term_program: Option<&str>, wt_session: bool) -> IconProtocol {
    match setting {
        "iterm2" => IconProtocol::Iterm2,
        "sixel" => IconProtocol::Sixel,
        "off" => IconProtocol::Off,
        _ => {
            if term_program.is_some_and(|t| t.eq_ignore_ascii_case("WezTerm")) {
                IconProtocol::Iterm2
            } else if wt_session {
                IconProtocol::Sixel
            } else {
                IconProtocol::Off
            }
        }
    }
}

/// Protocol for `setting` in the current terminal (reads `TERM_PROGRAM` and `WT_SESSION`).
pub fn choose_from_env(setting: &str) -> IconProtocol {
    choose(
        setting,
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var_os("WT_SESSION").is_some(),
    )
}

/// Pixels per cell `(width, height)`, from the terminal's pixel size; `(10, 20)` when unknown.
pub fn font_size() -> (u16, u16) {
    match crossterm::terminal::window_size() {
        Ok(w) if w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0 => {
            (w.width / w.columns, w.height / w.rows)
        }
        _ => (10, 20),
    }
}

/// Picker fixed to `protocol` at the terminal's font size; `None` for `Off`.
pub fn picker(protocol: IconProtocol) -> Option<Picker> {
    let kind = match protocol {
        IconProtocol::Iterm2 => ProtocolType::Iterm2,
        IconProtocol::Sixel => ProtocolType::Sixel,
        IconProtocol::Off => return None,
    };
    // Deprecated only in favor of the terminal query, which this module must not use.
    #[allow(deprecated)]
    let mut p = Picker::from_fontsize(FontSize::from(font_size()));
    p.set_protocol_type(kind);
    Some(p)
}

/// Decodes `bytes` and encodes them for `picker`, fitted into `size` cells (columns, rows).
/// Slow: runs off the UI thread.
pub fn encode(picker: &Picker, bytes: &[u8], size: (u16, u16)) -> Result<Protocol, String> {
    let img = decode(bytes)?;
    picker
        .new_protocol(img, Size::new(size.0, size.1), Resize::Fit(None))
        .map_err(|e| e.to_string())
}

/// Largest icon side, in pixels, that is decoded.
pub const MAX_ICON_SIDE: u32 = 1024;
/// Most memory one icon decode may allocate.
pub const MAX_ICON_ALLOC: u64 = 64 * 1024 * 1024;

/// Decodes icon bytes with bounded size: a small file declaring a huge image (a decompression
/// bomb) is refused before its pixels are allocated.
pub fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_ICON_SIDE);
    limits.max_image_height = Some(MAX_ICON_SIDE);
    limits.max_alloc = Some(MAX_ICON_ALLOC);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

/// Most encoded icons kept at once.
pub const ICON_CACHE_MAX: usize = 64;

/// Encoded icons keyed by `(package key, columns, rows)`; past `ICON_CACHE_MAX` the oldest goes (never the app icon).
#[derive(Default)]
pub struct IconCache {
    entries: VecDeque<((String, u16, u16), Protocol)>,
}

impl IconCache {
    pub fn get(&self, key: &str, size: (u16, u16)) -> Option<&Protocol> {
        self.entries
            .iter()
            .find(|((k, c, r), _)| k == key && (*c, *r) == size)
            .map(|(_, p)| p)
    }

    pub fn insert(&mut self, key: String, size: (u16, u16), proto: Protocol) {
        self.entries
            .retain(|((k, c, r), _)| !(*k == key && (*c, *r) == size));
        self.entries.push_back(((key, size.0, size.1), proto));
        while self.entries.len() > ICON_CACHE_MAX {
            // The app icon stays: it is drawn on every frame.
            let oldest = self
                .entries
                .iter()
                .position(|((k, ..), _)| k != APP_ICON_KEY);
            self.entries.remove(oldest.unwrap_or(0));
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_protocol() {
        let cases: [(&str, Option<&str>, bool, IconProtocol); 8] = [
            ("auto", Some("WezTerm"), false, IconProtocol::Iterm2),
            ("auto", Some("wezterm"), false, IconProtocol::Iterm2),
            ("auto", None, true, IconProtocol::Sixel),
            ("auto", Some("WezTerm"), true, IconProtocol::Iterm2),
            ("auto", None, false, IconProtocol::Off),
            ("sixel", Some("WezTerm"), false, IconProtocol::Sixel),
            ("off", None, true, IconProtocol::Off),
            ("iterm2", None, false, IconProtocol::Iterm2),
        ];
        for (setting, term, wt, want) in cases {
            assert_eq!(choose(setting, term, wt), want, "{setting} {term:?} {wt}");
        }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let img = image::DynamicImage::ImageLuma8(image::GrayImage::new(w, h));
        let written = img.write_to(&mut out, image::ImageFormat::Png);
        assert!(written.is_ok());
        out.into_inner()
    }

    /// PNG signature and an IHDR declaring `w`x`h` RGBA, with no pixel data.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&w.to_be_bytes());
        chunk.extend_from_slice(&h.to_be_bytes());
        chunk.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut crc = flate2::Crc::new();
        crc.update(&chunk);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc.sum().to_be_bytes());
        out
    }

    #[test]
    fn oversized_icons_are_refused() {
        // Valid and decodable without limits (2 KB of pixels), but wider/taller than allowed.
        assert!(decode(&png(MAX_ICON_SIDE + 1, 2)).is_err());
        assert!(decode(&png(2, MAX_ICON_SIDE + 1)).is_err());
        // A bomb header: 20000x20000 RGBA in a few bytes.
        assert!(decode(&png_header(20_000, 20_000)).is_err());
        // Normal icons decode.
        let ok = decode(&png(256, 256));
        assert!(ok.is_ok());
        assert_eq!(ok.map(|i| (i.width(), i.height())), Ok((256, 256)));
        assert!(decode(&png(MAX_ICON_SIDE, 2)).is_ok());
        let mut ico = std::io::Cursor::new(Vec::new());
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(256, 256));
        assert!(img.write_to(&mut ico, image::ImageFormat::Ico).is_ok());
        assert!(decode(&ico.into_inner()).is_ok());
    }

    #[test]
    fn garbage_bytes_do_not_encode() {
        let p = picker(IconProtocol::Sixel);
        assert!(p.is_some());
        if let Some(p) = p {
            assert!(encode(&p, b"not an image", (8, 4)).is_err());
        }
        assert!(picker(IconProtocol::Off).is_none());
    }
}
