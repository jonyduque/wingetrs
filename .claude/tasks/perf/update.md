# Revisão de desempenho: `wingetrs update` (e `update --background`)

Somente leitura, nada compilado nem medido. Todo ganho abaixo é hipótese "não medido" com o experimento que a mediria. Evidências reais usadas: `docs/ARMADILHAS.md`, `docs/superpowers/reports/2026-10-07-fase1-validacao.md:16-17,43-45,58-64` (só o caminho Fresh foi medido: 65,6 ms mediana, piso do processo ~44 ms; o caminho 304 e o caminho com download **nunca foram medidos**), tamanhos reais em `%LOCALAPPDATA%\wingetrs` (catalog.db 24,2 MB, search.idx 3,0 MB, http.json 1,3 KB) e o log de 2026-10-10 (conexões, ver D1).

Não repito: filtro de nível do log (tarefa existente). Não proponho o que ARMADILHAS refutou: paralelizar leitura/alocação com mais threads (heap do Windows serializa), mais conexões SQLite paralelas (`prepare` compete), `COUNT(*)`, palpites sem span.

## 1. Mapa do caminho quente

Gatilhos: `wingetrs update [--force]` (`src/cli/mod.rs:537`), `update --background` disparado por `refresh_if_stale` (`src/cli/mod.rs:375-380`, chamado em `:431` e `:556` e nos demais comandos), bloqueante em catálogo vazio (`:419`), e TUI (`src/tui/run.rs:219`, `LockWait::NoWait`).

Em `update --background` (processo novo):
1. `spawn_background_update` (`cli/mod.rs:348-366`): `current_exe` + `Command::spawn` DETACHED. Custo no PAI: criação de processo; no FILHO: tudo abaixo.
2. `run` (`cli/mod.rs:772-`): `DataDir::from_env`, `data.ensure()` (cria diretórios), `init_logging` (appender diário + thread de escrita), `Config::load`, `tokio::runtime::Runtime::new()` (multi-thread, 1 worker por núcleo) em `:807`.
3. `run_async` (`:526-532`): `Http::new()` constrói DOIS `reqwest::Client` (`src/http.rs:51-61`), cada um com configuração TLS/rustls. `Endpoints::production` (`update.rs:25`) resolve buckets da config.
4. `run_update` (`update.rs:150`): trava de arquivo via `spawn_blocking` (`:160`, laço de `sleep(100ms)` em `cache/mod.rs:209-225`), `HttpState::load` (leitura + JSON, `:167`), monta jobs (winget + N buckets Scoop).
5. Fetch concorrente (`update.rs:182-192`): `join_all` de `fetch_one`. Cada job faz GET condicional (`http.rs:76-112`; `Body.bytes().to_vec()` copia o corpo inteiro em `:110`).
   - Winget (`sources/winget/mod.rs:49-71`): `validate_msix` (`index.rs:63-102`: abre zip em memória, descomprime `Public/index.db` inteiro para `Vec`, descomprime `AppxBlockMap.xml`, parse roxmltree, SHA-256 por bloco de 64 KiB e depois SHA-256 do arquivo inteiro = duas passadas), grava `index-<pid>.db` em `tmp/` (`mod.rs:63`), reabre com rusqlite e `read_index` (`index.rs:112-212`: 2 JOINs de tags/commands para HashMap, varredura de `packages`, 5 consultas de mapas com `id.clone()` por linha), apaga o tmp.
   - Scoop (`sources/scoop/bucket.rs:42-93`): gunzip + tar sequencial do tarball inteiro; por manifesto `bucket/*.json`: `read_to_end` num Vec novo, `serde_json::from_slice::<Value>` (DOM completo, `manifest.rs:58-98`), `raw = from_utf8_lossy(..).into_owned()` (outra cópia, `:95`).
   - ATENÇÃO: todo o CPU (inflate, SHA, tar, JSON, SQLite do índice) roda DENTRO do future, dentro de `join_all`, que é UMA tarefa tokio: não há paralelismo de CPU e o parse de um job pára o progresso de rede dos outros.
