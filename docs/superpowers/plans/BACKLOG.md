# wingetrs — fila depois das fases 2c/2d

Decisão do usuário (2026-10-18): estes itens esperam o fim das fases 2c e 2d. Cada um vira tarefa com brief quando chegar a vez.

| # | Item | Dado medido | Antes de começar |
|---|---|---|---|
| A | `list` mais rápido: `scan_msix_props` | 2026-10-17, release: `list` 293 ms (delta 262 ms sobre o processo vazio); `scan_all` 195 ms, dos quais `scan_msix_props` 155–168 ms; `normalizer_new` 64 ms em paralelo; `correlate` 30 ms. Ganho estimado ~150 ms. | Spans por etapa já existem; A/B com `scripts/bench.ps1 -Baseline`. |
| B | Quadros lentos na TUI | Quadros de até 190 ms vistos na medição de 2026-10-17. | Medir primeiro o que custa no quadro (`examples/tui_frame.rs`), não otimizar no palpite. |
| C | Nota de aplicabilidade no `list --upgradable` do CLI | A TUI já mostra `Denied`/`ScopeMismatch` (Task 135; Android Studio, Anki); o CLI não. `explicit_only` ainda sem leitor. | Reusar `bridge::winget_applicability`; buscar manifestos com o mesmo limite de concorrência da TUI. |
| D | Remover várias linhas ARP/MSIX marcadas na TUI | Hoje são "pulados" (Task 187); cada uma exige escolhas próprias (cópia de pastas, Kindle). | Fila de `PlanWingetUninstall` um após o outro, cada um com sua confirmação. |
