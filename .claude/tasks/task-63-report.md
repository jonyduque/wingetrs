Status: DONE

Commits:
(see below; feature commit first, report commit after)

Gate:
[OK]   rules 2s
[OK]   rules-selftest 10s
[OK]   fmt 1s
[OK]   clippy 39s
[OK]   test (546 passed, 16 ignored) 127s
[OK]   gate verde

Mutation proofs (cada uma com o teste dedicado):
- fundo aberto da ativa (┘/└ -> ┴/┴), `active_first_tab`: [OK] mutação PEGA
- canto da primeira aba (├ -> │), `active_middle_tab`: [OK] mutação PEGA
- limiar `total + 1` -> `total + 100`: com o teste 3 do brief (largura 30) SOBREVIVEU (sempre compacto em 30); adicionado `tabs_threshold_is_exact` (46 largo / 45 estreito): [OK] mutação PEGA (também `total > width`: PEGA)
- rótulo ativo com fg_dim, `active_label`: [OK] mutação PEGA
- sem FILTER_GAP (Length(0)), `window_spacing`: [OK] mutação PEGA
- Layout 55/45 sem vãos no lugar de split_with_gutter, `window_spacing`: [OK] mutação PEGA

Snapshot tui_view__search_screen_snapshot.snap:
"╭───────────╮╭────────────╮╭──────────────╮                                                         "
"│ Pesquisar ││ Instalados ││ Atualizações │                                               wingetrs  "
"│           └┴────────────┴┴──────────────┴────────────────────────────────────────────────────────╮"
"│ / powert▏                                                                                        │"
"│                                                                                                  │"
"│ > W PowerToys                           0.101.2362.0 │ PowerToys                                 │"
"│   S powertoys                                0.101.2 │ # Microsoft.PowerToys                     │"
"│   W EverythingPowerToys                       0.91.1 │ versão 0.101.2362.0                       │"
"│                                                      │ Utilitários para power users              │"
"│                                                      │                                           │"
"│                                                      │ | carregando…                             │"
"│                                                      │                                           │"
"│                                                      │                                           │"
"│                                                      │                                           │"
"╰──────────────────────────────────────────────────────────────────────────────────────────────────╯"
" 3 de 3 · ↑↓ mover · ←→ aba · Enter detalhes · Ctrl+R refresh · ? ajuda                             "

Decisions:
- draw_tabs recebe só rótulos; "aba Atualizações com contagem" (warn) é detectada por índice 2 + rótulo terminando em dígito.
- draw_list perdeu o parâmetro `chrome` (nunca há caixa própria); draw_details também sem caixa; divisor usa token `border`.
- Rótulo com contagem agora é "Atualizações N" (sem parênteses), como o brief.
- Testes 53 (`selected_row_moves...`) e 59 (`long_list_end...`) passaram sem ajuste; `list_and_details_text_start_one_column_after_the_border` teve as linhas deslocadas (4..=6 -> 5..=7).
- Adicionado teste de clamp/página: `end_and_page_down_keep_the_selection_on_screen_at_100x24`; CHROME_ROWS=7 coincide com a altura real da lista (24-7=17).
- Usado CARGO_TARGET_DIR=target/t63 (exe travado); removido ao final.

Concerns:
- Perdeu-se o realce de foco (border_focus) da lista/detalhes: não há mais caixas separadas; o foco em Detalhes não tem indicação visual. Fica para o orquestrador decidir (ex.: divisor em border_focus).
- Teste 3 literal do brief não pega a mutação do limiar sozinho (ver acima).

Follow-up (pedido do coordenador):
- Divisor com border_focus quando Focus::Details (token border caso contrário); teste divider_is_focus_colored_only_when_details_has_focus. Mutação "sempre border" (if false): [OK] mutação PEGA.
- Assinatura draw_tabs(.., labels: &[TabLabel{text, warn}], ..); teste warn_label_is_warn_when_inactive_and_accent_when_active. Mutação "ignorar warn" (else if false): [OK] mutação PEGA.
- Gate: [OK]   test (548 passed, 16 ignored) 135s / [OK]   gate verde
- Concerns anteriores resolvidos.
