# Desempenho de `wingetrs show` — brainstorm (somente leitura, nada medido)

Todo ganho abaixo é hipótese "não medido". Experimento padrão: spans `tracing` por etapa (nomes sugeridos entre crases) e depois `scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'show <id>' -Runs 15 -Baseline target/release/examples/noop.exe`, com `leftovers.ps1` antes. O relatório da fase 1 não traz números de `show` (grep "show" sem resultado), então não há baseline do comando.

## 1. Mapa do caminho quente

1. `main.rs:4` `Cli::parse()` (clap, CPU).
2. `cli/mod.rs:773` `DataDir::from_env`; `:790` `data.ensure()` (create_dir_all de vários diretórios, syscalls); `:791` `init_logging` (abre/cria arquivo de log, I/O); `:793` `Config::load` (lê e parseia TOML).
3. `cli/mod.rs:808` `tokio::runtime::Runtime::new()` (multi-thread, um worker por CPU, cria threads).
4. `cli/mod.rs:532` `Http::new()` constrói DOIS `reqwest::Client` (`http.rs:51,57`: rustls + http2, carga de raízes de certificado), mesmo no `show` de Scoop, que nunca usa rede.
5. `cli/mod.rs:587` -> `ensure_catalog_ran` (`:389`): `Endpoints::production`, `path.exists()`, `open_read_only` (abre SQLite, `mmap_size` 256 MB, `catalog/mod.rs:144-152`), `is_empty` com `EXISTS` (primeira consulta; Armadilhas: abrir + 1ª consulta = 11-15 ms).
6. `cli/mod.rs:431` `refresh_if_stale` (`:375`): `HttpState::load` (JSON de disco) + `needs_refresh`; se velho, `spawn_background_update` (`:348`) cria um processo destacado (CreateProcess).
7. `cli/mod.rs:588` `catalog.find_by_id` (`catalog/mod.rs:469`): `WHERE lower(id) = lower(?1)` não usa o índice `UNIQUE(source,id)` (`catalog/mod.rs:33`), logo varre a tabela `packages` inteira (lendo páginas com `description`, `tags`, `raw`...). Seleciona `PKG_COLS` inclusive `raw` (`:187`) e converte `tags`/`commands` em Vec (`split_words`) nas linhas que casam.
8. Winget (`cli/mod.rs:604-620` -> `sources/winget/details.rs`):
   a. `details.rs:16-17` GET de `versionData.mszyml` (rede, ~5 KB, SEMPRE), `mszip::decode` (CPU, inflate de blocos de 32 KiB, `mszip.rs:24-57`) e parse YAML `serde_saphyr` (`version_data.rs:44`).
   b. `details.rs:19-25` escolhe a versão: `Version::parse` dentro do comparador do `max_by`.
   c. `details.rs:28-38`: `BlobStore::get` (`cache/mod.rs:166`) lê o arquivo inteiro e RECALCULA sha256; se faltar, GET do manifesto (2ª requisição sequencial) + `put_verified` (sha256 de novo, escreve `.tmp`, rename).
   d. `details.rs:40-41` ordena todas as versões com `Version::parse` em cada comparação; `manifest::parse` (UTF-8 + YAML `serde_saphyr`, `manifest.rs:43`) e `into_details`.
9. Scoop (`cli/mod.rs:621-624`): `details_from_raw` (`scoop/manifest.rs:101`) reparseia o JSON `raw` do SQLite. Sem rede.
10. Saída: `serde_json::to_string_pretty` ou `output::details_text` + `println!`/`print!` (`cli/mod.rs:627-629`). Depois: drop do `Catalog` (fecha SQLite) e do runtime (join de workers); `search` evita isso com `mem::forget(catalog)` (`:578`), `show` não.

Caso quente Winget (blob em cache): 1 GET obrigatório de ~5 KB (latência de rede + TLS se conexão nova) provavelmente domina; caso frio: 2 GETs. Scoop: sem rede; dominam processo + SQLite + varredura (hipótese, não medido).

## 2. Ideias (prioridade A alta, M média, B baixa)

### 1. Eliminar o GET de versionData com cache local por hash do catálogo (A)
- O quê: gravar em disco o resultado reduzido de `versionData` (lista `{version, rel_path, sha256}`) ou o manifesto escolhido, indexado por `(id, hash completo do catálogo)`.
- Onde: `sources/winget/details.rs:14-17`; `cache/mod.rs` (novo armazenamento).
- Mecanismo: a URL já contém `hex8` do `winget_hash` do catálogo (`version_data.rs:77-80`), isto é, é endereçada pelo conteúdo do índice; quando o catálogo muda, o hash muda e o cache invalida sozinho. O comentário em `details.rs:14` ("sem hash conhecido, sempre baixa") ignora que a chave de invalidação já existe. Alternativa mais fraca: `Http::get_conditional` com ETag (continua custando 1 round-trip).
- Recurso: rede (1 requisição + handshake), CPU de mszip+YAML (pequena); `show` funciona offline para pacotes já vistos.
- Medir antes: spans `show.versiondata_http`, `show.versiondata_decode`, `show.manifest_get`; comparar com `-Baseline noop`. Se a rede for >50% do tempo, o ganho é grande.
- Risco/custo: cache velho se o CDN reescrever versionData sob o mesmo hash (usar o hash inteiro como chave; o sha256 do manifesto continua verificado). Complexidade média; exige teste de equivalência (cache vs rede) e fixture com cache adulterado que falha só nessa checagem.

