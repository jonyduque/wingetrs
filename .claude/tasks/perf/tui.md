# Revisão de desempenho da TUI (somente leitura; nada medido)

Todo ganho abaixo é hipótese "não medido". Único número do repositório: primeiro frame 3–6 ms (9 de 10 aberturas, 1 outlier de 104 ms) em `docs/superpowers/reports/2026-10-07-fase1-validacao.md:15,73-87`. Esse span cobre só `run()` até o primeiro `draw` (`src/tui/run.rs:40-63`), não o tempo até o catálogo aparecer.

## 1. Mapa do caminho quente

### Inicialização (`src/tui/run.rs`)
1. `run` (39): `DataDir::from_env`/`ensure` (41-42), `Config::load` (43), `Theme` (44), `App::new(vec![])` (51), que chama `research()` num índice vazio (`app.rs:223`).
2. `ratatui::init`, `terminal.size`, primeiro `draw` com "carregando catálogo…" (54-59); span `first_frame_ms` (60).
3. `catch_unwind` + `Runtime::new()` (multi-thread, 66-67) + `event_loop` (89).
4. `event_loop`: `Http::new()` (100), `EventStream`, ticker de 100 ms, `spawn_boot` (108).
5. `spawn_boot` (176, `spawn_blocking`): `Catalog::open` + `load_entries` (179-181; `catalog/mod.rs:394-439`, leitura paralela por faixas de rowid, uma conexão SQLite por worker, aloca ~19,5 mil `PackageEntry` com `String`s) e envia `CatalogLoaded`; depois `index_age` abre o catálogo outra vez (83-87, 184) e `startup_needs_refresh` lê `HttpState` (194-203).
6. Msg de boot (120-129): `spawn_scan` incondicional (124) e, se preciso, `spawn_refresh`.
7. `CatalogLoaded` (`app.rs:352-357`): `SearchIndex::new` + `research()` (query vazia: pontua 19,5 mil, ordena por nome minúsculo, trunca em 1000) + `clamp`.
8. `spawn_scan` (245-281): `spawn_blocking` -> thread `winget_tracking::load_default` + `scan_all` (coletores em threads, `installed/mod.rs:173-203`) -> `Catalog::open` (de novo, com DDL) -> `correlate` sequencial por app (`installed/correlate.rs:290`, consulta SQLite por app).

### Laço por evento (`run.rs:110-166`)
`select!` entre `EventStream`, canal `rx`, boot e ticker (só ativo se `refreshing || details_loading`, linha 130) -> `jobs::propagate` -> `update` -> executa `Cmd` -> `terminal.draw` **depois de toda mensagem** que não seja tick ocioso (163-165).

Por tecla de texto: `on_key` (`app.rs:512-517`) -> `query.push` -> `refresh_view` -> `research()` **síncrono na thread de UI** -> `SearchIndex::search` (`catalog/search.rs:182-225`), que cria até 8 threads com `thread::scope`, cada uma com `Matcher::new`, pontua todas as ~19,5 mil entradas, junta, ordena por score e, no desempate, `to_lowercase()` do nome dentro do comparador (`search.rs:216-222,231-241`), trunca em 1000 -> `details_cmd` -> `spawn_details` (debounce 150 ms por geração, `run.rs:284-298`) -> `draw`.

Por frame (`view.rs:18-56`): `draw_list` -> `list_lines` (238-296) monta **uma `Line` por hit (até 1000)** com `name_spans` (aloca `String` por trecho; `matched.contains` linear por char) e `compose` (`" ".repeat`), e só depois faz `skip(start).take(h)` (341). `draw_details` -> `details_lines` -> `basics` (386-425), e `format!("{s}/{id}")` para consultar o cache de detalhes.

## 2. Ideias numeradas

