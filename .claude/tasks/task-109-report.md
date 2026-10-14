Status: DONE_WITH_CONCERNS

Commits:
(see git log 902e1a3..HEAD: feat(tui): action buttons under the details pane)

Gate:
[OK]   rules-selftest 11s
[OK]   fmt 2s
[OK]   clippy 3s
[OK]   test (832 passed, 20 ignored) 76s
[OK]   gate verde

Mutation proofs:
- regions button x shifted +1: [OK] mutação PEGA: '(Rect::new(area.x + x, top + row, w, 1), id)' -> '(Rect::new(area.x + x + 1, top + row, w, 1), id)' fez o teste falhar (exit 101). (buttons_match_regions, 80x24 Installed)
- apply to all marked without filtering (`.filter(|&_r| true)` in run_button): [OK] mutação PEGA ... fez o teste falhar (exit 101). (installed_buttons_apply_to_possible_subset)

Decisions:
- Keys in labels only where a key exists today: Instalar/Atualizar (enter) in Search/Updates, Remover (u) in Installed; Atualizar/Fixar/Correspondência have no key in Installed, so no parentheses.
- Choose button only with a single target row; Pin sets all possible rows to the opposite of the first row's state; left-out toast is Info, singular "1 item ficou de fora" / plural "N itens ficaram de fora" (Task 110 confirmation not present yet).
- Buttons are hidden when the pane lacks room (needs button rows + 1 blank + 3 text rows) or when width < 60 (no details pane): no mouse path to them there.
- Snapshot actions_modal_snapshot updated: a "Fixar" button now shows behind the modal. Extracted upgrade_one/remove_one from upgrade_selected/remove_selected.

Concerns:
- buttons() calls actions_for per target row on every frame/mouse event (regions + draw); with many marks this is O(marks x rows). Not measured.
- mutate.ps1 reported INCONCLUSIVO once for the unfiltered `cargo test` form (a "could not compile" line from a concurrent link kill); rerun with `--test tui_view` gave PEGA.

## Review fixes (second commit)
Commits: see `git log --format='%h %s' 902e1a3..HEAD` (e9cdab8 feat, then the fix commit).
Gate: [OK] test (833 passed, 21 ignored) / [OK] gate verde.
Bench (ignored test bench_regions_with_many_marks, --release, 300 rows all marked, regions()): before 68.5 ms/call; after one installed_view per call 1.23 ms; after skipping the full menu for Pin/Choose/Undo 0.35 ms.
Mutation: `row += 1;` -> `row += 0;` : [OK] mutação PEGA (buttons_wrap_without_overlap). Filter mutation re-run on the new code: PEGA.
Keys i/u/U/Enter with marks now go through run_button; one message "N itens ficaram de fora (ação não se aplica)" (replaces "pulados: names"; 3 existing tests updated). Marks all hidden: keys/buttons act on nothing and report the hidden marks (no fallback to the selected row).
