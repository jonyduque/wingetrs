Status: BLOCKED

BLOCKED: a sonda mostra que a verificação pedida (`WinVerifyTrust`) precisa da feature nova `Win32_Security_WinTrust` do crate `windows` (confirmado em `windows-0.62.2/src/Windows/Win32/Security/mod.rs:29`, `#[cfg(feature = "Win32_Security_WinTrust")]`). Pela regra do brief, parado depois do relatório da sonda: `docs/superpowers/reports/2026-10-19-sonda-202.md`. Nenhum código escrito.

Commits:
a17d0c1 docs(report): probe for task 202 (signed catalog hash on the elevated side)

Gate: não rodado (nenhum `.rs` alterado).

Mutation proofs: nenhuma (sem código; a tarefa parou na sonda).

Decisions:
- Recomendação A: feature `Win32_Security_WinTrust` (sem crate nova, `Cargo.lock` sem mudança de pacote) + identidade fixada (signatário, raiz Microsoft com flag de raiz de aplicativo, `Identity Name="Microsoft.Winget.Source"`) + piso de recência próprio.
- Reserva B (sem feature nova) medida e descrita no relatório da sonda.

Concerns:
- Pedido só com `{id, versão, índice, escopo}` obriga o lado elevado a calcular os parâmetros do instalador (`select`/`switches`); `select.rs` está com a Task 189.
- HTTP no lado elevado: proxy hoje vem do ambiente do usuário; precisa de decisão.
- Certificado do msix vale 3 dias; com `WinVerifyTrust` (carimbo de tempo) um catálogo velho segue válido sem o piso de recência.
