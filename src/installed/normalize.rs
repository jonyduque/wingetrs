//! Port of winget's `NameNormalizer` (`NormalizationVersion::Initial` only).
//!
//! Source: `docs/reference/winget-cli/AppInstallerCommonCore_NameNormalization.cpp`.
//! The index stores `Normalize(FoldCase(name), FoldCase(publisher))`; see [`NameNormalizer::index_keys`].

use fancy_regex::{Regex, RegexBuilder};
use unicode_normalization::UnicodeNormalization;

#[path = "casefold_table.rs"]
mod casefold_table;

/// Full Unicode case folding (statuses C + F), as ICU `U_FOLD_CASE_DEFAULT`; unmapped chars pass through.
// port: AppInstallerStrings.cpp FoldCase (ucasemap_utf8FoldCase, default options)
pub fn fold_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
            continue;
        }
        match casefold_table::CASE_FOLDING.binary_search_by_key(&c, |&(k, _)| k) {
            Ok(i) => out.push_str(casefold_table::CASE_FOLDING[i].1),
            Err(_) => out.push(c),
        }
    }
    out
}

/// Detected processor architecture of a program name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Unknown,
    X86,
    X64,
    Arm,
    Arm64,
}

impl Architecture {
    // port: AppInstallerArchitecture ToString (used by GetNormalizedName)
    fn as_str(self) -> &'static str {
        match self {
            Architecture::Unknown => "Unknown",
            Architecture::X86 => "X86",
            Architecture::X64 => "X64",
            Architecture::Arm => "Arm",
            Architecture::Arm64 => "Arm64",
        }
    }
}

/// Result of normalizing a name/publisher pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedName {
    pub name: String,
    pub publisher: String,
    pub architecture: Architecture,
    pub locale: String,
}

impl NormalizedName {
    /// GetNormalizedName(NormalizationField::Architecture): name + "(X64)" etc. when known.
    // port: NameNormalization.cpp NormalizedName::GetNormalizedName
    pub fn name_with_arch(&self) -> String {
        if self.architecture == Architecture::Unknown {
            self.name.clone()
        } else {
            format!("{}({})", self.name, self.architecture.as_str())
        }
    }
}

/// Keys as stored in the winget index (`norm_names2` / `norm_publishers2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexKeys {
    pub name: String,
    pub name_with_arch: Option<String>,
    pub publisher: String,
}

// port: NameNormalization.cpp NormalizationInitial regex constants
const ARCHITECTURE_X32: &str = r"(?<=^|[^\p{L}\p{Nd}])(X32|X86)(?=\P{Nd}|$)(?:\sEDITION)?";
const ARCHITECTURE_X64: &str =
    r"(?<=^|[^\p{L}\p{Nd}])(X64|AMD64|X86([\p{Pd}\p{Pc}]64))(?=\P{Nd}|$)(?:\sEDITION)?";
const ARCHITECTURE_32_BIT: &str =
    r"(?<=^|[^\p{L}\p{Nd}])(32[\p{Pd}\p{Pc}\p{Z}]?BIT)S?(?:\sEDITION)?";
const ARCHITECTURE_64_BIT: &str =
    r"(?<=^|[^\p{L}\p{Nd}])(64[\p{Pd}\p{Pc}\p{Z}]?BIT)S?(?:\sEDITION)?";
const ARCHITECTURE_32_OR_64_BIT: &str =
    r"(?<=^|[^\p{L}\p{Nd}])((64[\\\/]32|32[\\\/]64)[\p{Pd}\p{Pc}\p{Z}]?BIT)S?(?:\sEDITION)?";
const LOCALE: &str =
    r"(?<![A-Z])((?:\p{Lu}{2,3}(-(CANS|CYRL|LATN|MONG))?-\p{Lu}{2})(?![A-Z])(?:-VALENCIA)?)";
