# Roteiro: lacunas da ponte, desempenho, 2b, 2c e 2d (2026-10-15)

Este roteiro diz a ordem de execução. Cada etapa só ganha tarefas com brief (formato de `docs/superpowers/plans/2026-10-10-wingetrs-fase2a.md`) depois que o usuário decidir as perguntas abertas da spec correspondente.

| Etapa | Fonte | Bloqueio | Tarefas |
|---|---|---|---|
| 1. Lacunas da ponte | plano 2a, Tasks 113–115 | nenhum | 113 (sonda: aviso de reinício), 114 (aviso de reinício), 115 (testes do cancelamento e medição da oscilação) |
| 2. Medição de desempenho | `reports/2026-10-15-desempenho-brainstorm.md` | nenhum | passo 0 da busca (`--version` × noop, spans e `search.join_wait`); oráculo do `list` versionado e medido de novo |
| 3. Melhorias medidas | idem | resultado da etapa 2 | piso do binário (`noop_imports`, `/DELAYLOAD`, `%LOCALAPPDATA%`); caminho rápido sem tokio; fallback no `installed.db` (MSYS2); `OtherRecord.correlation`; máscara de caracteres só se `join_wait > 0` |
| 4. Fase 2b | `specs/2026-10-15-wingetrs-fase2b-design.md` | D1–D4; autorização para as sondas S2 e S3 | S1–S4, depois P1–P8 |
| 5. Fase 2c | `specs/2026-10-15-wingetrs-fase2c-design.md` | D1–D8; emenda do CLAUDE.md (D2); autorização para as sondas D2, D4 e D5 | sondas, depois P1–P16 (primeiro psmodule e innosetup) |
| 6. Fase 2d | `specs/2026-10-15-wingetrs-fase2d-design.md` | D1–D8; emenda de segurança; S1 com o usuário clicando no UAC | S1–S3, P13a (ponte de install/uninstall) cedo, depois P1–P16 |

**Paralelismo entre etapas.**
- A 1 e a 2 podem rodar juntas: não têm arquivo em comum.
- Etapas que medem tempo (2 e 3) rodam sozinhas, sem `cargo test` em paralelo.
- A 2b vem antes da 2c, porque a 2c usa o extrator da 2b.
- A 2d é independente da 2b e da 2c. A P13a dela pode entrar logo depois da etapa 1.
