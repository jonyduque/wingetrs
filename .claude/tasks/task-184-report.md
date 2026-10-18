Status: DONE_WITH_CONCERNS

Commits:
9d43b9d feat(winget-install): native uninstall flow (wait for the key to go), shared-data warning and registry-only removal

Gate:
```
[OK]   rules 10s
[OK]   rules-selftest 25s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\install\executor.rs:1709:
  (log completo: target/verify.log)
```
Gate run once, as the rule says. It failed only in `src/install/executor.rs`, which Task 157 is editing in parallel (not my file). I checked my own files separately:
- `rustfmt --edition 2024 --check src/winget_install/uninstall.rs tests/it/wi_uninstall_flow.rs` → `fmt exit 0`
- `cargo clippy --all-targets --message-format short | grep uninstall.rs|wi_uninstall_flow` → no lines
- `pwsh -File scripts/check_rules.ps1` → `rules exit 0`
- `cargo test --quiet --test it wi_uninstall_flow::` → `test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 1563 filtered out; finished in 0.03s`

Mutation proofs (all `-TestCommand 'cargo test --quiet --test it wi_uninstall_flow::'`, `-Path src/winget_install/uninstall.rs`):
1. Loop stop condition inverted:
   `[OK] mutação PEGA: 'if !(ctx.key_exists)(hive, key) {' -> 'if (ctx.key_exists)(hive, key) {' fez o teste falhar (exit 101).`
2. `registry_only_allowed = is_main`:
   `[OK] mutação PEGA: 'registry_only_allowed: !is_main,' -> 'registry_only_allowed: is_main,' fez o teste falhar (exit 101).`
3. Follow reparse points during the copy (two forms):
   `[OK] mutação PEGA: '            let meta = match std::fs::symlink_metadata(&path) {' -> '            let meta = match std::fs::metadata(&path) {' fez o teste falhar (exit 101).`
   `[OK] mutação PEGA: '            if is_reparse(&meta) {' -> '            if false && is_reparse(&meta) {' fez o teste falhar (exit 101).`
   (The 2nd form first SURVIVED: `fs::copy` on the junction failed, so nothing leaked, but the note said "falha ao copiar". I made test 9 also require "pulado" in the note.)
4. Hklm32 becomes Hklm64 in the request to the helper:
   `[OK] mutação PEGA: '        Hive::MachineX86 => Some(MachineHive::Hklm32),' -> '        Hive::MachineX86 => Some(MachineHive::Hklm64),' fez o teste falhar (exit 101).`
5. (extra, Kindle / Task 182 review) drop the helper's duplicate rule:
   `[OK] mutação PEGA: '        && !elevated_duplicate(d, others)' -> '        && false' fez o teste falhar (exit 101).`

Decisions:
- `plan_uninstall` takes one more parameter, `u: &UserFolders { local_app_data, app_data, documents }`. The signature in the brief had no source for `%LOCALAPPDATA%`/`%APPDATA%`/`Documents`, and test 7 needs "APPDATA do teste". `folders` gets the expanded candidate path. Each name passes `safe_components` as exactly one unchanged component, with a `parent() == base` check. Duplicates are dropped.
- Kindle (Task 182 review): new `plan_uninstall_among(d, nsis_hint, others: &[ArpDetails], is_main, ..)`. It wraps `plan_uninstall`. For HKLM it only keeps option C when `elevated_duplicate(d, others)` holds. Otherwise it sets `SharedData::registry_only_refused` (an extra field) with the reason, which `Display` shows ("remover só o registro não está disponível: ... HKCU ..."). Option A (the uninstaller) stays planned. Test: `kindle_hklm_old_hkcu_new_offers_uninstaller_only` and others.
- `UninstallCtx` has an extra field `msiexec: &Path` (the injected `msiexec_path`). `user_expand` is a per-variable lookup used with `installed::arp::expand_vars`. A non-absolute program is refused before running (never PATH). `nsis_hint` sent to the helper = `rule == Nsis`.
- Report type: `UninstallReport { status: Removed|NoEffect, warning, reboot: Option<bool> }` plus `UninstallStatus::job_outcome()` mapping to `job_log::Outcome`. The `.reg` freshness check is duplicated (about 20 lines) from the private `orphans::fresh_backup_file`, because I could not edit `orphans.rs`.
- Reparse detection is `file_type().is_symlink()` OR `FILE_ATTRIBUTE_REPARSE_POINT` via `std::os::windows::fs::MetadataExt`, inside a `#[cfg(windows)]` fn with a `#[cfg(not(windows))]` stub. It uses std only (no `windows`/`winreg` crate), and `check_rules` passes.

