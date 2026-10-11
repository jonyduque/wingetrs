# Desempenho de `wingetrs upgrade` (brainstorm, somente leitura)

Nada aqui foi medido. Todo ganho é hipótese "não medido" com o experimento que a mediria. Números existentes vêm de `docs/superpowers/reports/2026-10-10-fase2a-validacao.md` (install com cache ~522 ms, broadcast 438–568 ms) e de `docs/ARMADILHAS.md` (SQLite abrir+1ª consulta 11–15 ms; alocação domina leitura em massa; piso de processo 38–300 ms). Números de linha com `~` são aproximados.

## 1. Mapa do caminho quente (`upgrade --all`, N apps)

| # | Etapa | Onde | Custo por app? |
|---|---|---|---|
| 1 | `ops::upgradable`: `read_state`, `state.apps()`, por app: `read_json(current/install.json)` (disco) + `catalog.get(source,id)` + `Version::parse` x2 | `src/install/ops.rs:761-783` | sim |
| 2 | `catalog.get` seleciona `PKG_COLS`, que inclui `raw` (manifesto JSON inteiro), só para comparar versão | `src/catalog/mod.rs:187-188,457` | sim |
| 3 | `read_state` de novo em `upgrade` | `src/cli/install_cmd.rs:~364` | 1x |
| 4 | por app: `state.get` + `ops::resolve_target` -> `target::resolve` -> **`catalog.load_entries()` (catálogo inteiro) + filtro com `to_lowercase()` por entrada** | `install_cmd.rs:~410`, `ops.rs:327`, `src/install/target.rs:61-69` | sim, O(N x catálogo) |
| 5 | por app: `plan_for`: `manifest_raw` (2º `catalog.get` com `raw`), `manifest::resolve`, **`read_state` outra vez (abre SQLite)**, `DepWalk` (`catalog.get` por dependência), `state.get` por nó, `sha256_hex(raw)`, `plan_install` | `ops.rs:461-505` | sim |
| 6 | `print_plans` por grupo, `proceed` | `install_cmd.rs:~420-430` | pouco |
| 7 | por grupo: `ops::apply` = `lock_async` + `open_state` (abre + `journal_mode=WAL` + DDL `SCHEMA`) + `revalidate` + `env_snapshot` x2 + `broadcast_if_changed` | `ops.rs:546-576`, `state.rs:88-96` | sim |
| 8 | `execute_install`: Download (`fetch_all` por passo, 1 fonte; `read_index` a cada chamada) -> Extract (zip single-thread em `.wingetrs-extract-*`, depois move p/ staging) -> Promote -> Persist -> LinkCurrent -> Shim -> Shortcut -> PATH -> Record (1 transação WAL) | `executor.rs:312-395,415-590`, `download.rs:148` | sim, serial |
| 9 | `download_one`: `get_to_file` -> **`fs::read` do arquivo inteiro** -> `verify` -> **2º SHA-256** só para nomear o blob -> rename | `download.rs:113-145` | sim; RAM = tamanho do instalador |
| 10 | `remove_upgrade_leftovers` + `remove_old_version` (`remove_dir_all`) **dentro do lock, antes de passar ao próximo app** | `executor.rs:357-380,397-403` | sim; proporcional ao nº de arquivos |

Crítica ao já planejado:
- (a) lote com 1 lock e 1 broadcast: `PartlyApplied { applied }` (`ops.rs:~570`) já serve a um lote. Mas `revalidate` (`ops.rs:523-541`) só tolera "dependência já instalada" para `i < last`; num lote cada raiz é um upgrade, então ele precisa saber quais planos são raízes, não só o último da lista concatenada. Além disso, uma falha no app K aborta os K+1..N (`break`, `ops.rs:~568`): para upgrade em lote talvez convenha continuar com os independentes (decisão de produto, afeta semântica de `PartlyApplied`).
- (b) downloads em paralelo: `fetch_all` já aceita `&[Source]` com `MAX_PARALLEL = 4` (`download.rs:46,148`), mas o executor chama com uma fonte por passo (`executor.rs:~432`). Um pré-download único do lote que aquece o cache de blobs faz o passo `Download` virar acerto de `cached()` (`download.rs:159`), sem mexer no executor (ver I4).