const SAP_PACKAGE: &str = r"^(?:[\p{Lu}\p{Nd}]+[\._])+[\p{Lu}\p{Nd}]+(?:-(?:\p{Nd}+\.)+\p{Nd}+)(?:-(?:\p{Lu}{2}(?:_\p{Lu}{2})?|CORE))(?:-(?:\p{Lu}{2}|\p{Nd}{2}))$";
const KB_NUMBERS: &str = r"\((KB\d+)\)";
const NON_LETTERS_AND_DIGITS: &str = r"[^\p{L}\p{Nd}]";
const URI_PROTOCOL: &str = r"(?<!\p{L})(?:http[s]?|ftp):\/\/";
const VERSION_DELIMITED: &str = r"((?<!\p{L})(?:V|VER|VERSI(?:O|Ó)N|VERSÃO|VERSIE|WERSJA|BUILD|RELEASE|RC|SP)\P{L}?)?\p{Nd}+([\p{Po}\p{Pd}\p{Pc}]\p{Nd}?(RC|B|A|R|SP|K)?\p{Nd}+)+([\p{Po}\p{Pd}\p{Pc}]?[\p{L}\p{Nd}]+)*";
const VERSION: &str = r"(FOR\s)?(?<!\p{L})(?:P|V|R|VER|VERSI(?:O|Ó)N|VERSÃO|VERSIE|WERSJA|BUILD|RELEASE|RC|SP)(?:\P{L}|\P{L}\p{L})?(\p{Nd}|\.\p{Nd})+(?:RC|B|A|R|V|SP)?\p{Nd}?";
const VERSION_LETTER: &str = r"(?<!\p{L})(?:(?:V|VER|VERSI(?:O|Ó)N|VERSÃO|VERSIE|WERSJA|BUILD|RELEASE|RC|SP)\P{L})?\p{Lu}\p{Nd}+(?:[\p{Po}\p{Pd}\p{Pc}]\p{Nd}+)+";
const NON_NESTED_BRACKET: &str = r"\([^\(\)]*\)|\[[^\[\]]*\]";
const BRACKET_ENCLOSED: &str = r#"(?:\p{Ps}.*\p{Pe}|".*")"#;
const LEADING_SYMBOLS: &str = r"^[^\p{L}\p{Nd}]+";
const TRAILING_NON_LETTERS: &str = r"\P{L}+$";
const PREFIX_PARENS: &str = r"^\(.*?\)";
const EMPTY_PARENS: &str = r#"(\(\s*\)|\[\s*\]|"\s*")"#;
const EN: &str = r"\sEN\s*$";
const TRAILING_SYMBOLS: &str = r"[^\p{L}\p{Nd}]+$";
const FILE_PATH: &str = r"((INSTALLED\sAT|IN)\s)?[CDEF]:\\(.+?\\)*[^\s]*\\?";
const FILE_PATH_GHS: &str = r"\(CHANGE #\d{1,2} TO [CDEF]:\\(.+?\\)*[^\s]*\\?\)";
const FILE_PATH_PARENS: &str = r"\([CDEF]:\\(.+?\\)*[^\s]*\\?\)";
const FILE_PATH_QUOTES: &str = r#""[CDEF]:\\(.+?\\)*[^\s]*\\?""#;
const ROBLOX: &str = r"(?<=^ROBLOX\s(PLAYER|STUDIO))(\sFOR\s.*)";
const BOMGAR: &str =
    r"(?<=^BOMGAR\s(JUMP CLIENT|(ACCESS|REPRESENTATIVE) CONSOLE|BUTTON)|^EMBEDDED CALLBACK)(\s.*)";
const ACRONYM_SEPARATORS: &str = r"(?:(?<=^\p{L})|(?<=\P{L}\p{L}))(\.|\/)(?=\p{L}(?:\P{L}|$))";
const NON_LETTERS: &str = r"(?<=^|\s)[^\p{L}]+(?=\s|$)";
const PROGRAM_NAME_SPLIT: &str = r"[^\p{L}\p{Nd}\+\&]";
const PUBLISHER_NAME_SPLIT: &str = r"[^\p{L}\p{Nd}]";

const ALL_PATTERNS: &[&str] = &[
    ARCHITECTURE_X32,
    ARCHITECTURE_X64,
    ARCHITECTURE_32_BIT,
    ARCHITECTURE_64_BIT,
    ARCHITECTURE_32_OR_64_BIT,
    LOCALE,
    SAP_PACKAGE,
    KB_NUMBERS,
    NON_LETTERS_AND_DIGITS,
    URI_PROTOCOL,
    VERSION_DELIMITED,
    VERSION,
    VERSION_LETTER,
    NON_NESTED_BRACKET,
    BRACKET_ENCLOSED,
    LEADING_SYMBOLS,
    TRAILING_NON_LETTERS,
    PREFIX_PARENS,
    EMPTY_PARENS,
    EN,
    TRAILING_SYMBOLS,
    FILE_PATH,
    FILE_PATH_GHS,
    FILE_PATH_PARENS,
    FILE_PATH_QUOTES,
    ROBLOX,
    BOMGAR,
    ACRONYM_SEPARATORS,
    NON_LETTERS,
    PROGRAM_NAME_SPLIT,
    PUBLISHER_NAME_SPLIT,
];

