Status: DONE_WITH_CONCERNS

Commits: ea742f2 feat(tui): one spacing scale for every box and dialog

Gate (CARGO_TARGET_DIR=target/t62):
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 97s
[OK]   test (537 passed, 16 ignored) 146s
[OK]   gate verde

Mutation proofs:
- PAD_X=0 (layout.rs) -> list_and_details_text_start_one_column_after_the_border and layout_constants_are_the_documented_scale: "[OK] mutação PEGA: 'pub const PAD_X: u16 = 1;' -> 'pub const PAD_X: u16 = 0;' fez o teste falhar (exit 101)." (both)
- dialog without inset (render_dialog): "[OK] mutação PEGA: 'let inner = inset(blk.inner(rect), DIALOG_PAD_X, DIALOG_PAD_Y);..." (confirm_dialog_has_inner_padding)
- right GUTTER removed: "[OK] mutação PEGA: 'let right_x = divider.saturating_add(1 + GUTTER)' -> 'let right_x = divider.saturating_add(1)' fez o teste falhar (exit 101)." (split_with_gutter_leaves_blank_columns)
- saturating_sub -> '-': "[OK] mutação PEGA: 'area.width.saturating_sub(x.saturating_mul(2))' -> 'area.width - x * 2' fez o teste falhar (exit 101)." (inset_saturates_on_tiny_rects)

New search_screen_snapshot:
"╭  Pesquisar  Instalados  Atualizações ────────────────────────────────────────────────────────────╮"
"│ / powert▏                                                                                        │"
"╰──────────────────────────────────────────────────────────────────────────────────────────────────╯"
"╭─────────────────────────────────────────────────────╮╭───────────────────────────────────────────╮"
"│ > W PowerToys                          0.101.2362.0 ││ PowerToys                                 │"
"│   S powertoys                               0.101.2 ││ # Microsoft.PowerToys                     │"
"│   W EverythingPowerToys                      0.91.1 ││ versão 0.101.2362.0                       │"
"│                                                     ││ Utilitários para power users              │"
"│                                                     ││                                           │"
"│                                                     ││ | carregando…                             │"
(...blank rows...)
" 3 de 3 · ↑↓ mover · ←→ aba · Enter detalhes · Ctrl+R refresh · ? ajuda                             "

Decisions:
- Added layout::BORDER (=1 per side) so dialog sizing has no loose "2"; extra to the documented list, not asserted by test 5.
- Dialogs grow outer size by 2*PAD_X / 2*PAD_Y over the old text size via dialog_size/render_dialog helpers (help, confirm, running, picker). Removed the literal leading space of the search prompt (PAD_X covers it) and of the status bar (BAR_PAD_X).
- split_with_gutter exists and is tested but is not yet used by view (layout unchanged this task; Task 63+ uses it). TAB_PAD_X/FILTER_GAP/ITEM_GAP/TOAST_* also unused yet.
- list_lines/tabs_line untouched (internal " " belong to the item/tab).

Concerns:
- At 100 columns the picker/residues dialog is clamped to the screen, so its inner text lost 4 columns; the long residues line is now cut (")" of "(pode ser de outra instalação)" disappears). Picker lines do not wrap.
- Used CARGO_TARGET_DIR=target/t62 (removed at the end).
