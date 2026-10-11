# Task 44 report

Status: DONE

Commits:
```
bb745e2 fix(tui): await background catalog jobs and show catalog errors
```

TDD step (a): rule + selftest before touching run.rs:
```
$ pwsh -NoProfile -File scripts/check_rules.ps1
  src\tui\run.rs:178: [spawn_blocking sem handle] tokio::task::spawn_blocking(move || {
  src\tui\run.rs:235: [spawn_blocking sem handle] tokio::task::spawn_blocking(move || {
exit 1
$ pwsh -NoProfile -File scripts/check_rules_selftest.ps1
[OK] check_rules: 9 casos plantados
exit 0
```
(178 = spawn_boot, 235 = spawn_reload.) After the TUI fix: `check_rules.ps1` exit 0.
New tests failed first by compile error (no `Msg::Booted`, `Msg::CatalogFailed`, `jobs::read_catalog`), then passed.

Gate:
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 2s
[OK]   clippy 92s
[OK]   test (436 passed, 16 ignored) 21s
[OK]   gate verde
```
(Two earlier runs failed only in clippy on Task 45's in-progress `tests/spans/mod.rs` / `tests/update.rs` / `tests/install_ops.rs`; re-ran after they settled.)

Mutation proofs (driver script with the exact anchors: scratchpad `mut44.ps1`; each ran `pwsh -File scripts/mutate.ps1 -Path <p> -Anchor <a> -Replacement <r> -TestCommand <t>`):
1. app.rs Booted `Err` branch -> reset index to empty, no status; `-TestCommand 'cargo test --quiet --test tui_state boot_'`
   `[OK] mutação PEGA: '                Err(e) => {` (exit 0)
2. app.rs refresh condition -> only `needs_refresh` (both `empty || needs_refresh` and the Err `true`); same test cmd
   `[OK] mutação PEGA: '                    empty || needs_refresh` (exit 0)
3. app.rs `CatalogFailed` also sets `app.search = SearchIndex::new(Vec::new())`; `-TestCommand 'cargo test --quiet --test tui_state a_failed_reload'`
   `[OK] mutação PEGA: '        Msg::CatalogFailed(e) => {` (exit 0)
4. jobs.rs `.map_err(|e| e.to_string())` -> `.or_else(|_| Ok(Vec::new()))` (anchor includes the preceding `.and_then(..)` line for uniqueness); `-TestCommand 'cargo test --quiet --test tui_jobs read_catalog'`
   `[OK] mutação PEGA: '        .and_then(|c| c.load_entries())` (exit 0)
5. check_rules.ps1 `-not $bound -and $i -gt 0` -> `-not $bound`; `-TestCommand 'pwsh -NoProfile -File scripts/check_rules_selftest.ps1'`
   `[OK] mutação PEGA: '-not $bound -and $i -gt 0' -> '-not $bound' fez o teste falhar (exit 1).`
6. check_rules.ps1 `[A-Za-z]\w*\s*(:[^=]+)?=.*spawn_blocking` -> `\w+\s*(:[^=]+)?=.*spawn_blocking`; selftest
   `[OK] mutação PEGA: '[A-Za-z]\w*\s*(:[^=]+)?=.*spawn_blocking' -> '\w+\s*(:[^=]+)?=.*spawn_blocking' fez o teste falhar (exit 1).`
7. check_rules.ps1 next-line `.await` block deleted; selftest
   `[OK] mutação PEGA: '            if (-not $bound -and $i + 1 -lt $lines.Count) {` (exit 0)
8. run.rs: `tokio::task::spawn_blocking(move || { let _ = 1; });` inserted unbound at the top of `spawn_reload`; `-TestCommand 'pwsh -NoProfile -File scripts/check_rules.ps1'`
   `[OK] mutação PEGA: 'fn spawn_reload(ctx: &Arc<Ctx>) {` (exit 0)
9. No automated proof (as the brief says). Grep: `grep -rn "pânico interno" src` finds nothing; `spawn_scan` is `spawn_job(ctx, |c, _| { .. })`.
(For multi-line anchors mutate.ps1 prints the anchor across lines, so only the first line of the `[OK]` message is shown above.)

Decisions:
- `Msg::InstalledFailed` doc comment changed to say a worker panic is re-raised instead (it used to say "error or worker panic").
- Scan's correlation-failure `tracing::warn!` kept. The old `tracing::error!` on JoinError went away because join_job handles that path now.
- If the runtime cancels a boot/reload/scan job, `join_job` turns it into `Msg::Planned(Err("tarefa cancelada"))`, which is the existing behaviour. Cancellation only happens at runtime shutdown.
- The first redraw after boot now comes from the `Booted` message itself (not a tick), so the synthetic `Msg::Resize` after the oneshot is no longer needed.

Concerns:
- Out of scope, as the brief says: `spawn_refresh` and `spawn_details` still drop their `tokio::spawn` handles. A panic in `run_update` would leave `app.refreshing` stuck.
- Reviewer focus (a): a scan panic now ends the TUI. `run` wraps `block_on` in `catch_unwind` and calls `ratatui::restore()` before `resume_unwind`, so the terminal gets restored. Not exercised live (no harness).
- Reviewer focus (b): the first catalog draw now waits for `index_age` and `startup_needs_refresh` as well. Not measured.
- Line endings: my Python edit wrote CRLF into `src/tui/jobs.rs` and `tests/tui_jobs.rs`. I normalized both back to LF before committing, so the diffs are additions only.
