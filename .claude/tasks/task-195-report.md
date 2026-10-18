Status: DONE_WITH_CONCERNS

Commits:
6b63144 fix(winget-install): preferred UI languages and unsupported architectures in installer selection

Gate: NOT green for reasons outside this task. `verify.ps1` stopped at rules: `tests\it\wi_cli.rs:854: [limite de relogio em teste]` (another agent's file). `cargo test --quiet --test it wi_select::` passes (all select tests, including the 4 new/changed). `wi_` run: 267 passed, 2 failed, both in other agents' in-flight work (wi_cli::cli_ctrl_c_after_launch_prints_notice_once, wi_upgrade::upgrade_same_version_after_warns). Clippy/fmt on my files: no findings. Orchestrator should run the gate.

Mutation proofs:
- Ignore UnsupportedOSArchitectures (`.any(|a| a.eq_ignore_ascii_case(&host.arch))` -> `.any(|_| false)`): `[OK] mutação PEGA ... fez o teste falhar (exit 101).`
- Reverse language list order (`for preferred in host.as_slice()` -> `.iter().rev()`): `[OK] mutação PEGA ... fez o teste falhar (exit 101).`
- Language-only level to 0 (`u8::from(false)`): failed locale_language_only_match_ranks_below_exact at wi_select.rs (assertion left==right); the OK line itself was not pasted (output was truncated by tail).

Decisions:
- `Host.locale` is now `Languages` (newtype over Vec<String>, `From<&str>`/`From<Vec<String>>`) so the literals `locale: "pt-BR".into()` in wi_cli/wi_install_flow/wi_tui/wi_upgrade keep compiling without editing others' files.
- `GetUserPreferredUILanguages` declared as a raw `unsafe extern "system"` in host_win.rs (Win32_Globalization feature not in Cargo.toml; avoided touching it). Buffer capped at 16K units. Not exercised by a test beyond the ignored live `host_current_prints`.
- Addendum: removed the Platform filter from `choose` (winget's ManifestComparator.cpp has no Platform reference; grep confirmed). Replaced `platform_filter` and `platform_refusal_and_case_insensitive_desktop` with `platform_is_not_a_filter`. Leftover refusal text for min-OS-empty case: "nenhum instalador aplicável a este Windows".
- Edited `src/sources/winget/manifest.rs` (outside Files list): added `UnsupportedOSArchitectures` (root + installer) and `EffectiveInstaller.unsupported_os_architectures`; it was not parsed before.

Concerns: Task 185 equivalence test not run separately (the `wi_` run covered everything except the 2 foreign failures). No live check of Microsoft.PowerShell.Preview --dry-run.

## Review round 1 (fixes)
Commits: second commit above (see git log; "one architecture rule for native and bridge").
- `select::arch_applicable` (runs_on + UnsupportedOSArchitectures, installer level else root) now used by `choose` and `bridge::winget_applicability` / `install_job`; `bridge::runs_on` deleted. Note: `winget_install/engine.rs:181` still uses `arch_rank` directly (not mine).
- `wi_upgrade::native_agrees_with_winget_applicability` extended with the unsupported-architecture case (installer-level and root-level); test uses the host arch.
- New test `empty_language_list_ignores_locale` (both orders).
- Gate: not rerun in full (foreign rules failure earlier); `wi_` run: 271 passed, 0 failed, 4 ignored. clippy: no findings in my files.
Mutation proofs:
[OK] mutação PEGA: 'u8::from(!lang(l).is_empty() && lang(l) == lang(host))' -> 'u8::from(false)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if host.is_empty() {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'arch_applicable(&i.architecture, unsupported.map_or(&[], Vec::as_slice), arch)' -> 'arch_applicable(&i.architecture, &[], arch)' fez o teste falhar (exit 101).

## Review round 2
- `plan_upgrade` retain now uses `select::arch_applicable` (unsupported list: installer, else root); all three paths share it.
- Equivalence case added to `native_agrees_with_winget_applicability`: unsupported host-arch installer + a `neutral` installer with UpgradeBehavior deny; native refusal text equals the deny-only refusal and the bridge says Denied.
- `wi_` run: 272 passed, 0 failed. Mutation:
[OK] mutação PEGA: 'unsupported.map_or(&[], Vec::as_slice),' -> '&[],' fez o teste falhar (exit 101).
(The first test version survived this mutation because `plan` also refuses; fixed by asserting the refusal text.)