## 2. Ideias

### I1. `upgradable` sem `raw` e sem N consultas (ALTA, barato)
- O quê: trocar `catalog.get` (`ops.rs:~773`) por consulta leve só com `id, version`: uma por bucket (`SELECT id, version FROM packages WHERE source=?1`) montando `HashMap`. Parse de versão uma vez por lado. Opcional: ler `install.json` só dos apps cuja versão difere (mudança de contrato: app quebrado sem atualização deixaria de ser avisado; decisão de produto).
- Mecanismo: elimina N leituras do blob `raw` (manifestos de dezenas de KB) e N `prepare` (ARMADILHAS: `prepare` domina em SQLite), e N leituras de disco.
- Recurso: CPU, alocação, I/O, conexão SQLite.
- Medir: span `tracing` em `upgradable` com state.db sintético de N=50; A/B com `bench.ps1 -Baseline` em `upgrade` (listagem).
- Risco: baixo (só leitura). Preserva rollback, PartlyApplied e revalidação (não toca `apply`). Exige teste de equivalência: nova consulta vs `catalog.get` dão a mesma `UpgradeList`.

### I2. Eliminar `resolve_target`/`load_entries` por app (ALTA, maior desperdício)
- O quê: em `upgrade` (`install_cmd.rs:~408-418`) o par (bucket, id) já vem do `state.db` e de `upgradable`. Montar o `Candidate` direto de `catalog.get(source_name, app)` (ou fazer `upgradable` devolver o `StoredPackage` só dos atualizáveis) em vez de `resolve_target`, que carrega o catálogo inteiro (com `data_version` duplo) e baixa caixa de cada entrada, N vezes. Passar o `raw` já lido adiante para `plan_for`, evitando a 2ª leitura do manifesto.
- Mecanismo: O(N x catálogo) vira O(N).
- Recurso: CPU e alocação (ARMADILHAS: 19,5 mil linhas como `String` custaram 78 ms; aqui isso se repete por app).
- Medir: span em `resolve_target` e `plan_for` dentro do laço, N=1, 10, 40. Se `load_entries` custar X ms, o desperdício é N x X (não medido).
- Risco: baixo/médio: precisa manter a recusa de bucket divergente e `selectable`/`note` usados em `plan_for` (`ops.rs:~465`). Teste de equivalência entre `resolve_target(Some(bucket))` e o caminho direto sobre uma fixture. O plano gerado é idêntico, então rollback/Partly/revalidação ficam intactos.

### I3. Um único `StateDb` de leitura no planejamento
- O quê: `plan_for` chama `read_state` por app (`ops.rs:~478`; abrir + 1ª consulta 11–15 ms por ARMADILHAS) e `upgrade` já abriu outra. Criar variante interna `plan_for_with(&StateDb, ...)` e passar a mesma conexão; `read_state` continua nunca criando `state.db`.
- Recurso: ~N x 11–15 ms (extrapolado do número medido, não medido neste caminho).
- Medir: contagem e span de `read_state`.
- Risco: baixo. O estado de planejamento já é "de antes do lock"; `revalidate` sob lock segue como defesa.

### I4. Pré-download do lote (complementa o planejado (b))
- O quê: antes do lock, juntar todos os `Step::Download` de todos os planos e chamar `fetch_all` uma vez (`MAX_PARALLEL=4`). Depois `apply` roda como hoje e cada `Download` acerta o cache.
- Mecanismo: rede sobreposta; e fora do lock (hoje o lock é aberto em `ops.rs:550` antes do download, ficando preso durante rede), reduz a janela de contenção com outro `wingetrs`/TUI.
- Recurso: tempo de parede e duração do lock.
- Medir: servidor local com latência/banda fixas, N=5; A/B sequencial vs paralelo; variar `MAX_PARALLEL` 2/4/8.
- Risco: baixo. Blobs são verificados e imutáveis por hash; falha de rede antes do lock = nada aplicado (melhor que hoje, em que falha no app 7 gera `PartlyApplied`). Decisão de produto: pular só o app que falhou no download e atualizar o resto.

