#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `pins.json`: the set of pinned apps, tolerant of a corrupt file.
use wingetrs::cache::DataDir;
use wingetrs::pins;

#[test]
fn pins_round_trip_and_survive_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    assert!(pins::load(&data).is_empty());

    let a = pins::key("wingetrs", "solo");
    assert_eq!(a, "wingetrs/solo");
    pins::set(&data, &a, true).unwrap();
    pins::set(&data, &pins::key("winget", "Foo.Bar"), true).unwrap();
    let loaded = pins::load(&data);
    assert_eq!(loaded.len(), 2);
    assert!(pins::is_pinned(&loaded, "wingetrs", "solo"));
    assert!(!pins::is_pinned(&loaded, "winget", "solo"));

    pins::set(&data, &a, false).unwrap();
    assert!(!pins::is_pinned(&pins::load(&data), "wingetrs", "solo"));

    // A corrupt file reads as empty and the next `set` writes a valid one.
    std::fs::write(dir.path().join("pins.json"), b"{not json").unwrap();
    assert!(pins::load(&data).is_empty());
    pins::set(&data, &a, true).unwrap();
    assert_eq!(pins::load(&data).len(), 1);
    assert!(pins::is_pinned(&pins::load(&data), "wingetrs", "solo"));
}
