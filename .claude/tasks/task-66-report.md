Status: DONE

Commits:
```
8ad4d7e feat(tui): event toasts with a single wake-up deadline
```
(+ commit deste relatório, ver `git log d832dca..HEAD`)

Gate (com `CARGO_TARGET_DIR=target/t66`, porque `target/debug/wingetrs.exe` estava em uso pelo PID 3316):
```
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (579 passed, 16 ignored) 40s
[OK]   gate verde
```
Uma rodada anterior do gate falhou em `install_ops::apply_skips_a_dependency_installed_meanwhile` com `E/S: Acesso negado. (os error 5)` na extração (sem relação com a TUI; provavelmente antivírus segurando o arquivo). Rodei esse teste sozinho 3 vezes (3 ok) e o gate completo de novo: verde.

Mutation proofs:
1. Prazo
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'ToastKind::Success => Some(Duration::from_secs(3)),' -Replacement 'ToastKind::Success => Some(Duration::from_secs(30)),' -TestCommand 'cargo test --quiet --test tui_state success_toast_expires_after_three_seconds'`
   `[OK] mutação PEGA: 'ToastKind::Success => Some(Duration::from_secs(3)),' -> 'ToastKind::Success => Some(Duration::from_secs(30)),' fez o teste falhar (exit 101).`
2. Erro fixo
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'ToastKind::Error => None,' -Replacement 'ToastKind::Error => Some(Duration::from_secs(4)),' -TestCommand 'cargo test --quiet --test tui_state catalog_error_toast_stays_until_esc'`
   `[OK] mutação PEGA: 'ToastKind::Error => None,' -> 'ToastKind::Error => Some(Duration::from_secs(4)),' fez o teste falhar (exit 101).`
3. Esc não engole
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'app.toasts.iter().position(|t| t.kind == ToastKind::Error)' -Replacement 'app.toasts.iter().position(|_| true)' -TestCommand 'cargo test --quiet --test tui_state esc_ignores_timed_toasts'`
   `[OK] mutação PEGA: 'app.toasts.iter().position(|t| t.kind == ToastKind::Error)' -> 'app.toasts.iter().position(|_| true)' fez o teste falhar (exit 101).`
4. Pilha
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '.rposition(|t| t.expires.is_some())' -Replacement '.rposition(|_| true)' -TestCommand 'cargo test --quiet --test tui_state stack_keeps_three_and_drops_the_oldest_timed'`
   `[OK] mutação PEGA: '.rposition(|t| t.expires.is_some())' -> '.rposition(|_| true)' fez o teste falhar (exit 101).`
5. Despertar único (maior prazo)
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'self.toasts.iter().filter_map(|t| t.expires).min()' -Replacement 'self.toasts.iter().filter_map(|t| t.expires).max()' -TestCommand 'cargo test --quiet --test tui_state next_deadline_is_the_earliest_timed_toast'`
   `[OK] mutação PEGA: 'self.toasts.iter().filter_map(|t| t.expires).min()' -> 'self.toasts.iter().filter_map(|t| t.expires).max()' fez o teste falhar (exit 101).`
6. Silêncio do automático
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'if forced {' -Replacement 'if true {' -TestCommand 'cargo test --quiet --test tui_state forced_refresh_success_toasts_and_automatic_is_silent'`
   `[OK] mutação PEGA: 'if forced {' -> 'if true {' fez o teste falhar (exit 101).`
