Status: DONE_WITH_CONCERNS

Commits:
6e350a2 feat(log): spans per stage with durations in the file log  (SHA 6e350a2; other commits in range, if any, belong to Task 44)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 3s
[OK]   test (436 passed, 16 ignored) 118s
[OK]   gate verde

Mutation proofs (all "[OK] mutação PEGA"):
1. with_span_events removed in file_layer: PEGA (cargo test --lib file_layer_logs).
2. install.execute per-batch (span only for first plan, Span::none afterwards, count 1 instead of 2): PEGA (install_emits_one).
3. debug_span!("install.step" -> trace_span!: PEGA (install_emits_one, filter assertion).
4a. update.parse removed from fetch_bucket: PEGA (update_emits_a_fetch). 4b. update.parse wrapping fetch_one: PEGA.
5. uninstall.residue_scan replaced by Span::none(): PEGA (uninstall_emits).
6. stderr_span_events verbose > 0: PEGA (stderr_shows_span_durations_only_with_vv).
7. via fixed to "idx": PEGA (early_search_reports_which_path_answered).
Item 8: no mutation (reviewer checks diff).

Sample log line (real, -vv list, debug build):
2026-10-11T15:26:31.118837Z  INFO step{step="winget_tracking"}: wingetrs::installed: close time.busy=789µs time.idle=226µs
(install.prefetch line not captured manually; format is identical, test 5 asserts "close" + "time.busy".)

Decisions:
- tests/spans/mod.rs deviates from the literal: the per-test set_default made spans vanish at random when other tests of the binary ran in parallel (callsite interest race; reproduced 6/6 failures, fixed with rebuild_interest_cache NOT sufficient). It now installs one process-global subscriber (Once) with a Router layer that records closed spans into a thread-local Closed set by install(); install() returns a guard (type Installed). Test code unchanged. Added type alias Row (clippy type_complexity).
- Sync spans use entered()/in_scope; async use Instrument. cleanup RemoveOldVersion uses in_scope.

Concerns:
- Test 5 mod deviation above (reviewer should look at it). Test 1 depends on current_thread (thread-local routing) and fails, not passes silently, if the flavor changes.

## Review fix (1297352 test(log): span tests require entered spans and their parents)
- tests/spans/mod.rs now records only spans entered at least once (on_enter) and the parent span name; helper Closed::parents(name). Asserts added: install.step parent is install.execute (15 steps), update.parse parent is update.fetch (x2). prefetch/revalidate have no parent span in apply, so only the entered check covers them.
- Surviving mutants now caught: [OK] mutação PEGA for install.prefetch as unentered span (test install_emits_one) and for update.rs .instrument(span) -> .instrument(tracing::Span::none()) (cargo test --quiet --test update update_emits).
- Mutations 1-7 re-run, all [OK] mutação PEGA. Gate: [OK]   test (436 passed, 16 ignored) / [OK]   gate verde.
