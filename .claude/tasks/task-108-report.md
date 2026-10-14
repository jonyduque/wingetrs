Status: DONE

Commits:
2ed2a4d feat(tui): left click marks, right click opens the item's full action menu

Gate:
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (828 passed, 20 ignored) 49s
[OK]   gate verde

Mutation proofs:
1. scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'let cmds = app.move_to(index);' -Replacement 'let cmds = app.move_to(index); return cmds;' -TestCommand 'cargo test --quiet --test tui_state left_click_selects_and_toggles_mark'
   [OK] mutação PEGA: 'let cmds = app.move_to(index);' -> 'let cmds = app.move_to(index); return cmds;' fez o teste falhar (exit 101).
2. scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'note.is_none() && *id != ActionId::Remove)' -Replacement 'note.is_none())' -TestCommand 'cargo test --quiet --test tui_state menu_never_opens_on_remove'
   [OK] mutação PEGA: 'note.is_none() && *id != ActionId::Remove)' -> 'note.is_none())' fez o teste falhar (exit 101).
3. grep -rn "DOUBLE_CLICK\|last_click" src tests: empty output.

Decisions:
- Modal::Actions gained `ids: Vec<ActionId>`; dispatch is by id (run_action) instead of index arithmetic. Marked-rows menu unchanged (ids empty).
- Order kept from before: Escolher correspondência first, Undo after Fixar, Marcar last. Cursor = first enabled non-Remover item, so existing tests that expected the cursor on item 0 were adapted.
- Right click opens the single-row menu even when rows are marked; Enter in Installed with marks still opens the marked menu.
- "Instalar" in Search is disabled with a note for winget rows (same as `i`); it uses install_one (ignores marks).

Concerns: edited tests/cli_install.rs (two Downs removed, cursor now starts on Fixar) and updated snapshot tui_view__actions_modal_snapshot.snap (menu gained Marcar, cursor on Fixar); both outside the Files list. Enter in Updates/Search still direct; Task 109 can reuse actions_for.
