//! Version comparison (winget `Utility::Version` semantics).
use std::cmp::Ordering;
use std::fmt;

const DEFAULT_SPLIT: &str = ".";
const LATEST: &str = "Latest";
const UNKNOWN: &str = "Unknown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub integer: u64,
    pub other: String,
}

impl Part {
    // port: Versions.cpp Version::Part::Part(const std::string&)
    fn from_text(part: &str) -> Part {
        let t = part.trim();
        let digits = t.bytes().take_while(u8::is_ascii_digit).count();
        let (num, rest) = t.split_at(digits);
        if num.is_empty() {
            return Part {
                integer: 0,
                other: rest.to_string(),
            };
        }
        match num.parse::<u64>() {
            Ok(integer) => Part {
                integer,
                other: rest.to_string(),
            },
            // out of range: whole text goes to `other`
            Err(_) => Part {
                integer: 0,
                other: t.to_string(),
            },
        }
    }

    fn is_zero(&self) -> bool {
        self.integer == 0 && self.other.is_empty()
    }

    fn folded(&self) -> String {
        self.other.to_lowercase()
    }

    // port: Versions.cpp Version::Part::operator<
    fn compare(&self, other: &Part) -> Ordering {
        match self.integer.cmp(&other.integer) {
            Ordering::Equal => {}
            o => return o,
        }
        match (self.other.is_empty(), other.other.is_empty()) {
            (true, true) => Ordering::Equal,
            // empty suffix sorts after a non-empty one ("1-rc" < "1")
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.folded().cmp(&other.folded()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Version {
    raw: String,
    parts: Vec<Part>,
}

impl Version {
    pub fn parse(s: &str) -> Version {
        Self::parse_with(s, DEFAULT_SPLIT)
    }

    // port: Versions.cpp Version::Assign
    pub fn parse_with(s: &str, split_chars: &str) -> Version {
        let raw = s.trim().to_string();
        let mut base: &str = &raw;

        // Leading non-digits are dropped when a digit precedes the first split char (or none exists).
        let digit_pos = base.find(|c: char| c.is_ascii_digit());
        let split_pos = base.find(|c: char| split_chars.contains(c));
        if let Some(d) = digit_pos
            && split_pos.is_none_or(|sp| d < sp)
        {
            base = &base[d..];
        }

        let mut parts = Vec::new();
        let mut rest = base;
        while !rest.is_empty() {
            match rest.find(|c: char| split_chars.contains(c)) {
                Some(i) => {
                    parts.push(Part::from_text(&rest[..i]));
                    let sep_len = rest[i..].chars().next().map_or(1, char::len_utf8);
                    rest = &rest[i + sep_len..];
                }
                None => {
                    parts.push(Part::from_text(rest));
                    rest = "";
                }
            }
        }

        let mut v = Version { raw, parts };
        v.trim();
        v
    }

    // port: Versions.cpp Version::Trim
    fn trim(&mut self) {
        while self.parts.last().is_some_and(Part::is_zero) {
            self.parts.pop();
        }
    }

    // port: Versions.cpp Version::CreateLatest
    pub fn latest() -> Version {
        Version {
            raw: LATEST.to_string(),
            parts: vec![Part {
                integer: 0,
                other: LATEST.to_string(),
            }],
        }
    }

    // port: Versions.cpp Version::CreateUnknown
    pub fn unknown() -> Version {
        Version {
            raw: UNKNOWN.to_string(),
            parts: vec![Part {
                integer: 0,
                other: UNKNOWN.to_string(),
            }],
        }
    }

    fn is_single(&self, word: &str) -> bool {
        matches!(self.parts.as_slice(), [p] if p.integer == 0 && p.other.eq_ignore_ascii_case(word))
    }

    // port: Versions.cpp Version::IsLatest
    pub fn is_latest(&self) -> bool {
        self.is_single(LATEST)
    }

    // port: Versions.cpp Version::IsUnknown
    pub fn is_unknown(&self) -> bool {
        self.is_single(UNKNOWN)
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    // port: Versions.cpp Version::PartAt
    pub fn part_at(&self, index: usize) -> Part {
        self.parts.get(index).cloned().unwrap_or(Part {
            integer: 0,
            other: String::new(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    // port: Versions.cpp Version::operator<
    fn cmp(&self, other: &Self) -> Ordering {
        // Latest sorts above everything.
        match (self.is_latest(), other.is_latest()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        // Unknown sorts below every known value.
        match (self.is_unknown(), other.is_unknown()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let zero = Part {
            integer: 0,
            other: String::new(),
        };
        for i in 0..self.parts.len().max(other.parts.len()) {
            let a = self.parts.get(i).unwrap_or(&zero);
            let b = other.parts.get(i).unwrap_or(&zero);
            match a.compare(b) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}
