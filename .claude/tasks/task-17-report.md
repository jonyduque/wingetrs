Status: DONE

Commits:
1c0c97a feat(list): include MSIX framework packages, one per family

Gate:
[OK]   fmt 1s
[OK]   clippy 7s
[OK]   test (150 passed, 2 ignored) 44s
[OK]   gate verde

Mutation proofs:
- dedupe mantendo a primeira (`if new_v > old_v {` -> `if false && new_v > old_v {`):
  [OK] mutação PEGA: 'if new_v > old_v {' -> 'if false && new_v > old_v {' fez o teste falhar (exit 101).
- sem fallback ms-resource (`!d.starts_with("ms-resource:")` -> `true`):
  [OK] mutação PEGA: '!d.starts_with("ms-resource:")' -> 'true' fez o teste falhar (exit 101).
(ambos com -TestCommand 'cargo test --quiet --test installed')

Live:
- live_scan: apps=282 errors=[] elapsed_ms=268
- live_list: rows=282 matched=133 elapsed_ms=385

Bench `list` (bench.ps1, 10 runs, 2 warmup):
- antes:  min=356,5 median=386,4 max=442,6 ms
- depois: min=411,2 median=454,6 max=479,0 ms  (< 500 ms, mas margem apertada; +~70 ms)

Oráculo (winget list --source winget --disable-interactivity x wingetrs list --json, mesmo método da Task 15; script próprio de parsing por colunas, case-insensitive):
winget distinct_ids=116 hits=110/116 rate=94.8%  (antes 81,9%; meta >= 90% ATINGIDA)
Divergências restantes (6, todas não-framework, já conhecidas da Task 15, F3): Devolutions.UniGetUI, Google.AndroidStudio, Gyan.FFmpeg, MSYS2.MSYS2, RubyInstallerTeam.RubyWithDevKit.4.0, wez.wezterm.

Decisions:
- dedupe_msix_by_family só toca itens Msix com family_name; os demais passam; FamilyName comparado case-insensitive; ordem da primeira ocorrência preservada.
- A dedupe é aplicada em collect_msix (mod.rs), não no _win.rs, conforme o brief.
- TDD: a "falha" inicial foi de compilação (funções inexistentes); não rodei vermelho separado.

Concerns:
- list subiu ~70 ms (mediana 455 ms): dentro do orçamento, margem de ~45 ms.
- Aviso de CRLF->LF do git em src/installed/mod.rs (arquivo já era CRLF); sem ação.

## Adendo: perf(installed) d82eec3
Commits agora: d82eec3 perf(installed): read MSIX display data only for the newest package per family; 1c0c97a feat(list)...
Gate: [OK] fmt / clippy / test (150 passed, 2 ignored) / gate verde.
Mudança: read_appx identifica (Id: família/nome/versão, sem resource lookup) em paralelo, escolhe a maior versão por família, e só então lê DisplayName/PublisherDisplayName do vencedor (paralelo). dedupe_msix_by_family mantida.
Diff de `list --json` antes/depois (ordenado, por linha): 282 vs 282, identical.
Bench list (3 rodadas x 10 runs), medianas:
- antes:  452,9 / 472,2 / 460,3 ms (max 490,2 / 502,5 / 523,5)
- depois: 463,3 / 458,0 / 481,4 ms (max 527,0 / 483,3 / 501,1)
Resultado: sem ganho mensurável (dentro do ruído). Conclusão: leitura de DisplayName/Publisher não domina; o gargalo de `list` está em outro ponto (F5: medir por coletor / catálogo+correlação). O orçamento de 500 ms continua estourando em máximos ocasionais.
