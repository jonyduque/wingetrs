Status: DONE

Commits: (ver git log d9dc67f..HEAD)

Gate:
[OK]   test (691 passed, 17 ignored) 182s
[OK]   gate verde

Mutation proofs:
scripts/mutate.ps1 -Path src/tui/theme.rs -Anchor '    gap: "  ",' -Replacement '    gap: " ",' -TestCommand 'cargo test --quiet --test tui_view nerd_detail'
[OK] mutação PEGA: '    gap: "  ",' -> '    gap: " ",' fez o teste falhar (exit 101).

Decisions:
- `labeled` recebe `gap: &str` como parametro extra.
- Linhas dos detalhes migradas para `g.gap` (view.rs): labeled (ID, publisher, licenca), install_status (upgrade, ok, unknown quebrada, unknown sem correspondencia), erro de detalhes (`ambiguous`), spinner "carregando...".
- Nenhum snapshot mudou: Ascii e Unicode usam " " (gap de 1 espaco), logo so Nerd muda.

Concerns: none (verify falhou uma vez por pagefile esgotado com outros agentes compilando; repetido, verde).
