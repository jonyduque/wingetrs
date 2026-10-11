Status: DONE

Commits:
581e0d2 fix(manifest): accept bin entries with extra args and validate shim names
(base ae7ec20; c53d95c, 77245ab, eeb6a62 are other agents' commits)

Gate:
[OK]   fmt 3s
[OK]   clippy 3s
[OK]   test (462 passed, 16 ignored) 37s
[OK]   gate verde
(Earlier runs failed on Task 50's in-progress extract.rs, and twice on the flaky tests/index_file.rs search_retries_while_the_file_is_briefly_locked under load; it passes in isolation.)

Mutation proofs (cargo test --quiet --test install_manifest --test install_planner):
[OK] mutação PEGA: '"bin", usize::MAX)' -> '"bin", 3)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'rest.join(" ")' -> 'rest.join("")' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'rest.join(" ")' -> 'rest[0].clone()' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'Some(a) if !a.is_empty() => a.to_string(),' -> 'Some(a) => a.to_string(),' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !crate::install::shim::is_safe_shim_name(&name) {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'a.chars().any(char::is_control)' -> 'false' fez o teste falhar (exit 101).
Check 7 (oracle_2a) is the orchestrator's.

Decisions:
- Predicate is `pub(crate) fn is_safe_shim_name` in shim.rs, used by check_name and the planner.
- bin passes usize::MAX as max to tuple_items (no new parameter type).
- Red phase not run separately; the mutations prove the tests bind each rule.
- Planner checks args after substitute, so control chars coming from substituted variables are also refused.

Concerns: none

Follow-up (review fixes), commit: see `git log -1 --grep "refuse control characters"`
- bin path with control char or `"` refused in planner (UnsafePath); is_safe_shim_name now also refuses `* ? < > | "` and control chars. Tests: control_characters_and_quotes_in_bin_paths_are_refused (alias "ok" so the path check is what fires), alias list extended ("a/", ".\a", control, `|`, `*`).
- Mutations PEGA: bin-path check -> `if false`; `'|'` removal; control-char removal in shim name; `'/'` removal (unsafe_shim_alias).
- Note: the bin path check alone was first masked by the stem-based name check; test uses an explicit alias to bind it.
- Gate verde (index_file flake retried).
