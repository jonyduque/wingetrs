Status: DONE

Commits:
55e429a feat(tui): mark several items with space and act on all of them

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 48s
[OK]   test (757 passed, 18 ignored) 215s
[OK]   gate verde

Mutation proofs:
1. batch includes items `upgrade_target` refuses (test upgrade_marked_plans_batch_and_one_confirm):
   [OK] mutação PEGA: 'Err(_) => skipped.push(r.name.clone()),' -> 'Err(_) => items.push((r.name.clone(), Cmd::PlanInstall { source_name: String::new(), id: r.name.clone(), keep })),' fez o teste falhar (exit 101).
2. mark by index instead of identity (view_keys keyed by position; test marks_survive_rescan_and_sort_by_identity):
   [OK] mutação PEGA: '.map(|r| (r.origin.clone(), r.name.clone()))  ...Tab::Settings' (view_keys Installed/Updates -> index keys) fez o teste falhar (exit 101).
3. act on hidden/unmarked rows (test hidden_marks_are_not_acted_on):
   [OK] mutação PEGA: '.filter(|(_, k)| marks.contains(*k))' -> '.filter(|(_, k)| marks.contains(*k) || true)' fez o teste falhar (exit 101).
   [OK] mutação PEGA: 'let hidden = marks.len() - rows.len();' -> 'let hidden = 0usize;' fez o teste falhar (exit 101).

Decisions:
- Hidden marked rows cannot be planned (not in the view), so the proof is on the "marcados fora da vista" toast and on only visible marked rows being acted on.
- `Modal::Confirm` gained `then: Vec<Action>` and `clear_marks: Option<Tab>` (batch of removals = one confirm, several queue entries); `Modal::Actions` gained `marked: bool` (menu acts on marks: "Atualizar/Remover marcados (N)").
- Skipped/hidden/ambiguous/error toasts are emitted when `BatchPlanned` arrives (or at key time if nothing is plannable); marks are cleared when the actions are queued, not when the confirm is cancelled.
- The "N marcados" count is a prefix of the help bar (hints stay `&'static str`); a mark column is reserved on every row of a tab that has marks, so names stay aligned. Help dialog got a "Espaço" line (1 row taller).
- Marks pruned on InstalledLoaded/CatalogLoaded/Booted (Updates: only rows still upgradable).

Concerns: none (extra tests added: marked_winget_hits_are_skipped_and_named, ambiguous_and_failed_batch_items_are_reported, space_toggles_mark_in_state). Space no longer types a space in the search query (per brief).
