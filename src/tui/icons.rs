//! Package icons in the details pane: protocol choice, picker and off-UI encoding.
//! The terminal is never queried (the picker's stdio query leaves a thread that is never
//! joined and hangs with stdin at EOF): the protocol comes from the setting and the environment.
use ratatui::layout::Size;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FontSize, Resize};
use std::collections::VecDeque;

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
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    picker
        .new_protocol(img, Size::new(size.0, size.1), Resize::Fit(None))
        .map_err(|e| e.to_string())
}

/// Most encoded icons kept at once.
pub const ICON_CACHE_MAX: usize = 64;

/// Encoded icons keyed by `(package key, columns, rows)`; past `ICON_CACHE_MAX` the oldest goes.
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
            self.entries.pop_front();
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