// port: NameNormalization.cpp LocaleViews
const LOCALE_VIEWS: &[&str] = &[
    "AF-ZA",
    "AM-ET",
    "AR-AE",
    "AR-BH",
    "AR-DZ",
    "AR-EG",
    "AR-IQ",
    "AR-JO",
    "AR-KW",
    "AR-LB",
    "AR-LY",
    "AR-MA",
    "ARN-CL",
    "AR-OM",
    "AR-QA",
    "AR-SA",
    "AR-SY",
    "AR-TN",
    "AR-YE",
    "AS-IN",
    "BA-RU",
    "BE-BY",
    "BG-BG",
    "BN-BD",
    "BN-IN",
    "BO-CN",
    "BR-FR",
    "CA-ES",
    "CA-ES-VALENCIA",
    "CO-FR",
    "CS-CZ",
    "CY-GB",
    "DA-DK",
    "DE-AT",
    "DE-CH",
    "DE-DE",
    "DE-LI",
    "DE-LU",
    "DSB-DE",
    "DV-MV",
    "EL-GR",
    "EN-AU",
    "EN-BZ",
    "EN-CA",
    "EN-GB",
    "EN-IE",
    "EN-IN",
    "EN-JM",
    "EN-MY",
    "EN-NZ",
    "EN-PH",
    "EN-SG",
    "EN-TT",
    "EN-US",
    "EN-ZA",
    "EN-ZW",
    "ES-AR",
    "ES-BO",
    "ES-CL",
    "ES-CO",
    "ES-CR",
    "ES-DO",
    "ES-EC",
    "ES-ES",
    "ES-GT",
    "ES-HN",
    "ES-MX",
    "ES-NI",
    "ES-PA",
    "ES-PE",
    "ES-PR",
    "ES-PY",
    "ES-SV",
    "ES-US",
    "ES-UY",
    "ES-VE",
    "ET-EE",
    "EU-ES",
    "FA-IR",
    "FI-FI",
    "FIL-PH",
    "FO-FO",
    "FR-BE",
    "FR-CA",
    "FR-CH",
    "FR-FR",
    "FR-LU",
    "FR-MC",
    "FY-NL",
    "GA-IE",
    "GD-DB",
    "GL-ES",
    "GSW-FR",
    "GU-IN",
    "HE-IL",
    "HI-IN",
    "HR-BA",
    "HR-HR",
    "HSB-DE",
    "HU-HU",
    "HY-AM",
    "ID-ID",
    "IG-NG",
    "II-CN",
    "IS-IS",
    "IT-CH",
    "IT-IT",
    "JA-JP",
    "KA-GE",
    "KK-KZ",
    "KL-GL",
    "KM-KH",
    "KN-IN",
    "KOK-IN",
    "KO-KR",
    "KY-KG",
    "LB-LU",
    "LO-LA",
    "LT-LT",
    "LV-LV",
    "MI-NZ",
    "MK-MK",
    "ML-IN",
    "MN-MN",
    "MOH-CA",
    "MR-IN",
    "MS-BN",
    "MS-MY",
    "MT-MT",
    "NB-NO",
    "NE-NP",
    "NL-BE",
    "NL-NL",
    "NN-NO",
    "NSO-ZA",
    "OC-FR",
    "OR-IN",
    "PA-IN",
    "PL-PL",
    "PRS-AF",
    "PS-AF",
    "PT-BR",
    "PT-PT",
    "QUT-GT",
    "QUZ-BO",
    "QUZ-EC",
    "QUZ-PE",
    "RM-CH",
    "RO-RO",
    "RU-RU",
    "RW-RW",
    "SAH-RU",
    "SA-IN",
    "SE-FI",
    "SE-NO",
    "SE-SE",
    "SI-LK",
    "SK-SK",
    "SL-SI",
    "SMA-NO",
    "SMA-SE",
    "SMJ-NO",
    "SMJ-SE",
    "SMN-FI",
    "SMS-FI",
    "SQ-AL",
    "SV-FI",
    "SV-SE",
    "SW-KE",
    "SYR-SY",
    "TA-IN",
    "TE-IN",
    "TH-TH",
    "TK-TM",
    "TN-ZA",
    "TR-TR",
    "TT-RU",
    "UG-CN",
    "UK-UA",
    "UR-PK",
    "VI-VN",
    "WO-SN",
    "XH-ZA",
    "YO-NG",
    "ZH-CN",
    "ZH-HK",
    "ZH-MO",
    "ZH-SG",
    "ZH-TW",
    "ZU-ZA",
    "AZ-CYRL-AZ",
    "AZ-LATN-AZ",
    "BS-CYRL-BA",
    "BS-LATN-BA",
    "HA-LATN-NG",
    "IU-CANS-CA",
    "IU-LATN-CA",
    "MN-MONG-CN",
    "SR-CYRL-BA",
    "SR-CYRL-CS",
    "SR-CYRL-ME",
    "SR-CYRL-RS",
    "SR-LATN-BA",
    "SR-LATN-CS",
    "SR-LATN-ME",
    "SR-LATN-RS",
    "TG-CYRL-TJ",
    "TZM-LATN-DZ",
    "UZ-CYRL-UZ",
    "UZ-LATN-UZ",
];