### 2. Cache do `PackageDetails` já resolvido (M)
- O quê: após a ideia 1, guardar o `PackageDetails` serializado por `(sha256 do manifesto, hash das versões)`, pulando YAML parse e sort.
- Onde: `details.rs:40-41`.
- Mecanismo: troca parse YAML (serde_saphyr; manifestos com muitos installers) por leitura de arquivo pequeno.
- Recurso: CPU. Medir: spans `show.manifest_parse` + `show.versions_sort`; só vale se somarem alguns ms. Risco: formato em disco a versionar quando `PackageDetails` mudar.

### 3. Consulta indexada no lugar de `lower(id)=lower(?)` (A)
- O quê: `CREATE INDEX ix_pkg_id_lower ON packages(lower(id))` (índice de expressão; SQLite o usa para `lower(id)=lower(?1)`), ou coluna `id_lower`, ou `id = ?1 COLLATE NOCASE` com índice NOCASE.
- Onde: `catalog/mod.rs:469-475` e schema `:17-34`; o índice entra no `Catalog::open` (escritor), não no caminho read-only.
- Mecanismo: hoje é varredura completa; índice vira O(log n) com poucas páginas tocadas (mesma classe do "COUNT(*) percorre a tabela" em ARMADILHAS).
- Recurso: CPU e I/O. Medir: span `show.find_by_id` e `EXPLAIN QUERY PLAN` numa CÓPIA do catálogo (fora do repositório, ex. scratchpad).
- Risco/custo: catálogo e `update` ficam um pouco maiores/lentos (mais um índice); catálogos antigos sem o índice continuam corretos, só lentos. `lower()` do SQLite é só ASCII: teste de equivalência com ids em caixas variadas e não ASCII.

### 4. Resolver o id pelo `search.idx` em vez do SQLite (M)
- O quê: se o `search.idx` (`catalog/index_file.rs`) permitir achar id -> rowid (e guardar `winget_hash`), ir direto por `WHERE rowid=?` ou evitar o SQLite no caso winget.
- Mecanismo: ARMADILHAS: abrir SQLite + 1ª consulta custa 11-15 ms; para leitura quente, arquivo próprio lido de uma vez ganha. Scoop ainda precisa de `raw` do SQLite.
- Recurso: CPU+I/O. Medir: `show.catalog_open` + `show.find_by_id`.
- Risco/custo: muda formato do idx (versão, validação) e cria um segundo caminho para o mesmo resultado, exigindo teste de equivalência (regra do projeto). Só se 3 e 5 não bastarem.

### 5. Fundir `is_empty` com `find_by_id` (B)
- Onde: `cli/mod.rs:398-404`. Só rodar `EXISTS` quando `find_by_id` achar 0 linhas. Economiza um `prepare`+`step` (Armadilhas: `prepare` custa). Ganho provavelmente pequeno (não medido). Risco: `ensure_catalog_ran` é compartilhado com search/list/TUI.

### 6. Cliente HTTP preguiçoso (A para Scoop, M geral)
- O quê: criar `Http` só quando houver rede (`SourceKind::Winget` com cache frio), e `dl_client` só em download (`http.rs:51-62`, `cli/mod.rs:532`).
- Mecanismo: `reqwest::Client::build` com rustls inicializa provedor e raízes de certificado; dois clients dobram. É custo morto no `show` Scoop e no `show` Winget com cache (ideia 1).
- Recurso: CPU, alocações, startup. Medir: span `show.http_new`, e `show <pkg scoop>` contra o `noop`.
- Risco: baixo (mudança local, `OnceCell`/construção sob demanda).

### 7. Runtime tokio menor ou ausente (M)
- O quê: `Runtime::new()` (`cli/mod.rs:808`) cria um worker por CPU; o `show` é sequencial. Usar `Builder::new_current_thread()` só para `Show`, ou nem criar runtime para Scoop/cache.
- Recurso: threads/CPU/memória. Medir: spans `rt.new`/`rt.drop`; A/B current_thread vs multi. Risco: baixo-médio (`update` usa concorrência; separar por comando).

