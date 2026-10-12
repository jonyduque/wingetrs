Status: DONE_WITH_CONCERNS

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

Concerns:
- O toast de resumo do `on_finished` agora aparece por cima do diálogo de resíduos que abre junto com ele, cobrindo o canto superior direito do diálogo por 3 s (é o que o brief pede: toasts por cima de diálogos). Em `tests/tui_view.rs`, `with_residues()` e `residue_conflict_warning_survives_narrow_terminals` passaram a chamar `a.toasts.clear()` antes de renderizar, sem mudar as asserções. O snapshot `tui_view__residues_modal_snapshot.snap` foi atualizado: a última linha passou de `powertoys removido` (que era o `status`) para `1 de 3 · 3 no catálogo`. O orquestrador pode querer decidir se o toast deve esperar o diálogo fechar.
- Em terminal pequeno, a pepita mostra o toast mais novo enquanto ele não vence; um `Error` fica lá até `Esc`.
- Não medido: custo de desenho dos toasts e acordadas do laço (nenhum benchmark pedido).
