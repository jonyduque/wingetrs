Status: DONE_WITH_CONCERNS

Commits:
e226e46 feat(log): start and end of every job; winget run that keeps the old version is not a success

Gate:
[OK]   rules 8s
[OK]   rules-selftest 30s
[OK]   fmt 3s
[OK]   clippy 10s
[OK]   test (990 passed, 24 ignored) 52s
[OK]   gate verde

Mutation proofs:
[OK] mutação PEGA: '&& chosen != available' -> '&& false' fez o teste falhar (exit 101).   (src/tui/jobs.rs, job_log::stale)
[OK] mutação PEGA: 'self.lines[self.lines.len().saturating_sub(END_TAIL)..]' -> 'self.lines[..]' fez o teste falhar (exit 101).   (src/job_log.rs, job_log::failure)
[OK] mutação PEGA: 'if outcome == Outcome::Ok {' -> 'if outcome == Outcome::Ok { return;' fez o teste falhar (exit 101).   (src/job_log.rs, job_log::start_and_end)

Decisions:
- New module src/job_log.rs (+ `pub mod` in lib.rs): shared JobInfo/JobLog so TUI and CLI use one format; Drop writes `cancelado` (or `falhou` on panic), so no run.rs change was needed (run_bridge/run_action log themselves).
- The `message` field of job.end is named `detail`: `message` is the event's own field (the literal "job.end").
- Bridge.TAIL_LINES 20 -> 40 so production output reaches the log with 40 lines; tests/it/bridge.rs `tail_splits_lines` updated (20->40).
- Target::Bridge now holds Box<BridgeJob> (clippy large_enum_variant on Modal after BridgeJob grew).
- Test 7 uses `install m --yes` (MSI refused, fails before any real effect) via Lab, made pub(crate) in tests/it/cli_install.rs. Test 2 expects the "Antigravity: " name prefix like every other bridge failure.

Concerns:
- Files outside the Files list: src/job_log.rs, src/lib.rs, src/tui/app.rs, tests/it/cli_install.rs, tests/it/bridge.rs (see above).
- TDD red phase not observed separately (tests were written after the implementation, all passed first run; mutations prove they bite).
- sem_efeito only for the winget version check; scoop no-change stays `falhou`.

## Correções (commit fix(log))
1. Comparação por `Version` (`chosen < available` => sem_efeito); testes 2.25.0 vs 2.24.0 e 2.24 vs 2.24.0 => Ok. Mutação de volta para `!=`: [OK] mutação PEGA.
2. Testes de `run_action` (install + remove, kind exato, tool, ok). Mutação do revisor (`let log: Option<JobLog> = { drop(...); None }`): [OK] mutação PEGA.
3. Outcomes `ocupado` e `pede_confirmacao` em INFO, detail legível; teste cobre os dois.
4. `jobs::log_not_started` chamado em run.rs quando `with_ops` falha; teste unitário da função (o trecho de run.rs em si não tem teste).
5. Teste de pânico do Drop (catch_unwind): outcome=falhou "terminou em pânico".
6. Id sem diferenciar maiúsculas (ASCII); a linha "Encontrado" é capturada no on_line, antes do corte (feito, barato); mutação: [OK] mutação PEGA.
Gate: [OK]   test (997 passed, 24 ignored) / [OK]   gate verde
