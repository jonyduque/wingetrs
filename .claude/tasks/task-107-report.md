Status: DONE

Commits:
5cbf65f test(tui): cover show filter scope, selection kept on ctrl+e, empty-filter message
5b851de feat(tui): show filter for unmatched and duplicate apps; lighter bold help keys

Gate:
[OK]   rules-selftest 11s
[OK]   fmt 2s
[OK]   clippy 15s
[OK]   test (824 passed, 20 ignored) 222s
[OK]   gate verde

Mutation proofs:
[OK] mutação PEGA: 'Correlation::Unmatched | Correlation::Ambiguous { .. }' -> 'Correlation::Unmatched' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'KeyCode::Char('e') if app.tab == Tab::Installed' -> 'KeyCode::Char('e')' fez o teste falhar (exit 101).
[OK] mutação PEGA: '(Tab::Installed, ShowFilter::Unmatched)' -> '(_, ShowFilter::Unmatched)' fez o teste falhar (exit 101).
cycle_show `position_of(kept).unwrap_or(0)` -> `0`: mutate.ps1 anchor was ambiguous (same line in cycle_sort), so done by hand (sed on the line, test run, file restored): ctrl_e_keeps_selection FAILED (panicked at tests/tui_state.rs:4373).

Decisions:
- Empty-list message "nenhum pacote neste filtro" shows in Installed when show or source filter is active.
- Full help keeps "filtrar fonte" for Ctrl+S and adds "Ctrl+E mostrar (aba Instalados)"; dialog height 17 -> 18.
- Settings bar is now just "enter mudar" (+ "f fila") + "?".

Concerns:
- Edited tests/tui_theme.rs (not in Files list): hardcoded old help_key colors.
- Only snapshot changed: tui_view__settings_screen_snapshot (bar line only).
