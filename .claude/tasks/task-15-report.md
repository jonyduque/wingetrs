Status: DONE_WITH_CONCERNS

Commits:
```
13539d4 docs(report): phase 1 live validation and benchmarks
```

Gate:
```
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (131 passed, 2 ignored) 7s
[OK]   gate verde
```

Mutation proofs: **no mutation proof** — a tarefa não entrega código de produto (só `scripts/bench.ps1` e o relatório `docs/superpowers/reports/2026-10-07-fase1-validacao.md`). Verificação proposta: reexecutar `pwsh -NoProfile -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'list'` e comparar com a mediana colada (409,4 / 404,0 ms) ±30%.

Decisions:
- `-Args` é uma string única separada por espaços (alias de `$Arguments`): `pwsh -File` não repassa arrays e `-Args search,powertoys` chegava como um argumento só (exit=2 observado). Adicionado `-DelayMs` para o bench do `update` "após 1 s".
- `update` sem `--force` mede o caminho Fresh (MAX_AGE_SECS=900, sem rede); documentado no relatório. Caminho 304 não medido.
- Primeiro frame medido em console real (`Start-Process`) com proxy 127.0.0.1:9 para impedir rede; 10 aberturas em vez de 5.
- Perfil do `search` feito com crate de scratch fora do repo (depende de `wingetrs` por path), sem alterar código de produto.
- Taxa do oráculo reportada como definida (81,9%) e, à parte, sem a classe framework MSIX (94,1%).

Concerns:
- Metas falhas: `search` ~192–206 ms (meta 100 ms; causa: `load_entries` lê 19 478 linhas com `raw` 5,6 MB, ~90–105 ms); correlação 81,9% (meta 90%; 15/21 divergências são pacotes framework MSIX descartados em `appx_win.rs:10`, 5 Ambiguous, 1 Unmatched). Follow-ups F1–F5 no relatório.
- `update` 304 não medido: `wingetrs update --force` foi bloqueado pelo classificador de permissões do ambiente; não contornei.
- Outlier de 104 ms em 1 de 10 aberturas da TUI, não investigado.
- Trailer do commit usa as linhas de atribuição do harness (`Claude Opus 5.5` + `Claude-Session`), não a linha genérica do brief.
