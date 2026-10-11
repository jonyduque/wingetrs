# Revisão de desempenho: `wingetrs search <termo>`

Somente leitura: nada foi compilado nem medido nesta revisão. Todo ganho abaixo é hipótese **não medida**; cada ideia traz o experimento que a mediria. Números existentes (ledger `.claude/tasks/progress.md`, linhas das Tasks 16, 18, 19, 19-bench): índice `search.idx` com `search powertoys` = delta 30,6–41,5 ms sobre o `noop` (absoluto 69–81 ms); sem índice (SQLite) delta 47,6–55,6 ms; `index_file::search` em processo 28–35 ms; exe carrega ~15 ms acima de hello-world; nucleo em memória ~17 ms single-thread (relatório fase 1, 19,5 mil entradas); `search.idx` real = 3,0 MB.

Já refutado/medido em `docs/ARMADILHAS.md` e portanto NÃO proposto: reduzir alocações por cópia em leitura em massa (heap do Windows serializa), mais threads com heap padrão, conexões SQLite paralelas, `COUNT(*)`, abrir SQLite para busca quente, `Catalog::open` com DDL, palpites sem span.

## 1. Mapa do caminho quente (índice válido, catálogo existente)

Ordem real (processo único; a thread antecipada roda em paralelo ao setup):

1. `src/main.rs:4` `Cli::parse()` (clap derive com todos os subcomandos) e `cli::run`.
2. `src/cli/mod.rs:773` `DataDir::from_env()` (crate `directories`, consulta de pasta conhecida do Windows).
3. `src/cli/mod.rs:775-790` `data.catalog_path().exists()` (1 stat) e `std::thread::spawn` da busca antecipada (`EarlySearch`).
4. Thread antecipada, `search_early` (`mod.rs:502`) -> `index_file::search` (`src/catalog/index_file.rs:359`):
   - `read_retrying` (`index_file.rs:339`): `std::fs::read` do arquivo inteiro (3,0 MB: 1 alocação grande, cópia do cache de páginas, ~750 page faults).
   - `IndexView::parse` (`:165`): valida cabeçalho/trailer e percorre os N offsets (19,5 mil) conferindo ordem. CPU leve, sequencial.
   - `Query::new` (`search.rs:44`): `to_lowercase` + `Pattern::parse`.
   - Até 8 threads `std::thread::scope` (`index_file.rs:385-396`), fatias contíguas fixas; cada uma cria `Matcher::new` e roda `scan` (`:305`).
   - `scan` por registro: `raw(i)` (`:216`) caminha os 9 campos com checagem de limites, incluindo version/publisher/description, que a pontuação não usa; `scoring` (`:257`) roda `from_utf8` em id/name/moniker/tags/commands; `score_entry` (`search.rs:73`) -> `score_fields` (`:93`): `Pattern::indices` no nome (com `Vec idx` + sort + dedup), `Pattern::score` em id, moniker e CADA palavra de tags e commands (`split_whitespace`); por acerto aloca `Scored` com `name_indices` e `lower_name = to_lowercase()` (`index_file.rs:326`).
   - `sort_ranked` (`search.rs:246`): ordenação estável de TODOS os acertos com comparação por nome em minúsculas, depois `truncate(limit)`.
   - `view.entry(i)` só para os top `limit` (`index_file.rs:411-418`), bom.
   - Drop do `Vec<u8>` de 3 MB ao sair da função.
5. Thread principal em paralelo: `data.ensure()` (`mod.rs:791` -> `cache/mod.rs:92`: 4x `create_dir_all`), `init_logging` (`mod.rs:321`: abre log diário, cria thread de escrita não bloqueante), `Config::load` (`mod.rs:793`, lê `config.toml`), `tokio::runtime::Runtime::new()` (`mod.rs:808`, multi-thread, `features = ["full"]`), `Http::new()` (`mod.rs:532`, constrói cliente reqwest/rustls; 0,4 ms medido na fase 1).
6. `run_async` -> `Command::Search` (`mod.rs:545-580`): `catalog_path().exists()` de novo + `index_file::probe` (`index_file.rs:99`: abre o `search.idx` uma segunda vez, lê 24 bytes) + `Endpoints::production(&cfg)` + `refresh_if_stale` (`mod.rs:375`: lê e faz parse de `http.json` pretty-printed; se defasado (>900 s, `update.rs:12`), `spawn_background_update` (`mod.rs:348`) cria um processo novo `wingetrs update --background`).
7. `EarlySearch::join` (`mod.rs:562`), `search_table` (`cli/output.rs:12`, alocações `String` por linha) e `print!`.
8. Saída: `Ok(())` -> drop do runtime tokio (desliga N workers), drop do `WorkerGuard` do log (junta a thread de log), teardown do processo.