6. Só depois que TODOS os fetches terminam (`update.rs:192`): `Catalog::open` (`catalog/mod.rs:229-240`: `Connection::open`, `PRAGMA journal_mode=WAL`, `mmap_size`, `execute_batch(SCHEMA)`) mesmo que tudo seja Fresh/304 (`update.rs:194`).
7. Por fonte atualizada, serial: `replace_source` (`catalog/mod.rs:301-374`): 1 transação; `delete_source_rows` (`:169-179`) faz `DELETE FROM map_x WHERE package IN (SELECT rowid FROM packages WHERE source=?)` nas 5 tabelas de mapa (sem índice em `package`); INSERT linha a linha de `packages` (com `raw`) e de mapas com 5 índices em `key` sendo mantidos; `key.to_lowercase()` por linha de mapa.
8. `state.save` (`update.rs:232`, `cache/mod.rs:135-145`): JSON `to_vec_pretty`, `File::create`, `sync_all` (FlushFileBuffers), `rename`. Roda SEMPRE, até quando tudo foi Fresh.
9. `write_search_index` (`update.rs:237-239`, `catalog/index_file.rs:40-96`) se algo mudou: `catalog.load_entries()` (varredura de todas as linhas, até 8 conexões, ~90-105 ms medidos na fase 1 para materializar entradas) + serialização + `fs::write` + `rename` com retry.
10. `drop` do runtime/catálogo/vetores e saída.

## 2. Ideias

Legenda: tempo/CPU/mem/disco/rede/proc. "NM" = não medido.

### I1. Não disparar o `--background` quando já há atualizador vivo ou falha recente (prioridade ALTA)
- O quê: (a) antes de `spawn_background_update`, tentar `UpdateLock::try_acquire` não bloqueante e, se ocupado, não disparar; (b) gravar `last_attempt_at`/`last_failure_at` no `http.json` e fazer `needs_refresh` respeitar backoff (ex.: não tentar de novo por alguns minutos após falha); (c) no filho, `--background` usar `LockWait::NoWait` e sair em silêncio se `Busy`.
- Onde: `src/cli/mod.rs:375-380`, `:348`, `:537-543`; `src/update.rs:80-85,157-165`; `src/cache/mod.rs:209-225`.
- Mecanismo: hoje `needs_refresh` só considera `checked_at`, que só é gravado em 200/304 bem-sucedido (`update.rs:200-205,272-273`). Falha (offline, HTTP 5xx, bucket quebrado) deixa a fonte "velha" para sempre, então CADA comando (`search`, `show`, `list`...) cria um processo desacoplado que sobe runtime, log e clientes TLS, tenta conexão (connect_timeout 10 s, `http.rs:47`) e falha. Com atualizador já rodando, cada novo processo ainda espera até 30 s pela trava (`DEFAULT_LOCK_TIMEOUT_MS`, `update.rs:13`) em laço de 100 ms e depois reavalia, mantendo processos vivos.
- Economiza: processos, CPU, rede, memória (cada filho carrega runtime + 2 clientes TLS).
- Medir antes: contar processos `wingetrs` filhos (`scripts/leftovers.ps1`/Get-Process) ao rodar 20 `search` seguidos com a rede cortada e com o http.json envelhecido; contar linhas "starting new connection" no log. (O log de 2026-10-10 tem só 13 conexões novas em 3 execuções, então não há tempestade observada hoje: o ganho é de robustez, não comprovado.)
- Risco: baixo/médio. Tocar o formato de `http.json` (campo novo com `#[serde(default)]`) exige teste de compatibilidade com arquivo antigo. Regra: teste de equivalência não se aplica; precisa de teste que falha só no backoff e mutação nomeada.

