Status: DONE_WITH_CONCERNS

Commits:
```
2136544 feat(install): real environment and shortcut effects for Windows
```
(`git log e4ed737..HEAD` also lists e74e8cd, 9e037ef, 8c62970: commits of the parallel agents, not mine.)

Gate:
In the shared checkout the gate failed only on other agents' in-progress files (`fmt`: `src/install/shim.rs` unparsable, `registry_win.rs:61`, `tests/install_win_scan.rs:135/143`). So I ran it in a temporary worktree at the base e4ed737 plus only my 3 files (worktree removed afterwards). Run 1/2: `tests/update.rs::no_wait_returns_busy_immediately` failed on timing (192 ms > 100 ms under load; alone it passes 3/3, 0.03–0.20 s). Run 3:
```
[OK]   rules 7s
[OK]   fmt 4s
[OK]   clippy 13s
[OK]   test (272 passed, 7 ignored) 67s
[OK]   gate verde
```
Live tests (`cargo test --quiet --release --test install_win_effects -- --ignored`):
```
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.25s
```

Mutation proofs:
1. Type preserved (always REG_SZ):
`pwsh -File scripts/mutate.ps1 -Path src/install/env_win.rs -Anchor 'key.set_raw_value(name, &raw)' -Replacement 'key.set_raw_value(name, &RegValue { bytes: raw.bytes.clone(), vtype: RegType::REG_SZ })' -TestCommand 'cargo test --quiet --release --test install_win_effects -- --ignored'`
`[OK] mutação PEGA: 'key.set_raw_value(name, &raw)' -> 'key.set_raw_value(name, &RegValue { bytes: raw.bytes.clone(), vtype: RegType::REG_SZ })' fez o teste falhar (exit 101).`
2. No expansion (read expands every %VAR%), same TestCommand, run through a .ps1 because of the quoting:
`[OK] mutação PEGA: 'value: decode(&raw.bytes),' -> 'value: { let s = decode(&raw.bytes); if expand { s.split('%').enumerate().map(|(i, p)| if i % 2 == 1 { std::env::var(p).unwrap_or_default() } else { p.to_string() }).collect::<String>() } else { s } },' fez o teste falhar (exit 101).`
(failed: expand_sz_is_preserved_verbatim, new_values_use_requested_type_and_remove_deletes)
3. Broadcast off in tests. By inspection: `broadcast()` returns early when `self.broadcast` is false, and `new()` sets it from `should_broadcast(WINGETRS_TEST_ENV_KEY)`. `for_test_key` is always false. Pure test:
`pwsh -File scripts/mutate.ps1 -Path src/install/env_win.rs -Anchor 'test_key.is_none()' -Replacement 'true' -TestCommand 'cargo test --quiet --test install_win_effects broadcast'`
`[OK] mutação PEGA: 'test_key.is_none()' -> 'true' fez o teste falhar (exit 101).`

Decisions:
- `set`: the type of the existing value wins (REG_EXPAND_SZ/REG_SZ). If the value is new, `expand` decides, and `Path` is always REG_EXPAND_SZ. Reason: a REG_EXPAND_SZ Path must never degrade.
- `key_path(Option<&str>)` and `should_broadcast(Option<&str>)` are pure associated fns. I also added `RegEnvStore::for_test_key(k)` (never broadcasts), so my extra live tests avoid racing on the process-wide env var with the literal test.
- `ComShortcuts::new() -> Result` (SHGetKnownFolderPath can fail). `with_root(p)` is for tests. `pub fn link_target(link)` reads the target back via IShellLinkW::GetPath, for the live test.
- create/remove refuse a `link` that is not strictly under the root or that has `..`/`.` components. remove is idempotent and prunes empty folders up to the root. It removes the root only when its name is `wingetrs Apps`, so the test root (tempdir) is kept. create also sets the working directory to the target's parent folder.
- An env value of non-string type (e.g. DWORD) returns EffectError on `get`.

Concerns:
- Registry incident: in a manual cleanup of the leftover keys from my mutants (`env-<pid>-10/remove/pathdefault`), a PowerShell escaping error left `$n` empty. `Remove-Item -Recurse` then deleted all of `HKCU\Software\wingetrs-test`. A listing taken a second earlier showed only my 8 keys there. If a parallel agent had a live test running at that moment, it may have failed spuriously. Nothing outside `wingetrs-test` was touched.
- `tests/update.rs::no_wait_returns_busy_immediately` (not mine) is sensitive to machine load (100 ms limit).
- `leftovers.ps1` lists 3 bash.exe loops ("gate verde") started at 12:19 that do not look like mine (my commands all finished). I did not kill them.

---

## Review round 1 (CHANGES_REQUESTED) → ad2c3b0

Commits:
```
ad2c3b0 fix(install): empty test env key never reaches real Environment; pin shortcut root climb
```

Gate (shared checkout, after the fix):
```
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 11s
[OK]   test (305 passed, 15 ignored) 73s
[OK]   gate verde
```
Live: `cargo test --quiet --release --test install_win_effects -- --ignored`, result `test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.25s`. After all runs, `HKCU\Software\wingetrs-test` has no leftover subkey (read-only listing; no manual deletion).

Changes:
1. `key_path`/`should_broadcast` now take the raw `Option<OsString>` (`new()` uses `var_os`). Present but empty gives `EMPTY_TEST_KEY = Software\wingetrs-test\env-empty` and no broadcast. I chose a fixed key over an error so that `new()` stays infallible, and it still never reaches the real key. The same hole existed in `WINGETRS_TEST_SHORTCUT_DIR`, so `ComShortcuts::test_root(Option<OsString>)` now makes "empty" an error: there is no safe default directory.
2. New live test `empty_wingetrs_apps_root_is_removed_but_never_its_parent` uses `with_root(tmp.join("wingetrs Apps"))`.
3. `set` on an existing non-text value (DWORD) returns an error and leaves the value untouched. A read error other than NotFound is no longer swallowed. Live test `non_text_value_is_refused_by_get_and_set`.
4. Outside the literal brief test, tests use `for_test_key`/`with_root` (no `set_var`). The live tests' keys are deleted by a guard (`OwnKey`, `Drop`) that only touches the test's own subkey, even on panic.

Mutation proofs (run via `scratchpad/muts.ps1`, pure = `cargo test --quiet --test install_win_effects`, live = `cargo test --quiet --release --test install_win_effects -- --ignored`):
- M1 (pure) `[OK] mutação PEGA: 'Some(k) if k.is_empty() => EMPTY_TEST_KEY.to_string(),' -> 'Some(k) if k.is_empty() => USER_ENV_KEY.to_string(),' fez o teste falhar (exit 101).`
- M2 (pure) `[OK] mutação PEGA: 'test_key.is_none()' -> 'test_key.is_none_or(|k| k.is_empty())' fez o teste falhar (exit 101).`
- M3 (pure) `[OK] mutação PEGA: 'Some(v) if v.is_empty() => Err(EffectError(format!("{TEST_DIR_VAR} está vazia"))),' -> 'Some(v) if v.is_empty() => Ok(None),' fez o teste falhar (exit 101).`
- M4 (live) `[OK] mutação PEGA: '|| is_root {' -> '{' fez o teste falhar (exit 101).`
- M5 (live) `[OK] mutação PEGA: 'RegType::REG_SZ => RegType::REG_SZ,' -> 'RegType::REG_SZ => RegType::REG_SZ, _ => RegType::REG_SZ,' fez o teste falhar (exit 101).`

Concerns: none new.
