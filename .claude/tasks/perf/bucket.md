# Revisão de desempenho: `bucket add | list | rm`

Somente leitura; nada foi compilado nem medido. Todo ganho abaixo é hipótese "não medido".

Achado de enquadramento: `bucket add` NÃO baixa nem indexa nada. Só edita `config.toml`
(`src/cli/mod.rs:748` imprime "Rode `wingetrs update`"). O download/indexação do bucket é do
`update` (`src/update.rs:269` -> `Catalog::replace_source`, `src/catalog/mod.rs:301`).
Só `rm` toca o catálogo SQLite e o `search.idx`.

## 1. Mapa do caminho quente

Prefixo comum a add/list/rm (`src/cli/mod.rs:772-809` `run`, e `:526-532` `run_async`):
1. `DataDir::from_env` + `data.ensure()` (`:773`, `:791`): syscalls de diretório.
2. `init_logging` (`:792`, def. `:321-343`): `rolling::daily` abre/cria arquivo de log + `non_blocking` cria thread de worker + registry global, mesmo para `list`.
3. `Config::load` (`:793`, `src/config.rs:46`): leitura + parse TOML (barato).
4. `tokio::runtime::Runtime::new()` (`:808`): runtime multi-thread (1 worker por CPU) criado antes do `match`.
5. `Http::new()` (`:532`, `src/http.rs:46-67`): constrói DOIS `reqwest::Client` (client + dl_client), incluindo setup de TLS/raízes de certificados. Nenhum dos comandos de bucket usa rede.
6. `run_bucket` (`:718`).

- `list` (`:721-726`): `buckets::resolved` (`src/buckets.rs:88`), O(n) em memória com `find` aninhado (irrelevante); `println!` por linha.
- `add` (`:727-754`, `src/buckets.rs:108`): `valid_part`, `parse_repo`, `resolved` (clona Strings), push em `cfg`; depois `cfg.save` (`src/config.rs:64`: toml pretty + tmp + rename). Sem catálogo, sem rede.
- `rm` (`:751`, `src/buckets.rs:182-216` `remove_and_cleanup`):
  a. `resolved` + `remove` (`:160`): abre `state.db` read-only (`:146`; conexão custa ~11-15 ms segundo ARMADILHAS) só para um `SELECT app ... WHERE bucket`.
  b. `Catalog::open(catalog_path)` (`:195`, `src/catalog/mod.rs:227-239`): conexão de escrita + `PRAGMA journal_mode=WAL` (escrita) + `mmap_size` + `execute_batch(SCHEMA)` (10 DDL `IF NOT EXISTS`, incl. 5 índices).
  c. `remove_source` (`mod.rs:293-299`): transação -> `delete_source_rows` (`:169-180`): 5 `DELETE FROM map_* WHERE package IN (SELECT rowid FROM packages WHERE source=?)` + `DELETE FROM packages WHERE source=?` + `DELETE FROM meta`; commit (fsync do WAL).
  d. `index_file::write` (`src/catalog/index_file.rs:40-96`): `catalog.load_entries()` (`mod.rs:394-439`) RELÊ O CATÁLOGO INTEIRO (winget ~19,5 mil + scoop restantes): `PRAGMA data_version` x2, `rowid_bounds`, até 8 threads, cada uma com sua conexão (`open_worker_connection`, 11-15 ms cada, competem no prepare), materializa `Vec<PackageEntry>` com `String`s alocadas (ARMADILHAS: alocação domina), re-serializa tudo em `Vec<u8>` (body + offsets + out: ~3 cópias), `fs::write` do `.tmp` inteiro, rename com até 5 tentativas x 20 ms de sleep.
  e. `HttpState::load` + `remove` + `save` de `http.json` (`:207-213`): leitura/parse/escrita do JSON inteiro.
  f. `cfg.save`.

Custo previsto: `list` e `add` ~ piso de processo + prefixo (itens 2, 4, 5). `rm` = prefixo + b + c + d, com d provavelmente dominante (hipótese; medir).

## 2. Ideias

Formato: o quê / onde / mecanismo / recurso / como medir / risco / prioridade.

