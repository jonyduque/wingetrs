Status: DONE

Commits (git log base..HEAD; the other two are the parallel agent's task 11):
7da0cf1 feat(update): parallel source refresh, config and search/show commands
8ea831c chore(tasks): record task 11
c1edf63 feat(installed): collect ARP, MSIX and scoop installs

Red step: before implementing, `cargo test --test update --test config` failed with `error[E0432]: unresolved import wingetrs::config::Config` / `unresolved imports wingetrs::update::Endpoints, Outcome, needs_refresh, run_update`.

Gate:
[OK]   fmt 1s
[OK]   clippy 6s
[OK]   test (106 passed, 1 ignored) 34s
[OK]   gate verde
(clippy did not reject any test literal; no lint allowed beyond the brief's header.)

Mutation proofs (-TestCommand 'cargo test --quiet --test update'):
1. per-source error -> `return Err(..)`: [OK] mutação PEGA: 'Some(Err(msg)) => Outcome::Failed(msg),' -> 'Some(Err(msg)) => return Err(UpdateError::Cache(crate::cache::CacheError::Json(msg))),' fez o teste falhar (exit 101).
2. MAX_AGE_SECS 900 -> 1000: [OK] mutação PEGA: 'MAX_AGE_SECS: i64 = 900' -> 'MAX_AGE_SECS: i64 = 1000' fez o teste falhar (exit 101).
3. ignore lock None (`return Err(UpdateError::Busy);` -> `return Ok(Vec::new());`): [OK] mutação PEGA: 'return Err(UpdateError::Busy);...' fez o teste falhar. Files restored; update tests 4 passed afterwards.

Step 4 (real network):
$ cargo run --quiet -- update --force
winget: 15403 pacotes (0 ignorados)
scoop:main: 1669 pacotes (0 ignorados)
scoop:extras: 2406 pacotes (0 ignorados)
$ cargo run --quiet -- search powertoys -n 5
Nome                                             Id                                  Versão        Fonte
PowerToys                                        Microsoft.PowerToys                 0.101.2362.0  winget
powertoys                                        powertoys                           0.101.2362.0  scoop:extras
Browser Bookmarks for PowerToys Command Palette  GOODJINC.CmdPalBrowserBookmarks     0.2.3         winget
PowerToys-Run-Currency-Converter                 advaith.CurrencyConverterPowerToys  1.5.4         winget
everything-powertoys                             everything-powertoys                0.91.1        scoop:extras
(both exit 0)

Decisions:
- cli::data_dir honors the LOCALAPPDATA env var before DataDir::default_location, because the directories crate uses the Windows known-folder API and ignores the env var; without this cli_offline would touch the real catalog (now populated by Step 4).
- Scoop test endpoints: WINGETRS_SCOOP_BASE_URL gives `{base}/{bucket}.tgz`.
- `list` stays "ainda não implementado" (not in this task); search/show use ensure_catalog (blocking update if empty, detached background refresh if stale).
- `show` ambiguity error: "pacote ambíguo: <id> existe em a, b; use --source".
- Lock acquisition runs in spawn_blocking.

Concerns: none. Files touched beyond list: none. show/background spawn not exercised by tests (show not manually run).

## Follow-up: force refresh when catalog empty (a3d1e74)
- ensure_catalog now calls run_update with force = true when the catalog is empty.
- New test cli_offline::deleted_catalog_with_fresh_http_state_is_rebuilt_by_search (binary + wiremock; update, delete catalog.db, search --json gives 28 entries).
- Mutation: [OK] mutação PEGA: 'run_update(data, &ep, http, true)' -> 'run_update(data, &ep, http, false)' fez o teste falhar (exit 101).
- Gate: [OK] test (113 passed, 2 ignored) / [OK] gate verde.
