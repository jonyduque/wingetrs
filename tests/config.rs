#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::config::Config;

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
