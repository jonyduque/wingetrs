Status: DONE_WITH_CONCERNS (portão de medição: "não vale"; só o commit de spans ficou)

Commits:
9155b97 perf(cli): spans for runtime and http setup

Gate (verificado antes do commit de spans):
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 132s
[OK]   test (628 passed, 16 ignored) 342s
[OK]   gate verde

Medição (15 execuções de `bucket list`, binário release do commit 9155b97, raiz isolada, catálogo fresco, `time.busy` do log):
- startup.runtime:      mediana 764 µs  (580..878 µs)
- startup.http:         mediana 263 µs  (200..717 µs)
- startup.runtime_drop: mediana 821 µs  (620..2070 µs)
- Soma das medianas: 1,85 ms < 2 ms ⇒ "não vale". Mudanças 1 e 2 (Http preguiçoso, runtime de thread única) NÃO implementadas; sem testes, sem mutações, sem A/B (nada a provar).
Linhas reais (amostra): `startup.runtime: close time.busy=580µs`, `startup.http: close time.busy=218µs`, `startup.runtime_drop: close time.busy=859µs` (15 trios colhidos, medianas acima).

Mutation proofs: não aplicável (nenhuma mudança de comportamento foi implementada, portão de medição parou a tarefa).

Decisions:
- Spans: `debug_span!(...).in_scope(...)` em volta de `Http::new`, `Runtime::new` e `drop(rt)`; resultado do block_on guardado antes do drop.

Concerns:
- Acidente meu: um perf-root.ps1 gerado errado (variável $Name vazia) apagou o conteúdo do diretório scratchpad do agente (só arquivos temporários antigos de tarefas anteriores; nada no repositório nem em %LOCALAPPDATA%\wingetrs real). Scripts de medição foram regenerados.
- Ganho de ~1,85 ms é menor que o ruído de piso do processo; reabrir só se o custo crescer.