**1. Spinner e ticker eternos por detalhes que ninguém pediu (provável bug de CPU ociosa).**
- Onde: `run.rs:130,163,170-173`; `app.rs:352-357` (`CatalogLoaded`) e `app.rs:358-361` (`InstalledLoaded`) devolvem `Vec::new()` sem `details_cmd()`; `view.rs:512-519` mostra o spinner.
- Mecanismo: após a carga o item 0 fica selecionado (`selected_key()` = Some), `details` não tem a chave e nenhum `LoadDetails` foi emitido. `details_loading` fica true até o usuário mover a seleção: tick a 10 Hz + `terminal.draw` a 10 Hz com "carregando…" animado, sem fetch correspondente. O mesmo ocorre ao trocar de aba (`set_tab`, `app.rs:324-333`, não chama `details_cmd`) e no Esc que limpa a query (`app.rs:489-491`, idem). Hipótese lida no código, não observada rodando.
- Economiza: CPU ociosa (10 despertares/s e 10 `draw`/s, cada um com `list_lines` de 1000 linhas, ver 3) e bateria; corrige também o spinner mentiroso.
- Medir: abrir a TUI, deixar parada 30 s, contar `draw` por um contador `tracing` por frame (ou CPU do processo em amostras); A/B emitindo `details_cmd()` nesses pontos. Esperado: CPU ociosa cai a ~0 (não medido).
- Risco/custo: baixo (poucas linhas + teste de `update`). Alternativa mais robusta: conjunto `pending_details` de requisições em voo e spinner só enquanto houver uma. Prioridade: **alta**.

**2. Busca fora da thread de UI, com cancelamento por geração e coalescência de teclas.**
- Onde: `app.rs:275-291`, `catalog/search.rs:182-225`.
- Mecanismo: cada tecla bloqueia o laço até terminar a varredura completa (+ criação/junção de até 8 threads + sort). Em rajada (colar, key repeat) cada tecla roda sua busca e seu `draw`. Mover para `spawn_blocking` com `Msg::SearchDone{gen, hits}` descartando gerações antigas (mesmo padrão de `details_gen`, `run.rs:33,286`) e/ou drenar os eventos já disponíveis antes de pesquisar (n teclas -> 1 busca).
- Economiza: latência de eco da tecla e trabalho das teclas intermediárias.
- Medir: span `search_ms` em torno de `research()`; tempo tecla -> `draw` colando 10 caracteres; mediana e p95 antes/depois. Não medido (a busca sozinha pode já ser de poucos ms; o ganho de coalescer depende disso).
- Risco/custo: médio (novo `Msg`, estado "buscando"; teste de equivalência: resultado final igual ao síncrono; handle de `spawn_blocking` sempre aguardado, conforme ARMADILHAS). Prioridade: alta se `search_ms` > ~8 ms; senão média.

**3. Montar só as linhas visíveis, não 1000 por frame.**
- Onde: `view.rs:238-296` (monta tudo) e `view.rs:331-342` (descarta com `skip/take`).
- Mecanismo: `list_lines` roda `name_spans` + `compose` + `format!` para até `MAX_HITS=1000` linhas para mostrar ~20-50. Calcular `start`/`h` antes (a lógica já existe em 331-340) e construir só `hits[start..start+h]`; o destaque da seleção (291) vira índice relativo. Idem para Instalados/Atualizações.
- Economiza: CPU e alocações por frame (ordem de 20x menos linhas), ainda mais com o ticker de 10 Hz (ideia 1).
- Medir: bench de `view()` com `TestBackend` e 1000 hits; ou span `draw_ms`. Não medido.
- Risco/custo: baixo-médio (teste de snapshot igual antes/depois; cuidar da consistência entre `clamp` em `app.rs:293-302` e o recálculo de `view.rs:331-340`). Prioridade: alta.

**4. `installed_view()` recalcula e aloca a cada chamada, e é chamada várias vezes por evento e por frame.**
- Onde: `app.rs:228-246` (`to_lowercase` da query e de **cada** `r.name`; novo `Vec<&ListRow>`); chamado por `visible_len` (248; `on_key` 453 e `clamp`), `selected_key` (265, também a cada tick via `details_loading`, `run.rs:170`), `view.rs:271` e `basics` (`view.rs:407`).
- Mecanismo: cachear o índice filtrado (`Vec<usize>`) em `App`, invalidado em mudança de query/filtro/aba/`InstalledLoaded`; guardar o nome minúsculo em `ListRow` ou comparar sem alocar (como `eq_lower`, `search.rs:29`).
- Economiza: alocações por frame/tecla. Medir: contagem de alocações num teste com 500-2000 `ListRow`, ou bench. Não medido. Risco baixo; prioridade média.