Fallback sem índice: `search_on_disk` (`search.rs:332`): `open_read_only` + `data_version` + `rowid_bounds` + até 8 conexões (11–15 ms cada, armadilha medida) + `entries_by_rowid`.

## 2. Ideias

Legenda de medição: `S` = spans `tracing` por etapa (fora do caminho quente de produção, ligados por env/feature de teste); `B` = `scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'search powertoys' -Runs 15 -Baseline target/release/examples/noop.exe`; `AB` = B contra a base em worktree temporário. Rodar `scripts/leftovers.ps1` antes.

### Ideia 1 (alta): primeiro, um perfil de etapas do processo inteiro (pré-requisito, não otimização)
- O quê: spans/`Instant` por etapa de 1 a 8 do mapa, impressos num teste/exemplo (`examples/`), inclusive o tempo até `main` e o tempo de saída (drop de runtime/guard).
- Onde: `cli/mod.rs:773-810`, `index_file.rs:359-420`.
- Por quê: o delta de 30–41 ms não está decomposto. In-process é 28–35 ms, mas só ~17 ms é nucleo single-thread; read, parse, spawn de threads e merge são desconhecidos. Armadilha "palpite de gargalo falha".
- Recurso: nenhum direto; define o ranking das demais.
- Como medir: S + B.
- Risco: baixo. Prioridade: alta.

### Ideia 2 (alta): fast path de `search` que não inicializa o que a busca não usa
- O quê: com índice válido, `search` não precisa de `tokio::Runtime::new()` multi-thread, `Http::new()`, `Config::load` completo, `init_logging` (cria thread + abre arquivo de log só para não escrever nada), nem os 4 `create_dir_all`. Opções: runtime `current_thread` ou nenhum runtime (só construir quando `ensure_catalog_ran` precisar atualizar); logging preguiçoso (inicializar apenas se houver `warn`/`-v`); `ensure()` só quando faltar o diretório (um `exists` no root cobre o caso comum, ou fazer no `update`).
- Onde: `cli/mod.rs:532` (`Http::new`), `:791-793`, `:792` (`init_logging`), `:808` (`Runtime::new`), `cache/mod.rs:92`.
- Mecanismo: elimina criação de N threads de worker do tokio (eager), a thread do appender, ~4 syscalls de diretório e a abertura do arquivo de log; reduz CPU total e o tempo de teardown (menos threads para juntar na saída). Compete menos com os 8 workers da busca nos mesmos núcleos.
- Recurso: processos/threads, CPU, syscalls de disco.
- Medir: S de cada etapa de setup; depois AB. Ganho: não medido.
- Risco: médio (o guard de log precisa continuar sendo juntado em todos os caminhos, regra do projeto; manter `warn` do índice via `log_warning`, hoje gravado depois do join em `mod.rs:563`). Prioridade: alta.

