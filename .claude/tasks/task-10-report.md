Status: DONE

Commits:
```
f0d218e feat(installed): port winget NameNormalizer (Initial)
```
(b0c1bf6 and fbc2428, also in 0d33f6a..HEAD, belong to other agents/orchestrator.)

golden: 1137/1137

```
golden: 1137/1137 ; primeiras falhas:
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 1.38s
```
`MEASURED_GOLDEN_PASS = 1137` (100% of corpus; no remaining divergences).

Gate:
```
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (87 passed, 0 ignored) 57s
[OK]   gate verde
```

Mutation proofs:
1. Remove `VersionDelimited` from the program-name regex list:
   `pwsh -File scripts/mutate.ps1 -Path src/installed/normalize.rs -Anchor "            VERSION_LETTER,`n            VERSION_DELIMITED," -Replacement "            VERSION_LETTER," -TestCommand 'cargo test --quiet --test normalize'`
   `[OK] mutação PEGA: '            VERSION_LETTER,` (anchor spans two lines; output line truncated by the line filter)
2. Remove fold before normalizing in `index_keys`:
   `pwsh -File scripts/mutate.ps1 -Path src/installed/normalize.rs -Anchor 'self.normalize(&display_name.to_lowercase(), &publisher.to_lowercase())' -Replacement 'self.normalize(display_name, publisher)' -TestCommand 'cargo test --quiet --test normalize'`
   `[OK] mutação PEGA: 'self.normalize(&display_name.to_lowercase(), &publisher.to_lowercase())' -> 'self.normalize(display_name, publisher)' fez o teste falhar (exit 101).`

Decisions:
- Regex application via own `for_each` over `find_iter` (port of ICU `Expression::ForEach`), so runtime regex errors (backtrack limit) stop matching and keep the text instead of panicking; `Remove`/KB replace built on it (KB: strip outer parens of the match instead of `$1`).
- `Split` faithfully emits separator segments as tokens too (the C++ callback ignores `isMatch`), then `NonLettersAndDigits` cleans them.
- `Utility::Normalize` (NFKC) is not applied — no crate for it in Cargo.toml; the corpus still passes 100%. `Trim` uses " \t\n\r\f\v". `find(L"@@", 3)` honors UTF-16 offsets.
- Added `pub fn pattern_errors()` + test `all_patterns_compile` (brief mentions it). Added `clippy::assertions_on_constants` to the test file's crate-level allow (assertion text unchanged) because clippy rejects `assert!(MEASURED_GOLDEN_PASS > 0)`.
- `index_keys_match_real_index_rows` passed with the brief's inputs unchanged.

Concerns:
- FoldCase approximated by `to_lowercase` (differs for e.g. ß, final sigma); NFKC missing (fullwidth/compat chars in real ARP names would diverge from the index). Not exercised by the corpus. Follow-up candidate if live correlation misses.
- During the first gate runs, clippy failed in other agents' uncommitted files (src/sources/winget/index.rs, mod.rs); resolved by itself before the final green gate.
