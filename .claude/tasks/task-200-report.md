# Task 200 report

Status: DONE_WITH_CONCERNS

Commits:
```
477c1ea feat(cli): every choice is an inline TUI selector
```

Gate: rodei uma vez. fmt e clippy passaram; o passo de testes falhou só em módulos de outros agentes (Task 201: elevated_*, wi_install_flow, wi_tui; Task 192 com mutação em andamento: script_scan). `target/verify.log`:
```
[OK] check_rules: 24 casos plantados
==== fmt (exit 0)
==== clippy (exit 0)
failures:
    elevated_install::folder_refusal_is_not_a_refused_uac
    elevated_worker::a_helper_that_exited_idle_is_started_again
    elevated_worker::a_lost_helper_fails_its_job_and_is_started_again
    elevated_worker::an_unsendable_request_restarts_the_helper_once
    elevated_worker::machine_scope_goes_elevated_others_do_not
    elevated_worker::refused_elevation_falls_back_once
    script_scan::package_program_call_is_confirm
    script_scan::path_in_command_position_is_a_call
    wi_install_flow::consumers_show_new_phases
    wi_tui::status_bar_waiting_uac_text
error: test failed, to rerun pass `--test it`
```
Os meus módulos, rodados à parte:
```
cli_cancel::   test result: ok. 17 passed; 0 failed
cli_scripts::  test result: ok. 25 passed; 0 failed
wi_cli::       test result: ok. 30 passed; 0 failed
--lib cli::    test result: ok. 13 passed; 0 failed
```
`cargo clippy --all-targets` filtrado por picker/install_cmd/cli_cancel/cli_scripts/wi_cli: nenhum aviso.

Mutation proofs:
- Guard desliga o raw mode no pânico: `mutate.ps1 -Path src/cli/picker.rs -Anchor 'let _ = self.term.disable_raw();' -Replacement '' -TestCommand 'cargo test --quiet --test it cli_cancel::selector::selector_restores_terminal_on_panic'`
  `[OK] mutação PEGA: 'let _ = self.term.disable_raw();' -> '' fez o teste falhar (exit 101).`
- Sem terminal o seletor não desenha nem lê: `-Anchor 'if !term.interactive() {' -Replacement 'if false && !term.interactive() {' -TestCommand '... cli_cancel::selector::selector_without_terminal_refuses'`
  `[OK] mutação PEGA: 'if !term.interactive() {' -> 'if false && !term.interactive() {' fez o teste falhar (exit 101).`
- O padrão do sim/não é "Não": `-Anchor 'yes_no_items(), Some(1), theme' -Replacement 'yes_no_items(), Some(0), theme' -TestCommand '... cli_scripts::continue_question_uses_selector'`
  `[OK] mutação PEGA: 'yes_no_items(), Some(1), theme' -> 'yes_no_items(), Some(0), theme' fez o teste falhar (exit 101).`
- `q` cancela: `-Anchor "if c == 'q' {" -Replacement "if c == '\u{0}' {" -TestCommand '... cli_cancel::selector::selector_esc_q_and_ctrl_c_cancel'`
  `[OK] mutação PEGA: 'if c == 'q' {' -> 'if c == '\u{0}' {' fez o teste falhar (exit 101).`
  (A primeira tentativa deu `[?] INCONCLUSIVO` porque o checkout não compilava naquele instante, por causa da edição de outro agente. Rodei de novo depois que o módulo passou.)
- Fica uma linha com a escolha: `-Anchor 'write!(out, "{line}\r\n")?;' -Replacement '' -TestCommand '... cli_cancel::selector::selector_arrows_and_enter_pick'`
  `[OK] mutação PEGA: 'write!(out, "{line}\r\n")?;' -> '' fez o teste falhar (exit 101).`
- O `confirm` do TtyUi passa pelo seletor: `-Path src/cli/install_cmd.rs -Anchor 'Ok(picker::yes_no(' -Replacement 'Ok(true || picker::yes_no(' -TestCommand '... cli_scripts::continue_question_uses_selector'`
  `[OK] mutação PEGA: 'Ok(picker::yes_no(' -> 'Ok(true || picker::yes_no(' fez o teste falhar (exit 101).`
- Check 1 (não editar arquivos da 201) e check 2 (nenhum teste lê o terminal real) não têm mutação. Os dois se provam pelo diff: o commit só toca os 5 arquivos acima, e todo teste usa `picker::fake::ScriptedTerm`.

