#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::config::{Behavior, Config, ConfigError, ConfirmMode, ResidueMode, update_file};

#[test]
fn missing_file_gives_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let c = Config::load(&dir.path().join("config.toml")).unwrap();
    assert_eq!(c, Config::default());
    assert_eq!(c.scoop_buckets, vec!["main", "extras"]);
    assert_eq!(c.glyphs, "nerd");
    assert_eq!(c.theme, "dark");
}

#[test]
fn partial_file_overrides_only_given_keys() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "scoop_buckets = [\"main\"]\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(c.scoop_buckets, vec!["main"]);
    assert_eq!(c.glyphs, "nerd");
}

#[test]
fn invalid_toml_error_mentions_path() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "scoop_buckets = [").unwrap();
    let e = Config::load(&p).unwrap_err();
    assert!(e.to_string().contains("config.toml"), "{e}");
}

#[test]
fn missing_behavior_section_gives_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "scoop_buckets = [\"main\"]\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(c.behavior, Behavior::default());
    assert_eq!(c.behavior.confirm, ConfirmMode::Always);
    assert_eq!(c.behavior.residues, ResidueMode::Ask);
    assert!(!c.behavior.purge);
    assert!(!c.behavior.keep_old);
}

#[test]
fn behavior_values_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(
        &p,
        "[behavior]\nconfirm = \"removals\"\npurge = true\nkeep_old = true\nresidues = \"high\"\n",
    )
    .unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(c.behavior.confirm, ConfirmMode::Removals);
    assert!(c.behavior.purge);
    assert!(c.behavior.keep_old);
    assert_eq!(c.behavior.residues, ResidueMode::High);
}

#[test]
fn behavior_round_trips_through_save() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    let c = Config {
        behavior: Behavior {
            confirm: ConfirmMode::Never,
            purge: true,
            keep_old: false,
            residues: ResidueMode::Keep,
            allow_http_redirect: true,
            mouse: false,
        },
        ..Config::default()
    };
    c.save(&p).unwrap();
    assert_eq!(Config::load(&p).unwrap(), c);
}

#[test]
fn invalid_confirm_value_mentions_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "[behavior]\nconfirm = \"as vezes\"\n").unwrap();
    let e = Config::load(&p).unwrap_err();
    assert!(e.to_string().contains("config.toml"), "{e}");
}

#[test]
fn update_file_keeps_changes_made_meanwhile() {
    use wingetrs::config::BucketEntry;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    let x = BucketEntry {
        name: "x".into(),
        repo: "a/x".into(),
    };
    let y = BucketEntry {
        name: "y".into(),
        repo: "a/y".into(),
    };
    let first = Config {
        buckets: vec![x.clone()],
        ..Config::default()
    };
    first.save(&p).unwrap();
    let stale = Config::load(&p).unwrap();
    let mut other = stale.clone();
    other.buckets.push(y.clone());
    other.save(&p).unwrap();
    let written = update_file(&p, |c| c.behavior.purge = true).unwrap();
    let on_disk = Config::load(&p).unwrap();
    assert_eq!(written, on_disk);
    assert_eq!(on_disk.buckets, vec![x, y]);
    assert!(on_disk.behavior.purge);
}

#[test]
fn update_file_refuses_invalid_file_and_leaves_it_intact() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    let original = "[behavior]\nconfirm = \"as vezes\"\n";
    std::fs::write(&p, original).unwrap();
    let r = update_file(&p, |c| c.behavior.purge = true);
    assert!(matches!(r, Err(ConfigError::Parse { .. })), "{r:?}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
}

#[test]
fn partial_behavior_section_fills_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "[behavior]\npurge = true\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert!(c.behavior.purge);
    assert_eq!(c.behavior.confirm, ConfirmMode::Always);
    assert_eq!(c.behavior.residues, ResidueMode::Ask);
    assert!(!c.behavior.keep_old);
}

#[test]
fn update_file_creates_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    update_file(&p, |c| c.behavior.purge = true).unwrap();
    assert!(p.exists());
    let c = Config::load(&p).unwrap();
    assert!(c.behavior.purge);
    assert_eq!(c.scoop_buckets, Config::default().scoop_buckets);
}

#[test]
fn config_icons_default_and_unknown() {
    use wingetrs::tui::icons::{IconProtocol, choose};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "glyphs = \"ascii\"\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(c.icons, "auto");
    std::fs::write(&p, "icons = \"bogus\"\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(
        choose(&c.icons, Some("WezTerm"), false),
        IconProtocol::Iterm2
    );
    assert_eq!(choose(&c.icons, None, true), IconProtocol::Sixel);
    assert_eq!(choose(&c.icons, None, false), IconProtocol::Off);
    std::fs::write(&p, "icons = \"off\"\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert_eq!(choose(&c.icons, Some("WezTerm"), true), IconProtocol::Off);
}

#[test]
fn behavior_allow_http_redirect_defaults_to_false() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "[behavior]\npurge = true\n").unwrap();
    assert!(!Config::load(&p).unwrap().behavior.allow_http_redirect);
    std::fs::write(&p, "[behavior]\nallow_http_redirect = true\n").unwrap();
    let c = Config::load(&p).unwrap();
    assert!(c.behavior.allow_http_redirect);
    let q = dir.path().join("saved.toml");
    c.save(&q).unwrap();
    assert!(Config::load(&q).unwrap().behavior.allow_http_redirect);
}

#[test]
fn config_list_icons_defaults_and_rejects_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(
        &p,
        "glyphs = \"ascii\"
",
    )
    .unwrap();
    assert_eq!(Config::load(&p).unwrap().list_icons, "visible");
    std::fs::write(
        &p,
        "list_icons = \"xyz\"
",
    )
    .unwrap();
    assert_eq!(Config::load(&p).unwrap().list_icons, "visible");
    let c = Config {
        list_icons: "opened".into(),
        ..Config::default()
    };
    c.save(&p).unwrap();
    assert_eq!(Config::load(&p).unwrap().list_icons, "opened");
}

#[test]
fn behavior_mouse_defaults_to_true() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "[behavior]\npurge = true\n").unwrap();
    assert!(Config::load(&p).unwrap().behavior.mouse);
    assert!(Behavior::default().mouse);
    let c = Config {
        behavior: Behavior {
            mouse: false,
            ..Behavior::default()
        },
        ..Config::default()
    };
    c.save(&p).unwrap();
    assert!(!Config::load(&p).unwrap().behavior.mouse);
}
