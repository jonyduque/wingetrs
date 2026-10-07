#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::nonminimal_bool,
    clippy::double_comparisons
)]
use wingetrs::core::version::{Part, Version};

fn p(i: u64, o: &str) -> Part {
    Part {
        integer: i,
        other: o.to_string(),
    }
}

fn lt(a: &str, b: &str) {
    let (va, vb) = (Version::parse(a), Version::parse(b));
    assert!(va < vb, "{a} < {b}");
    assert!(!(vb < va), "!({b} < {a})");
    assert!(va <= vb && !(vb <= va), "<= {a} {b}");
    assert!(vb > va && !(va > vb), "> {a} {b}");
    assert!(vb >= va && !(va >= vb), ">= {a} {b}");
    assert!(va != vb, "{a} != {b}");
}

fn eq(a: &str, b: &str) {
    let (va, vb) = (Version::parse(a), Version::parse(b));
    assert!(va == vb, "{a} == {b}");
    assert!(
        va <= vb && va >= vb && !(va < vb) && !(va > vb),
        "ord {a} {b}"
    );
    assert_eq!(va.cmp(&vb), std::cmp::Ordering::Equal);
}

#[test]
fn parse_basic_with_suffix() {
    assert_eq!(
        Version::parse("1.2.3.4-alpha").parts(),
        &[p(1, ""), p(2, ""), p(3, ""), p(4, "-alpha")]
    );
}

#[test]
fn parse_plus_dash_split() {
    assert_eq!(
        Version::parse_with("1.2.3.4-alpha", ".-").parts(),
        &[p(1, ""), p(2, ""), p(3, ""), p(4, ""), p(0, "alpha")]
    );
}

#[test]
fn parse_whitespace_and_preamble() {
    let four = [p(1, ""), p(2, ""), p(3, ""), p(4, "")];
    assert_eq!(Version::parse("1. 2.3 . 4 ").parts(), &four);
    assert_eq!(Version::parse("v1.2.3.4").parts(), &four);
}

#[test]
fn parse_corners() {
    assert!(Version::parse("").parts().is_empty());
    assert!(Version::parse(".").parts().is_empty());
    assert!(Version::parse(".0").parts().is_empty());
    assert_eq!(Version::parse(".1").parts(), &[p(0, ""), p(1, "")]);
    assert_eq!(Version::parse("version").parts(), &[p(0, "version")]);
    assert_eq!(Version::parse(". 1 ").parts(), &[p(0, ""), p(1, "")]);
    assert_eq!(Version::parse("v1.2a").parts(), &[p(1, ""), p(2, "a")]);
}

#[test]
fn compare_table() {
    lt("1", "2");
    lt("1.0.0", "2.0.0");
    lt("0.0.1", "0.0.2");
    lt("0.0.1-alpha", "0.0.2-alpha");
    lt("0.0.1-beta", "0.0.2-alpha");
    lt("13.9.8", "14.1");
    lt("1-rc", "1");
    lt("1.2-rc", "1.2");
    lt("1.0-rc", "1.0");
    lt("1.0.0-rc", "1");
    lt("22.0.0-rc.1", "22.0.0");
    lt("22.0.0-rc.1", "22.0.0.1");
    lt("22.0.0-rc.1", "22.0.0.1-rc");
    lt("22.0.0-rc.1", "22.0.0-rc.1.1");
    lt("22.0.0-rc.1.1", "22.0.0-rc.1.2");
    lt("22.0.0-rc.1.2", "22.0.0-rc.2");
    eq("1.0", "1.0.0");
    eq("1.2.00.3", "1.2.0.3");
    eq("1.2.003.4", "1.2.3.4");
    eq("01.02.03.04", "1.2.3.4");
    eq("1.2.03-beta", "1.2.3-beta");
    eq("1.0", "1.0 ");
    eq("1.0", "1. 0");
    eq("1.0", "1.0.");
    eq("1.0", "Version 1.0");
    eq("foo1", "bar1");
    lt("v0.0.1", "0.0.2");
    lt("v0.0.1", "v0.0.2");
    lt("1.a2", "1.b1");
    lt("alpha", "beta");
}

#[test]
fn latest_and_unknown() {
    assert!(Version::latest().is_latest());
    assert!(Version::parse("latest").is_latest());
    assert!(Version::parse("LATEST").is_latest());
    assert!(!Version::parse("1.0").is_latest());
    lt("1.0", "latest");
    lt("100", "latest");
    lt("943849587389754876.1", "latest");
    eq("latest", "LATEST");
    assert!(Version::unknown().is_unknown());
    assert!(Version::parse("unknown").is_unknown());
    assert!(Version::parse("UNKNOWN").is_unknown());
    assert!(!Version::parse("1.0").is_unknown());
    lt("unknown", "1.0");
    lt("unknown", "1.fork");
    eq("unknown", "UNKNOWN");
    assert!(Version::unknown() < Version::latest());
}

#[test]
fn empty_and_part_at() {
    assert!(Version::default().is_empty());
    assert!(Version::parse("").is_empty());
    assert!(!Version::parse("1").is_empty());
    assert!(!Version::parse("0").is_empty());
    assert_eq!(Version::default().part_at(0).integer, 0);
    assert_eq!(Version::parse("1").part_at(0).integer, 1);
    assert_eq!(Version::parse("1").part_at(1).integer, 0);
    assert_eq!(Version::parse("1").part_at(9999).integer, 0);
}

#[test]
fn sort_is_consistent_with_eq() {
    let mut v: Vec<Version> = ["2.0", "1.0.0", "latest", "1.0", "unknown", "1.0-rc"]
        .iter()
        .map(|s| Version::parse(s))
        .collect();
    v.sort();
    let s: Vec<&str> = v.iter().map(|x| x.as_str()).collect();
    assert_eq!(s[0], "unknown");
    assert_eq!(s[1], "1.0-rc");
    assert_eq!(&s[4..], &["2.0", "latest"]);
}
