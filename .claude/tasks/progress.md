# Ledger — wingetrs fase 1

Plano: `docs/superpowers/plans/2026-10-07-wingetrs-fase1.md`. Uma linha por tarefa concluída; SHAs verificados com `git cat-file -t`.

| Task | Título | base..head | Status | Modelo (impl/review) | Notas |
|---|---|---|---|---|---|
| 1 | Scaffold do crate, tipos centrais e CLI | f4166ab..9149cb8 | DONE | sonnet/sonnet | revisor achou teste de flag oculta que não falhava (defeito do plano), corrigido em 9149cb8; mutate.ps1 corrigido em e528230 |
| 2 | Comparação de versões estilo winget | 068bd1d..9eb0c18 | DONE | sonnet/sonnet | revisor: comparação case-insensitive de sufixo sem teste, pinada em 9eb0c18; plano apontou teste errado p/ mutação do trim (parse_corners é o que pega) |
| 3 | Decodificador MSZIP e versionData | ec75b34..e52ef17 | DONE | sonnet/sonnet | APPROVE sem Critical/Important; revisor confirmou janela de 32 KiB do total e mutação própria pega |
| 4 | Manifesto merged do winget | e52ef17..61055c5 | DONE | sonnet/sonnet | revisor: herança de Scope e fallback de homepage sem teste, pinados em 61055c5; achado de CRLF recusado (fixture tem 180 CR, sha confere) |
| 6 | Diretório de dados, cache e HTTP condicional | e52ef17..e15b29f | DONE | sonnet/sonnet | APPROVE; desvio wiremock (headers divididos por vírgula) aceito e plano corrigido |
| 5 | Catálogo SQLite e busca fuzzy | 084c7a7..3139123 | DONE | sonnet/sonnet | implementador pinou prioridade de match exato (mutação sobrevivia no plano); revisor: lowercase de chaves, índices de char e WAL pinados em 3139123 |
| 10 | Porte do NameNormalizer | 0d33f6a..f0d218e | DONE | opus/sonnet | golden 1137/1137 (conferido); follow-up conhecido: NFKC não aplicado e FoldCase≈to_lowercase (ß, sigma final, İ) |
| 7 | Fonte winget: índice, integridade e detalhes | b0c1bf6..c71e3e9 | DONE | sonnet/sonnet | revisor: contagem de blocos e FileHash sem teste; fixtures novas (3732495) revelaram BUG real: FileHash (elemento filho) nunca era checado — corrigido em c71e3e9 |
| 8 | Fonte scoop: tarball, manifestos e detalhes | b0c1bf6..3c0de53 | DONE | sonnet/sonnet | revisor: filtro de caminho e bin por arquitetura sem teste, pinados; char::from(92) trocado por escape; fallback neutral p/ architecture vazio |
