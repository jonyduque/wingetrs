# Ledger — wingetrs fase 1

Plano: `docs/superpowers/plans/2026-10-07-wingetrs-fase1.md`. Uma linha por tarefa concluída; SHAs verificados com `git cat-file -t`.

| Task | Título | base..head | Status | Modelo (impl/review) | Notas |
|---|---|---|---|---|---|
| 1 | Scaffold do crate, tipos centrais e CLI | f4166ab..9149cb8 | DONE | sonnet/sonnet | revisor achou teste de flag oculta que não falhava (defeito do plano), corrigido em 9149cb8; mutate.ps1 corrigido em e528230 |
| 2 | Comparação de versões estilo winget | 068bd1d..9eb0c18 | DONE | sonnet/sonnet | revisor: comparação case-insensitive de sufixo sem teste, pinada em 9eb0c18; plano apontou teste errado p/ mutação do trim (parse_corners é o que pega) |
