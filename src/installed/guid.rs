//! MSI "packed" GUID conversion (squished GUIDs used under Installer\UpgradeCodes).

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The transform is its own inverse: reverse 8, 4, 4 hex digits, swap nibbles in the rest.
fn transform(hex32: &str) -> String {
    let rev = |s: &str| s.chars().rev().collect::<String>();
    let tail: String = hex32.as_bytes()[16..]
        .chunks(2)
        .flat_map(|p| [p[1] as char, p[0] as char])
        .collect();
    format!(
        "{}{}{}{}",
        rev(&hex32[0..8]),
        rev(&hex32[8..12]),
        rev(&hex32[12..16]),
        tail
    )
}

/// "{...}" -> 32 uppercase hex.
pub fn pack_guid(guid: &str) -> Option<String> {
    let inner = guid.strip_prefix('{')?.strip_suffix('}')?;
    let parts: Vec<&str> = inner.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    if parts.len() != 5
        || parts
            .iter()
            .zip(lens)
            .any(|(p, l)| p.len() != l || !is_hex(p))
    {
        return None;
    }
    Some(transform(&parts.concat().to_ascii_uppercase()))
}

/// 32 hex -> "{...}" uppercase.
pub fn unpack_guid(packed: &str) -> Option<String> {
    if packed.len() != 32 || !is_hex(packed) {
        return None;
    }
    let h = transform(&packed.to_ascii_uppercase());
    Some(format!(
        "{{{}-{}-{}-{}-{}}}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}