### 8. Inicialização antecipada em paralelo, como no `search` (M)
- O quê: `search` abre o catálogo/idx numa thread antes de logging, config e runtime (`cli/mod.rs:775-789`). Replicar para `show`: thread que abre o catálogo e roda `find_by_id` enquanto a principal faz ensure/logging/config/runtime/Http.
- Mecanismo: sobrepõe preparação com a consulta (reduz tempo de parede, não CPU total). Regras: `join` em todos os caminhos, `resume_unwind`, e descartar o resultado se houve update bloqueante (como `:568`).
- Medir: timestamps de spans para ver a sobreposição possível (teto = menor dos dois ramos). Risco: duplica fluxo; só após 3 e 6, que reduzem o que há a sobrepor.

### 9. `mem::forget` do catálogo (B)
- Onde: `show` (`cli/mod.rs:581-632`) não faz o `forget` que `search` faz (`:578`); drop do SQLite (fecha e desmapeia 256 MB) e do runtime rodam antes de sair. Medir spans `show.drop_catalog`/`show.drop_rt`. `process::exit` pularia o guard de logging (flush): evitar; `forget` do catálogo é simples e seguro.

### 10. Não recalcular sha256 do blob a cada leitura (B)
- Onde: `cache/mod.rs:166-169`. `put_verified` já grava por rename atômico após verificar. Hash de dezenas de KB é provavelmente desprezível; medir `show.blob_read_hash`; se <1 ms, NÃO mexer (otimização sem ganho é revertida; perde detecção de corrupção).

### 11. Ordenação com chaves pré-parseadas (B)
- Onde: `details.rs:21-24` e `:40`. `Version::parse` roda em cada comparação; usar `sort_by_cached_key`/`max_by_key`. Relevante só com centenas de versões. Medir `show.versions_sort`. Risco mínimo; teste de equivalência de ordem (empates, versões inválidas).

### 12. Compressão HTTP (M, bloqueado por regra de dependências)
- `reqwest` está com `default-features = false` sem `gzip` (`Cargo.toml:29`). Habilitar muda `Cargo.toml` (proibido sem brief) e depende de o CDN responder `Content-Encoding`, não verificado (sem rede aqui). Medir antes: tamanho transferido vs descomprimido do manifesto de um pacote grande. O pool do reqwest já reaproveita a conexão entre os dois GETs (confirmar com spans).

### 13. Disparo do refresh em segundo plano (B)
- Onde: `cli/mod.rs:375-380` e `:348`. Lê e parseia `http_state` inteiro e, se velho, cria um processo destacado a cada `show` até o update concluir (o lock impede concorrência, mas o processo sobe, inicializa e sai). Opções: spawn depois de imprimir; marca de "tentativa recente" (mtime de arquivo) por N minutos; ler só mtime em vez de parsear JSON.
- Medir: contar processos `update --background` em N `show` seguidos com catálogo velho (`leftovers.ps1`) e span `show.spawn_bg`. Risco baixo; manter equivalência com `needs_refresh`.

### 14. `ensure`/logging/config preguiçosos (B)
- Onde: `cli/mod.rs:790-793`. `create_dir_all` a cada execução e abertura do arquivo de log. Medir `show.ensure`, `show.logging`, `show.config`. Risco: logging tardio perde mensagens iniciais; afeta todos os comandos.

### 15. Perfil release (B)
- `Cargo.toml:71-74`: `lto = "thin"`, `codegen-units = 1`, `strip`. `panic = "abort"` NÃO propor (quebra a regra de `resume_unwind`). `lto = "fat"` só com A/B de binário; sem ganho medido, reverter.

## 3. Desperdícios evidentes

- Dois `reqwest::Client` construídos em todo comando, inclusive `show` Scoop (`http.rs:51,57`; `cli/mod.rs:532`).
- `find_by_id` varre a tabela inteira (`catalog/mod.rs:469-475`; só há `UNIQUE(source,id)`, `:33`).
- Sempre baixa `versionData` embora a URL seja endereçada pelo hash do catálogo (`details.rs:14-17`, `version_data.rs:77-80`).
- `Version::parse` repetido em cada comparação (`details.rs:21-24`, `:40`).
- sha256 do blob recalculado a cada leitura (`cache/mod.rs:166-169`), provavelmente barato (não medido).
- Runtime multi-thread para fluxo sequencial (`cli/mod.rs:808`).
- Teardown de SQLite/runtime antes de sair, que `search` já evita (`cli/mod.rs:578`).
- `is_empty` como consulta extra antes de `find_by_id` (`cli/mod.rs:398-404`).
- Possível respawn de `update --background` a cada `show` enquanto velho (`cli/mod.rs:375-380`).

## 4. Top 3 recomendado

Passo zero: spans (itens acima) + `bench.ps1 -Baseline` para separar startup, SQLite, rede e parse.

1. Ideia 1: cache de versionData/manifesto resolvido por hash do catálogo (remove a única requisição obrigatória do caso quente; habilita offline).
2. Ideia 3: índice para `lower(id)` (remove varredura completa, vale para Winget e Scoop).
3. Ideia 6: cliente HTTP preguiçoso (corta custo fixo de startup, simples e seguro; mais efeito no Scoop e no caso com cache).

Segunda leva, depois dos spans: ideias 7, 8, 9 e 13.
