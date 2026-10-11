Status: DONE

Commits:
824497d perf(update): encode search.idx straight from catalog rows on one connection
(other SHAs in b5a63d7..HEAD belong to Tasks 55/56)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 11s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (495 passed, 16 ignored) 34s
[OK]   gate verde
(An earlier run failed on Task 55's in-progress catalog/mod.rs clippy errors; another on a flaky install_executor "Acesso negado" that passed alone and on re-run.)

Mutation proofs (test: cargo test --quiet --test index_file streamed_bytes):
[OK] mutação PEGA: 'put_words(&mut body, s(8)?);' -> 'put_str(&mut body, Some(s(8)?));' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'put_words(&mut body, s(9)?);' -> 'put_str(&mut body, Some(s(9)?));' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'put_str(&mut body, o(6)?);' -> 'put_str(&mut body, o(7)?);' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'put_str(&mut body, o(4)?);' -> 'put_str(&mut body, Some(o(4)?.unwrap_or_default()));' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'ORDER BY rowid' -> 'ORDER BY id' fez o teste falhar (exit 101).
Check 6: grep "load_entries|open_worker_connection|rowid_ranges" src/catalog/index_file.rs returned nothing; encode_catalog/write/persist have no thread:: (persist keeps thread::sleep for rename retry, as before).

Decisions:
- Test bodies transcribed verbatim; rustfmt reformatted them (formatting only).
- ARMADILHAS item inserted before the "Processos órfãos" item in the Desempenho section.
- A/B benchmark not run (orchestrator's job); not measured.

Concerns: none
