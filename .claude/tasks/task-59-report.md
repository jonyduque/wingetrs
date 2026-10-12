Status: DONE_WITH_CONCERNS

Commits:
aaddb78 perf(tui): add tui_frame benchmark example
bdfc4cf perf(tui): build only the visible list rows
1948a61 test(tui): cached installed view must equal a fresh filter
(8a1ae3b na faixa e do orquestrador, nao meu)

Gate: `[OK]   clippy` / `[OK]   test (522 passed, 16 ignored) 83s` / `[OK]   gate verde`

Benchmark (tui_frame, 200 frames, TestBackend 120x40, us; median / p95)
Antes (base, 3 runs):
- Pesquisar: 1365/1958, 1254/1892, 970/1515
- Pesquisar "code": 1977/2702, 1611/2940, 1550/2314
- Instalados: 855/1267, 626/1089, 605/1013
- Atualizacoes: 447/681, 316/648, 335/741
Depois da mudanca 1 (so linhas visiveis), 3 runs:
- Pesquisar: 397/604, 268/604, 327/645 (~3-4x)
- Pesquisar "code": 396/705, 295/519, 379/568 (~4-5x)
- Instalados: 508/943, 284/458, 344/533 (~2x)
- Atualizacoes: 430/777, 289/469, 277/481 (~1.3x)
Mudanca 2 (catalog_pos) A/B contra a mudanca 1, 4 runs alternados: Instalados A 284-318 vs B 277-481; Atualizacoes A 225-299 vs B 246-520 => sem ganho (ruido). REVERTIDA (nunca commitada).
Mudanca 3 (view_rows) A/B 5 runs alternados: Instalados A 301-508 vs B 295-736; Atualizacoes A 248-397 vs B 235-474; Pesquisar identico => sem ganho. REVERTIDA (nunca commitada).

Mutation proofs:
1. `mutate.ps1 -Path src/tui/view.rs -Anchor 'selected + 1 - height' -Replacement 'selected - height' -TestCommand 'cargo test --quiet --test tui_view list_window'`
   [OK] mutação PEGA: 'selected + 1 - height' -> 'selected - height' fez o teste falhar (exit 101).
2. `... -Anchor 'let window = list_window(sel, app.scroll, len, h);' -Replacement 'let window = list_window(app.scroll, app.scroll, len, h);' -TestCommand 'cargo test --quiet --test tui_view long_list_end'`
   [OK] mutação PEGA: 'let window = list_window(sel, app.scroll, len, h);' -> '...list_window(app.scroll, app.scroll, len, h);' fez o teste falhar (exit 101).
3. (catalog_pos) provada quando implementada (removendo reindex em CatalogLoaded: [OK] mutação PEGA), mas a mudanca foi revertida por falta de ganho; sem prova no codigo final.
4. (view_rows) idem: provada quando implementada (removendo refresh_view no Ctrl+S: [OK] mutação PEGA), mudanca revertida; sem prova no codigo final.

Decisions:
- Regra do projeto (sem ganho medido => reverter) aplicada as mudancas 2 e 3; o teste 4 e o teste 3 ficaram como guarda de regressao (passam no codigo final).
- Teste 2: o brief pede pkg1499, mas App limita os hits a MAX_HITS=1000; a ultima linha com End e pkg0999. Assertion trocada para pkg0999 (comentario no teste). A linha e achada com trim_start_matches('"'), como no teste vizinho.

Concerns: brief assumia ganho nas mudancas 2 e 3; medicao nao confirmou. Orquestrador pode querer revisar o brief (catalog_pos/view_rows nao implementados).
PID 7900 (bash de outro projeto) permanece, nao encerrado.

Addendum (review fixes)
- Novo teste `scrolled_selection_highlights_the_selected_row` (tests/tui_view.rs, tema TrueColor, confere bg no buffer).
  `mutate.ps1 -Path src/tui/view.rs -Anchor '.checked_sub(window.start)' -Replacement '.checked_sub(0)' -TestCommand 'cargo test --quiet --test tui_view scrolled'`
  [OK] mutação PEGA: '.checked_sub(window.start)' -> '.checked_sub(0)' fez o teste falhar (exit 101).
- examples/tui_frame.rs agora usa Catalog::open_read_only.