### I2. Pré-checar "fresco?" no filho antes de montar runtime/clientes/log (ALTA em frequência, ganho NM)
- O quê: em `update --background`, ler `http.json` e sair antes de `init_logging`, `Config::load` e `Runtime::new` se nada estiver velho (condição possível quando dois filhos corriam e o primeiro já atualizou). Em geral, qualquer `update` sem `--force` com tudo Fresh já precisa só de `HttpState::load` + `needs_refresh`; hoje paga também `Http::new` (2 clientes, `cli/mod.rs:532`), runtime multi-thread, `spawn_blocking` da trava, e `Catalog::open` (`update.rs:194`).
- Onde: `src/cli/mod.rs:526-544`, `src/update.rs:150-231`.
- Mecanismo: caminho Fresh medido 65,6 ms contra piso de 44 ms (fase 1, `:17,43-45`) = ~21 ms de trabalho evitável no máximo; a parcela atribuível a cada etapa é NM. Sem os 21 ms ainda sobra processo.
- Economiza: tempo, CPU, memória, um `Catalog::open` (arquivo SQLite aberto + pragma WAL + DDL, 11-15 ms por ARMADILHAS).
- Medir antes: spans `tracing` `http_new`, `runtime_new`, `lock`, `state_load`, `catalog_open`, `state_save` no Fresh; `bench.ps1 ... -Args 'update' -Baseline noop.exe`; A/B contra a base.
- Risco: baixo. Cuidado com a ordem trava/estado: o filho deve reavaliar `needs_refresh` DEPOIS de obter a trava (já é assim, `update.rs:167`); a pré-checagem é só otimização.

### I3. `Catalog::open` e `state.save` só quando houver o que escrever (ALTA, simples)
- O quê: abrir o catálogo de escrita (`update.rs:194`) apenas ao chegar ao primeiro `Fetched::Winget/Scoop`; pular `state.save` (`:232`) se nenhuma entrada mudou (todas Fresh, ou nada alterou `checked_at`); no 304 só `checked_at` muda, então gravar assim mesmo, mas sem fsync (I4).
- Mecanismo: o caminho 304/Fresh, o mais comum, hoje abre SQLite com `journal_mode=WAL` + DDL (`catalog/mod.rs:232-235`) e regrava e dá fsync no JSON (`cache/mod.rs:135-145`) sem necessidade.
- Economiza: tempo (~11-15 ms de abertura SQLite por ARMADILHAS; fsync NM), disco (escrita + flush), I/O no WAL.
- Medir: spans por etapa no Fresh e no 304 (usar `--force` fora de horário de pico só com autorização de rede); `bench.ps1` com baseline.
- Risco: baixo. `index_path.exists()` (`update.rs:237`) hoje depende de `catalog` aberto; reorganizar para abrir só se `!index_path.exists() || updated`. Teste: Fresh não toca mtime de `catalog.db` nem de `http.json`.

### I4. Remover `sync_all` e `to_vec_pretty` do `http.json` (MÉDIA)
- Onde: `src/cache/mod.rs:136,142`.
- Mecanismo: `http.json` é cache regenerável (arquivo corrompido já vira estado vazio, `:121-131`); fsync (FlushFileBuffers) é a operação mais lenta do caminho em disco e protege algo que se refaz com um GET. Manter `tmp` + `rename` (atomicidade) sem flush. `pretty` só custa bytes/alocação (1,3 KB).
- Economiza: tempo (NM; fsync em NTFS costuma ser ms a dezenas de ms, mas NÃO MEDIDO nesta máquina), disco.
- Medir: span em volta de `sync_all` (um `Instant` basta) em 20 execuções; A/B.
- Risco: baixo. Efeito de queda de energia: pior caso é arquivo vazio/antigo, que `load` já trata. Registrar a decisão.