// port: NameNormalization.cpp LegalEntitySuffixViews
const LEGAL_ENTITY_SUFFIX_VIEWS: &[&str] = &[
    "AB",
    "AD",
    "AG",
    "APS",
    "AS",
    "ASA",
    "BV",
    "CO",
    "CV",
    "DOO",
    "eV",
    "GES",
    "GESMBH",
    "GMBH",
    "INC",
    "KG",
    "KS",
    "PS",
    "LLC",
    "LP",
    "LTD",
    "LTDA",
    "MBH",
    "NV",
    "PLC",
    "SL",
    "PTY",
    "PVT",
    "SA",
    "SARL",
    "SC",
    "SCA",
    "SL",
    "SP",
    "SPA",
    "SRL",
    "SRO",
    "COMPANY",
    "CORP",
    "CORPORATION",
    "HOLDING",
    "HOLDINGS",
    "INCORPORATED",
    "LIMITED",
    "SUBSIDIARY",
];

/// Patterns that fail to compile with `fancy-regex` (expected: none).
pub fn pattern_errors() -> Vec<(&'static str, String)> {
    ALL_PATTERNS
        .iter()
        .filter_map(|p| build(p).err().map(|e| (*p, e.to_string())))
        .collect()
}

fn build(pattern: &str) -> Result<Regex, fancy_regex::Error> {
    RegexBuilder::new(pattern).case_insensitive(true).build()
}

/// A compiled pattern; `None` never matches (only if a constant failed to compile).
struct Re(Option<Regex>);

impl Re {
    fn new(pattern: &str) -> Re {
        match build(pattern) {
            Ok(r) => Re(Some(r)),
            Err(e) => {
                tracing::error!(pattern, error = %e, "name normalization pattern failed to compile");
                Re(Regex::new("(?!)").ok())
            }
        }
    }

    // port: Regex.cpp Expression::ForEach — calls `f(is_match, segment)` for every
    // matched and unmatched segment in order; stops when `f` returns false.
    fn for_each(&self, input: &str, mut f: impl FnMut(bool, &str) -> bool) {
        let mut last = 0;
        if let Some(re) = &self.0 {
            for m in re.find_iter(input) {
                let Ok(m) = m else {
                    tracing::warn!("name normalization regex hit a runtime error");
                    break;
                };
                if m.start() > last && !f(false, &input[last..m.start()]) {
                    return;
                }
                if !f(true, m.as_str()) {
                    return;
                }
                last = m.end();
            }
        }
        if last < input.len() {
            f(false, &input[last..]);
        }
    }

    fn replace_with(&self, input: &str, mut g: impl FnMut(&str) -> String) -> String {
        let mut out = String::with_capacity(input.len());
        self.for_each(input, |is_match, text| {
            if is_match {
                out.push_str(&g(text));
            } else {
                out.push_str(text);
            }
            true
        });
        out
    }

    fn is_match(&self, input: &str) -> bool {
        self.0
            .as_ref()
            .is_some_and(|re| re.is_match(input).unwrap_or(false))
    }
}

/// Compiled winget `NormalizationInitial` normalizer.
pub struct NameNormalizer {
    architecture_x32: Re,
    architecture_x64: Re,
    architecture_32_bit: Re,
    architecture_64_bit: Re,
    architecture_32_or_64_bit: Re,
    locale: Re,
    sap_package: Re,
    kb_numbers: Re,
    non_letters_and_digits: Re,
    program_name_split: Re,
    publisher_name_split: Re,
    program_name_regexes: Vec<Re>,
    publisher_name_regexes: Vec<Re>,
    locales: Vec<String>,
    legal_entity_suffixes: Vec<String>,
}

