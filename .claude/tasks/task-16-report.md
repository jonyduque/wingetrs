Status: DONE_WITH_CONCERNS (meta < 100 ms NÃO atingida: mediana ~105-108 ms; antes ~190-200 ms)

Commits:
473254d perf(search): load only search columns from the catalog

Gate:
[OK]   fmt 1s
[OK]   clippy 7s
[OK]   test (145 passed, 2 ignored) 73s
[OK]   gate verde

Mutation proofs:
pwsh -File scripts/mutate.ps1 -Path src/catalog/mod.rs -Anchor 'SELECT {ENTRY_COLS} FROM packages WHERE rowid BETWEEN' -Replacement 'SELECT {PKG_COLS} FROM packages WHERE rowid BETWEEN' -TestCommand 'cargo test --quiet --test catalog'
[X] mutação SOBREVIVEU: ... e os testes passaram. A regra não está coberta.
(Esperado pelo brief: é otimização, não regra observável. A prova desta tarefa é o benchmark.)

Benchmark (bench.ps1, release, catálogo real 19 528 linhas, 3 rodadas, medianas em ms; "antes" = binário do commit base cfb0981 em worktree separado):
FRESH antes : 193,4 / 193,7 / 189,0
FRESH depois: 106,6 / 104,9 / 105,5
STALE antes : 219,8 / 191,1 / 183,3  (http.json com checked_at=1 antes de cada rodada; o refresh em segundo plano é disparado e pode terminar no meio da rodada)
STALE depois: 107,8 / 105,7 / 108,5
Piso do processo (`--help`): mediana 46,1 ms.
Só o item 1 (SELECT enxuto): ~203 -> ~184 ms (medido isolado).

Perfil (eprintln temporário, removido): antes do trabalho extra, load ~100 ms, busca ~17-22, drop ~20, ensure_catalog ~5, init de log/DataDir ~4.
Mudanças (todas no commit, cada uma com ganho medido):
- load_entries com ENTRY_COLS + entry_from_row (obrigatório): -19 ms.
- PRAGMA mmap_size=256MB: passo de leitura 35 -> 15 ms (e mais rápido nos workers que sem mmap).
- load_entries paralelo (até 8 conexões read-only por faixa de rowid, só catálogo em disco): load ~75 -> ~42 ms.
- SearchIndex::search em chunks paralelos (>=4096 entradas, matcher por thread, ordem preservada) + eq_lower sem alocação: busca ~17 -> ~5 ms.
- mem::forget(index/catalog) no search da CLI: ~-20 ms de teardown.
- split_words sem String temporária e com capacidade exata: ganho não mensurável (mantido, inofensivo).

Decisions:
- Threads (std::thread::scope) em vez de nova dependência/cache/schema, conforme proibições do brief. Adicionei variante CatalogError::LoaderPanicked e campo privado path em Catalog (só acréscimos).
- tests/search.rs e tests/catalog.rs existentes passaram sem alteração; um teste novo em tests/catalog.rs (corpo literal do brief).
- src/catalog/search.rs foi editado (não está na lista Files): o perfil apontou a busca (~17 ms) como 3ª maior etapa.
- tui::run não alterado: se beneficia do load mais rápido sem mudança.

Concerns:
- Meta não atingida: teto medido ~105 ms. Soma: piso 46 + init 4 + ensure 5 + load ~42 + busca ~5 + saída/saída do processo ~3.
- Causa do load restante: cada worker leva ~19 ms só percorrendo as páginas (custo por conexão/mmap; paralelizar o passo não escala além de ~8), mais ~17 ms decodificando; a alocação de ~150k Strings/Vec (tags/commands) pesa. Follow-ups possíveis (fora das restrições do brief): PackageEntry com Box<str>/Arc<str> para source_name e tags (muda tipo público), ou coluna/tabela estreita dedicada à busca (mudança de schema/cache em disco, proibida aqui), ou alocador mimalloc (dependência nova).
- Medição de STALE é aproximada (refresh em segundo plano concorrente). Medi com a máquina ociosa; variância ±8 ms.
- Colisão potencial: mmap mantém o arquivo mapeado; o update escreve via SQLite (WAL), sem truncar, então sem conflito observado, mas não testado com update simultâneo além dos benchmarks STALE.

## Revisão (CHANGES_REQUESTED) - correções
Commits: ver `git log cfb0981..HEAD` (473254d + fix(catalog) de endurecimento).
- Testes novos: parallel_load_matches_sequential_order_on_sparse_rowids, load_entries_on_empty_disk_catalog_is_empty (tests/catalog.rs); parallel_search_equals_sequential (tests/search.rs, via SearchIndex::search_with_threads).
- Mutação de borda do load: `[OK] mutação PEGA: '(lo + (i + 1) * step - 1).min(hi)' -> '(lo + (i + 1) * step).min(hi)' fez o teste falhar (exit 101).`
- Mutação do base da busca: `[OK] mutação PEGA: 'scan(part, i * chunk,' -> 'scan(part, 0,' fez o teste falhar (exit 101).`
- join agora usa resume_unwind (load e busca); variante LoaderPanicked removida.
- load_entries lê PRAGMA data_version antes/depois e refaz uma vez se mudou.
- split_words revertido ao original.
- Gate: `[OK]   test (148 passed, 2 ignored)` / `[OK]   gate verde`.
- Bench (catálogo fresco, medianas): 107,4 / 112,6 / 107,2 ms. Meta < 100 ms continua não atingida.
