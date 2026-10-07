Status: DONE_WITH_CONCERNS

Commits:
e34ae1b feat(tui): layout, event loop and snapshot tests
9c41cf9 chore(tasks): record task 12
e2d8737 fix(list): never match MSIX packages by name and publisher

Gate:
[OK]   fmt 1s
[OK]   clippy 7s
[OK]   test (131 passed, 2 ignored) 108s
[OK]   gate verde

Mutation proofs:
- scripts/mutate.ps1 -Path src/tui/view.rs -Anchor 'MIN_WIDTH_FOR_DETAILS: u16 = 60' -Replacement 'MIN_WIDTH_FOR_DETAILS: u16 = 40' -TestCommand 'cargo test --quiet --test tui_view'
  [OK] mutação PEGA: 'MIN_WIDTH_FOR_DETAILS: u16 = 60' -> 'MIN_WIDTH_FOR_DETAILS: u16 = 40' fez o teste falhar (exit 101).
  (1a tentativa SOBREVIVEU: com id longo quebrando em 3 linhas a descricao saia do painel em 50x12; removi a linha em branco antes da descricao.)
- scripts/mutate.ps1 -Path src/tui/view.rs -Anchor 'theme.matched().add_modifier(Modifier::BOLD)' -Replacement 'theme.name()' -TestCommand 'cargo test --quiet --test tui_view'
  [OK] mutação PEGA: 'theme.matched().add_modifier(Modifier::BOLD)' -> 'theme.name()' fez o teste falhar (exit 101).  (teste novo matched_letters_are_underlined)

Decisions:
- App nao tem contador de geracao e app.rs nao esta em Files: redesenho apos todo Msg exceto Tick ocioso (Tick so redesenha se refreshing/detalhe carregando).
- Debounce de 150 ms via AtomicU64 de geracao no Ctx: requisicao antiga e descartada ao acordar.
- Apos RefreshFinished recarrego o catalogo (Msg::CatalogLoaded) e a idade do indice.
- Sem subcomando, cli::run chama tui::run antes de criar runtime (tui::run cria o proprio); glifos: flag > config > nerd.

Concerns:
- Manual (Step 4) NAO verificado: nao vi a TUI interativa, nem digitar/trocar abas/Ctrl+R/redimensionar/Esc, nem restauracao em panico (so ratatui::init/restore padrao). Executei release com stdin nulo e catalogo vazio (rede inatingivel): desenhou o 1o frame (escape de tela alternativa + layout visiveis), morto por timeout. first_frame_ms medido no log: 100 e 269 (duas execucoes, catalogo vazio, maquina com build recem-feito); com catalogo real: nao medido.
- Spinner de detalhes so anima quando app.refreshing (spinner_frame so avanca la).

search_screen_snapshot:
```
"╭  Pesquisar  Instalados  Atualizações ────────────────────────────────────────────────────────────╮"
"│ / powert▏                                                                                        │"
"╰──────────────────────────────────────────────────────────────────────────────────────────────────╯"
"╭─────────────────────────────────────────────────────╮╭───────────────────────────────────────────╮"
"│> W PowerToys                            0.101.2362.0││PowerToys                                  │"
"│  S powertoys                                 0.101.2││# Microsoft.PowerToys                      │"
"│  W EverythingPowerToys                        0.91.1││versão 0.101.2362.0                        │"
"│                                                     ││Utilitários para power users               │"
"│                                                     ││                                           │"
"│                                                     ││| carregando…                              │"
"│                                                     ││                                           │"
"│                                                     ││                                           │"
"│                                                     ││                                           │"
"│                                                     ││                                           │"
"╰─────────────────────────────────────────────────────╯╰───────────────────────────────────────────╯"
" 3 de 3 · ↑↓ mover · Tab aba · Enter detalhes · Ctrl+R refresh · ? ajuda                            "
```

## Follow-up: perf(tui) f1bc6e5
Primeiro frame agora antes de ler o catalogo (App::new vazio + status "carregando catalogo..."); open+load_entries+index_age+needs_refresh em spawn_blocking -> Msg::CatalogLoaded + Boot (oneshot) que dispara scan/refresh. Http::new apos o 1o draw. Tick avanca spinner tambem com detalhe carregando.
Gate: [OK] fmt / clippy / test (131 passed, 2 ignored) / gate verde. Snapshots inalterados.
first_frame_ms com catalogo REAL (release, stdin nulo, timeout 4s, 3 execucoes): 4, 3, 2 (sem TTY; em terminal real pode ser maior).