### I5. Concorrência real de CPU entre fontes: `tokio::spawn` + `spawn_blocking` por job e aplicar à medida que terminam (ALTA no caminho com download; ganho NM)
- O quê: (a) rodar a parte de CPU de cada job (`validate_msix`+`read_index`, `read_tarball`) em `spawn_blocking` (handle sempre aguardado, regra do CLAUDE.md) e os jobs em `FuturesUnordered`/`JoinSet` em vez de `join_all` numa única tarefa; (b) aplicar `replace_source` de cada fonte assim que o seu import chegar, em vez de esperar o mais lento (`update.rs:192-231`).
- Onde: `src/update.rs:101-123,182-231`, `sources/scoop/bucket.rs:81-93`, `sources/winget/mod.rs:49-71`.
- Mecanismo: hoje, enquanto um tarball é descompactado/parseado no thread que faz o poll, os sockets dos outros downloads não são servidos (single task); soma de tempos em vez de máximo. Com `spawn_blocking`, inflate+SHA do MSIX corre em paralelo ao parse dos tarballs. Atenção: ARMADILHAS diz que o heap do Windows serializa threads em alocação massiva (8 threads só deram 42 ms contra 27 ms), então o ganho vem de SOBREPOR rede e CPU, não de dividir uma tarefa pesada em muitas threads: não paralelizar dentro do tarball.
- Economiza: tempo de parede no caminho com download (não no 304).
- Medir: spans por job (início/fim de download, fim de parse, fim de apply) numa execução `--force`; comparar soma vs. máximo. Só vale se download e parse tiverem durações comparáveis.
- Risco: médio. Pânico de worker tem de ser propagado (`resume_unwind`), handles aguardados em todo caminho; a ordem de relatórios deve permanecer (testes de saída). SQLite com um único escritor continua serial.

### I6. Carregar o MSIX sem copiar nem hashear duas vezes (MÉDIA)
- O quê: (a) `get_conditional` devolver `Bytes` (sem `.to_vec()` em `http.rs:110`) ou acumular via `bytes_stream` em um `Vec` com `with_capacity(content_length)`; (b) em `validate_msix` (`index.rs:86-100`), calcular o SHA-256 de blocos e o do arquivo na mesma passada (`Sha256` incremental alimentado bloco a bloco) e comparar `FileHash` no fim; (c) evitar o segundo `Vec` de `read_entry` lendo direto de `ZipFile` com `Vec::with_capacity(size())`.
- Mecanismo: a cópia `to_vec()` duplica o corpo inteiro (o tamanho do MSIX NM; o log mostra um download de ~2 MB por stream h2 com janela de 5 MB, mas sem total), e hoje o `index.db` descomprimido é lido por SHA duas vezes (cache L2 frio entre passadas).
- Economiza: memória (um corpo inteiro a menos de pico), CPU (uma passada de SHA, ~metade desse custo), alocação.
- Medir: span `winget_validate` com e sem a mudança; RSS de pico (Get-Process PeakWorkingSet64 após execução `--force`); contadores de alocação não são necessários.
- Risco: baixo. Cada checagem tem de manter a fixture que falha só nela (BlockMap e FileHash, ARMADILHAS). Não mexer na semântica de erro.
- Subideia: habilitar a feature de SHA com instruções de hardware do `sha2` (CPU x86 tem SHA-NI em parte dos processadores): o `sha2` já usa autodetecção de CPU em runtime (`cpufeatures`) para x86; NM se está ativo; verificar com um span antes de qualquer trabalho. Não adicionar crate (regra do Cargo).

