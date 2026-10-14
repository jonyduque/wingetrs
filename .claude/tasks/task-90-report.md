Status: DONE

Commits: (ver git log f20232c..HEAD)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 1s
[OK]   clippy 3s
[OK]   test (696 passed, 17 ignored) 228s
[OK]   gate verde

Mutation proofs:
- guarda de lista vazia: `mutate.ps1 -Path src/tui/app.rs -Anchor 'if app.visible_len() == 0 {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test tui_state enter_in_updates'`
  [OK] mutação PEGA: 'if app.visible_len() == 0 {' -> 'if false {' fez o teste falhar (exit 101).
- Atualizar selecionável para arp: `-Anchor 'selectable: upgrade.is_ok(),' -Replacement 'selectable: true,' -TestCommand 'cargo test --quiet --test tui_state enter_in_installed'`
  [OK] mutação PEGA: 'selectable: upgrade.is_ok(),' -> 'selectable: true,' fez o teste falhar (exit 101).
- `grep -rn "Focus::Details" src tests`: saída vazia.

Decisions:
- Regra única `upgrade_target(&ListRow)` (origem wingetrs + Matched + Available) usada por `U` e pelo menu; `U` com falha continua com o toast NOT_OURS. `plan_upgrade`/`plan_remove` compartilhados por teclas e menu.
- Tipo `Focus` removido por inteiro (ficava com uma variante); `border_focus` do tema continua usado em outro lugar.
- Dicas: `enter <ação>` seguido da tecla própria (`i`/`u`/`U`), que continuam existindo.
- O picker começa no primeiro item selecionável, então Enter ao abrir o menu de um app wingetrs sem atualização escolhe "Remover".
- Testes antigos de Focus (escape_ladder, arrows..., divisor, hints) ajustados; snapshot novo actions_modal_snapshot.

Concerns: none
