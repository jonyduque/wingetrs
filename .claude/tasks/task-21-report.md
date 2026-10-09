Status: DONE_WITH_CONCERNS

Commits:
```
$ git log --format='%h %s' c199bab..HEAD
433f511 feat(list): objective tie-breaks for ambiguous correlations
a67ba56 chore(tasks): task 22 approved      <- orchestrator commit, not mine
```

Gate:
```
[OK]   fmt 1s
[OK]   clippy 8s
[OK]   test (177 passed, 2 ignored) 53s
[OK]   gate verde
```
`cargo test --quiet --test correlate`: `test result: ok. 17 passed; 0 failed; 1 ignored`.
Live check (`cargo test --release --test correlate -- --ignored live_list`, it now loads the real tracking db): `test result: ok. 1 passed` (the test asserts < 500 ms).

Mutation proofs (all `--test correlate`):
```
$ pwsh -File scripts/mutate.ps1 -Path src/installed/tiebreak.rs -Anchor '.filter(|c| tracked.iter().any(|t| t.eq_ignore_ascii_case(c)))' -Replacement '.filter(|c| !tracked.iter().any(|t| t.eq_ignore_ascii_case(c)))' -TestCommand 'cargo test --quiet --test correlate'
[OK] mutação PEGA: '.filter(|c| tracked.iter().any(|t| t.eq_ignore_ascii_case(c)))' -> '.filter(|c| !tracked.iter().any(|t| t.eq_ignore_ascii_case(c)))' fez o teste falhar (exit 101).
$ pwsh -File scripts/mutate.ps1 -Path src/installed/tiebreak.rs -Anchor 'if Version::parse(min) <= inst && inst <= Version::parse(max) {' -Replacement 'if !(Version::parse(min) <= inst && inst <= Version::parse(max)) {' -TestCommand 'cargo test --quiet --test correlate'
[OK] mutação PEGA: 'if Version::parse(min) <= inst && inst <= Version::parse(max) {' -> 'if !(Version::parse(min) <= inst && inst <= Version::parse(max)) {' fez o teste falhar (exit 101).
$ pwsh -File scripts/mutate.ps1 -Path src/installed/tiebreak.rs -Anchor '.all(|o| channel_of(base, o).is_some_and(|s| !name.contains(&squash(s))))' -Replacement '.all(|o| channel_of(base, o).is_some())' -TestCommand 'cargo test --quiet --test correlate'
[OK] mutação PEGA: '.all(|o| channel_of(base, o).is_some_and(|s| !name.contains(&squash(s))))' -> '.all(|o| channel_of(base, o).is_some())' fez o teste falhar (exit 101).
$ (extra: rule 2 "every candidate needs a range": in the let-else, `return all();` -> `continue;`)
[OK] mutação PEGA: '        ) else {   (multi-line anchor)
  tiebreak_arp_range_never_drops_a_candidate_without_range --- FAILED
```
Tests that caught them: tracking -> `tiebreak_tracking_picks_the_id_winget_installed`; ARP range contains -> `tiebreak_arp_range_keeps_only_the_candidate_containing_the_version`; channel name -> `tiebreak_channel_display_name_with_suffix_does_not_pick_base`; range-required -> `tiebreak_arp_range_never_drops_a_candidate_without_range`.

Research (winget-cli @ 39739564, excerpt in `docs/reference/winget-cli/AppInstallerRepositoryCore_CompositeSource_SearchInstalled_3973956.cpp`):
- `CompositeSource::SearchInstalled`: the installed package's system references (ProductCode, UpgradeCode, PFN, NormalizedName+Publisher) are searched in the available source. With >1 match, `GetMatchingPackage` -> `FindOnlyStrongMatchFieldResult` accepts only a single strong match (ProductCode/UpgradeCode/PFN); otherwise it is "indeterminate". Then the **tracking catalog** of the source is searched with the same references (if several match, the latest `TrackingWriteTime` wins), and that id is used when the available search was indeterminate or empty.
- Tracking catalog: `%LOCALAPPDATA%\Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe\installed.db`, SQLite, winget V1 index schema 1.7 (`ids`, `manifest`, `productcodes(_map)`, `upgradecodes(_map)`, `pfns(_map)`, `norm_names(_map)`, `norm_publishers(_map)`, `manifest_metadata`). Inspected only on a copy in the scratchpad; wingetrs opens it with `SQLITE_OPEN_READ_ONLY`. The file's mtime (Oct 5 20:02) was the same after all runs.