### I7. Não passar o `index.db` por arquivo temporário (MÉDIA/BAIXA, depende de feature)
- O quê: abrir o `index.db` de memória com `rusqlite` `serialize` (`Connection::deserialize`) em vez de `fs::write` + abrir + `remove_file` (`sources/winget/mod.rs:60-66`).
- Mecanismo: elimina escrita e releitura de um arquivo (tamanho NM) e a exposição ao antivírus/Defender no diretório `tmp\` (NM) e o `remove_file`.
- Risco/custo: exige habilitar a feature `serialize` do rusqlite, mudança no `Cargo.toml` (a regra do projeto proíbe novas dependências sem brief; é feature de crate existente, mas a decisão é do orquestrador) e um teste de que o índice lido de memória é igual ao do arquivo (equivalência de caminhos).
- Medir: span `winget_tmp_write` + `winget_read_index` (já separáveis) e A/B.

### I8. `read_index` e `replace_source` sem clones e sem lowercase repetido (MÉDIA)
- O quê: manter, para os mapas do winget, o `rowid` original do índice do winget e mapeá-lo para o rowid do catálogo (HashMap<i64,i64> ou Vec indexado) em vez de `by_rowid: HashMap<i64,String>` + `id.clone()` por linha (`index.rs:167,171,198-199`) seguido de busca `rowids.get(id.as_str())` (`catalog/mod.rs:351`); fazer `to_lowercase` já no SELECT (`lower(productcode)`) ou uma vez em lote em vez de `String` por linha (`:354-358`).
- Mecanismo: ARMADILHAS mostra que a alocação domina leitura em massa (27 ms como `&str` contra 78 ms copiando `String`, 19,5 mil linhas). Cada linha de mapa hoje cria 1 `String` de chave, 1 clone de id, 1 `to_lowercase`; o número de linhas de mapa NM (5 tabelas × pacotes).
- Economiza: CPU, alocações, memória de pico.
- Medir: spans `read_index_maps` e `replace_source_maps`; contar `map_rows` (`ImportStats.map_rows`); só otimizar se somarem mais que uns ms relevantes. Palpite refutável: lembrar a armadilha "palpite de gargalo falha".
- Risco: médio (muda `WingetMaps`, usado por fixtures e testes).

### I9. `delete_source_rows` não deve varrer tabelas de mapa para fontes sem mapas (ALTA, mudança pequena)
- O quê: só apagar de `map_*` quando a fonte é Winget (as fontes Scoop nunca inserem mapas: `update.rs:223` usa `WingetMaps::default()`); para Scoop, só `DELETE FROM packages WHERE source=?`. Alternativa para ambos: índice em `package` (custa escrita) ou `DELETE ... WHERE package BETWEEN lo AND hi` usando a faixa de rowid da fonte.
- Onde: `src/catalog/mod.rs:169-179`, chamado de `:312` e `:295`.
- Mecanismo: sem índice em `map_*.package`, cada `DELETE ... WHERE package IN (SELECT rowid FROM packages WHERE source=?1)` percorre a tabela de mapa inteira (que tem as linhas do winget). Atualizar o bucket `main` ou `extras` paga 5 varreduras completas de tabelas do winget, que sempre se resolvem em zero linhas. Em cada atualização de bucket, frequência alta porque o ETag do tarball do GitHub muda a cada commit (ver I10).
- Economiza: CPU, leitura de disco/cache do SQLite, tempo de transação (com a trava de escrita segurada), tamanho do WAL se páginas forem tocadas (não são para leitura).
- Medir: `EXPLAIN QUERY PLAN` do DELETE (confirma SCAN) numa cópia do catálogo.db (NÃO no real) e um span `delete_source_rows` por fonte em `--force`.
- Risco: baixo. Regra: um teste que falha só aqui — importar winget com mapas, depois Scoop, garantir que mapas do winget permanecem; e `remove_source` genérico segue apagando tudo. Mutação: remover a condição e ver o teste cair.

### I10. Evitar baixar o tarball do bucket quando nada mudou, e baixar menos quando mudou (MÉDIA, depende de medir)
- O quê: (a) confirmar que `codeload.github.com` responde 304 ao `If-None-Match` (o código já envia, `http.rs:83-85`; o etag no `http.json` é do tipo `"sha256..."`, mas nenhum 304 aparece nos logs porque o filtro é só TRACE do h2; NM); (b) se codeload ignora o validador, trocar a checagem por uma requisição leve (por exemplo `HEAD`/API de commits com ETag: chamadas condicionais 304 não gastam o limite de taxa da API do GitHub, a confirmar) e baixar o tarball só se o SHA do HEAD mudou; (c) como o tarball de HEAD muda a cada commit, buckets como `extras` e `main` mudam várias vezes ao dia: aplicar upsert apenas dos manifestos alterados (comparar `(id, version, hash do raw)` com o catálogo) em vez de apagar e reinserir tudo.
- Onde: `src/http.rs:76-112`, `src/sources/scoop/bucket.rs:81-93`, `src/update.rs:269`, `catalog/mod.rs:301-374`.
- Mecanismo: reduz rede (tarball inteiro por mudança de HEAD), CPU de gunzip/parse/INSERT e escrita no WAL (hoje reescreve todo `raw`, ~5,6 MB segundo a fase 1, a cada mudança de commit, mesmo que só um manifesto tenha mudado).
- Medir antes: (1) uma única requisição condicional manual fora do binário confirma se há 304 (com autorização de rede, fora desta revisão); (2) span de `read_tarball` e `replace_source` por bucket, e `Content-Length` de cada tarball; (3) contagem de manifestos realmente diferentes entre dois downloads consecutivos.
- Risco: alto/médio. O upsert incremental precisa preservar a regra "mais de um caminho ⇒ teste de equivalência" (importação completa vs incremental dão o mesmo catálogo) e manter `rowid` estável ajuda o `search.idx` (I12). A API do GitHub tem limite de taxa e política de uso diferentes da CDN.
- Custo extra a considerar: o tarball do GitHub traz arquivos fora de `bucket/` (pastas `scripts/`, `deprecated/`, `.github`); inevitável com gzip sequencial.

### I11. Parse de manifesto Scoop leve (MÉDIA)
- O quê: trocar `serde_json::Value` (`manifest.rs:58-98`) por uma struct tipada com `#[serde(default)]` só para `version`, `description`, `bin`, `architecture.*.bin` e `IgnoredAny` para o resto; e montar `raw` direto de `buf` (`String::from_utf8(buf)` reaproveitando o Vec quando é UTF-8 válido, em vez de `from_utf8_lossy(..).into_owned()` que sempre copia, `:95`); evitar `Vec::new()` por entrada (reusar um buffer: `buf.clear()`, `bucket.rs:60`) quando possível.
- Mecanismo: o DOM completo aloca um `Value` por campo de cada manifesto (inclusive `autoupdate`, `checkver`, `hash`, `url`, `suggest`...) para depois jogar fora quase tudo; ARMADILHAS: alocação domina. O `raw` já guarda o JSON inteiro, então o DOM é pura sobra de CPU/memória.
- Economiza: CPU, alocação, memória de pico (milhares de manifestos por bucket; contagem NM).
- Medir: span em volta de `read_tarball` com contagem de manifestos e de bytes; A/B do parser em microbenchmark sobre uma fixture de tarball existente (`tests/fixtures/`, sem `cat`).
- Risco: baixo/médio. Comportamento de borda: manifesto sem `version` ou JSON inválido deve continuar indo para `skipped`; `bin` pode ser string, array ou array aninhado (`push_bin`), exigir testes com cada forma. Depende de a função `details_from_raw` (`:101`) continuar usando o DOM, já que é lida sob demanda.

