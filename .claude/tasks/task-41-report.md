Status: DONE

Commits: see `git log 2ca0e02..HEAD` (perf(ops): apply a batch of upgrades under one lock and one broadcast)

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 13s
[OK]   test (415 passed, 16 ignored) 87s
[OK]   gate verde

Mutation proofs (all "[OK] mutação PEGA"):
1. merge dedupe filter -> `.filter(|p| { seen.insert(..); true })`: PEGA (merge_groups test).
2. apply_groups calling `apply` per chunk(1): PEGA (apply_groups_broadcasts_once_for_two_env_apps).
3. run_action back to per-group loop: PEGA (run_action_applies_all_groups_with_one_broadcast).
4. revalidate target test -> `plan.app != plans[last].app`: PEGA (apply_groups_refuses_a_target_installed_meanwhile_even_in_the_middle).

Decisions: apply keeps its signature, delegating to private apply_with_targets (targets = last plan). tui_jobs World now accumulates packages (add_env) since replace_source replaced the whole source.
Concerns: cargo fmt also touched src/cli/mod.rs (Task 43's file) possibly; not staged by me.
