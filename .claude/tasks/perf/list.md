# Desempenho de `wingetrs list` — brainstorm (somente leitura, nada medido por mim)

Base medida (única disponível): `docs/superpowers/reports/2026-10-07-fase1-validacao.md:14,37,133-137`:
`list` mediana 404–409 ms; log `scan_ms` 169–266, `correlate_ms` 79–96. Medição anterior à Fase 2a (coletor
`wingetrs` ainda não existia). Hipótese do próprio relatório (`:140-141,199`, F5): MSIX domina o scan, **não medida por coletor**.
Os spans `timed(...)` já existem (`scan_arp_*`, `scan_upgrade_codes`, `scan_msix_*`, `scan_scoop`, `scan_wingetrs`,
`winget_tracking`, `normalizer_new`, `correlate`, `output`); rodar `wingetrs -v list` hoje dá o perfil novo sem código novo.
Tudo abaixo é hipótese "não medido" + o experimento que a mediria.

## 1. Mapa do caminho quente

| # | Etapa | arquivo:linha | Natureza | Paralelismo |
|---|---|---|---|---|
| 0 | `run`: `DataDir::from_env`, `data.ensure()`, `init_logging` (abre arquivo de log), `Config::load` | `src/cli/mod.rs:772-793` | disco, poucos syscalls | serial; só `search` tem thread antecipada (`:775-790`), `list` não |
| 1 | `tokio::runtime::Runtime::new()` (multi-thread, `features=full`) | `src/cli/mod.rs:808`, `Cargo.toml:39` | cria N threads worker | serial, antes do `list` |
| 2 | `Http::new()`: 2 `reqwest::Client` (rustls) construídos sempre | `src/cli/mod.rs:532`, `src/http.rs:46-67` | CPU/alloc (config TLS); não usado no caminho `list` quente | serial |
| 3 | Spawn de 3 threads: `scan_all`, `winget_tracking::load_default`, `NameNormalizer::new` | `src/cli/mod.rs:639-653` | ver 4-6 | só nascem depois de 0-2 |
| 4a | `scan_all` -> `scan_with`: 4 coletores em `thread::scope` | `src/installed/mod.rs:173-199` | n/a | 4 threads |
| 4b | Coletor ARP: `upgrade_code_map` (enumera TODO `HKLM\...\Installer\UpgradeCodes`, abre cada subchave, enumera valores), depois 3 hives em série (x64, x86, HKCU); por chave de app: `open_subkey` + até 8 `get_raw_value` | `src/installed/arp_win.rs:21-42,63-90,92-121` | registro: muitos syscalls pequenos | **1 thread só** para tudo |
| 4c | Coletor MSIX: `FindPackagesByUserSecurityId` (serial) -> `identify` (8 workers) -> dedupe por família -> `read_one` DisplayName/PublisherDisplayName (8 workers) | `src/installed/appx_win.rs:76-111` | WinRT/COM; candidato a mais lento | enumerate serial; resto 8 threads |
| 4d | Coletor Scoop: `read_dir` + 2 `fs::read` + `serde_json::Value` por app | `src/installed/scoop.rs:34-41,9-31` | disco/JSON | serial dentro do coletor |
| 4e | Coletor wingetrs: `StateDb::open` (RW, `create_dir_all`, `PRAGMA journal_mode=WAL`, DDL) + por app `read_install_json` e `StateDb::get` (2 `prepare`) | `src/install/state.rs:250-304,88-96,141-160` | SQLite + disco | serial dentro do coletor |
| 5 | `winget_tracking::load_default`: abre `installed.db` RO (lê 100 bytes de header antes), 1 query + 5 queries de map, monta `Vec<TrackedPackage>` | `src/installed/winget_tracking.rs:102-153,171` | SQLite (conexão ~11-15 ms, ARMADILHAS:26), alocação | 1 thread |
| 6 | `NameNormalizer::new`: compila ~30 regex `fancy_regex` + `fold_and_sort` | `src/installed/normalize.rs:529-584,425-441` | CPU pura | 1 thread |
| 7 | Main thread: `ensure_catalog` -> `Catalog::open_read_only` (+`mmap_size` 256 MiB) + `is_empty` (EXISTS) + `refresh_if_stale` (lê `http_state`, pode `spawn` processo destacado) | `src/cli/mod.rs:389-433,375-380`, `src/catalog/mod.rs:144-152,244-250` | SQLite; processo extra quando "velho" | paralelo com 4-6 |
| 8 | join das 3 threads, `resume_unwind` | `src/cli/mod.rs:657-669` | espera a mais lenta | n/a |
| 9 | `correlate`: por app, serial, 1 conexão: até 3 `lookup` por chave + NormName/NormPublisher + `get`; `prepare` a cada chamada; `index_keys` (NFKC + ~25 regex) por app ARP | `src/installed/correlate.rs:290-322,168-199,121-152,108-119`, `src/catalog/mod.rs:457-490` | CPU (regex) + SQLite (prepare+lookup) | **serial** |
| 10 | filtros + saída (`list_text`/JSON pretty) | `src/cli/mod.rs:682-695`, `src/cli/output.rs:117` | CPU trivial, 1 write | n/a |

