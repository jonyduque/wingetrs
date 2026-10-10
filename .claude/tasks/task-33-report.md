Status: DONE_WITH_CONCERNS

Commits:
6896087 feat(install): recycle bin, HKCU residue scan and in-use detection
(other SHAs in e4ed737..HEAD belong to parallel agents)

Gate:
[OK]   rules 8s
[OK]   fmt 3s
[OK]   clippy 23s
[OK]   test (306 passed, 11 ignored) 141s
[OK]   gate verde

Live (--release --ignored, 4 passed): registry scan/export/delete_key, delete_value, Recycle Bin, scan speed. Test keys are removed by Drop.

Mutation proofs:
- case-insensitive compare: mutate.ps1 -Path src/install/registry_win.rs -Anchor 'data.to_lowercase().contains(needle)' -Replacement 'data.contains(needle)' -TestCommand 'cargo test --quiet --release --test install_win_scan live_registry_scan_export_delete -- --ignored'
  [OK] mutação PEGA: 'data.to_lowercase().contains(needle)' -> 'data.contains(needle)' fez o teste falhar (exit 101).
- files_in_use: mutate.ps1 -Path src/install/fs_win.rs -Anchor '.share_mode(0)' -Replacement '.share_mode(7)' -TestCommand 'cargo test --quiet --test install_win_scan files_in_use'
  [OK] mutação PEGA: '.share_mode(0)' -> '.share_mode(7)' fez o teste falhar (exit 101).

Performance (span reg_values_containing + Instant, machine loaded by parallel agents, noisy):
- HKCU\Software\Classes has ~80,963 keys (40k under "*", 26k under "Local Settings", 8k CLSID).
- sequential 1.8-3.0 s, parallel (<=8 threads, subtrees split 2 levels) 0.9-1.4 s, same 3526 hits in the same order (equivalence asserted in live_scan_speed).
- Uninstall + App Paths + Run roots: 8 ms measured when idle; 171 ms measured in a loaded run (< 300 ms).

Decisions:
- RegScanner::from_env() reads WINGETRS_TEST_ENV_KEY; with_base(&str) added for tests (empty base = real HKCU). values_containing_with(roots, needle, threads) is the public implementation; the trait method uses min(8, cpus) threads. Workers are scoped and joined; panics propagate with resume_unwind.
- Access mask for scans is QUERY_VALUE|ENUMERATE_SUB_KEYS (no measured gain; kept for least privilege).
- delete_key refuses an empty key. recycle returns an error if GetAnyOperationsAborted.
- files_in_use skips unreadable entries and does not follow symlinks (uses file_type of dir entries).

Concerns:
- The < 300 ms bound is NOT met for the full HKCU\Software\Classes root (~0.9-1.4 s here, kernel-bound; threading saturates at ~2x). It is met for Uninstall/App Paths/Run. The test asserts the bound only on those small roots. If Classes must be < 300 ms, options (orchestrator decision): prune known-irrelevant subtrees ("Local Settings", CLSID scanning only on demand) or scan Classes lazily. Not done because it changes what the scan covers.
- Files touched outside the list: none (tests/install_win_scan.rs is the brief's new test file).

## Addendum: targeted Classes scan (commit acb0a01 perf(install): targeted Classes residue scan)
Status: DONE. Gate: [OK] test (305 passed, 15 ignored) / [OK] gate verde. Live: 6 passed.
- Classes children: pruned Local Settings, CLSID, Interface, TypeLib, WOW6432Node, shellex. Descend: own values, DefaultIcon, Application, shell\<verb>(+command, nested shell up to 3 levels), Background\shell\<verb>(+command). Applications and SystemFileAssociations are containers (their children treated as ProgIDs). AppUserModelId (toast icon registrations, small) is walked fully. Other roots unchanged (recursive).
- Added after the coverage proof showed misses outside the pruned list: nested `shell\x\shell\y\command` (Python.File editwithidle), `Application` subkey, AppUserModelId.
- Timing (5 runs, 4 spec roots, median, tracing span reg_values_containing): [104,105,105,107,107] ms, median 105 ms (< 300). An earlier run: median 91 ms. Before: 900-1400 ms parallel.
- Coverage (live, HKCU real, final): needle "\appdata\": full 1746 hits, targeted 1652, only-full 94 (CLSID 10, TypeLib 77, WOW6432Node 7), only-targeted 0. Needle "c:\": full 7788, targeted 1716, only-full 6072 (CLSID 4179, Local Settings 1799, TypeLib 79, WOW6432Node 15), only-targeted 0. Zero misses outside pruned subtrees (asserted).
- live_classes_scan_is_targeted: Classes\.foo, Applications\x.exe, Directory\Background, Prog\DefaultIcon found; Local Settings\shell\open\command and CLSID\shell\open\command not found; the full scan sees all 6.
- Mutation: -Anchor '"Local Settings",' -Replacement '"Local Settings Off",' -> [OK] mutação PEGA: '"Local Settings",' -> '"Local Settings Off",' fez o teste falhar (exit 101). (First attempt survived because the fixture did not contain a shell\command under Local Settings; fixture fixed.)
- Reported key paths use the stored casing (open_ci enumerates to find it); full and targeted agree.