### Ideia 3 (alta): back-off para a atualização em segundo plano
- O quê: `refresh_if_stale` dispara `spawn_background_update` sempre que alguma URL tem `checked_at` antigo (`update.rs:80-85`). Falhas (offline, `update.rs:199,278`) NÃO atualizam `checked_at` (só o sucesso em `:202,:271`), então enquanto estiver offline ou o CDN falhar, CADA `search` cria um processo novo (CreateProcess + runtime + tentativa de rede). Registrar `last_attempt` (no `http.json` ou arquivo próprio/mtime de um marcador) e só relançar após um intervalo (ex.: 5–15 min, valor a decidir).
- Onde: `cli/mod.rs:375-380`, `update.rs:80,199,278`, `cache/mod.rs:105-117` (`HttpEntry`).
- Mecanismo: elimina spawns repetidos sem utilidade; o custo de criar processo no Windows é alto e a thread principal paga o `CreateProcess` antes de imprimir.
- Recurso: processos, CPU, rede, disco (lock/arquivos).
- Medir: contar spawns em 20 buscas seguidas com rede bloqueada (log de `tracing` com contador ou `leftovers.ps1`); custo do spawn por S em `spawn_background_update`. Ganho: não medido.
- Risco: médio (muda semântica de atualização; precisa teste com `HttpState` sintético: tentativa recente suprime, tentativa antiga não; e que falha não marca como fresco para o `needs_refresh` de outros comandos). Prioridade: alta (corrige comportamento patológico, não só velocidade).

### Ideia 4 (média): disparar o `spawn_background_update` depois de imprimir
- O quê: mover `refresh_if_stale` (leitura+parse de `http.json` e `CreateProcess`) para depois do `print!` do resultado, ou para a thread antecipada em paralelo.
- Onde: `cli/mod.rs:555-556` e `:571-576`.
- Mecanismo: tira do caminho crítico até a saída um trabalho que não afeta o resultado. Mantém a saída idêntica.
- Recurso: latência percebida (não reduz CPU total).
- Medir: S em `refresh_if_stale`; B.
- Risco: baixo (cuidado: stdout deve ser descarregado antes; processo filho é destacado). Prioridade: média.

### Ideia 5 (média): `probe` e `exists` redundantes
- O quê: `search` abre `search.idx` duas vezes (probe em `mod.rs:553`, depois leitura inteira na thread antecipada `index_file.rs:365`) e faz `catalog_path().exists()` duas vezes (`mod.rs:781` e `:552`). Reaproveitar: a thread antecipada devolve `Ok(Some)` com a contagem; o `n > 0` pode vir do resultado do `IndexView` (hoje o resultado de uma busca vazia é indistinguível de "índice vazio", então devolver `(Vec<Found>, entry_count)`).
- Mecanismo: elimina 1 `CreateFile`+leitura de 24 bytes+`stat`s; sob antivírus (hook em abertura de arquivo) cada abertura pode custar muito mais que o syscall nu, mas isso é hipótese.
- Recurso: syscalls/I-O.
- Medir: S em `probe`; contar aberturas com Process Monitor numa sessão sem outro agente.
- Risco: baixo-médio (muda ordem de decisão `ensure_catalog_ran`: manter o teste "índice vazio -> cair em ensure_catalog"). Prioridade: média.

### Ideia 6 (média): selecionar top-k em vez de ordenar todos os acertos
- O quê: com `limit` pequeno (padrão do CLI a confirmar na definição do `Command::Search`; grep de `limit:` não achou o default no `cli/mod.rs`), usar `select_nth_unstable_by` / heap de tamanho `limit`, com desempate final por índice de catálogo para reproduzir a estabilidade de `sort_ranked`.
- Onde: `search.rs:246-261` (`sort_ranked`, único ponto compartilhado pelos três caminhos), chamado de `search.rs:233`, `:389`, `index_file.rs:404`.
- Mecanismo: O(n log k) em vez de O(n log n) quando há muitos acertos (consultas curtas como `a`, `code`). Para `powertoys` (poucos acertos) o ganho é nulo.
- Recurso: CPU, em consultas largas.
- Medir: B com consultas de 1, 2 e 4 letras além de `powertoys`; contar acertos.
- Equivalência: por estar em `sort_ranked` os três caminhos mudam juntos; adicionar teste que compara `sort_ranked` novo com o antigo (sort completo + truncate) em entradas com empates de score e de nome, nos três caminhos (já existe teste de equivalência; estendê-lo com `limit` < acertos e ties). Mutação: remover o desempate por índice.
- Risco: médio. Prioridade: média.