Caminho crítico (hipótese, não medido): 0+1+2 -> max(4a [ARP ou MSIX], 5, 6, 7) -> 9 -> 10.

## 2. Ideias

### I1. Disparar scan/tracking/normalizador no início de `run()`, antes de logging/config/runtime/Http
- Onde: `src/cli/mod.rs:772-793` (modelo: `EarlySearch`, `:775-790`); consumo em `:639-669`.
- Mecanismo: hoje as 3 threads só nascem depois de `data.ensure`, `init_logging`, `Config::load`, `Runtime::new` e `Http::new`. Mover o `spawn` para o topo de `run` (como já se faz para `search`) sobrepõe esses passos ao scan (150-266 ms). Cuidado igual ao `EarlySearch`: guard com `Drop`/`join` em todo caminho de erro (regra do projeto). `scan_all` já usa `DataDir::from_env` (`installed/mod.rs:178-180`).
- Recurso: tempo de parede (latência); CPU igual.
- Medir antes: timestamp de início de `run` vs. início de `scan_all` (spans); `bench.ps1 -Baseline noop.exe`, A/B em worktree. Delta esperado = soma dos passos 0-2 (não medido).
- Risco/custo: baixo-médio (ciclo de vida da thread; replicar o padrão `EarlySearch`). Prioridade: **alta**.

### I2. Construir `Http` e/ou runtime tokio só quando necessário
- Onde: `src/cli/mod.rs:532` (`Http::new()?` no topo de `run_async` para todos os comandos), `:808` (`Runtime::new()` multi-thread).
- Mecanismo: `list` com catálogo populado não usa rede (só `refresh_if_stale` dispara processo separado). `Http` preguiçoso (`OnceCell`/closure) evita 2 builders reqwest+rustls; `Builder::new_current_thread()` evita criar N threads worker. `ensure_catalog` só precisa de rede se cair em `run_update`.
- Recurso: CPU, memória, threads criadas.
- Medir antes: span em volta de `Http::new()` e de `Runtime::new()` (hoje sem span); `-Baseline`. Pode ser ~0 (não medido); armadilha "palpite de gargalo falha": só aplicar com delta medido.
- Risco: baixo, mas toca todos os comandos. Prioridade: média (medir primeiro; barato).

### I3. `scan_wingetrs`: leitura somente-leitura e uma query só
- Onde: `src/install/state.rs:250-258` (usa `StateDb::open` = `create_dir_all` + `PRAGMA journal_mode=WAL` + DDL a cada `list`), `:276-279` (`d.get(&key)` por app = 2 `prepare` + carrega `artifacts` que `list` descarta), `:141-160`.
- Mecanismo: abrir `SQLITE_OPEN_READ_ONLY` (`open_existing` em `:100-106` existe, mas é READ_WRITE) e um único `SELECT app, version FROM apps` em `HashMap`, sem tocar `artifacts`. Remove DDL/pragma de escrita da leitura e N x 2 prepares. `read_install_json` (`:243-246`) parseia o JSON inteiro só para `bucket`/`version`.
- Recurso: disco (evita escrita potencial de pragma/DDL), CPU, syscalls; cresce com o nº de apps wingetrs.
- Medir antes: span `scan_wingetrs` já existe (`installed/mod.rs:148`); medir com 0, 5 e 30 apps.
- Risco: baixo (`state.db` é do próprio wingetrs). Teste de equivalência nova função x `get`. Prioridade: média (ganho pequeno hoje; corrige "leitura que escreve").

