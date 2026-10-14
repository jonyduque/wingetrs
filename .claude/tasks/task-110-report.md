Status: DONE

Commits:
```
0e2d8b9 feat(tui): confirmation lists packages with icons, versions and manager; scrollable with buttons
```

Gate:
```
[OK]   rules 4s
[OK]   rules-selftest 12s
[OK]   fmt 3s
[OK]   clippy 42s
[OK]   test (839 passed, 22 ignored) 300s
[OK]   gate verde
```

Mutation proofs:
1. Continuar ignores `checked` (test 4):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'for action in checked_actions(&items, actions) {' -Replacement 'for action in actions {' -TestCommand 'cargo test --quiet --test tui_state unchecking_drops_the_item'`
   `[OK] mutação PEGA: 'for action in checked_actions(&items, actions) {' -> 'for action in actions {' fez o teste falhar (exit 101).`
2. No `…` cut (test 2):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/view.rs -Anchor 'let text = clip(text, usize::from(rect.width));' -Replacement 'let text = text.to_string();' -TestCommand 'cargo test --quiet --test tui_view confirm_text_never_overflows'`
   `[OK] mutação PEGA: 'let text = clip(text, usize::from(rect.width));' -> 'let text = text.to_string();' fez o teste falhar (exit 101).`
3. Scroll does not follow the cursor (test 3):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '*scroll = crate::tui::view::list_window(*cursor, *scroll, items.len(), rows).start;' -Replacement 'let _ = (rows, &scroll);' -TestCommand 'cargo test --quiet --test tui_view confirm_scrolls_with_many_items'`
   `[OK] mutação PEGA: '*scroll = crate::tui::view::list_window(*cursor, *scroll, items.len(), rows).start;' -> 'let _ = (rows, &scroll);' fez o teste falhar (exit 101).`

Bench (ARMADILHAS, drawing code): `bench_confirm_with_many_items` (#[ignore], tests/tui_view.rs), release, 300 items at 100x30: `frame with a 300-item confirmation: 1.67556ms` (whole frame, TestBackend). Items are built once when the dialog opens (`confirm_items`, scanning `app.installed`, never `installed_view`); nothing per frame touches the installed view.

Decisions:
- Quit is a separate `Modal::ConfirmQuit { text }`; `Modal::Confirm` holds `items, notes, scroll, cursor, actions: Vec<Action>, clear_marks` (`action`/`then` merged into `actions`, aligned with items: one item per non-empty `Apply` group — its last plan — then one per Remove/Bridge).
- Left-out count travels in `Batch.left_out`/`confirm_batch(.., left_out)`: note in the confirmation when one opens, toast otherwise (confirm off, nothing to act on, or planner busy). Non-batch jobs (pin/choose/undo) still toast.
- Notes: bridge warnings (deduped) + new purge warning for removals with purge ("Os dados persistentes de X serão APAGADOS (purge).") since the plan text that said it is no longer shown; notes are word-wrapped and yield rows to at least one item.
- Geometry in `regions::confirm_geom` (dialog, notes, list, buttons) used by view, mouse and `Regions.{confirm, confirm_list, confirm_buttons}`; `confirm_window` clamps scroll the same way for drawing and clicks. Check boxes `✓`/`☐` (ascii `[x]`/`[ ]`) hardcoded in view (theme.rs not in Files).
- Wheel scrolls 3 inside the dialog and clamps the cursor into the visible window.

Concerns:
- Existing tests changed because the brief changes the behaviour: command-line assertions in `bridge_upgrade_from_menu`/`marked_batch_mixes_ours_and_bridge` now check `name manager` (test 1 forbids commands); "itens ficaram de fora" toast assertions in `upgrade_marked_plans_batch_and_one_confirm`, `marked_winget_hits_are_skipped_and_named` (plus a confirm-off toast case), `installed_buttons_apply_to_possible_subset` now read the confirmation notes; quit tests read `ConfirmQuit`; `confirm_open` helper accepts both variants; `confirm_dialog_has_inner_padding` and `confirm_modal_snapshot` updated to the new layout.
- `Planned::Ready.text` is no longer shown by the TUI (still produced by jobs.rs); the per-step plan detail (downloads, artifacts) is gone from the dialog by design of the brief.
- `run.rs` was not touched (icons are returned by `draw_modal` and drawn last by `view`).

## Review round 1 (f765885)

Commits:
```
f765885 fix(tui): note unchecked dependencies, match bridge rows by tool and target, cover group unchecking
0e2d8b9 feat(tui): confirmation lists packages with icons, versions and manager; scrollable with buttons
```

Gate:
```
[OK]   rules 4s
[OK]   rules-selftest 13s
[OK]   fmt 2s
[OK]   clippy 22s
[OK]   test (842 passed, 22 ignored) 304s
[OK]   gate verde
```

Fixes:
1. New test `unchecking_an_apply_group` (alpha, beta, Git.Git; uncheck beta ⇒ `Execute(Apply([[alpha]]))`, Bridge queued).
2. `refresh_kept_deps` (on every toggle, key or click) adds "<dep> será instalado mesmo assim: é dependência de <owner>" when an unchecked group target is a non-target plan of a checked group; test `unchecked_dependency_is_noted`.
3. Bridge row lookup: scoop ⇒ origin `scoop`; winget ⇒ origin arp/msix with matched package id == `job.target`; test `bridge_item_takes_versions_from_its_own_row`.
4. `confirm_scrolls_with_many_items` clicks a row after the wheel (scroll 3) ⇒ only item 4 unchecked.
5. `confirm_text_never_overflows` adds a 100-char note; its line ends in `…` (item count assertion relaxed to ≥1: two note lines + gap leave one item at 60x15).

Mutation proofs:
- `[OK] mutação PEGA: '.filter(|g| !g.is_empty() && checked.next().unwrap_or(false))' -> '.filter(|g| !g.is_empty() && checked.next().map(|_| true).unwrap_or(false))' fez o teste falhar (exit 101).` (unchecking_an_apply_group)
- `[OK] mutação PEGA: 'notes.push(format!("{}{KEPT_DEP_NOTE}{}", target.app, owner.app));' -> 'let _ = (target, owner);' fez o teste falhar (exit 101).` (unchecked_dependency_is_noted)
- `[OK] mutação PEGA: 'notes.retain(|n| !n.contains(KEPT_DEP_NOTE));' -> '' fez o teste falhar (exit 101).` (unchecked_dependency_is_noted)
- `[OK] mutação PEGA: 'Tool::Scoop => r.origin == "scoop",' -> 'Tool::Scoop => r.origin != "wingetrs",' fez o teste falhar (exit 101).` (bridge_item_takes_versions_from_its_own_row)
- `[OK] mutação PEGA: 'let index = shown.start +' -> 'let index = 0 +' fez o teste falhar (exit 101).` (confirm_scrolls_with_many_items)
