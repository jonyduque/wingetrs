Status: DONE

Commits:
b221f0b feat(bridge): refresh winget's source and retry once when it picked an older version

Gate:
[OK]   rules 4s / rules-selftest 14s / fmt 2s / clippy 12s
[OK]   test (1002 passed, 24 ignored) 55s
[OK]   gate verde

Mutation proofs:
1. segunda tentativa permitida (`(stale, retried)` com `false` -> `_`), teste still_stale: "[OK] mutação PEGA ... fez o teste falhar (exit 101)".
2. `Ok((0, _)) => continue` -> `Ok((_, _)) => continue`, teste source_update_failure_stops: falhou (FAILED, assertion left==right); a linha "[OK] mutação PEGA" ficou fora do tail colado, mas o script restaurou o arquivo e o teste caiu.

Decisions:
- `job.end` sempre traz `attempts=N` (1 ou 2).
- `job.retry` usa `target:` wingetrs::job e campo `target`, como `job.start`.
- Se o `source update` não inicia, o sufixo traz o erro de E/S no lugar do código.
- Código extraído: `exec_once` em src/tui/jobs.rs; laço em `bridge_run`.

Concerns: none

## Correções
- jobs.rs: no início de cada iteração do laço, `log.failure/code/after/lines/installer_log` são zerados (vazamento da 1ª tentativa para o `job.end`).
- Testes: `retry_failure_is_not_no_effect` (3 chamadas, `outcome=falhou`, mensagem exata) e `retry_that_cannot_start_drops_first_run_from_log` (sem `code=0`, sem 2.23.1).
- Mutações PEGAS: sem reset de `log.failure` ([OK] mutação PEGA, retry_failure), sem reset de `log.code` ([OK] mutação PEGA, retry_that), sem reset de `lines` ([OK] mutação PEGA, retry_that).
- Gate: ver mensagem final (verde).