### I5. Remover a versão antiga fora do caminho crítico
- O quê: `remove_old_version` e `remove_upgrade_leftovers` (`executor.rs:357-380,397`) rodam dentro do lock a cada app. Em lote: aplicar todos, depois (ainda sob lock, ou com lock curto reaberto) limpar as versões antigas dos `applied`, em `spawn_blocking` aguardado/threads juntadas, com `remove_dir_all` de apps diferentes em paralelo; pode sobrepor com o broadcast (I6).
- Mecanismo: tira trabalho proporcional ao nº de arquivos (e ao AV por arquivo apagado) do caminho crítico.
- Recurso: tempo de parede, duração do lock.
- Medir: span em `remove_old_version` com árvore de ~5 mil arquivos; sequencial vs em fase vs paralelo; separar custo do AV comparando com árvore criada por `robocopy`.
- Risco: médio. Rollback: a remoção já ocorre só depois do `Record` e é "aviso" em caso de falha (`executor.rs:~366-379`), então adiá-la não piora o rollback do app; mas com `PartlyApplied` a limpeza precisa ainda cobrir os `applied`. Crash entre aplicar e limpar deixa pasta antiga órfã (conferir se `ops::repair`, `ops.rs:850`, trata). Sem lock, outro `install` do mesmo app poderia colidir com `version_dir`: manter sob lock é mais seguro. Sem processo desacoplado (regra de threads).

### I6. Broadcast: só se necessário, e sobreposto
- Conferir se um upgrade típico muda PATH/env: `broadcast_if_changed` (`ops.rs:293-304`) só dispara com snapshot diferente; os `EnvSet` de upgrade são normalmente idempotentes (hipótese: 0 broadcasts no caso comum, não medido). Se for assim, o custo de 438–568 ms quase não aparece em upgrade e o ganho de (a) vem do lock e `open_state`, não do broadcast.
- Quando mudar: rodar o broadcast numa thread (juntada, pânico via `resume_unwind`) enquanto roda I5; avaliar `SMTO_ABORTIFHUNG`/timeout menor em `env_win.rs` (picos de 2,5 s com janela lenta).
- Medir: contar broadcasts num `upgrade --all` real de 3 apps com a chave real em máquina de teste.
- Risco: baixo; o snapshot já cobre rollback.

### I7. Extração fora do lock e em paralelo / pipeline (risco alto, só após spans)
- O quê: `Step::Extract` escreve só em `staging_dir(app,version)` (pasta privada, `executor.rs:~456`). Dividir `execute_install` em "preparar" (Download+Extract, sem lock, K workers) e "commit" (Promote, Persist, Link, Shim, Shortcut, PATH, Record, sob lock). Pipeline: extrair A enquanto B baixa.
- Recurso: tempo de parede; lock curto. A divisão dos ~410 ms do install (`fase2a-validacao.md:148`) em extração vs junção vs shim vs registro ainda não é conhecida: spans primeiro.
- Medir: spans por etapa em `execute_install`; workers 1/2/4 (ARMADILHAS: heap padrão do Windows serializa threads, o ganho pode ser pequeno).
- Risco: alto. O journal de rollback (`run.journal`) é por plano e hoje vive dentro de uma só chamada; precisa atravessar as fases e limpar staging em cancelamento/falha. `PartlyApplied` ganha a classe "preparados e não aplicados". Mais disco/RAM temporários: limitar. Revalidação continua válida se feita antes do commit.

### I8. Hash e I/O do download
- O quê: `download_one` faz `fs::read` do arquivo inteiro, `verify`, e um 2º SHA-256 para nomear o blob (`download.rs:~128-136`). Com hash SHA-256 no manifesto, o nome é o hash esperado: um passe só; ou hash em streaming dentro de `get_to_file`, sem reler o disco (e sem pico de RAM; em lote paralelo o pico é 4x o maior instalador).
- Recurso: CPU, RAM, releitura (que o AV também escaneia).
- Medir: instalador sintético 100–500 MB, `bench.ps1` e Working Set A/B.
- Risco: baixo/médio. Manter fixtures que falham só em cada checagem (hash errado por algoritmo, blob adulterado). Não altera rollback.