Decisions:
- O componente ficou em `src/cli/picker.rs`, reescrito com `trait Term` injetável, `StdTerm` real e `pub mod fake` (`ScriptedTerm`, `plain`). Segue o precedente de `install::traits::fake`, e assim o teste unitário e o de integração usam o mesmo falso. `TtyUi::with_term` injeta o terminal.
- Atalho por letra: é a primeira letra ou dígito do rótulo, só quando é único entre as linhas escolhíveis e não é `q`. Na escolha única, a letra escolhe na hora; na múltipla, só move o cursor (quem marca é o Espaço). Sim/Não ⇒ `s`/`n`; Enter sozinho fica em "Não", como o antigo `[s/N]`.
- Cancelar não deixa linha nenhuma, porque os chamadores já imprimem "Cancelado."/"Nenhum … removido". Escolher deixa `Pergunta Resposta` ou `> rótulo(s)`, e na múltipla vazia `> nenhum`.
- Não editei `src/tui/picker.rs`, que é `src/tui/*` e portanto da 201. O `q` e as letras ficam num `pre_key` no CLI; Ctrl+C e Esc continuam com o `Picker` existente.
- Testes do componente em `tests/it/cli_cancel.rs` (`mod selector`), porque não posso criar módulo novo no `main.rs`. Um teste por pergunta: `cli_scripts::{continue_question_uses_selector, ambiguous_target_uses_selector, no_tty_never_prompts}`, `wi_cli::{kindle_question_uses_selector, orphans_question_uses_selector, ambiguous_installed_uses_selector}` e resíduos no teste unitário `install_cmd::tests::yes_cleans_only_high_confidence_preselected_residues`. O `wi_cli.rs` está fora do padrão `cli_*` da lista Files, mas era lá que estavam os falsos da Kindle e dos órfãos.

Concerns:
- O `confirm` do `bucket add` (`src/cli/mod.rs`, fora da lista Files) já passa pelo `TtyUi::confirm`, portanto pelo seletor, mas não tem teste próprio: ele monta `TtyUi::new` com o terminal real.
- Na Kindle os rótulos "Copiar…"/"Cancelar" e "Remover sem…"/"Remover só…" repetem a inicial, então essas linhas ficam sem atalho por letra (só setas + Enter). Se o atalho tiver que valer sempre, o próximo passo seria um atalho numérico.
- Sem `.join` com dado externo nesta tarefa.
- A barra de progresso não é encerrada antes de uma pergunta (comportamento de antes, sem mudança).

## Rodada 2 (revisão CHANGES_REQUESTED)

Status: DONE

Commits (`git log --format='%h %s' d16cf51..HEAD -- <arquivos da tarefa>`):
```
6a8381c style(buckets): rustfmt the bucket confirmation message
e185089 fix(cli): selector review: width, height, resize, letters, bucket add
477c1ea feat(cli): every choice is an inline TUI selector
```
Do `src/cli/winget_cmd.rs` só entraram os meus 2 trechos, via `git apply --cached --recount`: `pick_row` → `pick_target` e os rótulos da Kindle. Conferi com `git show :src/cli/winget_cmd.rs | rustfmt --check --edition 2024`, que passou. Os trechos da 201 continuam fora do índice.

Gate: rodei uma vez e falhou no passo de regras, num arquivo da Task 201:
```
==== rules (exit 1)
  tests\it\wi_upgrade.rs:711: [limite de relogio em teste] while w.phases.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(10) {
[FAIL] rules exit 1
```
O `cargo fmt --check` também acusa só `src\winget_install\removal.rs:187`, que é da 201. Os meus, rodados à parte:
```
cli_cancel::  test result: ok. 20 passed; 0 failed
cli_scripts:: test result: ok. 27 passed; 0 failed
wi_cli::      test result: ok. 31 passed; 0 failed
buckets::     test result: ok. 13 passed; 0 failed
--lib cli::   test result: ok. 13 passed; 0 failed
```
`cargo clippy --all-targets` filtrado pelos meus arquivos: 0 avisos.

