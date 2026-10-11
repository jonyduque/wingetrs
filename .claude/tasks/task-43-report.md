Status: DONE

Commits:
(see git log 2ca0e02..HEAD, commit "fix(log): filter the file log to wingetrs and keep 14 days")

Gate:
[OK]   rules 2s
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (415 passed, 16 ignored) 87s
[OK]   gate verde
(gate rodado com a Task 41 em andamento no mesmo checkout; passou na 3a tentativa, falhas anteriores eram fmt de src/install/ops.rs)

Mutation proofs:
[OK] mutação PEGA: '.with_default(Level::WARN) ...' (WARN -> TRACE) fez o teste falhar
[OK] mutação PEGA: 'names.len().saturating_sub(keep)' -> 'names.len().saturating_sub(keep + 1)' fez o teste falhar (exit 101).

Decisions:
- Retenção só considera nomes com sufixo estrito AAAA-MM-DD.
- Filtro de stderr analogo: wingetrs em INFO/DEBUG, resto WARN (fn stderr_log_filter, sem teste dedicado).
- prune_logs roda antes de abrir o appender em init_logging.

Concerns: none (checagem 3 e do orquestrador). Rodei `cargo fmt` global uma vez no inicio, pode ter tocado formatacao de arquivos da Task 41.

Follow-up (review Minor): teste stderr_filter_follows_verbosity_and_drops_dependency_noise adicionado.
[OK] mutação PEGA: 'let level = if verbose > 1 {' -> 'let level = if verbose > 2 {' fez o teste falhar (exit 101).
Gate completo nao rerodado (Task 41 em andamento); rodados `cargo test --lib log_tests` (4 passed) e clippy limpo.
