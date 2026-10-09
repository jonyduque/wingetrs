Status: DONE_WITH_CONCERNS (meta de bench < 30 ms NAO comprovada: maquina muito ruidosa)

Commits:
c088121 perf(search): compact search.idx written by update and read by search

Gate:
[OK]   fmt 1s
[OK]   clippy 10s
[OK]   test (160 passed, 2 ignored) 56s
[OK]   gate verde

Mutation proofs:
- trailer (`--test index_file`): [OK] mutação PEGA: 'if expected != bytes.len() as u64 || &bytes[bytes.len() - 8..] != MAGIC {' -> 'if false {' fez o teste falhar (exit 101).
- desempate por nome (`--test index_file`): [OK] mutação PEGA: '.then_with(|| a.lower_name.cmp(&b.lower_name))' -> '.then_with(|| a.index.cmp(&b.index))' fez o teste falhar (exit 101).
- condição "índice ausente" (`--test update`): [OK] mutação PEGA: 'if updated || !index_path.exists() {' -> 'if updated {' fez o teste falhar (exit 101).

Real-data checks (catalog real, 19587 entradas):
- search.idx real: 3.020.589 bytes (criado por `wingetrs update` sem --force via condição "índice ausente").
- Saída de `search powertoys`, `--json`, `"" -n 5`, `--source scoop`: sem idx (caminho SQLite) vs com idx -> `cmp` idêntico nos 4 (arquivos before-N/after-N).
- Equivalência Found (index vs search_on_disk) em "powertoys", "", "code", "git": equal=true.

Bench (`-Baseline noop.exe`, 15 runs; "antes" = mesmo binário com search.idx removido):
antes 1: delta median=211,9 ms (baseline 80,2) | antes 2: 185,5 ms (baseline 169,9)
depois 1: delta median=132,6 ms (baseline 141,2) | depois 2: 68,7 ms (baseline 71,1)
Rodadas anteriores na sessão (mesma máquina, carga variável) deram deltas 61–222 ms em ambos os lados; o piso do baseline foi de 46 a 277 ms. Tempo in-process medido: index_file::search 28–35 ms (fs::read 2,7 ms, parse 28 µs) vs search_on_disk 52–109 ms, na mesma sessão. Meta < 30 ms de delta: não medida de forma conclusiva.

Decisions:
- `index_file::probe` (cabeçalho + tamanho) decide em `search` se dispensa abrir o SQLite (n>0); não relê os 3 MB.
- Falha ao gravar o índice remove o `search.idx` antigo (evita resultado obsoleto) além de `tracing::warn!`.
- Helpers `search_best` (pós-setup) e `search_early` (thread antecipada) no CLI; `refresh_if_stale` extraído de `ensure_catalog_ran`.
- Teste extra `rewriting_while_a_reader_loops_never_fails_or_tears` cobre rename concorrente com leitor.
- Em `tests/index_file.rs` apenas `i % N == 0` -> `i.is_multiple_of(N)` (lint clippy); asserções intactas.

Concerns:
- Meta de bench não comprovada (ruído da máquina); o ganho in-process (~2x vs SQLite) é do índice, o restante é pontuação nucleo (~25 ms) em ~19,6k entradas.
- Durante a medição um `update --background` disparado pelo próprio `search` reescreveu catálogo+idx; restaurar um idx antigo à mão gerou idx obsoleto (erro meu, corrigido removendo e rodando `update`). O idx obsoleto só ocorre por manipulação manual, pois update regrava o idx sob o lock.

## Correções da revisão (commit fix(search): pin search.idx trailer and require catalog.db for the index shortcut)
1. Teste de dano ganhou trailer com último byte trocado (tamanho correto). Mutação: [OK] mutação PEGA: '|| &bytes[bytes.len() - 8..] != MAGIC' -> '' fez o teste falhar (exit 101).
2. Atalho do índice no `search` exige `catalog.db` existente; `deleted_catalog_with_fresh_http_state_is_rebuilt_by_search` (tests/cli_offline.rs) agora também afirma que o catalog.db foi recriado. Sem prova de mutação para esta regra.
3. `run_update` grava o índice antes de propagar o erro de `state.save`.
Gate: [OK]   test (160 passed, 2 ignored) 77s / [OK]   gate verde

## Segunda correção
Teste agora abre o catalog.db read-only e exige count(*) == 28. [OK] mutação PEGA: 'let (catalog, updated) = if data.catalog_path().exists()' -> 'let (catalog, updated) = if true' fez o teste falhar (exit 101).