### I4. ARP: `upgrade_code_map` preguiçoso/filtrado e hives em paralelo
- Onde: `src/installed/arp_win.rs:21-42` (varre todo `UpgradeCodes`, abre cada subchave, monta mapa de todos os produtos do sistema), `:92-121` (3 hives em série numa só thread), `:69-89` (por app: 1 `open_subkey` + até 8 `get_raw_value`).
- Mecanismo: (a) rodar `upgrade_code_map` em paralelo com a leitura das hives e juntar no fim (separar leitura crua de resolução de upgrade code, pois `filter_arp` precisa do mapa); (b) 3 hives em 3 threads via `par::parallel` (`par.rs:6`); (c) resolver upgrade code só para entradas MSI (`WindowsInstaller=1`); (d) um `enum_values` por chave em vez de 8 `get_raw_value`.
- Recurso: tempo de parede e syscalls de registro.
- Medir antes: spans `scan_upgrade_codes`, `scan_arp_x64/x86/user` (`arp_win.rs:60,93`); só vale se ARP estiver no caminho crítico (comparar com `scan_msix_*`).
- Risco: médio (muda a estrutura do coletor; fixtures de ARP em `arp.rs` mantêm equivalência). Prioridade: média, **condicionada ao perfil**.

### I5. MSIX: reduzir o provável caminho crítico (sem repetir a armadilha)
- Onde: `src/installed/appx_win.rs:76-111`.
- Atenção: ARMADILHAS.md:24 diz que **reduzir chamadas `DisplayName` não deu ganho**; não repetir. Alternativas:
  1. Iniciar `PackageManager::new()`/enumeração já no passo I1 (é serial e bloqueante antes dos 8 workers).
  2. Cache (família+versão) -> (nome, publisher) em arquivo próprio no dir de dados: `identify` é barato (propriedades do `PackageId`); `read_one` só roda para famílias cuja versão mudou. Invalidação exata por `FamilyName+Version`.
  3. Variar `WORKERS` (`:9`, 8 fixo): testar 4 vs 8 (heap padrão serializa threads, ARMADILHAS:25). Alocador novo exigiria crate nova, fora do escopo sem brief.
- Recurso: tempo de parede/CPU do COM.
- Medir antes: spans `scan_msix_enumerate/identify/props` (`:77,86,108`) separados; responde F5 do relatório. Só implementar (2) se `props` for fração relevante; A/B contra a base.
- Risco: (2) médio (cache + invalidação; teste de equivalência cache x sem cache obrigatório). Prioridade: média/alta **se** o perfil apontar MSIX.

### I6. Correlação: `prepare_cached` e menos consultas
- Onde: `src/catalog/mod.rs:478-490` (`lookup`: `format!` + `prepare` a cada chamada), `:457-466` (`get` via `query_row` também prepara), `src/installed/correlate.rs:108-119,136-144`.
- Mecanismo: `conn.prepare_cached` evita recompilar SQL de 5 tabelas fixas (ARMADILHAS:26 mede `prepare` como custo). Hoje cada app ARP faz até ProductCode, UpgradeCode, Pfn (3 prepares), NormName (1-2), NormPublisher (1) e `get` (1) = 6-8 prepares.
- Recurso: CPU (parse de SQL), alocação (`format!`).
- Medir antes: span `correlate` (79-96 ms, `cli/mod.rs:674`); contar chamadas a `lookup` por execução; A/B.
- Risco: baixo; resultado idêntico (testes de correlação cobrem). Prioridade: **alta** (poucas linhas; ganho não medido).