impl Default for NameNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

fn fold_and_sort(input: &[&str]) -> Vec<String> {
    // port: NameNormalization.cpp FoldAndSort
    let mut v: Vec<String> = input.iter().map(|s| fold_case(s)).collect();
    v.sort();
    v
}

// port: AppInstallerStrings.cpp Trim (whitespace set " \t\n\r\f\v")
fn trim_ws(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C' | '\x0B'))
}

fn is_empty_or_whitespace(s: &str) -> bool {
    trim_ws(s).is_empty()
}

impl NameNormalizer {
    pub fn new() -> NameNormalizer {
        // port: NameNormalization.cpp ProgramNameRegexes (order matters)
        let program_name_regexes = [
            ROBLOX,
            BOMGAR,
            PREFIX_PARENS,
            EMPTY_PARENS,
            FILE_PATH_GHS,
            FILE_PATH_PARENS,
            FILE_PATH_QUOTES,
            FILE_PATH,
            VERSION_LETTER,
            VERSION_DELIMITED,
            VERSION,
            EN,
            NON_NESTED_BRACKET,
            BRACKET_ENCLOSED,
            URI_PROTOCOL,
            LEADING_SYMBOLS,
            TRAILING_SYMBOLS,
        ]
        .iter()
        .map(|p| Re::new(p))
        .collect();
        // port: NameNormalization.cpp PublisherNameRegexes (order matters)
        let publisher_name_regexes = [
            VERSION_DELIMITED,
            VERSION,
            NON_NESTED_BRACKET,
            BRACKET_ENCLOSED,
            URI_PROTOCOL,
            NON_LETTERS,
            TRAILING_NON_LETTERS,
            ACRONYM_SEPARATORS,
        ]
        .iter()
        .map(|p| Re::new(p))
        .collect();
        NameNormalizer {
            architecture_x32: Re::new(ARCHITECTURE_X32),
            architecture_x64: Re::new(ARCHITECTURE_X64),
            architecture_32_bit: Re::new(ARCHITECTURE_32_BIT),
            architecture_64_bit: Re::new(ARCHITECTURE_64_BIT),
            architecture_32_or_64_bit: Re::new(ARCHITECTURE_32_OR_64_BIT),
            locale: Re::new(LOCALE),
            sap_package: Re::new(SAP_PACKAGE),
            kb_numbers: Re::new(KB_NUMBERS),
            non_letters_and_digits: Re::new(NON_LETTERS_AND_DIGITS),
            program_name_split: Re::new(PROGRAM_NAME_SPLIT),
            publisher_name_split: Re::new(PUBLISHER_NAME_SPLIT),
            program_name_regexes,
            publisher_name_regexes,
            locales: fold_and_sort(LOCALE_VIEWS),
            legal_entity_suffixes: fold_and_sort(LEGAL_ENTITY_SUFFIX_VIEWS),
        }
    }

    // port: NameNormalization.cpp NormalizationInitial::Normalize
    pub fn normalize(&self, name: &str, publisher: &str) -> NormalizedName {
        let (name, architecture, locale) = self.normalize_name_internal(name);
        NormalizedName {
            name,
            publisher: self.normalize_publisher_internal(publisher),
            architecture,
            locale,
        }
    }

    /// Keys as stored in the index: normalize(fold(name), fold(publisher)).
    // port: Interface_2_0.cpp UpdateNormalizedNameAndPublisher
    pub fn index_keys(&self, display_name: &str, publisher: &str) -> IndexKeys {
        let r = self.normalize(&fold_case(display_name), &fold_case(publisher));
        let name_with_arch = (r.architecture != Architecture::Unknown).then(|| r.name_with_arch());
        IndexKeys {
            name: r.name,
            name_with_arch,
            publisher: r.publisher,
        }
    }

    // port: NameNormalization.cpp PrepareForValidation (Utility::Normalize, NFKC)
    fn prepare_for_validation(value: &str) -> String {
        let normalized: String = value.nfkc().collect();
        let trimmed = trim_ws(&normalized);
        // `find(L"@@", 3)`: search starts at UTF-16 offset 3.
        let mut utf16 = 0usize;
        for (i, c) in trimmed.char_indices() {
            if utf16 >= 3 && trimmed[i..].starts_with("@@") {
                return trimmed[..i].to_string();
            }
            utf16 += c.len_utf16();
        }
        trimmed.to_string()
    }