Oracle (`winget list --source winget --disable-interactivity` x `wingetrs list --json`, release build, my own parser by header columns, case-insensitive, same method as Tasks 15/17):
```
BEFORE (base c199bab)
winget rows=152 distinct_ids=116 truncated_ids=0
wingetrs rows=282 matched_rows=133 matched_distinct_ids=130
oracle hits=110/116 rate=94.8%
wingetrs_matched_not_in_winget=20 (winget-source=0)
MISS Devolutions.UniGetUI / Google.AndroidStudio / Gyan.FFmpeg / MSYS2.MSYS2 / RubyInstallerTeam.RubyWithDevKit.4.0 / wez.wezterm

AFTER (433f511)
winget rows=152 distinct_ids=116 truncated_ids=0
wingetrs rows=282 matched_rows=137 matched_distinct_ids=134
oracle hits=114/116 rate=98.3%
wingetrs_matched_not_in_winget=20 (winget-source=0)
MISS MSYS2.MSYS2 []
MISS RubyInstallerTeam.RubyWithDevKit.4.0 []
TIEBREAK ('Android Studio', 'Google.AndroidStudio', 'WingetTracking')
TIEBREAK ('WezTerm version 20260917-114457-b09b56c2', 'wez.wezterm', 'WingetTracking')
TIEBREAK ('FFmpeg', 'Gyan.FFmpeg', 'WingetTracking')
TIEBREAK ('UniGetUI', 'Devolutions.UniGetUI', 'WingetTracking')
```
No new false positives: the 20 matched ids missing from `winget list` are all `scoop:*`, as before. Still Ambiguous: Ruby 3.4.9-1 [Ruby.3.4, RubyWithDevKit.3.4], Microsoft Update Health Tools (3 ids), Windows Desktop Runtime 3.1 (8 ids).

Decisions:
- Rule 2, first version (keep candidates without a range, drop only ranges that exclude the version) created 2 wrong Matched on the oracle: the Windows SDK 10.0.26100.7175 install was matched to `Microsoft.WindowsSDK.10.0.17763`, and Tesseract to UB-Mannheim. Cause: the catalog range only covers published versions, so an older install falls outside its own package's range. Fixed: the rule now applies only when every candidate has both bounds. Test `..._never_drops_a_candidate_without_range` covers this (installed 0.5).
- Rule 1 only uses tracked ids that are among the tied candidates, matched by the same system references winget uses. With >1 tracked candidate it narrows but does not pick by write time. Ruby (tracking says 4.0, candidates are 3.4) stays Ambiguous on purpose.
- Rules are filters applied in sequence: a rule that would drop every candidate is ignored. When nothing resolves, Ambiguous keeps the original candidate list. `ListRow.tiebreak: Option<Tiebreak>` = `WingetTracking | ArpVersionRange | Channel`.
- Channel suffix check: substring match on letters/digits only, lowercased (so "Pre-Release" = "PreRelease"). This errs on the side of not resolving (e.g. "Dev" is found inside "Devolutions").
- `correlate` now takes a 4th argument `&Tracking`. The CLI loads it in the scan thread, the TUI in `spawn_scan`. A missing or unreadable db means empty tracking plus a `tracing::warn`.

Concerns:
- Files edited outside the brief's **Files** list, required by the new signature and field: `src/cli/mod.rs` and `src/tui/run.rs` (pass the tracking), `tests/tui_state.rs` (`tiebreak: None` in the ListRow literal). In `src/cli/mod.rs` the two `join` results that hid worker panics with `map_err(|_| ...)` now use `std::panic::resume_unwind`, per the CLAUDE.md rule. `scan_all` (src/installed/mod.rs) still turns a collector panic into an error string; I did not touch it, it is outside this task.
- MSYS2.MSYS2 is still Unmatched. The tracking db has its ProductCode `{31267c81-…}`, and winget uses the tracking id even when the available search finds nothing. The brief limits the rules to tied candidates, so I did not implement this. It is a possible follow-up for 115/116.
- Ruby: winget shows RubyWithDevKit.4.0 (from tracking) while 3.4.9-1 is installed. wingetrs keeps it Ambiguous; this is a deliberate divergence.
- Rule 2 still assumes ARP ranges of different packages do not overlap by coincidence. It resolves nothing on this machine; it is covered only by synthetic tests.

