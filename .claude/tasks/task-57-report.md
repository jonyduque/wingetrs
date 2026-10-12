Status: DONE

Commits:
2d031d8 feat(tui): left and right arrows switch tabs

Gate:
[OK]   clippy 132s
[OK]   test (512 passed, 16 ignored) 149s
[OK]   gate verde
(two earlier runs failed from other agents' work: a file lock on install_manifest.exe and an in-progress tui_theme test, Task 61; reran until green)

Mutation proofs:
1. Direction: [OK] mutação PEGA: 'KeyCode::Tab | KeyCode::Right => app.set_tab(next_tab(app.tab))' -> 'KeyCode::Tab | KeyCode::Right => app.set_tab(prev_tab(app.tab))' fez o teste falhar (exit 101).
2. Single cycle: [OK] mutação PEGA: 'Tab::Updates => Tab::Search,' -> 'Tab::Updates => Tab::Installed,' fez o teste falhar (exit 101).
3. Dialog first: [OK] mutação PEGA: 'if app.modal.is_some() {' -> 'if key.code == KeyCode::Right { return app.set_tab(next_tab(app.tab)); } if app.modal.is_some() {' fez o teste falhar (exit 101).
(Mutation 3 only handles Right ahead of the modal check; test 4 presses Right first, so it catches it.)

Decisions:
- Snapshot diffs checked: only the footer line changed ("Tab aba" -> "←→ aba"); insta also dropped the stale `assertion_line` metadata in some .snap files.
- Help dialog text changed in view.rs; no snapshot covers it.
- Test 2 uses on_tab(start, vec![]) for both apps.
- `cargo fmt` ran on the whole tree, which may have touched other agents' files; nothing of theirs was added to my commit.

Concerns: none

Addendum (review Minor): added `arrows_are_ignored_while_help_is_open` in tests/tui_state.rs.
[OK] mutação PEGA: 'if app.show_help {' -> 'if app.show_help && !matches!(key.code, KeyCode::Right) {' fez o teste falhar (exit 101).
