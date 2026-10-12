Status: DONE_WITH_CONCERNS

Commits:
8fdf455 perf(tui): search off the UI thread with generations
(+ commit de correção da revisão, SHA na mensagem ao orquestrador: fix(tui): coalesce searches, drop stale and off-tab results)

Passo 0 (medição, `cargo build --release --example tui_frame`, catálogo real, 19 646 entradas, 50 repetições cada; duas execuções):

```
busca c        median  47741 us   p95  70744 us      | median  49105 us   p95  62586 us
busca co       median  32784 us   p95  39520 us      | median  32234 us   p95  48870 us
busca cod      median  12606 us   p95  16040 us      | median  11612 us   p95  13923 us
busca code     median  10287 us   p95  16926 us      | median   9405 us   p95  11159 us
busca vscode   median   4443 us   p95   4943 us      | median   4572 us   p95   5181 us
```
p95 > 8 000 us em c/co/cod/code => implementado.

Ganho (A/B, custo de `update(Char('c'))` na thread de UI, 50 repetições, catálogo real):
- base f0d39de (worktree temporário, removido): `median 58780 us p95 83045 us` / `median 55633 us p95 73144 us`
- HEAD: `tecla 'c' median 0 us p95 0 us` (duas execuções; a busca roda em spawn_blocking)
- Frames (cenário do Task 59) inalterados: search 'code' median 433-443 us, p95 674-789 us.

Gate (após a correção da revisão):
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 101s
[OK]   test (530 passed, 16 ignored) 117s
[OK]   gate verde
```
(rodado com `CARGO_TARGET_DIR=target/t60`; ver Concerns)

Mutation proofs:
1. Equivalência (limite 999):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'index.search(query, kind, MAX_HITS)' -Replacement 'index.search(query, kind, 999)' -TestCommand 'cargo test --quiet --test tui_state async_search_equals_sync_search'`
   `[OK] mutação PEGA: 'index.search(query, kind, MAX_HITS)' -> 'index.search(query, kind, 999)' fez o teste falhar (exit 101).`
2. Geração (aplica qualquer SearchDone):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'if generation != app.search_generation {' -Replacement 'if generation == u64::MAX {' -TestCommand 'cargo test --quiet --test tui_state stale_search_results_are_dropped'`
   `[OK] mutação PEGA: 'if generation != app.search_generation {' -> 'if generation == u64::MAX {' fez o teste falhar (exit 101).`
3. Fora da UI (research() síncrono em refresh_view):
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '    fn refresh_view(&mut self) -> Vec<Cmd> {' -Replacement '    fn refresh_view(&mut self) -> Vec<Cmd> { if self.tab == Tab::Search { self.research(); }' -TestCommand 'cargo test --quiet --test tui_state typing_does_not_search_on_the_ui_thread'`
   `[OK] mutação PEGA: '    fn refresh_view(&mut self) -> Vec<Cmd> {' -> '    fn refresh_view(&mut self) -> Vec<Cmd> { if self.tab == Tab::Search { self.research(); }' fez o teste falhar (exit 101).`

Correção da revisão (CHANGES_REQUESTED):
- No máximo uma busca em voo (`search_in_flight: Option<(geração, consulta, filtro)>`). Teclas durante a busca só mudam `query`; no `SearchDone` da geração em voo, se consulta/filtro mudaram, emite UM `Cmd::Search` com a atual e não aplica o resultado velho.
- `SearchDone` fora da aba Pesquisar é descartado sem mexer em selected/scroll nem emitir comandos.
- Recarregar o índice (`research()`) zera a busca em voo, então o resultado sobre o índice antigo é descartado. O incremento de geração em `research()` saiu (ficaria redundante e sem teste).
- `stale_search_results_are_dropped` ajustado à coalescência sem enfraquecer: "p" (ger. 1), "o" não emite, `SearchDone` 1 => `Cmd::Search "po"` (ger. 2); entrega a 2 e depois a 1 de novo => hits de "po" e nenhum comando.
- Testes novos: `stale_search_after_catalog_reload_is_dropped`, `search_result_off_the_search_tab_is_dropped`, `typing_during_a_search_queues_only_the_latest`.