### Ideia 7 (média): calcular `lower_name` e `name_indices` só para o que sobrevive
- O quê: hoje cada acerto aloca `lower_name` (`index_file.rs:326`, `search.rs:321`) e `name_indices` (`search.rs:107-117`, `Pattern::indices` + `sort_unstable` + `dedup`). Pontuar com `Pattern::score` (sem índices) na fase de varredura e recalcular `indices` só para os top `limit`; `lower_name` só quando houver empate de score (comparar com `eq_ignore_ascii_case`/`to_lowercase` sob demanda a partir do `&str` emprestado do `IndexView`, que vive durante a busca).
- Mecanismo: elimina 2 alocações por acerto e o custo extra de `indices` (mais caro que `score` no nucleo) para acertos descartados. Alinhado com a armadilha "alocação domina".
- Recurso: CPU, alocações, serialização no heap entre threads.
- Medir: contar acertos de `powertoys` vs consultas largas; S na fase de varredura vs fase de ordenação; B em consulta larga (para `powertoys`, ganho esperado pequeno, não medido).
- Equivalência: ESSENCIAL. Precisa provar que `Pattern::score` == o score retornado por `Pattern::indices` para as mesmas entradas (nucleo implementa os dois com o mesmo algoritmo, mas verificar com teste de propriedade sobre as fixtures do catálogo comparando `score` e `indices` por entrada); manter um único `score_fields` para os três caminhos. Os testes de equivalência existentes (memória x SQLite x idx) cobrem o resultado final.
- Risco: médio. Prioridade: média (alta só se S mostrar que a varredura de consultas largas domina).

### Ideia 8 (média): formato do `search.idx` em duas seções, quente e fria
- O quê: gravar primeiro as colunas de pontuação (kind, id, name, moniker, tags, commands, e opcionalmente `lower_name` pré-calculado) e depois version/publisher/description/source_name. A busca lê/percorre só a seção quente e busca a fria por offset apenas para o top `limit`. Alternativamente guardar, por registro, os 5 offsets de campos de pontuação para `raw()` não caminhar descrição etc.
- Onde: `index_file.rs:40-96` (write), `:216-255` (`raw`), `:359-420`; mudar `MAGIC` (`:11`) para invalidar índices velhos (já cai no fallback com `index_warning`).
- Mecanismo: menos bytes percorridos e menos page faults no scan; descrição é provavelmente o maior campo dos 3,0 MB (não medido), logo a seção quente pode ser bem menor, melhorando uso de cache L2/L3 dos 8 workers.
- Recurso: memória (RSS), I/O de leitura, CPU.
- Medir: tamanho por campo (script que soma bytes por coluna do `search.idx` real, leitura só do cabeçalho/offsets, sem `cat` do binário); depois AB.
- Equivalência: o teste que compara `index_file::search` com a busca em memória continua valendo; adicionar teste de ida e volta do novo layout e fixture que falha só na validação da seção fria (regra "cada checagem tem fixture").
- Risco: médio-alto (formato, validação de trailer, escritor atômico em `update.rs:247` e `buckets.rs:200`). Prioridade: média.

### Ideia 9 (média): leitura do índice sem cópia (mmap) ou leitura parcial
- O quê: ler só a seção quente (Ideia 8) com `read`, ou mapear o arquivo. A Task 16 já usou mmap no catálogo; aqui há um conflito específico: o escritor troca o arquivo por `rename` (`index_file.rs:82`) e no Windows um mapeamento aberto faz o `rename` falhar (já há retry de 20 ms; ver `idx-flaky` no ledger com "Acesso negado" real). Por isso a leitura atual copia tudo.
- Mecanismo: evita cópia de 3 MB do cache de páginas para o heap e o `VirtualFree` posterior.
- Recurso: memória, CPU de cópia (~3 MB).
- Medir: S em `read_retrying` isolado (esperado ordem de 1 ms; se for, descartar).
- Risco: alto (unsafe, concorrência com o `rename`; pode piorar a flakiness). Prioridade: baixa, só se S mostrar leitura relevante.