### I7. Correlação: memoizar `index_keys` e/ou paralelizar por app
- Onde: `src/installed/correlate.rs:296-320` (loop serial), `:130` e `winget_tracking.rs:58` (`index_keys` do mesmo app em dobro quando há empate), `normalize.rs:599-607` (NFKC + ~25 `fancy_regex` por chamada).
- Mecanismo: (a) calcular `index_keys` uma vez por app e reutilizar; (b) calcular as chaves de todos os apps em paralelo (`par::parallel`; conferir se `NameNormalizer` é `Sync`) e só depois fazer os lookups seriais; (c) lookups em 2-3 conexões RO (ARMADILHAS:26: 8 conexões competem no `prepare`; usar com `prepare_cached`).
- Recurso: tempo de parede; (a) também CPU.
- Medir antes: separar `correlate` em span de normalização (regex) vs. SQLite; se regex dominar, (a)/(b); se SQLite dominar, I6.
- Risco: médio ((b)/(c) mudam a ordem de trabalho; preservar ordem de saída). Exige teste de equivalência serial x paralelo. Prioridade: média.

### I8. `Tracking::ids_for`: índice em vez de varredura linear com `to_lowercase`
- Onde: `src/installed/winget_tracking.rs:40-46,57-77`.
- Mecanismo: para cada app em empate percorre todos os `TrackedPackage` e faz `to_lowercase()` (alocação) em todo valor. Pré-indexar em `HashMap<lowercase, Vec<id>>` por tipo, na thread de load (fora do caminho crítico), dá O(1) por consulta.
- Recurso: CPU e alocação no caminho crítico (só em empates; frequência não medida).
- Medir antes: contar apps que chegam a `resolve_tie` (`correlate.rs:301`) e medir o tempo ali dentro.
- Risco: baixo; teste de equivalência índice x varredura. Prioridade: baixa.

### I9. `winget_tracking`: cache binário próprio
- Onde: `src/installed/winget_tracking.rs:81-92,115-147`.
- Mecanismo: 6 queries com JOIN. Alternativa: `tracking.bin` invalidado por (mtime, size) do `installed.db`, evitando abrir o SQLite de terceiros (11-15 ms de conexão+1ª query, ARMADILHAS:26). Provavelmente fora do caminho crítico (roda em paralelo ao scan).
- Medir antes: span `winget_tracking` (`cli/mod.rs:643`) vs. `scan_all`; se menor, ganho de parede = 0 (economiza só CPU/disco).
- Risco: médio (cache + invalidação; o `installed.db` continua só leitura). Prioridade: baixa.

### I10. `NameNormalizer::new`: compilar menos regex
- Onde: `src/installed/normalize.rs:529-584`, `Re::new` `:432-441`.
- Mecanismo: ~30 regex `fancy_regex` compiladas por processo (CPU pura, thread paralela ao scan). `OnceLock` não ajuda em processo curto. Trocar padrões sem lookaround por `regex` exigiria validar cada padrão (risco de mudar semântica) e crate nova sem brief. Realista: só medir.
- Medir antes: span `normalizer_new` (`cli/mod.rs:651`); se for menor que o scan, está fora do caminho crítico: **não mexer**.
- Prioridade: baixa.

### I11. Cache do resultado final de `list` com fingerprints de invalidação
- Onde: `src/cli/mod.rs:639-695` (envolver 3-9).
- Mecanismo: persistir `Vec<ListRow>` com chave = (versão do catálogo, `lpftLastWriteTime` das chaves Uninstall via `RegQueryInfoKey`, mtime/size de `installed.db`, mtime dos dirs `apps` do scoop e do wingetrs, lista de `FullName` MSIX). Hit pula scan+correlate (~250-350 ms de ~405 ms medidos em 2026-10-07).
- Recurso: tudo (CPU, disco, threads); `list` vira ~piso do processo.
- Medir antes: custo de calcular os fingerprints (a enumeração MSIX é parte do custo atual); taxa de falso hit; teste de equivalência cache x fresco.
- Risco: **alto** (instalação por outro instalador deve invalidar; "otimização que muda a resposta é bug"). Prioridade: baixa/exploratória, só depois de I1-I6.

