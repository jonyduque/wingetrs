Status: DONE

Commits: see `git log --format='%h %s' 084c7a7..HEAD` (catalog commit is the last "feat(catalog)"; other commits belong to the parallel agent)

Gate:
[OK]   fmt 1s
[OK]   clippy 17s
[OK]   test (60 passed, 0 ignored) 16s
[OK]   gate verde
(earlier runs failed only because of the other agent's in-progress files: src/cache, tests/http.rs; retried until green)

TDD: before implementing, `cargo test --quiet --test catalog` -> "error[E0432]: unresolved imports `wingetrs::catalog::Catalog`, ..." (items missing).

Mutation proofs:
- DELETE sem filtro de source: [OK] mutação PEGA: 'DELETE FROM packages WHERE source = ?1' -> 'DELETE FROM packages WHERE ?1 = ?1' fez o teste falhar (exit 101).
- Rollback (ignorar id ausente): [OK] mutação PEGA: 'return Err(CatalogError::DanglingMap(id.clone()));' -> 'continue;' fez o teste falhar (exit 101).
- Bônus de match exato: com os testes do brief a mutação SOBREVIVEU (o score fuzzy já põe os exatos no topo). Adicionei `exact_id_beats_equal_scoring_name_match` em tests/search.rs; depois: [OK] mutação PEGA: 'score = score.saturating_add(EXACT_BONUS);' -> 'let _ = EXACT_BONUS;' fez o teste falhar (exit 101).

Decisions: clippy::type_complexity -> alias `MapPair`; exact bonus = 1<<30 somado ao score; tags/commands com score/2; kind desconhecido no DB cai em Winget; query com trim.
Concerns: none (um teste extra além do brief, em tests/search.rs).

## Follow-up (review): key case folding, char indices, WAL
Commit: test(catalog): pin key case folding, char indices and WAL (ver git log). Novos testes: code_lookup_folds_case_in_both_directions, open_file_uses_wal_journal (tests/catalog.rs), non_ascii_name_indices_are_char_positions (tests/search.rs, [3,4]).
- (a) sem lowercase na escrita: [OK] mutação PEGA: '                    key.to_lowercase()' -> '                    key.to_string()' fez o teste falhar (exit 101).
- (b) sem lowercase na consulta: [OK] mutação PEGA (âncora 'let key = if table.lowercases() { key.to_lowercase()' -> 'key.to_string()') fez o teste falhar (exit 101).
Gate: [OK]   test (87 passed, 0 ignored) / [OK]   gate verde