### Ideia 10 (média-baixa): pré-filtro por máscara de caracteres por registro
- O quê: gravar no índice uma máscara de 64 bits (letras a-z, dígitos, alguns separadores) com a união dos caracteres de id/name/moniker/tags/commands em minúsculas; antes de chamar nucleo, descartar o registro se faltar algum caractere ASCII da consulta (condição necessária para qualquer átomo fuzzy/prefix/substring/exact do nucleo). Desligar o pré-filtro se a consulta tiver caracteres não ASCII ou átomos negados (`!`), que casam por ausência.
- Mecanismo: pula `Utf32Str::new` + 5+ chamadas por registro nos que não casam. Ressalva: o nucleo já tem pré-filtro ASCII interno que sai rápido, então o ganho pode ser pequeno; a ideia só vale se S mostrar muito tempo em `score_fields` para registros sem acerto.
- Recurso: CPU.
- Medir: contar tempo de `score_fields` em registros que não casam (S com contadores), ou micro-bench single-thread com e sem a máscara.
- Equivalência: teste de propriedade (máscara nunca descarta um registro que o nucleo pontuaria) sobre todo o catálogo de fixture e consultas variadas, incluindo aspas/`^`/`$`/`!`/maiúsculas/acentos. Manter caminhos SQLite e memória SEM máscara só funciona se o resultado for idêntico; é o ponto da propriedade.
- Risco: médio. Prioridade: baixa-média.

### Ideia 11 (média-baixa): balanceamento dinâmico entre workers e nº de workers
- O quê: fatias fixas contíguas (`index_file.rs:383-391`, `search.rs:212`) podem deixar uma thread com mais acertos (mais alocações) que outras; usar fila atômica de blocos (já usada em MSIX no list, Task 20) e/ou limitar workers pelo tamanho (ex.: 4 em vez de 8 para 19,5 mil entradas). Custo de criar/juntar 8 threads no Windows não está medido para este trabalho de ~17 ms.
- Recurso: CPU total (menos criação de threads), latência de cauda.
- Medir: S com tempo por worker e total de `thread::scope`; varrer `workers` ∈ {1,2,4,8} em B (14+ execuções intercaladas, `-Baseline`). `PARALLEL_MIN = 4096` (`index_file.rs:15`, `search.rs:34`) também tem sentido ser medido com o tamanho real.
- Equivalência: a ordem final não depende de workers (já testado por `search_with_threads`); manter o teste com 1, 3 e 8.
- Risco: baixo. Prioridade: baixa-média.

### Ideia 12 (baixa-média): sair do processo sem teardown
- O quê: após `print!` e descarga de stdout/log, encerrar com `std::process::exit` (como já se faz com `std::mem::forget(catalog)` em `mod.rs:578`), pulando o drop do runtime tokio e o join da thread de log, mais o `Vec<u8>` do índice. Sinergia com a Ideia 2 (que diminui o que há para derrubar).
- Onde: `cli/mod.rs:577-579`, `main.rs:5-11`.
- Risco: médio (o guard de log precisa ser descarregado explicitamente; regra de threads juntadas em todos os caminhos; cobrir com teste de integração que confere o log no arquivo). Medir: S do tempo entre o `print!` e o fim do processo (Measure-Command do processo menos o tempo interno). Prioridade: baixa-média.

### Ideia 13 (baixa-média): reduzir o piso do binário (15 ms acima de hello-world, medido na Task 18)
- O quê: investigar o que o exe paga antes de `main` (tamanho, relocações, DLLs importadas pelo crate `windows`/`winreg`/`tokio full`/`ratatui`, inicialização de clap). Experimentos: `[profile.release]` (`Cargo.toml:71-74`) com `panic = "abort"`, `opt-level`, `lto = "fat"`; comparar tamanho do exe e `noop` vs `--version`; `features` de tokio mais enxutas (`full` -> só o que for usado); mover a CLI de busca para um binário separado e minúsculo só com `search` (sem tokio/reqwest/ratatui/rusqlite) que cai para o binário principal quando o índice falta.
- Mecanismo: menos páginas a mapear/relocar e menos construtores estáticos no startup; o `--version` ≈ 44 ms vs `noop` 38 ms já indica custo fixo do exe, independente da busca.
- Recurso: tempo de carga, memória.
- Medir: `bench.ps1 -Args '--version'` contra `noop.exe` para cada variante de perfil (AB em worktree). Atenção: `panic = "abort"` quebra `resume_unwind`/`catch_unwind` que o projeto usa (regra de propagação de pânico, teste `early_search_propagates_worker_panic` em `mod.rs:817`); descartar essa opção ou reescrever a regra. Mudar perfil de release provavelmente precisa de aprovação do orquestrador (não adicionar crates).
- Risco: médio-alto no binário separado (duplicação de lógica de caminho/dados). Prioridade: baixa-média; maior teto de ganho possível, pois o piso é 38 ms dos ~70–80 ms absolutos.