Mutações da correção:
- `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '        self.search_in_flight = None;' -Replacement '' -TestCommand 'cargo test --quiet --test tui_state stale_search_after_catalog_reload_is_dropped'`
  `[OK] mutação PEGA: '        self.search_in_flight = None;' -> '' fez o teste falhar (exit 101).`
- tab check removido em `search_done` (âncora de 4 linhas `if self.tab != Tab::Search { return Vec::new(); } if query` -> `if query`), teste `search_result_off_the_search_tab_is_dropped`:
  `[OK] mutação PEGA: '        if self.tab != Tab::Search {` (linha da saída truncada pelo grep na primeira quebra de linha da âncora)
- emitir `Cmd::Search` a cada tecla:
  `[OK] mutação PEGA: 'if self.tab != Tab::Search || self.search_in_flight.is_some() {' -> 'if self.tab != Tab::Search {' fez o teste falhar (exit 101).`
- geração (aplicar qualquer SearchDone), refeita para o código novo, teste `stale_search_results_are_dropped`:
  `[OK] mutação PEGA: 'self.search_in_flight.take_if(|f| f.0 == generation)' -> 'self.search_in_flight.take().or(Some((generation, self.query.clone(), self.search_kind())))' fez o teste falhar (exit 101).`
- limite 999 e research() síncrono em refresh_view, refeitas no código novo: ambas `[OK] mutação PEGA` (mesmas âncoras de cima).

Latência da rajada c,o,d,e a cada 30 ms (da última tecla até os hits finais; `examples/tui_frame.rs`, 15 rodadas; duas execuções; com a TUI do usuário aberta, variância alta):
- antes (busca síncrona por tecla, simulada com tempos reais de `run_search`): `median 29139 us max 43313 us` / `median 43789 us max 83086 us`
- depois (worker real + coalescência): `median 16494 us max 29948 us` / `median 11535 us max 53274 us`
- versão intermediária 8fdf455 (async sem coalescência): não medido.

Decisions:
- `SearchIndex::search`/`search_with_threads` passaram a `&self` (o campo `matcher` saiu; o caminho sequencial cria um `Matcher` por chamada): sem isso `Arc<SearchIndex>` e `run_search(&SearchIndex, ..)` do brief não compilam. Removidos `let mut` que viraram aviso em `src/install/target.rs`, `tests/search.rs`, `tests/index_file.rs` (fora da lista Files).
- `run_search` mora em `src/tui/app.rs` (usado por App::new/CatalogLoaded/Booted e pelo worker em run.rs). Reconstruir o índice (`research()`) incrementa `search_generation`, descartando buscas em voo sobre o índice antigo.
- Na aba Pesquisar o `details_cmd()` após digitar/filtrar/Esc espera o `SearchDone` (evita pedir detalhes do hit antigo); nas outras abas segue imediato. Cancelamento do worker (`join_search` -> `None`) não envia nada; pânico vira `Msg::Panicked` (teste `search_worker_panic_reaches_the_loop` em tests/tui_loop.rs).
- Teste 1 roda também com um catálogo de 1 503 entradas (`big_entries`), senão a mutação "limite 999" não era detectável com as 3 entradas de `app()`.
- tests/tui_state.rs: `deliver_searches` processa também as buscas que um `SearchDone` dispara (como o laço de eventos); helper `answer` entrega uma busca só.
- Testes existentes que liam `hits` logo após digitar passaram a usar o helper `step` (update + entrega das buscas), sem mudar asserções; idem `typed()` em tests/tui_view.rs (fora da lista Files) e o cenário de frames do example.

Concerns:
- Um `cargo run` do usuário (PID 3316, `target\debug\wingetrs.exe`, desde 21:50) travava `target/debug/wingetrs.exe`; não foi encerrado. Gate, mutações e benchmarks rodaram com `CARGO_TARGET_DIR=target/t60`. As medições pós-mudança e o A/B rodaram com essa TUI aberta (ociosa); o passo 0 foi medido antes dela abrir.
- TDD: os testes novos foram escritos antes da implementação, mas não registrei a falha pré-implementação (falhariam por não compilar: `run_search`/`Cmd::Search` inexistentes); as provas de mutação acima cobrem cada um.