7. Poda em toda mensagem
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '    app.prune_toasts();' -Replacement '    if matches!(msg, Msg::Clock) { app.prune_toasts(); }' -TestCommand 'cargo test --quiet --test tui_state expired_toast_disappears_on_any_message'`
   `[OK] mutação PEGA: '    app.prune_toasts();' -> '    if matches!(msg, Msg::Clock) { app.prune_toasts(); }' fez o teste falhar (exit 101).`
8. Margem
   `pwsh -File scripts/mutate.ps1 -Path src/tui/toast.rs -Anchor 'window.right().saturating_sub(TOAST_MARGIN_X + w)' -Replacement 'window.right().saturating_sub(w)' -TestCommand 'cargo test --quiet --test tui_view toast_box_sits_top_right_with_margins'`
   `[OK] mutação PEGA: 'window.right().saturating_sub(TOAST_MARGIN_X + w)' -> 'window.right().saturating_sub(w)' fez o teste falhar (exit 101).`

Testes existentes que passaram a conferir o toast (nomes mantidos), em `tests/tui_state.rs`: `refresh_busy_is_not_offline_and_says_in_progress` (agora também confere `ToastKind::Info`), `only_network_failures_mark_offline`, `installed_failure_shows_status`, `i_on_winget_does_not_open_anything_and_says_why`, `busy_shows_status_and_rescans`, `finished_ok_rescans_and_reports`, `planning_error_becomes_status`, `u_removes_only_wingetrs_apps_on_installed_tab`, `upper_u_upgrades_selected_or_all`, o teste de boot com `Err` e `a_failed_reload_keeps_the_entries_and_shows_the_error`. Nas asserções, o texto/predicado continua igual; só a origem mudou de `a.status` para `a.toasts[0].text` (ou de `status.is_some()` para `!toasts.is_empty()`).

Decisions:
- `RefreshFinished` sem falha deixou de fazer `app.status = None`: as falhas não vão mais para o `status`, e limpar ali apagaria um `planejando…` em andamento.
- O `Esc` que fecha erro fica depois do tratamento da ajuda (`show_help`): a sobreposição de ajuda conta como diálogo e o `Esc` a fecha primeiro.
- O laço usa `wake.is_some()` + `sleep_until(from_std(wake.unwrap_or(app.now)))` (forma alternativa do brief). `view::clip` virou `pub(crate)` para o toast cortar a 3ª linha. A quebra de texto mede células com `Span::width`.
- Um toast só é desenhado se cabe inteiro acima da borda inferior da janela. Teste unitário extra em `toast.rs`, cobrindo a quebra por células com CJK e o corte em 3 linhas com `…`.

Correção pedida pelo orquestrador (toast nunca cobre diálogo):
- `view.rs`: `small_for_toasts(area)` virou `toasts_in_status_bar(app, area)` = `app.modal.is_some() || largura < 40 || altura < 12`. Com qualquer diálogo aberto, nenhuma caixa de toast é desenhada; o toast mais novo vai para a pepita de mensagem da barra de status quando `status` está vazio (mesmo caminho do terminal pequeno). A pilha e os prazos não mudam. Quando o diálogo fecha, os toasts ainda no prazo voltam a aparecer em caixa.
- Removi o `a.toasts.clear()` dos 2 testes de diálogo (`with_residues()` e `residue_conflict_warning_survives_narrow_terminals`). Os dois passam sem ele.
- Snapshot `tui_view__residues_modal_snapshot.snap` revertido ao conteúdo da base d832dca (`git diff d832dca -- tests/snapshots/` vazio). Com a regra nova, a última linha volta a mostrar ` powertoys removido`: antes era o `status`, agora é o toast de sucesso na pepita, porque há diálogo aberto e o `status` está vazio.
- Teste novo `toasts_never_cover_a_dialog` (`tests/tui_view.rs`), em 100×24 com remoção + resíduos abertos. Confere quatro coisas:
  - nenhuma célula interna do retângulo do diálogo tem `╭╮╰╯│─`, e a borda de cima não tem cantos alheios;
  - nenhuma linha contém `Sucesso`;
  - a linha 22 (barra de status) contém `powertoys removido`;
  - depois do `Esc` o diálogo fecha e aparece `+ Sucesso`.
- Mutação: `pwsh -File scripts/mutate.ps1 -Path src/tui/view.rs -Anchor '        && !toasts_in_status_bar(app, area)' -Replacement '        && !(area.width < TOAST_MIN_WIDTH || area.height < TOAST_MIN_HEIGHT)' -TestCommand 'cargo test --quiet --test tui_view toasts_never_cover_a_dialog'`
  `[OK] mutação PEGA: '        && !toasts_in_status_bar(app, area)' -> '        && !(area.width < TOAST_MIN_WIDTH || area.height < TOAST_MIN_HEIGHT)' fez o teste falhar (exit 101).` A asserção que pegou foi a de `Sucesso` (linha 1221). Em 100×24 o diálogo de resíduos, centralizado, começa abaixo da caixa do toast (linhas 4–7), então a varredura de bordas sozinha não teria pegado nesse tamanho.
- Gate depois da correção:
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 96s
[OK]   test (580 passed, 16 ignored) 116s
[OK]   gate verde
```

Pendências da revisão (opus), resolvidas:
1. `run.rs`: nova `pub fn clock_turn(app, now, msgs) -> Vec<Cmd>`, que atribui `app.now = now` e aplica o lote. O laço agora faz `propagate`, contagem e detecção de `RefreshFinished` num `map`/`inspect` e chama `clock_turn(&mut app, Instant::now(), batch)`. A atribuição existe só dentro da função (`grep -c "app.now = " src/tui/run.rs` = 1). Diferença: o `propagate` do lote inteiro agora roda antes de qualquer `update`, então um pânico de worker é relançado antes de aplicar o resto do lote. Teste `clock_turn_sets_the_clock_so_toasts_expire` (tests/tui_loop.rs).
   `pwsh -File scripts/mutate.ps1 -Path src/tui/run.rs -Anchor '    app.now = now;' -Replacement '' -TestCommand 'cargo test --quiet --test tui_loop clock_turn_sets_the_clock_so_toasts_expire'`
   `[OK] mutação PEGA: '    app.now = now;' -> '' fez o teste falhar (exit 101).`
2. `offline_refresh_failure_is_a_warning` ganhou o caso misto (uma falha offline + uma não ⇒ `Error`).
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'let kind = if all_offline {' -Replacement 'let kind = if any_offline {' -TestCommand 'cargo test --quiet --test tui_state offline_refresh_failure_is_a_warning'`
   `[OK] mutação PEGA: 'let kind = if all_offline {' -> 'let kind = if any_offline {' fez o teste falhar (exit 101).`
3. `esc_in_a_dialog_closes_the_dialog_and_keeps_the_error_toast` (tests/tui_state.rs): diálogo de confirmação + toast Error; `Esc` fecha o diálogo e o toast continua.
4. `error_toast_adds_the_esc_hint` (tests/tui_state.rs): sem erro não há a dica; com erro, `("esc", "fecha erro")` vem logo antes de `?`. `repeated_toast_shows_its_count_in_the_title` (tests/tui_view.rs): `count == 2` ⇒ `+ Sucesso ×2`.
Gate depois das pendências:
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 96s
[OK]   test (584 passed, 16 ignored) 132s
[OK]   gate verde
```

Concerns:
- Em terminal pequeno, a pepita mostra o toast mais novo enquanto ele não vence; um `Error` fica lá até `Esc`.
- Não medido: custo de desenho dos toasts e acordadas do laço (nenhum benchmark pedido).
