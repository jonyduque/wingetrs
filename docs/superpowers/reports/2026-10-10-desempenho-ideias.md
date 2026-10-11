# Desempenho — ideias por comando (brainstorm, 2026-10-10)

Oito revisores (somente leitura, sem compilar nem medir) leram o caminho quente de cada comando.
Os relatórios completos estão em `.claude/tasks/perf/<comando>.md` (mapa do caminho quente, ideias
numeradas com mecanismo, recurso economizado, como medir, risco e prioridade). **Nenhum ganho abaixo
foi medido**; cada item traz o experimento que o mediria. Números medidos de fato estão em
`2026-10-07-fase1-validacao.md` e `2026-10-10-fase2a-validacao.md`.

## Temas que atravessam vários comandos

| # | Ideia | Comandos | Onde | Recurso |
|---|---|---|---|---|
| T1 | **Spans `tracing` por etapa** antes de qualquer otimização (install, uninstall, update, search, list) | todos | — | decide o resto |
| T2 | `Http::new()` monta 2 clientes reqwest/rustls em todo comando, inclusive os que não usam rede | search, list, show (Scoop), bucket, uninstall | `src/cli/mod.rs:584`, `src/http.rs` | CPU/tempo de partida |
| T3 | Runtime tokio multi-thread para comandos síncronos | search, list, bucket | `src/cli/mod.rs:860` | threads, tempo de partida |
| T4 | `update --background` respawna sem backoff: falha não grava `checked_at`, então cada comando offline cria um processo novo | todos | `src/cli/mod.rs` `refresh_if_stale`, `src/update.rs:199,278` | processos, rede |
| T5 | SQLite: `prepare` a cada `lookup/get` (6–8 por app ARP); `find_by_id` varre a tabela (`lower(id)` sem índice); `delete_source_rows` apaga em 5 tabelas `map_*` do winget mesmo para bucket Scoop | list, show, update, bucket | `src/catalog/mod.rs:169-180, 457-490` | CPU, E/S |

## Por comando (top de cada revisor)

- **install/uninstall** — o broadcast já só acontece quando o PATH muda (`ops.rs:293-304`): sobra a 1ª instalação e a remoção do último app. Hash duplo no cache (`download.rs` `cached`: lê o blob inteiro e calcula SHA-256 duas vezes). Varredura de resíduos (105 ms) pode sobrepor a remoção. Os 80 ms do uninstall foram medidos **sem** varredura de resíduos (`ResidueScan::none()` sob a chave de teste) — os números não são comparáveis.
- **upgrade** — `resolve_target` por app carrega o **catálogo inteiro** (`load_entries`) embora `(bucket, id)` venha do `state.db` (`install_cmd.rs` → `ops.rs:327` → `target.rs:61`); `upgradable` lê a coluna `raw` de tudo; `StateDb` reaberto por `plan_for`. Remoção da versão antiga pode sair de baixo do lock.
- **update** — caminho 304/Fresh ainda abre o catálogo e grava `http.json` com `sync_all` sem mudança; duas conexões TLS simultâneas ao mesmo host `codeload.github.com`.
- **search** — perfil do processo inteiro primeiro; o `search.idx` poderia ter seção quente/fria; o exe carrega ~15 ms acima de um hello-world (piso do binário).
- **show** — `versionData` é baixado sempre, embora a URL já contenha o hash (cacheável para sempre); índice para `lower(id)`.
- **list** — threads de varredura nascem só depois de logging/config/runtime/Http; subir o spawn para o topo de `run()` como o `search` já faz; `scan_wingetrs` abre o `state.db` em escrita (WAL, DDL) para ler.
- **bucket** — `add` só edita `config.toml` (quem baixa é o `update`); `rm` relê o catálogo inteiro para refazer o `search.idx`, e poderia filtrar o índice direto ou não fazer nada quando a fonte tinha 0 linhas.
- **TUI** — spinner/ticker de 10 Hz pode ficar ligado para sempre (`details_loading` nunca vira falso sem mexer na seleção) ⇒ CPU ociosa; cada frame monta até 1000 linhas para mostrar ~30 e a aba Instalados varre ~19,5 mil entradas por frame; busca síncrona na thread de UI a cada tecla.

## Achado de correção (não é desempenho)

`src/tui/run.rs:178` (`spawn_boot`) e `:235` (`spawn_reload`) descartam o handle do `spawn_blocking`
(regra do CLAUDE.md; armadilha "Handle de `spawn_blocking` descartado"), e `spawn_boot` faz
`unwrap_or_default()` no erro do catálogo (catálogo ilegível vira "vazio" em silêncio). O
`check_rules.ps1` não pegou: o padrão dele não cobre `spawn_blocking` sem `let`. Conferido pelo
orquestrador lendo o código.

## Ideia que provavelmente não funciona

Trocar o broadcast por `SendNotifyMessageW`: a documentação do Windows diz que funções assíncronas de
mensagem não podem levar ponteiros para mensagens abaixo de `WM_USER`, e o `lParam` do
`WM_SETTINGCHANGE` é um ponteiro para "Environment". Só com sonda (terminal novo enxergando o PATH).