Concerns:
- The full gate is not green because of `src/install/executor.rs` (Task 157, parallel). The orchestrator should run the gate again.
- TDD order: tests and implementation were written in the same pass. I did not see a failing compile/run first. The mutation proofs above show the tests can fail.
- `.join` with external data: `data_folders` (`base.join(publisher|name)`, checked with safe_components as a single component plus parent==base) and `backup_folders` (`dest.join("<n>-<name>")`, same check; `to.join(entry.file_name())` inside the copy comes from `read_dir` of a folder that is not followed through links). The `.reg` path uses `orphans::backup_file` (sanitized).
- The brief named `UninstallCtx` and `plan_uninstall` fields and parameters; I added to both (see Decisions). Callers (CLI/TUI) are not wired in this task.

## Review fix (5305624 fix(winget_install): review of task 184)

Gate (run once): rules, fmt and clippy passed. The tests step failed only on `manifest_2c::planner_refuses_each_2c_key_with_old_text` (tests/it/manifest_2c.rs, src/install area, parallel task, not mine): `test result: FAILED. 1522 passed; 1 failed; 83 ignored`. My modules: `cargo test --quiet --test it wi_` → `test result: ok. 197 passed; 0 failed; 4 ignored`. rustfmt --check and clippy are clean on my 3 files.

Changes:
1. `measure_folders` counts bytes and entries without following reparse points. `SharedData.size` is shown in `Display` ("tamanho da cópia: X em N itens"). `backup_folders(folders, dest, &BackupLimits)` now returns `Result`. It refuses before copying when a folder contains `dest`, when the total is over `MAX_BACKUP_BYTES` (2 GiB), or when free space is too small or cannot be read. The copy also stops if the tree grew past the measured size. Generic names (closed list of 12, compared without case, for both Publisher and Name) are never offered, and neither is a candidate that contains `UserFolders.wingetrs_root`, which is a new field.
2. New test: valid Publisher with Name `NUL`/`Pub.`/`a/b` ⇒ only the Publisher folders are offered.
3. `plan_uninstall(d, nsis_hint, others: &[ArpDetails], is_main, k, u, folders)`: `plan_uninstall_among` is gone, and option C always applies `elevated_duplicate`.
4. `orphans::fresh_backup_file` is now `pub(crate)` and reused; my copy is deleted.
5. Skip note: "pulado (ponto de reanálise: link, junção ou arquivo só na nuvem do OneDrive)".

Free space is injected (`BackupLimits::free_space`), and nothing in the code base reads it yet; the CLI/TUI wiring (Tasks 186/187) needs a `GetDiskFreeSpaceExW` in a `*_win.rs`. The same tasks must fill `UserFolders.wingetrs_root`.

Mutation proofs (review):
[OK] mutação PEGA: '    let name = non_blank(&d.display_name)\n        .and_then(one_component)...' -> (without `.and_then(one_component)`) fez o teste falhar (exit 101).
[OK] mutação PEGA: 'let usable = |s: &str| !is_generic(s);' -> 'let usable = |_s: &str| true;' fez o teste falhar (exit 101).
[OK] mutação PEGA: '|| contains_path(&p, &u.wingetrs_root)' -> '|| false' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if size.bytes > limits.max_bytes {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if free < size.bytes {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if let Some(f) = folders.iter().find(|f| contains_path(f, dest)) {' -> 'if let Some(f) = folders.iter().find(|_| false) {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'GENERIC_NAMES.iter().any(|g| g.eq_ignore_ascii_case(name))' -> 'GENERIC_NAMES.iter().any(|g| *g == name)' fez o teste falhar (exit 101).
Rerun of the brief's checks with the new code: loop inverted, `registry_only_allowed: is_main && !helper_refuses`, `symlink_metadata`→`metadata` in the copy, `Hklm32`→`Hklm64`, and `helper_refuses = false`: all `[OK] mutação PEGA`.