### I12. Reconstruir o `search.idx` sem reler o SQLite (MÉDIA/ALTA, depende de medir)
- O quê: montar o `search.idx` a partir dos imports em memória (a fonte atualizada) e do `search.idx` anterior para as fontes não atualizadas, em vez de `catalog.load_entries()` (`index_file.rs:41`, varredura completa com até 8 conexões, ~90-105 ms materializando entradas segundo a fase 1, `:123`).
- Mecanismo: após cada update com mudança, todo o catálogo (winget + buckets) é decodificado do SQLite e reserializado, mesmo quando só um bucket pequeno mudou. Além disso `rowid_ranges` abre até 8 conexões SQLite (11-20 ms cada, contra a armadilha medida: "8 conexões paralelas competem no `prepare`").
- Economiza: CPU, tempo de parede do `update`, memória de pico (Vec<PackageEntry> do catálogo inteiro).
- Medir antes: span `search_index_write` (load_entries vs serialização vs escrita) numa atualização de um bucket só.
- Risco: alto. A ordem de linhas/rowids do `search.idx` precisa ser idêntica à da busca no SQLite e em memória (teste de equivalência já exigido pelo CLAUDE.md: acrescentar caso "índice incremental = índice completo"). Só vale se o span provar que `load_entries` domina.

