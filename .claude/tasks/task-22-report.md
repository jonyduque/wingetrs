Status: DONE

Commits:
1f7e6b9 feat(normalize): NFKC and full Unicode case folding as in winget
(base 470113d; the log range also lists other agents' commits)

Gate:
[OK]   fmt 1s
[OK]   clippy 6s
[OK]   test (167 passed, 2 ignored) 64s
[OK]   gate verde

golden: 1137/1137

Mutation proofs (--test normalize):
[OK] mutação PEGA: 'self.normalize(&fold_case(display_name), &fold_case(publisher))' -> 'self.normalize(&display_name.to_lowercase(), &publisher.to_lowercase())' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'let normalized: String = value.nfkc().collect();' -> 'let normalized: String = value.to_string();' fez o teste falhar (exit 101).

Decisions:
- Confirmed in winget @39739564: Utility::Normalize default form is NormalizationKC (AppInstallerStrings.h:27/30, Win32 NormalizeString); FoldCase = ucasemap_utf8FoldCase with U_FOLD_CASE_DEFAULT (AppInstallerStrings.cpp:491-). Matches the brief; both files saved in docs/reference/winget-cli/.
- NFKC is applied only in PrepareForValidation (before Trim), as in C++ line 35; fold_case replaces to_lowercase in index_keys, fold_and_sort, remove_locale, split.
- Table from Unicode 18.0.0 CaseFolding (C+F), 1612 lines; casefold_table.rs is included via #[path] in normalize.rs (not in installed/mod.rs, outside Files). Dependency unicode-normalization 0.1.25 (Cargo.lock updated, no cargo update).
- Test body is the brief's, plus a comment citing the C++ source; I did not observe the red run before implementing (proofs cover it).

Concerns:
- Timing impact of new+index_keys on list: not measured separately; live_list_runs_under_budget passes (0.30s release). The Unicode data version of the crate (NFKC) differs from Windows' OS NormalizeString and from table 18.0.0 in rare new characters.
