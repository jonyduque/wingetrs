#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::search::SearchIndex;
use wingetrs::core::{PackageEntry, SourceKind};

fn e(source: SourceKind, id: &str, name: &str, moniker: Option<&str>) -> PackageEntry {
    PackageEntry {
        source,
        source_name: source.as_str().into(),
        id: id.into(),
        name: name.into(),
        moniker: moniker.map(Into::into),
        version: "1".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
    }
}

fn idx() -> SearchIndex {
    SearchIndex::new(vec![
        e(
            SourceKind::Winget,
            "lin-ycv.EverythingPowerToys",
            "EverythingPowerToys",
            None,
        ),
        e(
            SourceKind::Winget,
            "Microsoft.PowerToys",
            "PowerToys",
            Some("powertoys"),
        ),
        e(SourceKind::Scoop, "powertoys", "powertoys", None),
        e(
            SourceKind::Winget,
            "Qihoo.360TotalSecurity",
            "360安全卫士",
            None,
        ),
        e(SourceKind::Winget, "Zeal.Zeal", "Zeal", None),
    ])
}

#[test]
fn exact_moniker_or_id_wins() {
    let mut i = idx();
    let hits = i.search("powertoys", None, 10);
    let top: Vec<&str> = hits
        .iter()
        .take(2)
        .map(|h| i.entries()[h.index].id.as_str())
        .collect();
    assert!(
        top.contains(&"Microsoft.PowerToys") && top.contains(&"powertoys"),
        "{top:?}"
    );
    assert_eq!(i.entries()[hits[2].index].id, "lin-ycv.EverythingPowerToys");
}

#[test]
fn source_filter() {
    let mut i = idx();
    let hits = i.search("powertoys", Some(SourceKind::Scoop), 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(i.entries()[hits[0].index].source, SourceKind::Scoop);
}

#[test]
fn empty_query_lists_by_name_case_insensitive() {
    let mut i = idx();
    let names: Vec<String> = i
        .search("", None, 3)
        .iter()
        .map(|h| i.entries()[h.index].name.clone())
        .collect();
    assert_eq!(names, ["360安全卫士", "EverythingPowerToys", "PowerToys"]);
}

#[test]
fn non_ascii_names_are_searchable() {
    let mut i = idx();
    let hits = i.search("360", None, 10);
    assert_eq!(i.entries()[hits[0].index].id, "Qihoo.360TotalSecurity");
    let hits = i.search("安全", None, 10);
    assert_eq!(hits.len(), 1);
}

#[test]
fn fuzzy_name_indices_point_at_matched_chars() {
    let mut i = idx();
    let hits = i.search("zel", None, 10);
    let h = &hits[0];
    assert_eq!(i.entries()[h.index].name, "Zeal");
    assert_eq!(h.name_indices, [0, 1, 3]);
}

#[test]
fn limit_is_respected() {
    let mut i = idx();
    assert_eq!(i.search("", None, 2).len(), 2);
}

#[test]
fn exact_id_beats_equal_scoring_name_match() {
    let mut i = SearchIndex::new(vec![
        e(SourceKind::Winget, "B.B", "foo", None),
        e(SourceKind::Winget, "foo", "Zzz", None),
    ]);
    let hits = i.search("foo", None, 10);
    assert_eq!(i.entries()[hits[0].index].id, "foo");
}

#[test]
fn non_ascii_name_indices_are_char_positions() {
    let mut i = idx();
    let hits = i.search("安全", None, 10);
    assert_eq!(hits[0].name_indices, [3, 4]);
}
