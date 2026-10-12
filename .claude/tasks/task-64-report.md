Status: DONE

Commits:
fdc66b5 feat(tui): two-row list items with a selection bar

Gate:
[OK]   fmt
[OK]   clippy 27s
[OK]   test (554 passed, 16 ignored) 91s
[OK]   gate verde

Mutation proofs:
1. page in items: `-Anchor '(ITEM_ROWS + usize::from(ITEM_GAP))' -Replacement '1'` in clamp, test `end_of_list_is_visible` (render) SOBREVIVEU (the draw recomputes the window, so render hides a wrong scroll). Added state test `end_scrolls_by_two_row_items` (scroll == 44 at 100x20) and reran:
   [OK] mutação PEGA: '(ITEM_ROWS + usize::from(ITEM_GAP))' -> '1' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: 'theme.sel_title().add_modifier(Modifier::BOLD)' -> 'theme.name()' fez o teste falhar (exit 101).  (selected_item_has_bar_and_pink_title)
3. [OK] mutação PEGA: 'Some(d) => format!("{} · {d}", e.id),' -> 'Some(_) => format!("{} · {}", e.id, e.source_name),' fez o teste falhar (exit 101).  (second_line_shows_id_and_description)

Decisions:
- Selected name keeps matched underline: only unmatched spans are restyled to sel_title+BOLD.
- Description clipped by char count with an ellipsis; second_line test renders at 120 cols (at 100 the list column clips it).
- Added end_scrolls_by_two_row_items and end_and_page_keep_selection_on_screen_at_other_heights (100x12, 100x40); scrolled_selection test now proves the bar/sel_title on the right two rows only.
- Existing tests adapted: narrow_terminal probe "versão" (description now appears in the list), list_and_details test (description rows), selected-row prefix "│ | ".

Concerns: none (full test build used CARGO_TARGET_DIR=target/t64 because target/debug/wingetrs.exe was locked; remove it).