    // port: NameNormalization.cpp Unwrap
    fn unwrap(value: &mut String) -> bool {
        if value.encode_utf16().count() >= 2 {
            let unwrap = match value.chars().next() {
                Some('"') => value.ends_with('"'),
                Some('(') => value.ends_with(')'),
                _ => false,
            };
            if unwrap {
                *value = value[1..value.len() - 1].to_string();
                return true;
            }
        }
        false
    }

    // port: NameNormalization.cpp Remove
    fn remove(re: &Re, input: &mut String) -> bool {
        let output = re.replace_with(input, |_| String::new());
        let changed = output != *input;
        *input = output;
        changed
    }

    // port: NameNormalization.cpp RemoveAll
    fn remove_all(regexes: &[Re], value: &mut String) -> bool {
        let mut result = false;
        for re in regexes {
            result = Self::remove(re, value) || result;
        }
        result
    }

    // port: NameNormalization.cpp RemoveArchitecture
    fn remove_architecture(&self, value: &mut String) -> Architecture {
        if Self::remove(&self.architecture_32_or_64_bit, value) {
            Architecture::Unknown
        } else if Self::remove(&self.architecture_x64, value)
            || Self::remove(&self.architecture_64_bit, value)
        {
            Architecture::X64
        } else if Self::remove(&self.architecture_x32, value)
            || Self::remove(&self.architecture_32_bit, value)
        {
            Architecture::X86
        } else {
            Architecture::Unknown
        }
    }

    // port: NameNormalization.cpp RemoveLocale
    fn remove_locale(&self, value: &mut String) -> String {
        let mut locale_found = false;
        let mut result = String::new();
        let mut new_value = String::with_capacity(value.len());
        self.locale.for_each(value, |is_match, text| {
            let mut copy = !is_match;
            if is_match {
                let mut folded = fold_case(text);
                if self.locales.binary_search(&folded).is_err() {
                    copy = true;
                } else if !locale_found {
                    result = folded;
                    locale_found = true;
                } else if !result.is_empty() && result != folded {
                    if let Some(p) = result.find('-') {
                        result.truncate(p);
                    }
                    if let Some(p) = folded.find('-') {
                        folded.truncate(p);
                    }
                    if result != folded {
                        result.clear();
                    }
                }
            }
            if copy {
                new_value.push_str(text);
            }
            true
        });
        *value = new_value;
        result
    }

    // port: NameNormalization.cpp Split
    fn split(&self, re: &Re, value: &str, stop_on_exclusion: bool) -> Vec<String> {
        let mut result: Vec<String> = Vec::new();
        re.for_each(value, |_, text| {
            if is_empty_or_whitespace(text) {
                return true;
            }
            if !result.is_empty() {
                let folded = fold_case(text);
                if self.legal_entity_suffixes.binary_search(&folded).is_ok() {
                    return !stop_on_exclusion;
                }
            }
            result.push(text.to_string());
            true
        });
        result
    }

    // port: NameNormalization.cpp NormalizeNameInternal
    fn normalize_name_internal(&self, name: &str) -> (String, Architecture, String) {
        let mut name = Self::prepare_for_validation(name);
        while Self::unwrap(&mut name) {}

        if self.sap_package.is_match(&name) {
            return (name, Architecture::Unknown, String::new());
        }

        let architecture = self.remove_architecture(&mut name);
        let locale = self.remove_locale(&mut name);

        // Extract KB numbers from their parens: "(KB123)" -> "KB123".
        name = self
            .kb_numbers
            .replace_with(&name, |m| m[1..m.len() - 1].to_string());

        while Self::remove_all(&self.program_name_regexes, &mut name) {}

        let tokens = self.split(&self.program_name_split, &name, false);
        let mut joined = tokens.concat();
        Self::remove(&self.non_letters_and_digits, &mut joined);
        (joined, architecture, locale)
    }

    // port: NameNormalization.cpp NormalizePublisherInternal
    fn normalize_publisher_internal(&self, publisher: &str) -> String {
        let mut publisher = Self::prepare_for_validation(publisher);
        while Self::unwrap(&mut publisher) {}

        while Self::remove_all(&self.publisher_name_regexes, &mut publisher) {}

        let tokens = self.split(&self.publisher_name_split, &publisher, true);
        let mut joined = tokens.concat();
        Self::remove(&self.non_letters_and_digits, &mut joined);
        joined
    }
}