### I13. Reduzir custo fixo de processo: runtime de 1 thread e cliente HTTP único/preguiçoso (MÉDIA/BAIXA)
- O quê: (a) `Runtime::new()` (`cli/mod.rs:807`) cria um worker por núcleo; para `update` usar `Builder::new_current_thread()` ou `worker_threads(2)` (o trabalho de CPU vai para `spawn_blocking` de qualquer forma, I5); (b) em `Http::with_timeouts` (`http.rs:51-61`) construir o `dl_client` sob demanda (`OnceLock`), já que `update` nunca usa `get_to_file`.
- Mecanismo: cada `reqwest::Client` com rustls/verificador de plataforma monta configuração TLS (carregamento de certificados do sistema pode ser o custo, NM); duas vezes por processo, inclusive no caminho Fresh em que nenhum socket abre.
- Economiza: tempo de inicialização, memória (threads e clientes), CPU.
- Medir: spans `http_new` e `runtime_new`; `bench.ps1` do `update` Fresh com baseline `noop.exe` (a diferença A/B é provavelmente de poucos ms; só manter se mensurável).
- Risco: baixo. Mexer em `Http` afeta o instalador (`get_to_file`), cobrir com os testes existentes de download.

### I14. Uma conexão HTTP/2 só por host (MÉDIA/BAIXA)
- Evidência: no log de 2026-10-10, às 18:06:40,305 e às 22:59:37,931, os três "starting new connection" ocorrem no mesmo milissegundo: um para `cdn.winget.microsoft.com` e DOIS para `codeload.github.com`. Ou seja, os requests concorrentes de Main e Extras abrem duas conexões TCP+TLS ao mesmo host em vez de multiplexar em uma.
- O quê: disparar o primeiro request de cada host e só liberar os demais após a conexão (ex.: fazer a requisição a codeload em série para o primeiro bucket, depois o resto em paralelo), ou usar `pool_max_idle_per_host` e um warm-up; ou aceitar o custo.
- Mecanismo: economiza um handshake TLS (RTT×2 + criptografia) por bucket extra por execução. Com `N` buckets, N−1 handshakes.
- Economiza: tempo de rede (principalmente no caminho 304, onde os handshakes são o grosso do tempo), CPU de TLS.
- Medir: span por GET com tempo até primeiro byte; comparar paralelo vs. "primeiro sozinho" com `--force` (depende de rede, não feito aqui).
- Risco: baixo, mas serializar piora a latência quando a conexão inicial é lenta; só decidir com dados.

### I15. Higiene do `http.json` (BAIXA)
- O arquivo atual guarda entradas de URLs que o código não usa mais (`.../tar.gz/refs/heads/master` para Main/Extras, `http.json` real). `HttpState` nunca remove (`cache/mod.rs:151-153`). Podar entradas fora de `all_urls` ao salvar. Ganho: ínfimo (1,3 KB), mas evita crescimento e confusão. Prioridade baixa.

