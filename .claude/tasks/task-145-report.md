Status: DONE_WITH_CONCERNS

Commits:
2ac7269 feat(install): native psmodule (junction + PSModulePath)
1fce021 fix(install): review of task 144   (another agent's commit swept my src/install/state.rs hunks: ArtifactKind::PsModule + apps_with_artifact)

Gate: verify.ps1 stopped at clippy, only in another agent's file:
[OK]   rules / rules-selftest / fmt
[FAIL] clippy: tests\it\wi_bridge_install.rs:410:37 unnecessary use of `clone` (Task 176, not mine)
Proof of mine: `cargo clippy --all-targets -- -D warnings` filtered by install/ and install_psmodule/install_planner => no lines.
`cargo test --quiet --test it install_`: 355 passed, 1 failed (wi_bridge_install::serve_runs_install_only_with_valid_target, 2d). install_psmodule:: 12/12 ok. `--lib` 62 ok.

Mutation proofs (mutate.ps1 on src/install/executor.rs, test = `cargo test --quiet --test it install_psmodule::`):
[OK] mutação PEGA: 'if others.is_empty() {' -> 'if true {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'r"Documents\WindowsPowerShell\Modules"' -> 'r"Documents\Modules"' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'format!("{};{}", modules.display(), base)' -> 'format!("{}", modules.display())' fez o teste falhar (exit 101).
Security (coordinator request): removing check_component + parent check in the executor install path (AnchorFile, multi-line): output began "[OK] mutação PEGA: '    check_component("módulo", name)?;" (line cut by my grep; the full line was not captured).

Decisions:
- Extra safety (coordinator): planner refuses unsafe psmodule name (PlanError::UnsafePath); executor install and removal run check_component + `parent == modules`. Tests: unsafe_module_names_refused_by_the_planner (hand-mutated Resolved) and ..._by_the_executor (hand-built plan), names `..\x`, `a/b`, `C:x`, `CON`, `x.`, "".
- Upgrade reuses Undo::RestoreCurrent to restore the previous junction. Extra tests: foreign_folder_is_not_replaced, upgrade_recreates_the_junction.
- Removal refuses a non-junction at <modules>\<name> instead of deleting it.
- Edited tests/it/install_planner.rs (one match arm for the new Step) - outside Files list, needed to compile.

Concerns:
- ops.rs batch env snapshot (`env_names`) covers Path and EnvSet names only, not PSModulePath: a multi-app batch failure restore would not roll back PSModulePath of already-applied plans. Not touched (outside Files, shared with 2d); suggest adding it for plans with Step::PsModule.
- state.rs was overwritten twice by a concurrent editor during the work; the final tree has my hunks (commit 1fce021).