### Ideia 14 (baixa): `http.json` e `config.toml`
- O quê: `refresh_if_stale` faz parse completo de `http.json` pretty-printed (`cache/mod.rs:121-132`, `:136` usa `to_vec_pretty`) a cada busca; usar JSON compacto ou checar primeiro o `mtime` do arquivo contra `MAX_AGE_SECS` como atalho conservador (arquivo gravado há menos de 900 s e nenhuma URL nova na config -> fresco).
- Equivalência: o atalho só pode dizer "fresco" quando o resultado completo também diria; testar com estado sintético e URL nova (bucket adicionado) forçando o caminho lento.
- Medir: S de `refresh_if_stale` (esperado: sub-milissegundo; provável descarte). Prioridade: baixa.

### Ideia 15 (baixa): saída
- O quê: `search_table` (`cli/output.rs:12`) cria `Vec<String>` + `join` por linha e conta `chars()` duas vezes; `limit` linhas apenas, custo desprezível. Só mexer se S mostrar. Prioridade: baixa (listada para registro de que foi avaliada).

## 3. Desperdícios evidentes (sem precisar medir para concluir que são trabalho inútil)

1. **Spawn repetido em offline**: toda busca com rede indisponível recria um processo `update --background` (`mod.rs:375-380`, `update.rs:199,278` sem marcar tentativa). Ver Ideia 3.
2. **Setup que a busca não usa**: runtime tokio multi-thread, cliente HTTP, thread de log, 4 `create_dir_all` e `Config::load` rodam antes/ao lado da busca mesmo quando o índice responde sozinho (`mod.rs:532,791-793,808`).
3. **Dupla abertura do `search.idx` e dupla checagem de `catalog.db`** (`mod.rs:553`+`:781`+`:552`+`index_file.rs:365`).
4. **Campos não usados na pontuação percorridos e validados em todo registro** (`raw()` caminha version/publisher/description; `index_file.rs:216-255`) e **lidos do disco** (3,0 MB totais).
5. **Alocações por acerto na fase de varredura**: `lower_name` e `name_indices` para todos os acertos, mesmo os que serão descartados pelo `truncate(limit)` (`index_file.rs:326`, `search.rs:107-117,321`).
6. **Ordenação completa** de todos os acertos para mostrar `limit` (`search.rs:246-261`).
7. **Teardown de threads na saída** (runtime tokio, appender) enquanto o próprio código já faz `mem::forget(catalog)` para evitar o mesmo tipo de custo (`mod.rs:577-578`).

## 4. Top 3 recomendado

1. **Ideia 1 + 2**: medir por etapa e então tirar do caminho do `search` tudo que o índice não usa (runtime tokio, `Http`, log, `ensure`). Maior item controlável de CPU/threads/processos e ataca o custo fixo fora da busca.
2. **Ideia 3**: back-off da atualização em segundo plano (corrige processo extra a cada busca offline; é também o maior desperdício de processos).
3. **Ideia 8 (com 7)**: índice em seções quente/fria e pontuação só com `score` + `indices`/`lower_name` apenas para o top `limit`; reduz bytes lidos, page faults e alocações por acerto. Condicionado ao perfil da Ideia 1 mostrar leitura+varredura como fatia relevante; senão, trocar pelo item 13 (piso do binário).

Nota de método: nenhuma estimativa de ganho foi dada em ms porque não há medição que a sustente; os experimentos acima são a ordem sugerida (1 -> 2/3 -> 8 -> 13).
