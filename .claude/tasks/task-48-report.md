Status: DONE

Commits:
(see below, filled by git log)

Gate:
[OK]   rules 4s
[OK]   rules-selftest 12s
[OK]   fmt 1s
[OK]   clippy 15s
[OK]   test (485 passed, 16 ignored) 129s
[OK]   gate verde

Mutation proofs (all with `cargo test --quiet --test install_state --test tui_jobs`):
[OK] mutação PEGA: '"Manter entrada no PATH: {} (ainda' -> '"Desfazer entrada no PATH: {} (ainda' fez o teste falhar (exit 101).
[OK] mutação PEGA: '.filter(|a| a.kind == ArtifactKind::Path)' -> '.filter(|_| true)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'users.join(", ")' -> 'users[0].clone()' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'plan.artifact_lines()' (cli/output.rs) -> map com Debug de kind fez o teste falhar (exit 101).
[OK] mutação PEGA: 'plan.artifact_lines()' (tui/jobs.rs) -> map com "{:?} {}" fez o teste falhar (exit 101).

Decisions:
- Check 2 of the brief: with the 5 brief tests, `.filter(|_| true)` SURVIVED (a Shim "rg" never matches another app's Path, so path_users stays empty). Added test uninstall_plan_only_path_artifacts_consult_other_users (Shim target equal to another app's Path entry) which kills it.
- Helper shared_path_db in tests/install_state.rs carries #[allow(clippy::unwrap_used)] (non-test fn, the file has no crate-level allow).
- plan.rs has CRLF line endings; preserved.

Concerns: src/tui/app.rs and src/tui/run.rs showed uncommitted modifications during the task (the parallel reviewer's mutations); not mine, not committed.
3446205 fix(uninstall): plan text says when a shared PATH entry is kept
469266e fix(tui): drain input with the task's waker; await the refresh task

Follow-up (review minors): test uninstall_plan_purge_line_is_pinned + coupling comment on remove_path_entry. Gate verde (486 passed). [OK] mutação PEGA: format!("Apagar {} (dados -> format!("Xpagar {} (dados fez o teste falhar (exit 101).