## 3. Desperdícios evidentes

1. `Catalog::open` (WAL pragma + DDL) no caminho 100% Fresh/304: `update.rs:194`.
2. `state.save` com `sync_all` mesmo sem nenhuma mudança: `update.rs:232`, `cache/mod.rs:142`.
3. Dois `reqwest::Client` por processo, um deles nunca usado em `update`: `http.rs:51-61`.
4. Runtime tokio multi-thread (um worker por núcleo) para 3-4 requisições: `cli/mod.rs:807`.
5. Cópia do corpo inteiro (`Bytes -> Vec`): `http.rs:110`; `from_utf8_lossy(...).into_owned()` copia cada manifesto Scoop: `manifest.rs:95`; `Vec::new()` por entrada de tar: `bucket.rs:60`.
6. SHA-256 duas vezes sobre o mesmo `index.db`: `index.rs:86-99`.
7. `index.db` escrito em disco e relido só para abrir no SQLite: `winget/mod.rs:60-66`.
8. `id.clone()` por linha de mapa, `to_lowercase` por linha, busca em HashMap por id depois de já ter o rowid: `index.rs:171,199`, `catalog/mod.rs:351-358`.
9. `DELETE` com varredura completa das 5 tabelas de mapa do winget ao atualizar um bucket Scoop: `catalog/mod.rs:169-179`.
10. Todo o CPU dentro de um único `join_all`; aplicação ao SQLite espera o fetch mais lento: `update.rs:182-231`.
11. Reescrita completa de `search.idx` (SQLite → memória → arquivo) a cada atualização de qualquer fonte, com até 8 conexões SQLite: `update.rs:237`, `index_file.rs:40-96`, `catalog/mod.rs:155-166,394-439`.
12. Falha de rede não registra tentativa: o próximo comando repete o spawn (até 10 s de `connect_timeout`) e, com atualizador vivo, filhos esperam até 30 s pela trava: `update.rs:13,80-85`, `cli/mod.rs:375-380`.
13. Duas conexões TCP+TLS ao mesmo host `codeload.github.com` (log de 2026-10-10, 18:06:40,305 e 22:59:37,931).
14. `http.json` acumula URLs obsoletas: `cache/mod.rs:151-153`.

## 4. Top 3 recomendado

1. **I3 + I2 + I4 (caminho 304/Fresh enxuto)**: é o caminho mais frequente (todo comando que acha cache velho) e a meta da fase 1 (`update` 304 < 300 ms) ainda não tem medição. Primeiro spans (`http_new`, `runtime_new`, `catalog_open`, `state_save`), depois cortar `Catalog::open` condicional, o `sync_all` e a pré-checagem. Barato, risco baixo.
2. **I9 (DELETE sem varrer mapas do winget para buckets Scoop)** junto de **I1 (não disparar `--background` com atualizador vivo ou após falha recente)**: duas correções pequenas com mecanismo claro no código; I9 é a única com prova por `EXPLAIN QUERY PLAN` numa cópia do banco, I1 elimina processos desnecessários.
3. **I5 (CPU fora do `join_all`, aplicar por ordem de chegada)**, mas só depois de medir uma execução `--force` com spans por job: se download for dominante e o parse pequeno, o ganho é nulo; se os tarballs/MSIX custarem dezenas de ms de CPU, é o maior ganho de tempo de parede do caminho com download. Respeitar a regra de pânico/`spawn_blocking` aguardado e lembrar que o heap do Windows serializa alocação (não dividir o parse).

Instrumento primeiro (vale para todas): adicionar spans `tracing` por etapa no `run_update` (lock, state_load, http por fonte, validate, read_index, read_tarball, catalog_open, replace_source por fonte, state_save, search_index) e rodar `scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'update' -Baseline target/release/examples/noop.exe` no Fresh; o caminho `--force` precisa de autorização de rede e de rodar sem outros agentes medindo.
