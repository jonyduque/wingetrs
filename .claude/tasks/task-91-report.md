Status: DONE

Commits: (ver abaixo, preenchido após o commit)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 3s
[OK]   test (701 passed, 18 ignored) 43s
[OK]   gate verde

Mutation proofs:
- State com UpToDate primeiro: `mutate.ps1 -Path src/tui/app.rs -Anchor 'UpdateState::UpToDate => 3,' -Replacement 'UpdateState::UpToDate => 0,' -TestCommand 'cargo test --quiet --test tui_state installed_sorts_by'`
  [OK] mutação PEGA: 'UpdateState::UpToDate => 3,' -> 'UpdateState::UpToDate => 0,' fez o teste falhar (exit 101).
- Não reposicionar selected: `-Anchor '.unwrap_or(0);' -Replacement '.unwrap_or(0) * 0 + self.selected;' -TestCommand 'cargo test --quiet --test tui_state sort_keeps'`
  [OK] mutação PEGA: '.unwrap_or(0);' -> '.unwrap_or(0) * 0 + self.selected;' fez o teste falhar (exit 101).

Medida: `cargo test --release --test tui_state timing -- --ignored --nocapture`: mediana de 200 chamadas de installed_view (2000 linhas, State) = 751 us (< 1 ms). Medido uma vez, sem A/B.

Decisions:
- `App::sort_key()` e `cycle_sort()` em app.rs; `o` também devolve `details_cmd()` (o item selecionado pode mudar de linha, não de pacote, mas o custo é nulo).
- Dica `o ordem: nome|fonte|estado` constante (não mostra a ordem atual), como o brief especifica.
- Chave de ordenação é tupla (u8, String, String) para um só tipo em Name/Source/State; Source usa o texto da fonte, sem correlação = grupo 1.
- Testes existentes ajustados: `cached_installed_view_matches_a_fresh_filter` (esperado agora ordenado por nome, tests/tui_state.rs) e `hints_change_with_the_tab` (tests/tui_view.rs, nova dica `o`).

Concerns: none
