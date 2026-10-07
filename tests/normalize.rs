#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::assertions_on_constants
)]
use wingetrs::installed::normalize::{Architecture, NameNormalizer};

const NAMES: &str =
    include_str!("../docs/reference/winget-cli/AppInstallerCLITests_TestData_InputNames.txt");
const PUBS: &str =
    include_str!("../docs/reference/winget-cli/AppInstallerCLITests_TestData_InputPublishers.txt");
const IDS: &str = include_str!(
    "../docs/reference/winget-cli/AppInstallerCLITests_TestData_NormalizationInitialIds.txt"
);

/// Measured pass count of the golden corpus. Raise it, never lower it.
const MEASURED_GOLDEN_PASS: usize = 1137; // measured 2026-10-07: 1137/1137

#[test]
fn golden_corpus_matches_winget() {
    let n = NameNormalizer::new();
    let (names, pubs, ids): (Vec<&str>, Vec<&str>, Vec<&str>) = (
        NAMES.lines().collect(),
        PUBS.lines().collect(),
        IDS.lines().collect(),
    );
    assert_eq!(names.len(), pubs.len());
    assert_eq!(names.len(), ids.len());
    assert!(
        names.len() > 900,
        "corpus must be the full file, got {}",
        names.len()
    );
    let mut passed = 0;
    let mut first_fail = Vec::new();
    for ((name, publisher), expected) in names.iter().zip(&pubs).zip(&ids) {
        let r = n.normalize(name, publisher);
        let got = format!("{}.{}", r.publisher, r.name);
        if got == *expected {
            passed += 1;
        } else if first_fail.len() < 20 {
            first_fail.push(format!(
                "[{name}] [{publisher}] => {got} (esperado {expected})"
            ));
        }
    }
    eprintln!(
        "golden: {passed}/{} ; primeiras falhas:\n{}",
        names.len(),
        first_fail.join("\n")
    );
    assert!(
        passed >= MEASURED_GOLDEN_PASS,
        "regressão: {passed} < {MEASURED_GOLDEN_PASS}"
    );
    assert!(
        MEASURED_GOLDEN_PASS > 0,
        "MEASURED_GOLDEN_PASS must be set to the measured value"
    );
}

#[test]
fn architecture_detection() {
    let n = NameNormalizer::new();
    assert_eq!(n.normalize("Name", "").architecture, Architecture::Unknown);
    assert_eq!(n.normalize("Name x86", "").architecture, Architecture::X86);
    assert_eq!(
        n.normalize("Name x86_64", "").architecture,
        Architecture::X64
    );
    assert_eq!(
        n.normalize("Name (64 bit)", "").architecture,
        Architecture::X64
    );
    assert_eq!(
        n.normalize("Name 32/64 bit", "").architecture,
        Architecture::Unknown
    );
    assert_eq!(n.normalize("Fox86", "").architecture, Architecture::Unknown);
}

#[test]
fn locale_detection() {
    let n = NameNormalizer::new();
    assert_eq!(n.normalize("Name", "").locale, "");
    assert_eq!(n.normalize("Name en-US", "").locale, "en-us");
    assert_eq!(n.normalize("Name (es-mx)", "").locale, "es-mx");
    assert_eq!(n.normalize("Names-mx", "").locale, "");
}

#[test]
fn kb_numbers_are_kept() {
    assert_eq!(
        NameNormalizer::new().normalize("Fix for (KB42)", "").name,
        "FixforKB42"
    );
}

#[test]
fn name_with_arch_suffix() {
    let n = NameNormalizer::new();
    let r = n.normalize("Name(X64)", "");
    assert_eq!(r.name, "Name");
    assert_eq!(r.name_with_arch(), "Name(X64)");
}

#[test]
fn index_keys_match_real_index_rows() {
    // Values copied from tests/fixtures/winget/index-mini.db (norm_names2/norm_publishers2).
    let n = NameNormalizer::new();
    let k = n.index_keys("Mozilla Firefox (x64 en-US)", "Mozilla");
    assert_eq!(k.name_with_arch.as_deref(), Some("mozillafirefox(X64)"));
    assert_eq!(k.name, "mozillafirefox");
    assert_eq!(k.publisher, "mozilla");
    let k = n.index_keys("7-Zip 24.08 (x64)", "Igor Pavlov");
    assert_eq!(k.name_with_arch.as_deref(), Some("7zip(X64)"));
    assert_eq!(k.publisher, "igorpavlov");
    let k = n.index_keys("Notepad++ (64-bit x64)", "Notepad++ Team");
    assert_eq!(k.name, "notepad");
    assert_eq!(k.publisher, "notepadteam");
}

#[test]
fn all_patterns_compile() {
    assert!(wingetrs::installed::normalize::pattern_errors().is_empty());
}
