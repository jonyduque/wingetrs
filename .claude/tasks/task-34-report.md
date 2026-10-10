Status: DONE_WITH_CONCERNS

Commits:
e74e8cd feat(install): residue classification and recoverable cleanup
(other commits between base e4ed737 and HEAD — 8c62970, 9e037ef — belong to the parallel shim task)

Gate (run in an isolated worktree at HEAD 9e037ef + only my two files, because the main checkout
was failing fmt/clippy on other agents' in-progress files install_shim.rs / install_win_scan.rs / env_win.rs;
a first run there failed only on the timing test cli_offline `< 5s` (5.95 s under load), rerun passed):
```
[OK]   rules 3s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (285 passed, 2 ignored) 53s
[OK]   gate verde
```
`cargo test --quiet --test install_residue`: `test result: ok. 17 passed; 0 failed`.

Mutation proofs:
1. other_installs never preselects low confidence:
   `scripts/mutate.ps1 -Path src/install/residue.rs -Anchor "        confidence: Confidence::Low,\n        preselected: false," -Replacement "        confidence: Confidence::Low,\n        preselected: input.other_installs," -TestCommand 'cargo test --quiet --test install_residue other_installs'`
   `[OK] mutação PEGA: '        confidence: Confidence::Low,` (test other_installs_notes_low_items_and_never_preselects_them failed)
   Extra (note): anchor `note: input.other_installs.then(|| OTHER_INSTALL_NOTE.to_string()),` -> `note: None,`
   `[OK] mutação PEGA: '        note: input.other_installs.then(|| OTHER_INSTALL_NOTE.to_string()),' -> '        note: None,' fez o teste falhar (exit 101).`
2. Export before delete (order inverted: delete_key then export):
   anchor `ResidueKind::RegKey { key } => export(key).and_then(|()| { effects.registry.delete_key(key)... })` -> `effects.registry.delete_key(key).map_err(..).and_then(|()| export(key))`, test `clean_exports_before_deleting`
   `[OK] mutação PEGA: 'ResidueKind::RegKey { key } => export(key).and_then(|()| {`
   Export failed => key intact (ignore export error), test `clean_failed_export`:
   `[OK] mutação PEGA: 'ResidueKind::RegKey { key } => export(key).and_then(' -> 'ResidueKind::RegKey { key } => Ok::<(), String>({ let _ = export(key); }).and_then(' fez o teste falhar (exit 101).`
   `[OK] mutação PEGA: 'ResidueKind::RegValue { key, value_name } => export(key).and_then(' -> 'ResidueKind::RegValue { key, value_name } => Ok::<(), String>({ let _ = export(key); }).and_then(' fez o teste falhar (exit 101).`
3. High confidence requires removed dir in data (match by app name only), test filter `hit_`:
   `[OK] mutação PEGA: '        if mentions_dir(&h.data, &dir) {' -> '        if h.data.to_lowercase().contains(&input.app.to_lowercase()) {' fez o teste falhar (exit 101).`

Named tests per "What binds" rule: high hit -> hit_pointing_into_removed_dir_is_high_and_preselected (+ case/slash, sibling-prefix, name-only variants);
high .lnk -> shortcut_whose_target_is_in_removed_dir_is_high_and_preselected, shortcut_to_sibling_app_dir_is_ignored;
low by app name -> folder_named_like_app_is_low_and_unselected; low by GitHub owner -> github_homepage_owner_names_low_confidence_key_and_folder,
non_github_homepage_gives_no_owner_name, github_homepage_without_repo_path_gives_no_owner_name;
other_installs -> other_installs_notes_low_items_and_never_preselects_them, without_other_installs_low_items_have_no_note;
execution -> clean_sends_folders_and_shortcuts_to_trash, clean_exports_before_deleting_key_and_value, clean_default_value_uses_empty_value_name, clean_failed_export_leaves_key_intact.
Order is verified by a test-side wrapper around FakeRegistry (FakeRegistry itself does not log op order) that logs ops and whether the key still existed in the fake at export time.

Decisions:
- Path match is case-insensitive, `/`→`\`, with a boundary after the dir (end, `\`, quote, `,`, `;`, space) so `apps\ripgrep-all` does not match `apps\ripgrep`. Hits not pointing into the dir are dropped (not demoted to Low).
- Several keys in one clean: first export `<app>.reg`, next ones `<app>-2.reg`, `<app>-3.reg`… (one `reg export` per file would overwrite); each distinct key exported once, result reused.
- Timestamp `AAAAMMDD-HHMMSS` is UTC (no chrono, no Win32 in a pure module); backup dir created only if a registry item is selected; dir creation failure => registry items fail, nothing deleted.
- Default value (`value_name: None`) is deleted with name `""`. GitHub owner: `http(s)://github.com/<owner>/<repo>`, owner `[A-Za-z0-9-]+`.

Concerns:
- Gate could not go green in the shared checkout during my run (other agents' files mid-edit); verified in an isolated worktree instead.
- Two `clean` calls for the same app in the same second share the stamp dir and would overwrite `<app>.reg`; UTC stamp may surprise users expecting local time.
- Leftover bash/python processes listed by leftovers.ps1 (PIDs 26260, 31228, 7888) are not mine; not killed.

## Review round 1 (CHANGES_REQUESTED) — fix commit

Commits: `0c15e23 fix(install): residue backups never overwrite; pin whole-name match`

Changes:
1. Test `names_must_match_whole_not_by_prefix`: app `git` vs `%APPDATA%\github`, `Software\GitHub`; app `ripgrep` vs `ripgrep-all` (folder and key) ⇒ no item.
2. `reserve_backup_dir`: `create_dir_all(backups)` then atomic `create_dir(backups\<stamp>)`; `AlreadyExists` ⇒ `<stamp>-2`, `-3`… Test `two_cleans_of_same_app_never_overwrite_a_backup` (export wrapper writes the file like `reg export /y`; retries until both cleans fall in the same second; asserts distinct paths, second dir `<stamp>-2`, first file content intact; backups root missing at start).
3. Stamp is now `AAAAMMDD-HHMMSSZ` — UTC, the `Z` marks it so it is not read as local time.
4. Test `malformed_github_homepage_gives_no_owner_candidate`: `https://github.com/../x`, `https://github.com/a?b/c`, `https://github.com/`, `https://github.com`, `https://github.com//x` ⇒ no owner candidate, no panic.

Mutation proofs:
```
[OK] mutação PEGA: 'names.iter().any(|x| x.eq_ignore_ascii_case(n))' -> 'names.iter().any(|x| n.to_lowercase().starts_with(&x.to_lowercase()))' fez o teste falhar (exit 101).
[OK] mutação PEGA: '        match std::fs::create_dir(&dir) {' -> '        match std::fs::create_dir_all(&dir) {' fez o teste falhar (exit 101).
[OK] mutação PEGA: '        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}Z",' -> '        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",' fez o teste falhar (exit 101).
[OK] mutação PEGA: '        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')' -> '' fez o teste falhar (exit 101).
```
(test commands: `cargo test --quiet --test install_residue names_must_match_whole` / `two_cleans` / `clean_` / `malformed_github`)

Gate: the shared checkout failed fmt only in other agents' files (registry_win.rs, install_win_scan.rs). Ran it in an isolated worktree at 6896087 plus my two files (now removed). First run hit the 590 s timeout during the fresh build. Second run failed only the load-dependent concurrency test `index_file::rewriting_while_a_reader_loops_never_fails_or_tears`. Third run:
```
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (301 passed, 2 ignored) 60s
[OK]   gate verde
```
Concern resolved: same-second cleans no longer overwrite (previous concern 2).
