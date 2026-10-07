Status: DONE_WITH_CONCERNS

Commits:
```
e10d590 feat(list): correlate installs with catalog and report upgrades
3950058 chore(tasks): record task 9
```
(3950058 is the orchestrator's ledger commit made in parallel; only e10d590 is this task.)

Gate:
```
[OK]   fmt 1s
[OK]   clippy 8s
[OK]   test (112 passed, 2 ignored) 51s
[OK]   gate verde
```

Tests before implementation (only the module declared, empty): `error[E0432]: unresolved imports wingetrs::installed::correlate::correlate, ...update_state, ...Correlation, ...Rule, ...UpdateState`.
After: `test result: ok. 6 passed; 0 failed; 1 ignored`. Test file was run through rustfmt only (no assertion changes).

Mutation proofs (all with `-TestCommand 'cargo test --quiet --test correlate'`):
1. Rule order — `pwsh -File scripts/mutate.ps1 -Path src/installed/correlate.rs -Anchor '    for (table, key, rule) in keyed {' -Replacement '    if let Some(hit) = by_name_and_publisher(app, catalog, normalizer)? { return Ok(Some(hit)); }<NL>    for (table, key, rule) in keyed {'`
   `[OK] mutação PEGA: '    for (table, key, rule) in keyed {' -> '    if let Some(hit) = by_name_and_publisher(...)...` — failed `product_code_match_is_case_insensitive` (tests\correlate.rs:38) and `package_family_name_match`.
2. ARP range — `-Anchor 'if inst >= Version::parse(max)' -Replacement 'if inst >= Version::parse(latest)'`
   `[OK] mutação PEGA: 'if inst >= Version::parse(max)' -> 'if inst >= Version::parse(latest)' fez o teste falhar (exit 101).` — `update_state_table`.
3. Ambiguous — `-Anchor 'candidates.len() > 1' -Replacement 'candidates.len() > 99'` (multi-candidate falls into the Matched arm, which picks a candidate)
   `[OK] mutação PEGA: 'candidates.len() > 1' -> 'candidates.len() > 99' fez o teste falhar (exit 101).` — `ambiguous_is_never_resolved_silently`.

Live (`cargo test --quiet --test correlate -- --ignored --nocapture live_list`):
- debug (the command as briefed): `rows=250 matched=121 elapsed_ms=988` -> FAILED (assert < 500).
- `--release`: `rows=250 matched=121 elapsed_ms=473`, then `rows=250 matched=121 elapsed_ms=397` -> ok.

`cargo run --quiet --release -- list` (5 runs, Measure-Command around the release exe, includes catalog open + scan + correlate + print):
```
ms=408 | 250 instalados: 121 correlacionados, 9 ambíguos, 120 não correlacionados
ms=368 | ...
ms=448 | ...
ms=435 | ...
ms=442 | ...
```
Per rule (real catalog): Matched ProductCode 46, UpgradeCode 4, PackageFamilyName 15, NameAndPublisher 36, ScoopManifest 20 (=121); Ambiguous 9 (ProductCode 5, NameAndPublisher 4); Unmatched 120. `list --upgradable`: 20 upgradable rows.

Time breakdown (temporary instrumented test, removed before commit), ms:
- release: catalog open 12, scan 285, `NameNormalizer::new` 57-60, correlate 63-67 (of which `index_keys` for all 250 apps ~22; one `lookup` ~37 µs).
- debug: open 12, scan 232, `NameNormalizer::new` 332, correlate 223 (`index_keys` 150-200). Debug blows the budget in regex compilation/matching (fancy-regex unoptimized), not in SQL.
- `prepare_cached` in `Catalog::lookup` tried: lookup 37 -> 12.6 µs, correlate total 67 -> 63 ms (noise-level). Reverted because `src/catalog/mod.rs` is outside the file list.

Decisions:
- Name+publisher rule is skipped when the normalized name or publisher is empty (avoids matching every catalog row with an empty publisher key).
- Map-table hits are attributed to source "winget" (only the winget import fills map tables); package fetched with `catalog.get("winget", id)`.
- `list --source K` keeps rows correlated to that kind (Ambiguous counts as winget; Unmatched dropped); `--upgradable` keeps only `Available` rows, also for `--json`.
- Table output ends with a summary line `N instalados: X correlacionados, Y ambíguos, Z não correlacionados` (not with `--upgradable`); timings logged via `tracing::info!` (scan_ms, correlate_ms).
- `update_state` ignores `arp_min` (brief rules only use `arp_max`); kept in the signature.

Concerns:
- Live test fails in debug (988 ms) and passes in release (397-473 ms, close to the limit). Fixing debug would need `[profile.dev.package.fancy-regex] opt-level = 3` (and regex-automata) in Cargo.toml, or a lazily shared NameNormalizer — both outside this task's files. Test not loosened.
- Release `list` is 368-448 ms; ~60 ms of it is building `NameNormalizer` (regex compilation in normalize.rs) — the cheapest next win if the budget gets tighter.

## Fix round (review: MSIX matched by name+publisher)

Commit: `e2d8737 fix(list): never match MSIX packages by name and publisher` (the commits between e10d590 and e2d8737 belong to other agents/orchestrator).

- `by_name_and_publisher` now returns `None` unless `app.source` is `InstallSource::Arp(_)`.
- New test `msix_is_never_matched_by_name_and_publisher` (as given). Before fix: panicked at tests\correlate.rs:222:5; after: `test result: ok. 7 passed; 0 failed; 1 ignored`.
- Mutation: `pwsh -File scripts/mutate.ps1 -Path src/installed/correlate.rs -Anchor 'if !matches!(app.source, InstallSource::Arp(_)) {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test correlate'`
  `[OK] mutação PEGA: 'if !matches!(app.source, InstallSource::Arp(_)) {' -> 'if false {' fez o teste falhar (exit 101).`
- Cargo.toml: `[profile.dev.package.{fancy-regex,regex-automata,regex-syntax}] opt-level = 3`. Cargo.lock unchanged.
- `live_list_runs_under_budget` ignore reason now says the budget holds only with `--release`; limit unchanged (< 500).

Gate:
```
[OK]   fmt 1s
[OK]   clippy 5s
[OK]   test (125 passed, 2 ignored) 37s
[OK]   gate verde
```

Live: debug `rows=250 matched=118 elapsed_ms=567` (FAILED, was 988); release `rows=250 matched=118 elapsed_ms=479`, then `elapsed_ms=362`.

`cargo run --quiet --release -- list --upgradable` (17 rows; no VS Code, Zed or Edge):
```
Nome                    Id                           Instalada      Disponível     Fonte
7-Zip 26.03 (x64)       7zip.7zip                    26.03          26.04          winget
Docker Desktop          Docker.DockerDesktop         4.93.0         4.94.0         winget
Git                     Git.Git                      2.55.0.3       2.55.0.5       winget
WinScript               flick9000.WinScript          2.25.1         2.26.0         winget
CMake                   Kitware.CMake                4.4.3          4.4.4          winget
Google Chrome           Google.Chrome                154.0.8037.98  155.0.8059.40  winget
Anki                    Anki.Anki                    26.5.0         26.09.3        winget
Adobe Acrobat (64-bit)  Adobe.Acrobat.Reader.64-bit  26.002.21931   26.002.21996   winget
Amazon Kindle           Amazon.Kindle                2.8.3.70995    2.9.1.71006    winget
Python Launcher         Python.Launcher              3.13.12150.0   3.14.7         winget
uv                      astral-sh.uv                 0.12.21        0.12.23        winget
Obsidian                Obsidian.Obsidian            1.13.7         1.14.4         winget
less                    jftuga.less                  704            710            winget
Codex CLI               OpenAI.Codex                 0.159.3        0.160.1        winget
Ollama version 0.35.0   Ollama.Ollama                0.35.0         0.40.0         winget
7zip                    7zip                         26.03          26.04          scoop:main
oh-my-posh              oh-my-posh                   27.4.4         31.5.0         scoop:main
```
`list` summary: `250 instalados: 118 correlacionados, 9 ambíguos, 123 não correlacionados`, ms=460 (one Measure-Command run). The 3 lost matches are the MSIX NameAndPublisher ones (NameAndPublisher 36 -> 33; per-rule counts not re-measured).

Remaining concern: release live test at 362-479 ms is close to the 500 ms limit; debug still exceeds it (567 ms).
