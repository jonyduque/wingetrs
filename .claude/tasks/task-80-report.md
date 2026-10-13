Status: DONE_WITH_CONCERNS

Commits: (ver git log; commit `fix(tui): open the catalog read-only for reads`)

Gate:
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 11s
[OK]   test (650 passed, 16 ignored) 223s
[OK]   gate verde

Mutation proofs:
1. Catalog::open_read_only(path) -> Catalog::open(path), teste reading_never_runs_the_schema_ddl:
   [OK] mutação PEGA: 'Catalog::open_read_only(path)' -> 'Catalog::open(path)' fez o teste falhar (exit 101).
2. if path.exists() { -> if true {:
   [OK] mutação PEGA: 'if path.exists() {' -> 'if true {' fez o teste falhar (exit 101).
3. read_catalog engolindo erro (.or_else(|_| Ok(Vec::new()))), teste read_catalog_reports_an_unreadable_file...:
   [OK] mutação PEGA: '.and_then(|c| c.load_entries())' -> '.and_then(|c| c.load_entries()).or_else(|_| Ok::<_, CatalogError>(Vec::new()))' fez o teste falhar (exit 101).
4. grep -rn "Catalog::open(" src/tui -> saída vazia.

Decisions:
- Teste 3 do brief ja existia (read_catalog_reports_an_unreadable_file_instead_of_an_empty_list); nao dupliquei.
- Premissa do brief falsa: Catalog::open num catalog.db somente leitura NAO falha (teste 1 passa com o caminho antigo; mutacao sobreviveu). Tambem open_read_only cria -shm/-wal em banco WAL, entao comparar listagem de diretorio nao serve.
- Adicionei teste reading_never_runs_the_schema_ddl: sqlite valido sem o schema; read_catalog deve dar Err e os bytes do arquivo ficam iguais (Catalog::open criaria as tabelas e daria Ok vazio). E ele que prova a mutacao 1. Teste 1 (somente leitura) mantido como no brief.

Concerns: gate falhou uma vez em tests/cache.rs (outro agente, task 81); passou na reexecucao.