**1. Reescrever `search.idx` no `rm` filtrando o próprio arquivo, sem passar pelo SQLite.**
- Onde: `src/buckets.rs:196-203`, `src/catalog/index_file.rs:40` (+ nova `index_file::drop_source`).
- Mecanismo: o `search.idx` já guarda `source_name` por registro (`index_file.rs:52`). Ler o arquivo atual, copiar só registros cujo `source_name != scoop:<nome>`, recalcular offsets e gravar. Evita `load_entries` (8 conexões, N `String`s, `Vec<PackageEntry>`) e o `join(" ")` de tags/commands. Fallback: arquivo ausente/inválido => apagar (como já se faz em falha) ou caminho atual.
- Recurso: CPU, memória (pico de heap), leitura do .db, threads/conexões.
- Medir: span `tracing` em volta de `index_file::write` dentro de `remove_and_cleanup`; `scripts/bench.ps1` exige estado repetível: preparar `bucket add extras` + `update` (ou catálogo-fixture) antes de cada rodada e cronometrar só o `rm`, com `-Baseline target/release/examples/noop.exe`; A/B contra a base em worktree.
- Risco: dois caminhos para o mesmo resultado => teste de equivalência obrigatório (arquivo filtrado == `write` do catálogo após `remove_source`). Médio.
- Prioridade: ALTA (provável maior fatia do `rm`; não medido).

**2. Pular reescrita do índice e a transação quando a fonte não tem linhas.**
- Onde: `src/buckets.rs:195-203`; `remove_source` já devolve `usize` (`mod.rs:293`) e o valor é descartado.
- Mecanismo: se `n == 0` (bucket adicionado e nunca `update`d, caso comum logo após `add`), não regrava `search.idx`. Melhor: checar antes com `EXISTS (SELECT 1 FROM packages WHERE source=?)` e nem abrir transação de escrita.
- Recurso: CPU, disco, fsync, latência.
- Medir: `bucket add X` + `bucket rm X` sem `update` no meio; delta com e sem a guarda.
- Risco: baixo. Teste: contagem 0 não toca o mtime do `.idx`.
- Prioridade: ALTA (barato).

**3. Tirar `Http::new()`, runtime tokio e (talvez) logging do caminho de `bucket`.**
- Onde: `src/cli/mod.rs:532` (Http), `:808` (Runtime), `:792` (logging); `run_bucket` é síncrono (`:718`).
- Mecanismo: despachar `Command::Bucket` em `run()` antes de criar runtime e Http (construção de 2 `reqwest::Client` carrega raízes TLS; custo não medido); ou `Runtime::new_current_thread`/`Http` preguiçoso para os demais comandos offline.
- Recurso: CPU de startup, threads (um worker tokio por CPU), memória.
- Medir: spans em `run` (ensure, init_logging, Config::load, Runtime::new, Http::new); `bench.ps1 -Args 'bucket list' -Baseline noop.exe` antes/depois. Primeiro medir `Http::new()` isolado (pode ser sub-ms; ARMADILHAS lembra que palpite falha).
- Risco: baixo; `cli/mod.rs` é compartilhado (conflito de merge). Prioridade: MÉDIA-ALTA (afeta list/add/rm e outros comandos offline).

**4. Reduzir o custo da conexão ao `state.db` no `rm`.**
- Onde: `src/buckets.rs:141-156`.
- Mecanismo: abrir SQLite custa 11-15 ms (ARMADILHAS). Opções: flags `SQLITE_OPEN_NO_MUTEX`; ou manter lista de buckets em uso em arquivo leve atualizado pelo install (cria segunda fonte de verdade: risco de inconsistência, e a regra do projeto é fail closed).
- Recurso: latência do `rm`.
- Medir: span em `installed_from_bucket`.
- Risco: médio se criar cache paralelo; baixo se só flags. Prioridade: BAIXA-MÉDIA.

**5. Evitar as varreduras dos `map_*` em `delete_source_rows` (índice em `package` ou pular para Scoop).**
- Onde: `src/catalog/mod.rs:169-180`, SCHEMA `:35-44`.
- Mecanismo: `DELETE FROM map_x WHERE package IN (SELECT rowid ...)` não tem índice em `package` (os índices são só em `key`, `:40-44`): cada DELETE varre a tabela de mapa inteira (cresce com o winget) mesmo para um bucket Scoop, que provavelmente não grava mapas (confirmar). Opções: (a) pular os 5 DELETEs para fonte Scoop; (b) `CREATE INDEX ... ON map_*(package)` (custa escrita extra no `update` do winget e tamanho do .db).
- Recurso: CPU e I/O do `rm` e do `replace_source` de bucket no `update` (`mod.rs:309`).
- Medir: `EXPLAIN QUERY PLAN` (esperado SCAN) numa cópia do catálogo fora do repo + span em `delete_source_rows`.
- Risco: (a) baixo, precisa teste de que Scoop não grava mapas; (b) médio (migração, custo no update do winget). Prioridade: MÉDIA.