**5. `basics()` faz varredura linear de ~19,5 mil entradas por frame nas abas Instalados/Atualizações.**
- Onde: `view.rs:407-413` (`entries().iter().find(|e| &e.source_name == source && &e.id == package_id)`).
- Mecanismo: O(N) comparações de `String` a cada `draw` (inclusive os ticks da ideia 1) só para exibir publisher/descrição. Trocar por `HashMap<(source,id), idx>` montado no `CatalogLoaded`, ou guardar o índice do catálogo na correlação, ou resolver uma vez ao mudar a seleção.
- Economiza: CPU por frame. Medir: span `draw_ms` na aba Instalados com catálogo real, antes/depois. Não medido. Risco baixo; prioridade média-alta.

**6. Não desenhar quando nada mudou.**
- Onde: `run.rs:163-165` (só o tick ocioso é pulado). Exemplos sem mudança visível: tecla `Release` ou ignorada (`app.rs:422-424,519`), `Resize` igual, `InstallProgress` sem modal (`app.rs` ~372), `DetailsLoaded` de chave não selecionada.
- Mecanismo: `update` sinalizar `dirty` (ou contador de revisão em `App`) e desenhar só se sujo. O ratatui já faz diff do buffer (poucos bytes escritos), então o ganho é o custo de montar o frame (ideias 3-5), não I/O.
- Economiza: CPU de montagem de frame. Medir: contar `draw` em 100 eventos de um roteiro. Não medido. Risco médio (esquecer de marcar sujo congela a UI; mitigar com teste por `Msg`). Prioridade média-baixa, depois de 1 e 3.

**7. Ticker de 10 Hz só para o spinner.**
- Onde: `run.rs:25,105,130`. `MissedTickBehavior` padrão é `Burst`: após um `draw` lento o `Interval` dispara ticks acumulados em rajada.
- Mecanismo: `set_missed_tick_behavior(Skip)`; spinner a ~6-8 Hz; criar/rearmar o `Interval` só quando algo anima (hoje o `if` no `select!` protege o `select`, mas o `Interval` continua contando).
- Economiza: despertares e redesenhos durante refresh de rede. Medir: `draw`/s durante `Ctrl+R`. Não medido. Risco baixo; prioridade baixa.

**8. Reduzir aberturas repetidas do SQLite.**
- Onde: `run.rs:84` (`index_age` -> `Catalog::open` com DDL), `179,184` (boot abre duas vezes), `259` (scan), `236` (reload), `306` (a cada detalhe), `371` (`with_ops`).
- Mecanismo: reusar a conexão do `spawn_boot` para `sources()`; usar `open_read_only` onde só se lê (ARMADILHAS: `Catalog::open` roda DDL; abrir conexão + primeira consulta custa 11-15 ms; 8 conexões paralelas competem no `prepare`). Manter uma conexão de leitura por worker de detalhes em vez de uma por seleção.
- Economiza: dezenas de ms de background por boot (valores de ARMADILHAS, não medidos na TUI); menos disputa com a carga paralela. Medir: spans por etapa do `spawn_boot` (`open`, `load_entries`, `index_age`, `needs_refresh`). Risco baixo; prioridade média-baixa (não bloqueia a UI).

**9. Tempo até o catálogo visível e memória: usar `search.idx` na TUI em vez de `load_entries`.**
- Onde: `run.rs:179-181` (aloca ~19,5 mil `PackageEntry` completos, com descrição, tags e comandos) vs. `catalog/index_file.rs` (`IndexView::parse`, `view(i)` empresta `&str` sem alocar; `search` em 359). A CLI já usa o `search.idx` (`cli/mod.rs:510,553`).
- Mecanismo: manter o `IndexView` na `App` e pontuar com `score_entry` sobre `EntryView<'a>` emprestado; materializar `PackageEntry` só para as linhas visíveis e para o painel de detalhes. ARMADILHAS: "Alocação domina leitura em massa" (27 ms como `&str` vs 78 ms copiando para `String`; 8 threads só chegaram a 42 ms).
- Economiza: tempo do boot até a lista e memória residente. Medir: span `load_ms` e WorkingSet nas duas variantes, cache quente, 15 execuções com `-Baseline`. Não medido para a TUI.
- Risco/custo: alto (a TUI usa `hit.index` -> `entries()[...]` em `app.rs:261`, `view.rs:244,392,407`; regra de equivalência entre caminhos de busca; ver o teste de equivalência existente). Prioridade: média (maior ganho estrutural, maior custo); só depois de 1-5.

