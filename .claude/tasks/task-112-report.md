Status: DONE

Commits:
(see git log 4f77376..HEAD; fix(tui): an open actions menu follows the tool detection)

Gate:
[OK]   rules 5s
[OK]   rules-selftest 25s
[OK]   fmt 4s
[OK]   clippy 18s
[OK]   test (851 passed, 22 ignored) 195s
[OK]   gate verde

Mutation proofs:
1. [OK] mutação PEGA: '            refresh_open_actions(app)' -> '            Vec::new()' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: '    let cmds = if marked {' -> '    let cmds = if marked && false {' fez o teste falhar (exit 101).  (always open_item_menu)

Decisions:
- Picker::selected() already existed; no picker.rs change.
- New fn refresh_open_actions in app.rs; cursor restored via Picker::single_at(items, at) only if that item is selectable.
- Test 4 uses Modal::ConfirmQuit (Modal has no PartialEq).
- mutate.ps1 anchors must be single-line (app.rs is CRLF).

Concerns: none
