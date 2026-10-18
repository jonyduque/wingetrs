//! Winget engine and default scope settings (Task 171).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::config::{Behavior, Config, ConfigError, DefaultScope, WingetEngine};
use wingetrs::tui::settings::{ORDER, SettingId, SettingsState};
use wingetrs::tui::theme::GlyphSet;

fn state_at(id: SettingId) -> SettingsState {
    let mut s = SettingsState::new(Behavior::default(), GlyphSet::Nerd, true, false);
    s.selected = ORDER.iter().position(|x| *x == id).unwrap();
    s
}

fn load_text(text: &str) -> Result<Config, ConfigError> {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, text).unwrap();
    Config::load(&p)
}

#[test]
fn defaults_are_wingetrs_and_user() {
    let b = Behavior::default();
    assert_eq!(b.winget_engine, WingetEngine::Wingetrs);
    assert_eq!(b.default_scope, DefaultScope::User);
}

#[test]
fn config_roundtrip_new_fields() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(
        &p,
        "[behavior]\nwinget_engine = \"winget\"\ndefault_scope = \"machine\"\n",
    )
    .unwrap();
    let cfg = Config::load(&p).unwrap();
    assert_eq!(cfg.behavior.winget_engine, WingetEngine::Winget);
    assert_eq!(cfg.behavior.default_scope, DefaultScope::Machine);
    let p2 = dir.path().join("out.toml");
    cfg.save(&p2).unwrap();
    assert_eq!(Config::load(&p2).unwrap().behavior, cfg.behavior);
}

#[test]
fn old_config_without_fields_loads() {
    let cfg = load_text("[behavior]\nconfirm = \"always\"\n").unwrap();
    assert_eq!(cfg.behavior.winget_engine, WingetEngine::Wingetrs);
    assert_eq!(cfg.behavior.default_scope, DefaultScope::User);
}

#[test]
fn invalid_engine_falls_back_to_default() {
    let cfg = load_text(
        "[behavior]
winget_engine = \"x\"
default_scope = \"machine\"
",
    )
    .unwrap();
    assert_eq!(cfg.behavior.winget_engine, WingetEngine::Wingetrs);
    assert_eq!(cfg.behavior.default_scope, DefaultScope::Machine);
}

#[test]
fn invalid_scope_falls_back_to_default() {
    let cfg = load_text(
        "[behavior]
winget_engine = \"winget\"
default_scope = \"x\"
",
    )
    .unwrap();
    assert_eq!(cfg.behavior.winget_engine, WingetEngine::Winget);
    assert_eq!(cfg.behavior.default_scope, DefaultScope::User);
}

#[test]
fn wrong_type_is_still_a_parse_error() {
    let r = load_text(
        "[behavior]
winget_engine = 3
",
    );
    assert!(matches!(r, Err(ConfigError::Parse { .. })));
}

#[test]
fn cycle_engine_and_scope() {
    let mut s = state_at(SettingId::WingetEngine);
    assert!(s.cycle(true));
    assert_eq!(s.behavior.winget_engine, WingetEngine::Winget);
    assert_eq!(s.behavior.default_scope, DefaultScope::User);
    s.cycle(true);
    assert_eq!(s.behavior.winget_engine, WingetEngine::Wingetrs);

    let mut s = state_at(SettingId::DefaultScope);
    assert!(s.cycle(true));
    assert_eq!(s.behavior.default_scope, DefaultScope::Machine);
    assert_eq!(s.behavior.winget_engine, WingetEngine::Wingetrs);
    s.cycle(true);
    assert_eq!(s.behavior.default_scope, DefaultScope::User);
}

#[test]
fn order_places_new_settings_after_redirect() {
    let i = ORDER
        .iter()
        .position(|x| *x == SettingId::HttpRedirect)
        .unwrap();
    assert_eq!(ORDER[i + 1], SettingId::WingetEngine);
    assert_eq!(ORDER[i + 2], SettingId::DefaultScope);
    assert_eq!(ORDER.len(), 12);
}

#[test]
fn save_settings_writes_fields() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    let b = Behavior {
        winget_engine: WingetEngine::Winget,
        default_scope: DefaultScope::Machine,
        ..Behavior::default()
    };
    wingetrs::tui::jobs::save_settings(&p, b, None, None, None, "dark").unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("winget_engine = \"winget\""), "{text}");
    assert!(text.contains("default_scope = \"machine\""), "{text}");
}
