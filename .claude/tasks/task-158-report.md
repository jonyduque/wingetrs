Status: DONE_WITH_CONCERNS

Commits:
3d01114 feat(install): run .ps1 installers through the script host (S-ps1)

Gate: verify.ps1 NOT green, stopped at fmt for OTHER agent's file only (Task 184, tests/it/wi_uninstall_flow.rs):
  [OK]   rules 9s
  [OK]   rules-selftest 24s
  [FAIL] fmt exit 1
    Diff in ...\tests\it\wi_uninstall_flow.rs:14 / :632 / :663
My files: rustfmt clean. `cargo clippy --all-targets -- -D warnings`: no output (clean).
`cargo test --quiet --test it install_`: 540 tests, 539 ok; the 1 failure (install_manifest::support_keys_inside_architecture_variant) was mine, fixed, now 16/16 in install_manifest.

Mutation proofs (scripts/mutate.ps1):
[OK] mutação PEGA: 'if file.to_lowercase().ends_with(".ps1") {' -> 'if false {' fez o teste falhar (exit 101).   (ps1_runs_via_host)
[OK] mutação PEGA: 'if report.verdict == script_scan::Verdict::Refuse {' -> 'if false {' ... (ps1_content_is_scanned_before_running)
[OK] mutação PEGA: '.any(|s| matches!(s, Step::Hook { .. } | Step::RunPs1Installer { .. }));' -> '.any(|s| matches!(s, Step::Hook { .. }));' ... (ps1_needs_consent)
[OK] mutação PEGA: 'if bytes.contains(&0) {' -> 'if false {' ... (ps1_oversized_or_utf16_content_is_refused)

Decisions:
- Executor generalizes run_hook with HookBody {Script, Ps1}; the .ps1 step reuses the whole Hook path (consent, workdir, BOM, effects, D5, guard).
- Scan at execution (requirement from the orchestrator): the real file is read (cap 1 MiB, NUL byte => refused as non-text/UTF-16), scanned with script_scan::scan(PRELUDE_FUNCTIONS); Refuse => step fails before anything is written. Env-seed names come from that content.
- The .ps1 runs from its own path (not copied), so $PSScriptRoot/relative files work.
- ps1_installer_report is provided but not wired into scan_manifest (would duplicate the InstallerScript key).
- Joins with external data: installer_program (safepath::safe_components + canonicalize starts_with vdir) is the only one; the Display in plan.rs uses safe_components too. Workdir names use app/kind (validated components).

Concerns:
- Files outside the Files list edited (needed): tests/it/install_planner.rs (exhaustive match: new variant), tests/it/install_file_installer.rs (test ps1_still_refused_with_old_text became ps1_plans_a_ps1_step_...), tests/it/install_manifest.rs (removed the installer a.ps1 row from the "unsup" list).
- TOCTOU between scan and run (file in the user's version folder); not mitigated.
- Gate not green only due to Task 184's wi_uninstall_flow.rs fmt.
