Status: DONE_WITH_CONCERNS

Commits:
17dc9b4 fix(winget-install): download before removing, burn reboot codes, msi queue on uninstall, version check

Gate:
`scripts/verify.ps1` (one run) stopped at the rules step because of a file that belongs to another agent:
```
Carregado em 300ms
[FAIL] rules exit 1
  (log completo: target/verify.log)
```
target/verify.log: `tests\it\wi_cli.rs:852: [limite de relogio em teste] while !gate.cancelled() && t0.elapsed() < ...` (wi_cli.rs is Task 193's file, not mine).
How I checked my own files instead:
- `cargo test --quiet --test it wi_` -> `test result: ok. 269 passed; 0 failed; 4 ignored; 0 measured; 1504 filtered out`
- `cargo test --quiet --lib winget_install` -> `test result: ok. 3 passed; 0 failed`
- `cargo clippy --all-targets`, filtered to my files: no warnings.
- `rustfmt --edition 2024 --check` on the 10 files: OK.

Mutation proofs:
- (a) removal back before the fetch (engine.rs, `-AnchorFile/-ReplacementFile` that move `uninstall_queued` above `fetch_verified` and the gate), test `wi_upgrade::upgrade_previous`:
  `[OK] mutação PEGA: '            let file = fetch_verified(wctx, &fresh).await?;`
- (b) Burn outside the 3010 path, test `wi_uninstall_flow::burn_`:
  `[OK] mutação PEGA: 'matches!(c.rule, Rule::Burn | Rule::Quiet)' -> 'matches!(c.rule, Rule::Quiet)' fez o teste falhar (exit 101).`
- Extra mutations:
  `[OK] mutação PEGA: 'if uac_refused(&e) {' -> 'if false {' fez o teste falhar (exit 101).`
  `[OK] mutação PEGA: '(Version::parse(have) < Version::parse(offered))' -> '(have != offered)' fez o teste falhar (exit 101).`
  `[OK] mutação PEGA: 'let interactive = command.is_some() && (interactive || mode == Mode::Interactive);' -> 'let interactive = interactive || mode == Mode::Interactive;' fez o teste falhar (exit 101).`
  `[OK] mutação PEGA: 'let arch = if bundle {' -> 'let arch = if false {' fez o teste falhar (exit 101).`
  `[OK] mutação PEGA: 'let code = if msiexec {' -> 'let code = if false {' fez o teste falhar (exit 101).`
  `[OK] mutação PEGA: '        if self.upgrade_from.is_some()' -> '        if false' fez o teste falhar (exit 101).`

Decisions:
- `install_locked` is split into `ops::fetch_verified` and `ops::install_fetched`. The 1618 retry moved into `ops::retry_1618`, which installs and `msiexec /x` now share. `uninstall::MsiQueue` and `uninstall_queued` take `msi.lock`. Plain `uninstall()` still retries 1618, but without the lock or the mutex check (see Concerns).
- "UAC refused" is recognised only by raw error 1223. `launch_win` now returns `from_raw_os_error(1223)`. I did not use `PermissionDenied`, because that kind also covers access denied (5). The uninstaller's unelevated launch returns the `UAC_REFUSED` text.
- Version check: the warning fires when `found < offered` (by `Version`). An installed version newer than the catalog's does not warn (lesson from Task 134). An unknown version says nothing.
- The 3010/1641 reboot path with the key check applies to `Rule::Burn` and to every `Rule::Quiet`. The plan does not record whether a Quiet command belongs to a bundle, and adding a field would break struct literals in files owned by others. When the key is gone, the uninstaller did remove the app.
- Addendum: MSIX/Appx plans are never `interactive`. A Burn bundle's architecture comes from tokens in its DisplayName or cache file name (x64/amd64/x86/arm64). With no token it is unknown and the system architecture is used. Tests: `msix_plan_never_says_installer_window`, `burn_bundle_arch_is_not_the_registry_view` (DesktopRuntime 8 and 10, x64 and x86).

Concerns:
- The plain uninstall in the CLI (`src/cli/run_win.rs`, `uninstall::uninstall(&u, plan)`) and the TUI (`tui/jobs.rs`/`run.rs`) does not take `msi.lock`, because those files are not mine. To queue it, each caller needs one line: `uninstall::uninstall_queued(&u, Some(&MsiQueue::of(&wctx)), plan)`. The upgrade path (`engine::upgrade`) already takes the lock.
- I did not watch each test fail before implementing: the checkout did not compile while other agents were editing. The mutations above stand in for that proof.
- No `.join` with external data in this diff.