### I9. SQLite do estado
- `open_state` roda WAL + DDL por `apply` (`state.rs:88-96`): vira 1x com o lote. `PRAGMA synchronous=NORMAL` reduziria fsync por commit, mas pode perder o último commit na queda de energia e divergir do disco: não recomendado sem medida; 1 commit por app deve permanecer para `PartlyApplied` refletir a realidade. Prioridade baixa.

### I10. Antivírus
- Superfície (hipóteses não medidas): AV escaneia cada arquivo escrito na extração, a releitura do blob (I8) e cada arquivo apagado (I5). Mitigações seguras: menos releituras (I8), apagar em paralelo (I5), manter tudo no mesmo volume para `rename` O(1) (já é: `extract.rs:~120-135`, `Promote`). Não sugerir exclusão no Defender (política do usuário). Medir: execução fria vs repetida, e `remove_dir_all` da árvore recém-extraída vs árvore igual criada por `robocopy`.

### I11. Comparação de versão
- `Version::parse` 2x por app (`ops.rs:~777`) é custo trivial (ARMADILHAS: palpite de gargalo falha); não priorizar. Vale checar correção: versões não numéricas do Scoop (`nightly-2024...`, `2.0-rc1`) podem gerar falso "atualizável" ou falso "em dia", isto é correção, não velocidade.

## 3. Desperdícios evidentes (confirmados lendo o código)

1. `resolve_target` por app carrega o catálogo inteiro (`target.rs:61`, `ops.rs:332`) embora (bucket,id) já seja conhecido: O(N x catálogo). Maior desperdício.
2. `upgradable` lê `raw` de cada app só para comparar versão (`catalog/mod.rs:188`) e `plan_for` lê o manifesto de novo (`ops.rs:~470`): duas leituras do mesmo dado.
3. `read_state` (abre SQLite) repetido em `upgrade` e em cada `plan_for` (`ops.rs:~478`); mais um `open_state` com DDL por `apply`.
4. Lock + `open_state` + `env_snapshot` + broadcast por app (já planejado); `fetch_all` por passo com `read_index` por chamada (`download.rs:154`).
5. Download lê o arquivo inteiro na RAM e faz SHA-256 duas vezes (`download.rs:~128-136`).
6. Remoção da versão antiga sob lock e no caminho crítico (`executor.rs:357-380`).
7. `install.json` de todos os apps lido antes de saber se há atualização (`ops.rs:~766`).
8. `exec_ctx`/`effects`/`layout(ctx)` remontados por plano (`ops.rs:~565`): trivial.

## 4. Top 3

1. **I2 + I1 + I3 ("planejamento O(N)")**: sem `load_entries` por app, consulta de versão sem `raw`, um só `StateDb` de leitura. Maior ganho esperado em lote, risco baixo, não toca `apply`/rollback/`PartlyApplied`/revalidação. Spans em `upgradable`/`resolve_target`/`plan_for` (N=1,10,40) antes de codar.
2. **I4 (+I8)**: pré-download do lote com um `fetch_all` fora do lock (quase sem código novo, melhora o modo de falha de rede), e hash de um passe/streaming sem `fs::read` inteiro.
3. **I5 (+I6)**: limpeza das versões antigas depois do loop de aplicação, paralela e sobreposta ao broadcast numa thread juntada; garantir cobertura dos `applied` em `PartlyApplied`. I7 (extração paralela/pipeline) só depois que spans por etapa de `execute_install` mostrarem a fatia da extração.

Protocolo de medida comum: `scripts/leftovers.ps1` antes; spans `tracing` por etapa; `bench.ps1 -Baseline noop.exe`; A/B contra a base em worktree temporário; cenário com N sintético (state.db falso, catálogo de fixtures, servidor local), nunca rede/registro reais (`WINGETRS_TEST_ENV_KEY`/`WINGETRS_TEST_SHORTCUT_DIR`).
