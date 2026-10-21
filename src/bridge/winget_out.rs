//! Classification of the lines `winget` prints with stdout redirected (Task 215). The CLI
//! (`cli::output::bridge_progress`) and the TUI (`set_bridge_line`) both go through
//! [`classify`], so they never read the same line differently. Matching is on
//! `plain()` (no ANSI, no accents) lowercased, in English and Portuguese.
use super::plain;

/// What one winget output line means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WingetLine {
    /// "Found <name> [<id>] Version <v>".
    Found {
        name: String,
        id: String,
        version: String,
    },
    /// One of the two license notices.
    License,
    /// "Downloading <url>".
    Downloading {
        url: String,
    },
    HashOk,
    /// "Starting package install/uninstall..." (`removing` for the uninstall).
    Starting {
        removing: bool,
    },
    Done,
    NoUpgrade,
    Other(String),
}

/// The kind of a fixed-text line in [`TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    License,
    HashOk,
    StartInstall,
    StartRemove,
    Done,
    NoUpgrade,
}

/// Known lines, already `plain()`-folded and lowercase; a line matches when it starts with
/// the text. Texts in other Portuguese wordings stay `Other` until the live proof adds them.
pub const TABLE: &[(&str, Kind)] = &[
    (
        "this application is licensed to you by its owner.",
        Kind::License,
    ),
    (
        "este aplicativo e licenciado para voce pelo proprietario.",
        Kind::License,
    ),
    (
        "microsoft is not responsible for, nor does it grant any licenses to, third-party packages.",
        Kind::License,
    ),
    (
        "a microsoft nao e responsavel por, nem concede licencas a pacotes de terceiros.",
        Kind::License,
    ),
    ("successfully verified installer hash", Kind::HashOk),
    ("hash do instalador verificado com exito", Kind::HashOk),
    ("starting package install", Kind::StartInstall),
    ("iniciando a instalacao do pacote", Kind::StartInstall),
    ("starting package uninstall", Kind::StartRemove),
    ("iniciando a desinstalacao do pacote", Kind::StartRemove),
    ("successfully installed", Kind::Done),
    ("instalado com exito", Kind::Done),
    ("successfully uninstalled", Kind::Done),
    ("desinstalado com exito", Kind::Done),
    ("no available upgrade found.", Kind::NoUpgrade),
    (
        "no newer package versions are available from the configured sources.",
        Kind::NoUpgrade,
    ),
];

/// Prefixes of "Downloading <url>" (folded, lowercase, with the space).
const DOWNLOADING: [&str; 2] = ["downloading ", "baixando "];
/// Prefixes of the "Found" line and the word before the version.
const FOUND: [&str; 2] = ["found ", "encontrado "];
const VERSION_WORD: [&str; 2] = ["version ", "versao "];

/// Classifies one output line (any language in the table, ANSI and case ignored).
pub fn classify(line: &str) -> WingetLine {
    let text = plain(line);
    let text = text.trim();
    let key = text.to_ascii_lowercase();
    if let Some((_, kind)) = TABLE.iter().find(|(t, _)| key.starts_with(t)) {
        return match kind {
            Kind::License => WingetLine::License,
            Kind::HashOk => WingetLine::HashOk,
            Kind::StartInstall => WingetLine::Starting { removing: false },
            Kind::StartRemove => WingetLine::Starting { removing: true },
            Kind::Done => WingetLine::Done,
            Kind::NoUpgrade => WingetLine::NoUpgrade,
        };
    }
    if let Some(p) = DOWNLOADING.iter().find(|p| key.starts_with(**p)) {
        let url = text[p.len()..].trim();
        if !url.is_empty() {
            return WingetLine::Downloading {
                url: url.to_string(),
            };
        }
    }
    if let Some(found) = found(text, &key) {
        return found;
    }
    WingetLine::Other(line.trim().to_string())
}

/// "Found <name> [<id>] Version <v>": `key` is `text` ASCII-lowercased, so both have the same
/// byte offsets. `text` is already folded by `plain()`, hence the name loses its accents
/// (known limitation).
fn found(text: &str, key: &str) -> Option<WingetLine> {
    let p = FOUND.iter().find(|p| key.starts_with(**p))?;
    let open = key.find(" [")?;
    let close = open + key[open..].find("] ")?;
    if open < p.len() {
        return None;
    }
    let rest = &key[close + 2..];
    let w = VERSION_WORD.iter().find(|w| rest.starts_with(**w))?;
    let version = text[close + 2 + w.len()..].trim();
    let name = text[p.len()..open].trim();
    let id = text[open + 2..close].trim();
    if version.is_empty() || name.is_empty() || id.is_empty() {
        return None;
    }
    Some(WingetLine::Found {
        name: name.to_string(),
        id: id.to_string(),
        version: version.to_string(),
    })
}

/// The short Portuguese label the TUI shows after the action (`None`: show the raw line).
pub fn tui_label(line: &WingetLine) -> Option<String> {
    Some(match line {
        WingetLine::Found { name, version, .. } => format!("encontrado {name} {version}"),
        WingetLine::License => "licença de terceiros aceita".to_string(),
        WingetLine::Downloading { .. } => "baixando o instalador".to_string(),
        WingetLine::HashOk => "hash do instalador conferido".to_string(),
        WingetLine::Starting { removing: false } => "instalando".to_string(),
        WingetLine::Starting { removing: true } => "removendo".to_string(),
        WingetLine::Done => "concluído".to_string(),
        WingetLine::NoUpgrade => "nenhuma atualização encontrada pelo winget".to_string(),
        WingetLine::Other(_) => return None,
    })
}

/// Line the wingetrs itself pushes before refreshing winget's source (tui/jobs.rs).
pub const UPDATING_SOURCE: &str = "atualizando a fonte do winget";

/// `line` without ANSI sequences and accents (`plain()`), for display.
pub fn clean(line: &str) -> String {
    plain(line)
}