**10. Ordenação da busca: `to_lowercase()` no comparador e alocação total na query vazia.**
- Onde: `search.rs:216-222` e `231-241` (`sort_ranked`). Com query, o desempate aloca duas `String` por comparação; com query vazia, `sort_by_cached_key` aloca ~19,5 mil `String` por chamada, e isso se repete a cada volta ao estado vazio (Esc, troca de aba/filtro, Backspace até esvaziar).
- Mecanismo: pré-computar o nome minúsculo uma vez em `SearchIndex::new`; para a query vazia, guardar a ordem canônica e reaproveitá-la; usar `select_nth_unstable_by`/heap de tamanho `limit` em vez de ordenar todos os hits quando só 1000 são usados. Manter `sort_ranked` como função única (regra de equivalência dos três caminhos).
- Economiza: CPU por tecla e picos de alocação. Medir: spans `scan_ms` e `sort_ms` separados dentro de `search_with_threads`; queries de 1, 2 e 5 caracteres. Não medido. Risco médio (ordem estável é contrato); prioridade média.

**11. Threads por busca (`thread::scope` até 8, um `Matcher::new` cada) e refino incremental.**
- Onde: `search.rs:182-225`. ARMADILHAS: o heap padrão do Windows serializa alocação e 8 threads não ajudaram na leitura.
- Mecanismo: A/B do número de workers (já exposto em `search_with_threads`, `search.rs:196`); pool persistente com `Matcher`s reutilizados; refino incremental (nova query que estende a anterior só reavalia os hits anteriores). O refino pode mudar o resultado com `Normalization::Smart` e padrões especiais do nucleo: só adotar com teste de equivalência contra a busca completa.
- Economiza: CPU, threads criadas por segundo, latência da 2ª tecla em diante. Medir: `search_ms` por `workers` (1, 2, 4, 8). Não medido. Risco médio; prioridade média (medir `workers` é barato).

**12. Runtime tokio com um worker por núcleo para uma TUI quase ociosa.**
- Onde: `run.rs:66` (`Runtime::new()`).
- Mecanismo: `Builder::new_multi_thread().worker_threads(2)` (as cargas pesadas vão para `spawn_blocking`/threads próprias). Reduz threads e pilhas; pode ou não afetar o outlier de 104 ms (hipótese do relatório, `fase1-validacao.md:85-87`, não investigada).
- Medir: `first_frame_ms`, tempo até `CatalogLoaded` (span novo), contagem de threads do processo, A/B de `worker_threads`. Não medido. Risco baixo; prioridade baixa-média.

**13. Cache de detalhes sem limite e chave alocada por consulta.**
- Onde: `App.details: HashMap<String, Result<PackageDetails, String>>` (`app.rs`, campo `details`), inserção em `app.rs:361-364`; `format!("{s}/{id}")` em `run.rs:172`, `app.rs:304-309,343` e `view.rs:463`; `details_lines` clona `String`s por frame (`view.rs:448-472`).
- Mecanismo: chave sem alocar (tupla de `&str` via `HashMap<(SourceKind, String), _>` com lookup emprestado, ou índice do catálogo); LRU de algumas dezenas de itens (cada `PackageDetails` guarda instaladores e versões); cachear as `Line`s do painel até a seleção ou os detalhes mudarem.
- Economiza: alocações por frame/tick e memória em sessão longa. Medir: WorkingSet após navegar 500 itens; alocações por `draw`. Não medido. Risco baixo; prioridade baixa.