O que mudou:
1. `bucket add`: a função nova `cli::bucket_add(ui: &dyn Ui, cfg, name, url, yes, say)` recebe o `Ui`, e o `run_bucket` monta o `TtyUi` e chama ela. A mensagem de `NeedsConfirmation` (`src/buckets.rs`, fora da lista Files) agora cita a flag: "confirme num terminal ou rode de novo com --yes". O teste `cli_scripts::bucket_add_question_uses_selector` cobre os quatro casos: sem TTY ⇒ erro com `--yes` e nada desenhado; Enter ⇒ "Cancelado." e o bucket não entra; `s` ⇒ adiciona; `--yes` ⇒ adiciona sem ler tecla.
2. O corte por largura agora é em colunas: `picker::fit` mede com `Span::raw(..).width()`, então CJK e emoji contam 2. Teste `selector_cuts_lines_to_the_terminal_width`, em 20x24, com toda linha ≤ 19 colunas.
3. Altura: `selector_window_follows_the_cursor_in_a_short_terminal`. Em 80x8, 30 itens e seta até o fim: todo quadro tem ≤ 7 linhas e o último item fica visível. Em 80x40 os quadros têm 16 linhas (15 + dica), e é esse caso que pega o `clamp(1, 1000)`: em 80x8 a janela é 6 e o mutante não muda nada.
4. `Event::Resize` relê o tamanho e redesenha. Cada redesenho apaga o quadro anterior (`MoveUp(drawn)` + `Clear(FromCursorDown)`) e atualiza `drawn`, então o apagamento final cobre o último quadro. Teste `selector_redraws_after_a_resize`: os saltos verticais são `[16, 7, 7, 16]`.
5. Rótulos da Kindle: "Backup das pastas e remover" / "Remover sem copiar" / "Só o registro" / "Cancelar", com atalhos b/r/s/c. Atualizei os testes antigos `cli_kindle_*`.
6. Na escolha única cada linha mostra a letra (`s) Sim`). Linhas com inicial repetida ficam com 3 espaços, e se nenhuma linha tem letra a coluna some. O `Ui::pick_target` é um método novo com padrão = `pick`, então os falsos não mudam. O `TtyUi` o implementa com `Letters::Move`: a letra só move o cursor e o Enter confirma. Ele é usado em `choose` (alvo ambíguo `bucket/nome`) e em `pick_row` (apps instalados com o mesmo nome). Teste `cli_scripts::target_letter_only_moves_the_cursor`.
7. `wi_cli::kindle_without_terminal_refused`: sem TTY e sem `--yes` dá o erro "rode num terminal para escolher", sem trava, sem cópia, sem desinstalador e sem desenhar nada.

Provas de mutação (novas):
```
[OK] mutação PEGA: 'fit(&l.text, width)' -> 'l.text' fez o teste falhar (exit 101).            (selector_cuts_lines_to_the_terminal_width)
[OK] mutação PEGA: '.clamp(1, 15)' -> '.clamp(1, 1000)' fez o teste falhar (exit 101).        (selector_window_follows_the_cursor_in_a_short_terminal)
[OK] mutação PEGA: 'height = window_height(r, prompt);' -> 'let _ = r;' fez o teste falhar (exit 101).  (selector_redraws_after_a_resize)
[OK] mutação PEGA: '.pick_target(items)?' -> '.pick(items)?' fez o teste falhar (exit 101).   (install_cmd.rs; target_letter_only_moves_the_cursor)
[OK] mutação PEGA: 'if ui.interactive() =>' -> 'if true =>' fez o teste falhar (exit 101).     (cli/mod.rs; bucket_add_question_uses_selector)
[OK] mutação PEGA: 'if !ui.confirm(&q)? {' -> 'if !true {' fez o teste falhar (exit 101).      (cli/mod.rs; bucket_add_question_uses_selector)
```
Comando: `pwsh -File scripts/mutate.ps1 -Path <arq> -Anchor '<âncora>' -Replacement '<troca>' -TestCommand 'cargo test --quiet --test it <teste>'`.

Não fiz mutação no `src/cli/winget_cmd.rs` (bail da Kindle sem TTY). O `mutate.ps1` reescreve e restaura o arquivo inteiro, e a 201 está editando esse arquivo: as edições dela no meio da janela se perderiam. A recusa está coberta por `kindle_without_terminal_refused`.

Concerns:
- Os rótulos da Kindle no CLI agora diferem dos da TUI (`REGISTRY_ONLY_ITEM` = "Remover só o registro", que o `wi_tui` confere). Não mexi na TUI porque `src/tui/*` é da 201.
