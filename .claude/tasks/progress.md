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
| 11 | Coleta de instalados (ARP, MSIX, scoop) | 532fc4e..c1edf63 | DONE | sonnet/sonnet | APPROVE; live apps=250 em 270-400 ms (margem curta p/ list<500ms); MSIX paralelizado em 8 threads (DisplayName ~530 ms serial) |
| 9 | Orquestração do update e comandos CLI | 2edab99..7da0cf1 | DONE | sonnet/sonnet | APPROVE; real: winget 15403, scoop main 1669, extras 2406; search 0,26 s (meta 0,1 s → Task 15). follow-up: force=true com catálogo vazio, corrigido e testado em a3d1e74 |
| 13 | TUI: tema, glifos e estado | 9dcbd09..faa8079 | DONE | sonnet/sonnet | revisor: NoColor, encolhimento de lista e filtro da aba Updates sem teste, pinados em faa8079 |
| 12 | Correlação e comando list | 7da0cf1..e2d8737 | DONE | opus/opus | revisor provou 3 falsos positivos (MSIX sparse VS Code/Zed/Edge casados por nome+publisher); só ARP usa a heurística agora (e2d8737); list real 250 apps: 118 correlacionados, 9 ambíguos, 123 sem correlação, 362-479 ms release; perfil dev opt-level 3 p/ regex |
| 14 | TUI: views, loop de eventos e snapshots | cf53cbf..f1bc6e5 | DONE | sonnet/sonnet | revisor: tudo bloqueava antes do 1º frame (100-269 ms); reordenado em f1bc6e5 → first_frame_ms 2-4 com catálogo real (sem TTY). Navegação interativa NÃO verificada por agente |
| 15 | Validação ao vivo: oráculo e benchmarks | 1d6f6f6..13539d4 | DONE_WITH_CONCERNS | opus/orquestrador | search 192 ms (meta 100: FALHA, causa load_entries com coluna raw ~90-105 ms); list 400-409 ms OK; 1º frame 3-6 ms OK; oráculo 95/116=81,9% (FALHA; 15 são MSIX framework descartados → 94,1% sem eles); benchmarks reexecutados pelo orquestrador dentro de ±5% |
| final | Correções da revisão final do branch | 35d4372..17f5771 | DONE | sonnet/opus | -v não corrompe TUI; TUI não espera lock (NoWait→"atualização em andamento"); offline só p/ erro de rede; DataDir::from_env; REG_DWORD testado; gate 144 passed |
| 16 | search abaixo de 100 ms | cfb0981..7db907b | DONE_WITH_CONCERNS | sonnet/sonnet | 192→107 ms (meta 100 NÃO atingida; piso de processo do ambiente 38 ms, hello-world). Colunas enxutas, mmap, load e busca paralelos; revisor exigiu testes de borda/equivalência, resume_unwind e data_version (7db907b) |