### I12. Processo destacado `update --background` em `refresh_if_stale`
- Onde: `src/cli/mod.rs:375-380,348-366,431`.
- Mecanismo: com fonte "velha", cada `list` pode spawnar outro `wingetrs.exe` (CreateProcess + disputa de CPU/disco com o próprio `list`). Verificar se `needs_refresh` evita relançamentos até o outro processo gravar `http_state`, e se o lock impede N processos simultâneos.
- Medir antes: `scripts/leftovers.ps1` e contagem de `wingetrs.exe` após 10 `list` seguidos com estado velho (distorce benchmarks, ARMADILHAS:29).
- Risco: baixo; prioridade baixa (investigação).

### I13. Detalhes menores no scan
- `collect_msix` aplica `dedupe_msix_by_family` (`installed/mod.rs:136-139`) sobre saída de `read_appx` que já deduplica por família (`appx_win.rs:87-107`): passada redundante, barata. `appx_win.rs:91-98`: `order.iter().position` é O(n^2) no pior caso e `to_ascii_lowercase()` duas vezes por candidato; higiene.
- Scoop `read_one` (`scoop.rs:9-31`) lê 2 JSONs inteiros para extrair `version`/`bucket` e aloca árvore `Value`; structs só com os campos reduzem alocação (ARMADILHAS:25). Medir `scan_scoop` com o nº real de apps antes.
- Saída: `to_string_pretty` + `println!` (`cli/mod.rs:690`) é um write; `list_text` clona strings (`output.rs:137-144`); desprezível.

## 3. Desperdícios evidentes

1. **Início tardio do trabalho do `list`**: scan/tracking/normalizador só começam depois de logging, config, `Runtime::new` e `Http::new` (`cli/mod.rs:772-808,532,639`), enquanto `search` já tem thread antecipada (`:775`). (I1)
2. **`Http::new()` constrói dois clientes reqwest+rustls para todo comando**, inclusive os que não tocam a rede (`cli/mod.rs:532`, `http.rs:50-61`). Custo não medido. (I2)
3. **Leitura que escreve**: `scan_wingetrs` usa `StateDb::open` com `create_dir_all`, `PRAGMA journal_mode=WAL` e DDL a cada `list` (`install/state.rs:88-96,254-258`), mais `get` com 2 prepares por app descartando `artifacts`. (I3)
4. **`prepare` a cada consulta** em `Catalog::lookup/get`, 6-8 por app ARP (`catalog/mod.rs:457-490`). (I6)
5. **`index_keys` (NFKC + ~25 regex) recomputado** para o mesmo app em `by_name_and_publisher` e `Tracking::ids_for` (`correlate.rs:130`, `winget_tracking.rs:58`). (I7a)
6. **`upgrade_code_map` varre o sistema inteiro** mesmo que poucos ARP precisem de UpgradeCode, e as 3 hives ARP rodam em série dentro de um único dos 4 coletores (`arp_win.rs:21-42,92-121`). (I4)
7. **Dedupe MSIX duplicado** (`installed/mod.rs:138` sobre saída já deduplicada). Higiene. (I13)
8. **`ids_for` com `to_lowercase` por valor por consulta** em varredura linear (`winget_tracking.rs:40-46`). (I8)

## 4. Top 3 recomendado

1. **Medir primeiro (sem código novo)**: `wingetrs -v list` com os spans existentes, ~15 execuções com `-Baseline`, para fechar F5 (qual coletor domina: `scan_arp_*`, `scan_msix_*` ou `scan_wingetrs`) e dividir `correlate` em regex vs. SQLite. Decide I4/I5/I7 e evita otimizar o lugar errado.
2. **I1: thread antecipada no topo de `run()` para `list`** (mais I2 se os spans de `Http::new`/`Runtime::new` mostrarem custo): põe os passos 0-2 dentro da janela do scan; resultado idêntico, ganho não medido.
3. **I6: `prepare_cached` em `Catalog::lookup/get`** (+ I3, `scan_wingetrs` RO com uma query): mudanças pequenas, resultado idêntico, atacam `correlate` (79-96 ms em 2026-10-07) e removem escrita numa operação de leitura. Validar cada uma por A/B com `bench.ps1 -Baseline`.

Não recomendado agora: I11 (risco de corretude alto), I9/I10 (fora do caminho crítico até prova em contrário), reduzir chamadas `DisplayName` do MSIX (ARMADILHAS:24, já refutado).