**14. Instalados: scan incondicional no boot, scans duplicados e correlação sequencial.**
- Onde: `run.rs:124` (`spawn_scan` sempre, mesmo na aba Pesquisar), `app.rs:324-333` (`set_tab` pede scan se `installed.is_empty()` enquanto o do boot ainda pode estar em voo: dois `Catalog::open` + `correlate`), `app.rs:666` (re-scan completo após cada job), `correlate.rs:290` (consulta SQLite por app, em série, uma conexão).
- Mecanismo: (a) flag `scanning` para não duplicar; (b) adiar o scan do boot até a primeira busca/idle, ou limitar a concorrência para não disputar CPU/SQLite com o primeiro uso; (c) pré-indexar nomes normalizados em memória (a `SearchIndex` já está carregada) em vez de SQLite por app, ou paralelizar `correlate` por fatias; (d) após job, atualizar só a linha afetada.
- Economiza: tempo até `InstalledLoaded` e CPU durante a digitação inicial. Medir: spans `scan_ms` e `correlate_ms` separados (hoje só `scan.elapsed_ms`, `installed/mod.rs`, e nada para `correlate`); A/B com scan adiado. Não medido. Risco médio (correlação é sensível a dados reais; ARMADILHAS cita MSIX sparse e ProductCode compartilhado; manter corpus de teste); prioridade média.

**15. Correção na mesma área: pânico em `spawn_boot` fica engolido.**
- Onde: `run.rs:178` descarta o `JoinHandle` do `spawn_blocking`. Se o closure entrar em pânico, o `oneshot` é fechado sem enviar, `booting` fica true e `boot_rx` nunca dispara: a UI fica sem status de refresh/scan e sem erro. É a armadilha "Handle de `spawn_blocking` descartado" de ARMADILHAS. Não é desempenho; corrigir junto com a 8 (aguardar o handle e enviar `Msg::Panicked`, como `spawn_job`, `run.rs:385-397`). O mesmo vale para `spawn_reload` (233-243), que também descarta o handle. Prioridade: média (correção).

## 3. Desperdícios evidentes

- Tick e `draw` a 10 Hz sem requisição de detalhes em voo (ideia 1): CPU ociosa contínua na tela inicial.
- `list_lines` constrói até 1000 linhas para mostrar ~30 (ideia 3, `view.rs:238-342`).
- Varredura linear de ~19,5 mil entradas por frame em `basics` (ideia 5, `view.rs:407-413`).
- `installed_view()` com `to_lowercase()` por linha, chamado 3-4 vezes por evento/frame (ideia 4).
- `format!("{s}/{id}")` alocado a cada consulta ao cache de detalhes, em todo tick/frame (ideia 13).
- Busca completa e síncrona por tecla, sem coalescer, com até 8 `Matcher::new` e sort com `to_lowercase()` no comparador (ideias 2, 10, 11).
- `Catalog::open` (com DDL) repetido em `index_age`, `spawn_scan`, `load_details`, `with_ops` (ideia 8).
- `Runtime::new()` com um worker por núcleo (ideia 12).
- `draw` após eventos que não alteram o estado (ideia 6).
- Handles de `spawn_blocking` descartados em `spawn_boot` e `spawn_reload` (ideia 15).

## 4. Top 3

1. **Ideia 1:** corrigir `details_loading` (emitir `LoadDetails` após `CatalogLoaded`/`InstalledLoaded`/troca de aba/Esc, ou rastrear requisições em voo). Menor custo, maior efeito em CPU ociosa; confirmar antes contando `draw` com a TUI parada.
2. **Ideia 3 (com 5):** renderizar só a janela visível da lista e trocar o `find` linear de `basics` por índice. Barateia todo frame e todo tick.
3. **Ideia 2, precedida do span `search_ms`:** busca em `spawn_blocking` com geração + coalescência de teclas para latência de tecla constante. Só depois, se os spans justificarem, ideias 10/11 (ordenação/threads) e 9 (`search.idx` na TUI) para boot e memória.

Instrumentação mínima a adicionar antes de qualquer mudança (spans `tracing`): `draw_ms` por frame, `search_ms` (separando `scan_ms` e `sort_ms`), `load_ms`/`index_age_ms`/`needs_refresh_ms` no boot, `scan_ms` e `correlate_ms`, tempo até `CatalogLoaded` e até `InstalledLoaded`, contador de `draw` por segundo em repouso.
