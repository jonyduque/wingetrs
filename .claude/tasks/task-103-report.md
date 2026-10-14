Status: DONE_WITH_CONCERNS

Commits:
```
9de2c51 feat(tui): placeholder icon when a package has none
0237d1a docs(armadilhas): parallel agents must not share any file, tests included
2bd24c3 fix: undo drops legacy ARP choice, wingetrs record always leads a group
```
(Only 9de2c51 is mine. 2bd24c3 belongs to the task-102 fix agent, but it also picked up my tests 4 and 5 from the shared `tests/tui_view.rs`. See Concerns.)

Gate (run on the working tree before the commit; it held exactly the content now in HEAD):
```
[OK]   fmt 2s
[OK]   clippy 3s
[OK]   test (797 passed, 19 ignored) 82s
[OK]   gate verde
```

Mutation proofs:
1. Retry treated as Absent (state):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'IconOutcome::Retry => {}' -Replacement 'IconOutcome::Retry => { app.icon_tried.insert(key.clone()); app.icon_absent.insert(key); }' -TestCommand 'cargo test --quiet --test tui_state outcome_retry_is_asked_again'`
   `[OK] mutação PEGA: 'IconOutcome::Retry => {}' -> 'IconOutcome::Retry => { app.icon_tried.insert(key.clone()); app.icon_absent.insert(key); }' fez o teste falhar (exit 101).`
   Retry treated as Absent (classification):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/run.rs -Anchor '_ if e.is_offline() => IconOutcome::Retry,' -Replacement '_ if e.is_offline() => IconOutcome::Absent,' -TestCommand 'cargo test --quiet --test tui_jobs offline_is_retry_not_absent'`
   `[OK] mutação PEGA: '_ if e.is_offline() => IconOutcome::Retry,' -> '_ if e.is_offline() => IconOutcome::Absent,' fez o teste falhar (exit 101).`
2. The drawing ignores `icon_absent`:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'self.icon_absent.contains(key) || self.icon_none.contains(key)' -Replacement 'self.icon_none.contains(key)' -TestCommand 'cargo test --quiet --test tui_view placeholder_drawn_only_when_absent'`
   `[OK] mutação PEGA: 'self.icon_absent.contains(key) || self.icon_none.contains(key)' -> 'self.icon_none.contains(key)' fez o teste falhar (exit 101).`
3. Placeholder also for a key with no result yet:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '} => self.icon_known_absent(&format!("{source}/{package_id}")),' -Replacement '} => { let _ = (source, package_id); true }' -TestCommand 'cargo test --quiet --test tui_view placeholder_drawn_only_when_absent'`
   `[OK] mutação PEGA: '} => self.icon_known_absent(&format!("{source}/{package_id}")),' -> '} => { let _ = (source, package_id); true }' fez o teste falhar (exit 101).`
4. The line that decides `Retry` for a network error on the Scoop twin (`src/tui/run.rs:1120`, in `twin_icon_from`):
   ```rust
   if details_outcome(&e) == IconOutcome::Retry {
       retry.store(true, Ordering::SeqCst);
   }
   ```
   followed by `None if retry.load(Ordering::SeqCst) => Err(IconOutcome::Retry),`. Proof:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/run.rs -Anchor 'if details_outcome(&e) == IconOutcome::Retry {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test tui_jobs offline_is_retry_not_absent'`
   `[OK] mutação PEGA: 'if details_outcome(&e) == IconOutcome::Retry {' -> 'if false {' fez o teste falhar (exit 101).`
   Extra proof (not requested), for the details placeholder:
   `[OK] mutação PEGA: 'None if icon_visible(app) && app.selected_placeholder() => Some(None),' -> '' fez o teste falhar (exit 101).`

Decisions:
- `twin_from` is unchanged. `twin_icon_from` in run.rs wraps it and records "a candidate failed in a way that may pass" in an `AtomicBool`; a missing twin becomes `Retry`. If a twin is found even though another candidate failed, the result is `Found`.
- `details_outcome` walks `anyhow::Error::chain()` and downcasts to `HttpError`, and also to `WingetError::Http`. It needs both because `#[error(transparent)]` hides the inner `HttpError` from `source()`. 5xx and `IconError::Cache` give `Retry`; a cancelled twin-candidate read also gives `Retry`, and a failed one gives `Absent`.
- The details path reuses `Msg::PrefetchDone { key, outcome }`. It is sent after the icon attempt, and also when the details fail. The brief only names the "no IconRef" and "icon Absent" cases; I included the details failure too so the details path and the list path follow the same rule.
- Test 3 checks the public helpers `fetch_icon`, `own_icon` and `twin_icon_from` against a closed port and wiremock (404), not `prefetch_one`, which needs the private `Ctx` and the hard-coded CDN. "Manifest without Icons" uses a `PackageDetails` with `icon: None`.
- The view tests use `GlyphSet::Unicode` (`▢` appears nowhere else on screen) and look only at the list pane, so the details pane on the same line does not affect the result.

Concerns:
- Files outside **Files**: `tests/scoop_winget_match.rs`. Its source-text test `details_are_sent_before_the_twin_search` looked for `icon_ref(` in `spawn_details`, which now calls `details_icon(` (and that calls `icon_ref`). I changed the anchor to `details_icon(` and added an assertion that `details_icon` contains `icon_ref(`. The order check is otherwise the same.
- Commit 2bd24c3 (the other agent's) includes my tests 4 and 5 in `tests/tui_view.rs`, so 2bd24c3 does not compile on its own (it uses `IconOutcome` and `PrefetchDone { outcome }`). 9de2c51 fixes the build. I did not rewrite history, since reset/rebase are forbidden.
- Test 3 takes about 4 s because Windows takes that long to refuse connections on a closed localhost port.

## Review fixes (6a7b421)

Commit: `6a7b421 fix(tui): cover 5xx and cache icon retries, unreadable twin candidates retry`

Gate:
```
[OK]   fmt 2s
[OK]   clippy 15s
[OK]   test (797 passed, 19 ignored) 280s
[OK]   gate verde
```

1. `offline_is_retry_not_absent` now covers an icon URL answering 503 (expects Retry) and a twin candidate whose CDN answers 503 to every path (expects Retry).
   `[OK] mutação PEGA: 'HttpError::Status(s) if *s >= 500 => IconOutcome::Retry,' -> 'HttpError::Status(s) if *s >= 500 => IconOutcome::Absent,' fez o teste falhar (exit 101).`
2. Cache case: the icon downloads and its hash checks out, but the blob store directory is actually a file, so the write fails and the result must be Retry.
   `[OK] mutação PEGA: 'IconError::Cache(_) => IconOutcome::Retry,' -> 'IconError::Cache(_) => IconOutcome::Absent,' fez o teste falhar (exit 101).`
3. `twin_icon`: when the winget candidates cannot be read (for example while an update replaces the catalog), the result is now `Retry` instead of `Absent`. No test covers this and no mutation proof was run.
4. `details_are_sent_before_the_twin_search`: if `icon_ref(` appears directly in `spawn_details`, it must come after `ctx.tx.send(msg)`.
