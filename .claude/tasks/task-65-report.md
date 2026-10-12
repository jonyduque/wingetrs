Status: DONE_WITH_CONCERNS

Commits:
(ver abaixo, preenchido apos o commit do relatorio)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 12s
[OK]   fmt 1s
[OK]   clippy 8s
[OK]   test (563 passed, 16 ignored) 96s
[OK]   gate verde

Mutation proofs (scripts/mutate.ps1):
1. [OK] mutação PEGA: 'Tab::Search => out.push(("i", "instalar")),' -> '... _ if true => out.push(("i", "instalar")),' fez o teste falhar (exit 101).  (hints_change_with_the_tab)
2. Corte inteiro: a primeira mutacao (retirar a reserva do " …") SOBREVIVEU com largura 40; reforcei o teste com varredura de larguras 0..=100. Depois:
   [OK] mutação PEGA: 'if used + lead + hint_w + reserve > width {' -> 'if used + lead + hint_w > width + 0 * reserve {' fez o teste falhar (exit 101).
   [OK] mutação PEGA: 'if cut && used + ellipsis.width() <= width {' -> 'if cut {' fez o teste falhar (exit 101).
   (mutacao literal `take(width)` nao aplicavel: o codigo nao usa take; as duas acima cobrem o limite exato de colunas.)
3. [OK] mutação PEGA: 'theme.pill_msg())],' -> 'theme.warn())],' fez o teste falhar (exit 101).
4. [OK] mutação PEGA: 'u16::from(area.height >= HELP_BAR_MIN_HEIGHT)' -> 'u16::from(area.height >= 0)' fez o teste falhar (exit 101).

Decisions:
- `chrome_rows(height)` e `HELP_BAR_MIN_HEIGHT` (20) ficam em app.rs; a view usa a mesma constante.
- Pepita offline sem glifo (" offline ", alert+BOLD), literal do brief; pepitas separadas por um espaco.
- Larguras medidas com Span::width. Snapshots atualizados (alturas <20 nao mostram a ajuda).
- Teste extra end_keeps_selection_visible_when_the_help_bar_hides (alturas 19 e 20).

Concerns:
- A ajuda completa (`?`) nao foi reorganizada em colunas: mantive o dialogo existente com as mesmas teclas (inclui "←→ / Tab"); o brief fixa so "mesmas teclas".
- Usei CARGO_TARGET_DIR=target/t65 (removido) porque target/debug/wingetrs.exe estava travado.
