#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::sources::winget::mszip::{MszipError, decode};
use wingetrs::sources::winget::version_data::{decode_and_parse, parse, version_data_path};

const PT: &[u8] = include_bytes!("../fixtures/winget/powertoys.versionData.mszyml");
const PT_YAML: &[u8] = include_bytes!("../fixtures/winget/powertoys.versionData.yaml");
const DC: &[u8] = include_bytes!("../fixtures/winget/discord-canary.versionData.mszyml");
const DC_YAML: &[u8] = include_bytes!("../fixtures/winget/discord-canary.versionData.yaml");

#[test]
fn single_block_matches_reference_decoder() {
    assert_eq!(decode(PT).unwrap(), PT_YAML);
}

#[test]
fn multi_block_needs_dictionary_chaining() {
    let out = decode(DC).unwrap();
    assert_eq!(out.len(), 112_103);
    assert_eq!(out, DC_YAML);
}

#[test]
fn truncated_input_is_an_error_not_a_panic() {
    for cut in [0, 3, 10, 23, 24, 27, 30, DC.len() / 2, DC.len() - 1] {
        assert!(decode(&DC[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn bad_signature() {
    let mut b = PT.to_vec();
    b[0] ^= 0xFF;
    assert_eq!(decode(&b), Err(MszipError::BadSignature));
}

#[test]
fn missing_ck_marker() {
    let mut b = PT.to_vec();
    b[28] = b'X'; // first block payload starts at 24 + 4
    assert_eq!(decode(&b), Err(MszipError::MissingCk));
}

#[test]
fn size_mismatch_is_reported() {
    let mut b = PT.to_vec();
    b[8] = b[8].wrapping_add(1); // header claims one byte more
    assert!(matches!(
        decode(&b),
        Err(MszipError::SizeMismatch { .. }) | Err(MszipError::Truncated)
    ));
}

#[test]
fn parses_powertoys_versions() {
    let vd = decode_and_parse(PT).unwrap();
    assert_eq!(vd.entries.len(), 115);
    let first = &vd.entries[0];
    assert_eq!(first.version, "0.101.2362.0");
    assert_eq!(
        first.rel_path,
        "manifests/m/Microsoft/PowerToys/0.101.2362.0/509b"
    );
    assert_eq!(
        first.sha256,
        "345056f4d70d74ef350a0e00a7c5a710657271a882be4fa0bc263af6bb088245"
    );
}

#[test]
fn parses_793_discord_canary_versions() {
    assert_eq!(decode_and_parse(DC).unwrap().entries.len(), 793);
}

#[test]
fn numeric_looking_versions_stay_text() {
    let vd =
        parse("sV: 1.0\nvD:\n- v: 1.10\n  rP: a/b\n  s256H: ab\n- v: 2\n  rP: c\n  s256H: cd\n")
            .unwrap();
    assert_eq!(vd.entries[0].version, "1.10");
    assert_eq!(vd.entries[1].version, "2");
}

#[test]
fn garbage_yaml_is_an_error() {
    assert!(parse("vD: [unterminated").is_err());
}

#[test]
fn path_uses_first_8_hex_of_hash() {
    let hash =
        hex::decode("d193b67ac71ce5d456c10f6731447abb6df68ae12ef8691c92af8ed82659e720").unwrap();
    assert_eq!(
        version_data_path("Microsoft.PowerToys", &hash),
        "packages/Microsoft.PowerToys/d193b67a/versionData.mszyml"
    );
}
