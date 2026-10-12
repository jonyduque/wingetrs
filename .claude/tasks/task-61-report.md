Status: DONE
Commits:
a97adf9 feat(tui): Charm palette tokens and Dot spinner
Gate:
[OK]   clippy 77s
[OK]   test (512 passed, 16 ignored) 41s
[OK]   gate verde
(uma rodada anterior falhou em tests/config.rs, arquivo da Task 67 em edicao; passou na seguinte)
Mutation proofs:
1. [OK] mutação PEGA: 'pick(((0x7D, 0x56, 0xF4), 99), ((0x87, 0x4B, 0xFD), 99))' -> 'pick(((122, 162, 247), 99), ((0x87, 0x4B, 0xFD), 99))' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: '((0x87, 0x4B, 0xFD), 99));' -> '((0x7D, 0x56, 0xF4), 99));' fez o teste falhar (exit 101).
3. [OK] mutação PEGA: 'if depth == ColorDepth::NoColor {' -> 'if false {' fez o teste falhar (exit 101).  (remove REVERSED em NoColor)
4. check_rules.ps1 verde (dentro do gate).
Decisions: help_*/sel_bar: primeiro valor escuro, segundo claro, conforme o brief; sel_desc = sel_bar; Unicode tambem usa BRAILLE renomeado para frames Dot (nome da const mantido).
Concerns: none. Nenhum .snap alterado.
Addendum (review Minor): added theme_table_matches_the_brief in tests/tui_theme.rs.
[OK] mutação PEGA: 'border_focus: fg(accent),' -> 'border_focus: fg(border),' fez o teste falhar (exit 101).
Gate: [OK]   test (518 passed, 16 ignored) / [OK]   gate verde