---

## Rodada de revisão (CHANGES_REQUESTED → 68bc7d1)

Commits:
```
68bc7d1 perf(list): load winget tracking in parallel and skip WAL catalogs
433f511 feat(list): objective tie-breaks for ambiguous correlations
a67ba56 chore(tasks): task 22 approved      <- orchestrator commit, not mine
```

Mudanças:
1. `src/cli/mod.rs`: `tracking_job` própria, criada junto de `scan_job`/`norm_job` e juntada antes de `catalog?` (todos os caminhos), com `unwrap_or_else(|p| std::panic::resume_unwind(p))`. `src/tui/run.rs` (`spawn_scan`): o rastreamento carrega numa thread em paralelo a `scan_all`, juntada logo depois com `resume_unwind`.
2. `winget_tracking::header_is_wal(&[u8]) -> bool` (byte 18 == 2). `read_tracking` lê os 100 primeiros bytes e devolve `TrackingError::Wal` (o `load_default` registra com `tracing::warn!`) antes de abrir a conexão. Sem `immutable=1`. Testes: `tracking_header_wal_detection` (byte 1 ⇒ false, 2 ⇒ true, curto/vazio ⇒ false) e `read_tracking_refuses_wal_catalogs_without_creating_files` (base WAL sintética; lista do diretório idêntica antes/depois).
3. `tiebreak.rs`: o ramo inalcançável virou `candidates.iter().copied().find(..)`, com um comentário de uma linha explicando por que o primeiro vencedor é o único.

Gate:
```
[OK]   fmt 1s
[OK]   clippy 8s
[OK]   test (179 passed, 2 ignored) 59s
[OK]   gate verde
```

Mutação (WAL):
```
$ pwsh -File scripts/mutate.ps1 -Path src/installed/winget_tracking.rs -Anchor 'header.get(18) == Some(&2)' -Replacement 'header.get(18) == Some(&1)' -TestCommand 'cargo test --quiet --test correlate'
[OK] mutação PEGA: 'header.get(18) == Some(&2)' -> 'header.get(18) == Some(&1)' fez o teste falhar (exit 101).
  read_tracking_refuses_wal_catalogs_without_creating_files --- FAILED
  tracking_header_wal_detection --- FAILED
```
O teste de diretório também falhou com o mutante: em WAL, a abertura RO de fato criou arquivos auxiliares. Isso confirma o risco apontado na revisão.

`list --json`: `cmp list_433.json list_new.json && echo IDENTICAL` ⇒ `IDENTICAL`, e também contra uma segunda execução do binário de 433f511 (`IDENTICAL2`).

Bench `list` (`-Runs 15 -Warmup 2 -Baseline target/release/examples/noop.exe`), 2 rodadas intercaladas: novo (68bc7d1), 433f511 e a base c199bab, compilada num worktree temporário no scratchpad e removido depois.
```
target/release (68bc7d1): median=350,7 ms max=420,5 ms | baseline median=34,7 ms | delta median=310,8 ms
bin433 (433f511):         median=342,7 ms max=370,9 ms | baseline median=34,5 ms | delta median=306,7 ms
base (c199bab):           median=372,7 ms max=742,1 ms | baseline median=39,2 ms | delta median=331,8 ms
target/release (68bc7d1): median=337,1 ms max=373,2 ms | baseline median=35,2 ms | delta median=301,3 ms
bin433 (433f511):         median=350,2 ms max=395,6 ms | baseline median=35,6 ms | delta median=315,3 ms
base (c199bab):           median=331,8 ms max=351,5 ms | baseline median=34,6 ms | delta median=300,2 ms
```
Leitura: os três binários ficam iguais dentro do ruído (delta 300–332 ms). A diferença em relação a 272–282 ms vem do estado da máquina, não desta tarefa. O log de spans mede `winget_tracking` em 5,6 ms (`span step="winget_tracking" ms=5.6049…`), longe dos ~50 ms suspeitos. Médias de `correlate_ms` no log: 98 ms (10-08), 70 ms (10-09 17h) e 62 ms (10-09 18h), sem regressão.
`installed.db` do winget: mtime continua Oct 5 20:02 e não surgiu nenhum arquivo `-shm`/`-wal`/`-journal`.