**6. Abrir o catálogo sem DDL/`journal_mode` no `rm`.**
- Onde: `src/buckets.rs:195` usa `Catalog::open` (`mod.rs:227`): `journal_mode=WAL` + `execute_batch(SCHEMA)` (10 statements) a cada abertura. ARMADILHAS já diz que `open` roda DDL.
- Mecanismo: variante que pula `SCHEMA` quando `PRAGMA user_version` indica schema atual e não reescreve `journal_mode` (WAL persiste no arquivo).
- Recurso: CPU/latência (parte dos 11-15 ms de abertura).
- Medir: span em `Catalog::open`; A/B.
- Risco: migrações futuras precisam do DDL (usar `user_version`). Prioridade: MÉDIA-BAIXA.

**7. `rm` com vários nomes (um só rewrite do índice, uma abertura, uma gravação).**
- Onde: `src/cli/mod.rs:751` (hoje 1 nome por chamada).
- Mecanismo: amortiza abertura de catálogo, transação, reescrita do `.idx`, `http.json` e `config.toml`.
- Recurso: tudo acima, em lote.
- Medir: N chamadas vs 1 chamada com N nomes.
- Risco: muda a CLI (decisão de produto). Prioridade: BAIXA.

**8. `index_file::write` com menos cópias.**
- Onde: `src/catalog/index_file.rs:42-71`.
- Mecanismo: pré-dimensionar `body` (`Vec::new()` na linha 43 realoca), escrever offsets+body direto num `BufWriter` sobre o `.tmp` em vez de montar `out` (3ª cópia).
- Recurso: memória (pico ~3x -> ~1x do tamanho do índice), memcpy.
- Medir: pico de working set no bench + span. Saída deve ficar bit-idêntica (teste por hash).
- Risco: baixo. Prioridade: BAIXA (benefício também no `update`; irrelevante para o `rm` se a ideia 1 entrar).

**9. `list` com uma única escrita em stdout.**
- Onde: `src/cli/mod.rs:722-724`.
- Mecanismo: montar `String` e imprimir uma vez, em vez de `println!` por linha (poucos buckets; ganho provavelmente imperceptível).
- Medir: só se o span de `run_bucket(List)` aparecer frente ao prefixo.
- Prioridade: MUITO BAIXA.

**10. `add` idempotente sem regravar `config.toml`.**
- Onde: `src/buckets.rs:116-117` (retorna `Ok(())` sem mudança) e `src/cli/mod.rs:746` (`cfg.save` sempre).
- Mecanismo: `add` indicar se mudou; `save` só se mudou (evita toml pretty + tmp + rename e varredura de antivírus).
- Recurso: disco/CPU pequenos.
- Medir: `bucket add extras` repetido, delta de bench.
- Risco: baixo (muda tipo de retorno e testes). Prioridade: BAIXA.

## 3. Desperdícios evidentes

- `Http::new()` (2 clients reqwest/TLS) e runtime tokio multi-thread criados para comandos que não usam rede nem async (`cli/mod.rs:532`, `:808`). Custo não medido, mas é trabalho sem consumidor.
- `rm` relê TODO o catálogo (outras fontes incluídas) para regravar o índice (`buckets.rs:200` -> `index_file.rs:41`), quando o arquivo já contém os registros.
- Índice regravado mesmo com 0 linhas removidas (retorno de `remove_source` descartado, `buckets.rs:196-199`).
- `delete_source_rows` varre 5 tabelas de mapa sem índice em `package` (`mod.rs:171-179`), provavelmente vazias de linhas Scoop.
- `Catalog::open` roda 10 DDL + escreve `journal_mode` em todo `rm` (`mod.rs:227-239`), contra a própria armadilha documentada.
- `bucket add` repetido regrava `config.toml` sem mudança (`cli/mod.rs:746`).

## 4. Top 3

1. Ideias 1+2: `rm` filtra o `search.idx` direto (sem `load_entries`) e não faz nada de catálogo/índice quando a fonte não tem linhas.
2. Ideia 3: tirar `Http::new()` e o runtime multi-thread do caminho de `bucket` (medir `Http::new()` isolado primeiro).
3. Ideia 5: `delete_source_rows` sem varreduras de `map_*` para Scoop (ou índice em `package`), confirmando com `EXPLAIN QUERY PLAN`.

Ordem de medição sugerida (spans antes de mudar): prefixo de `run` (ensure, logging, config, Runtime, Http), `installed_from_bucket`, `Catalog::open`, `remove_source`, `index_file::write` (separando `load_entries`, serialização, `fs::write`, `rename`). Sempre com `-Baseline target/release/examples/noop.exe` e `scripts/leftovers.ps1` antes.
