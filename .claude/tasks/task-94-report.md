Status: DONE

Commits: e5e8526 feat(tui): mouse for tabs, selection, wheel and double click

Gate:
```
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 49s
[OK]   test (728 passed, 18 ignored) 169s
[OK]   gate verde
```

Mutation proofs:
1. `scripts/mutate.ps1 -Path src/tui/regions.rs -Anchor 'let index = window.start + off / per_item;' -Replacement 'let index = off / per_item;' -TestCommand 'cargo test --quiet --test tui_state click_on_item_selects_it'`
   `[OK] mutação PEGA: 'let index = window.start + off / per_item;' -> 'let index = off / per_item;' fez o teste falhar (exit 101).`
2. `src/tui/app.rs` `Duration::from_millis(400)` -> `Duration::from_millis(0)` (double_click_runs_enter)
   `[OK] mutação PEGA: 'Duration::from_millis(400)' -> 'Duration::from_millis(0)' fez o teste falhar (exit 101).`
3. `src/tui/app.rs` removing `|| app.modal.is_some()` from the guard (mouse_ignored_under_modal_and_when_disabled)
   `[OK] mutação PEGA: 'if !app.mouse_enabled || app.modal.is_some() || app.show_help {' -> 'if !app.mouse_enabled || app.show_help {' fez o teste falhar (exit 101).`
4. Drawing reads `regions` (and `tab_rects`'s widths mirror draw_tabs), so the regions variant was run: `src/tui/tabs.rs` `tab_rects` shifted 1 column (tests tab_titles_match_regions, click_on_tab_switches_tab)
   `[OK] mutação PEGA: 'out.push(at(x, w + pad + 2, area.y, area.height.min(3)));' -> 'out.push(at(x + 1, w + pad + 2, area.y, area.height.min(3)));' fez o teste falhar (exit 1).`
5. Panic hook (src/tui/run.rs):
   `std::panic::set_hook(Box::new(move |info| { let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture); previous_hook(info); }));`
   Mouse capture goes off first so the previous hook (ratatui's restore + message) prints on a terminal that is already out of mouse mode.

Decisions:
- New module `src/tui/regions.rs` (`regions`, `item_at`, `list_visible`, `setting_line`); `tab_rects` in tabs.rs gives the title rects. draw_screen/draw_list take their areas from there; draw_tabs keeps its signature (its painting is covered by the equivalence test, not shared code).
- Double-click time comes from the existing `App.now` (set by clock_turn), so no `Instant::now()` in logic.
- Wheel is ignored on the Settings tab; click on a tab clears `last_click`.
- ITEM_GAP is 0, so the "gap line" case is untestable; test 3 checks the space below the last item instead (extra test click_below_the_last_item_hits_nothing). Added setting_rows_match_the_drawing test (tui_view.rs).
- Extra: src/tui/mod.rs (new module), made `tab_labels` and `MIN_WIDTH_FOR_DETAILS` pub(crate).

Concerns: mouse_enabled does not yet gate EnableMouseCapture (Task 95). The mouse capture/hook in run.rs is not exercised by tests (needs a real terminal): não medido/ao vivo.

## Revisão (fix commit)
Commits: e1cad42 feat(tui): mouse for tabs, selection, wheel and double click (SHA do relatório anterior "e5e8526" estava errado; este é o real); o fix vem em seguida (`fix(tui): mouse filters motion; tabs drawn from regions`, ver `git log`).
Gate: `[OK]   test (729 passed, 18 ignored) 187s` / `[OK]   gate verde`
Mutation proofs:
- `[OK] mutação PEGA: '| MouseEventKind::ScrollUp' -> '| MouseEventKind::Moved | MouseEventKind::ScrollUp' fez o teste falhar (exit 101).` (motion_is_not_a_message)
- `[OK] mutação PEGA: '.zip(TAB_ORDER)' -> '.map(|mut r| { r.x += 4; r }).zip(TAB_ORDER)' fez o teste falhar (exit 101).` (regions desloca 4 colunas; draw_tabs agora lê `tab_rects`, então deslocar o próprio `tab_rects` move desenho e clique juntos e não é detectável, correto por construção; 1 coluna não sai do retângulo por causa do padding, por isso 4)
Decisions: draw_tabs pinta cada título em `tab_rects` (helper `is_narrow` comum); teste de equivalência itera Ascii/Unicode/Nerd, compara por célula e afirma que a forma estreita foi coberta; `mouse_msg` pública em run.rs.
