Status: DONE (portão: "não vale", commit 2 não feito)

Commits:
37d4125 perf(list): measure setup before the scan threads

Gate:
[OK]   fmt 1s
[OK]   clippy 27s
[OK]   test (628 passed, 16 ignored) 164s
[OK]   gate verde

Mutation proofs: none (commit 2, com EarlyList e suas provas, não foi feito).

Medida do portão (15 execuções de `list`, raiz isolada, evento list.setup, ms):
1.84 1.45 1.61 1.28 1.40 1.43 1.43 1.35 1.73 1.60 1.41 1.55 1.23 1.13 1.79
Mediana = 1.45 ms < 3 ms ⇒ "não vale". Só o commit 1 fica. A/B não rodado (não medido).

Decisions: Instant guardado em run() logo após init_logging e passado a run_async como parâmetro `after_log`.
Concerns: none
