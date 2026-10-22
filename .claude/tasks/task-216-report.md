Status: DONE_WITH_CONCERNS

Commits:
03c14a8 feat(cli): themed progress bar and spinner shared with the TUI; phases in update and list

Gate:
```
[OK]   rules 13s
[OK]   rules-selftest 70s
[OK]   fmt 7s
[OK]   licenses 23s
[OK]   clippy 48s
[OK]   test (2016 passed, 89 ignored) 126s
[OK]   gate verde
```
(rodado com o trabalho da Task 222 ainda não commitado no checkout; o primeiro gate falhou só em `rules`: `src\update.rs:206: [spawn_blocking sem handle]` no meu `lock_within`, corrigido ligando o handle a `let lock =`.)

Mutation proofs (script `t216-mutate.ps1` no scratchpad; cada mutante falhou por asserção, não por compilação: `panicked at tests\it\...` conferido no log):
1. `pwsh -File scripts/mutate.ps1 -Path src/tui/view.rs -Anchor '    download_text(&p.label, &download_parts(&p.meter, now, g))' -Replacement '<formato antigo "{} [{}{}] {pct}%">' -TestCommand 'cargo test --quiet --test it install_progress::cli_and_tui_share_download_text'`
   `[OK] mutação PEGA: '    download_text(&p.label, &download_parts(&p.meter, now, g))' -> '    { let _ = (g, download_parts, download_text); let pct = ...; format!("{} [{}{}] {pct}%", p.label, "#".repeat(n), "-".repeat(20 - n)) }' fez o teste falhar (exit 101).`
2. `-Path src/tui/app.rs -Anchor 'let mut meter = r.progress.take().map(|p| p.meter).unwrap_or_default();' -Replacement 'let mut meter = crate::install::progress::DownloadMeter::default();' -TestCommand '... install_progress::tui_download_shows_size_speed_and_stall'`
   `[OK] mutação PEGA: 'let mut meter = r.progress.take().map(|p| p.meter).unwrap_or_default();' -> 'let mut meter = crate::install::progress::DownloadMeter::default();' fez o teste falhar (exit 101).`
3. `-Path src/cli/output.rs -Anchor '(self.spinner.clone(), Some(styles.spinner)),' -Replacement '(self.spinner.clone(), Some(WARN)),' -TestCommand '... install_progress::progress_frame_paints_with_theme_tokens'`
   `[OK] mutação PEGA: '(self.spinner.clone(), Some(styles.spinner)),' -> '(self.spinner.clone(), Some(WARN)),' fez o teste falhar (exit 101).`
4. `-Path src/cli/output.rs -Anchor 'Some(s) => out.push_str(&paint(s, &kept)),' -Replacement 'Some(s) => out.push_str(&style::paint_raw(s, &kept)),' -TestCommand '... install_progress::progress_frame_plain_without_color'`
   `[OK] mutação PEGA: 'Some(s) => out.push_str(&paint(s, &kept)),' -> 'Some(s) => out.push_str(&style::paint_raw(s, &kept)),' fez o teste falhar (exit 101).`
5. `-Path src/cli/output.rs -AnchorFile t216-anchor5.txt -ReplacementFile t216-repl5.txt` (pinta cada pedaço inteiro e só depois corta `out.chars().take(cols)`) `-TestCommand '... install_progress::progress_frame_cuts_before_painting'`
   `[OK] mutação PEGA: '    for (text, piece_style) in parts.pieces(styles) { ... (out, n)' fez o teste falhar (exit 101).` (falhou em `install_progress.rs:1694`, a contagem de 30 colunas)
6a. `-Path src/update.rs -Anchor '        progress(&ProgressEvent::Phase(Phase::WritingCatalog));' -Replacement '' -TestCommand '... install_progress::update_reports_phases_in_order'`
   `[OK] mutação PEGA: '        progress(&ProgressEvent::Phase(Phase::WritingCatalog));' -> '' fez o teste falhar (exit 101).`
6b. `-Path src/update.rs -Anchor 'sources: fetching,' -Replacement 'sources: jobs.len(),' -TestCommand '... install_progress::update_reports_phases_in_order'`
   `[OK] mutação PEGA: 'sources: fetching,' -> 'sources: jobs.len(),' fez o teste falhar (exit 101).` (falhou na 2ª rodada, `install_progress.rs:1769`)
7. `-Path src/cli/mod.rs -Anchor 'let line = (!background).then(|| install_cmd::line_progress(&cfg));' -Replacement 'let line = Some(install_cmd::line_progress(&cfg));' -TestCommand 'cargo test --quiet --test it cli_color::background_update_prints_nothing'`
   `[OK] mutação PEGA: 'let line = (!background).then(|| install_cmd::line_progress(&cfg));' -> 'let line = Some(install_cmd::line_progress(&cfg));' fez o teste falhar (exit 101).`

Check 8 (colados):
- Teste 4, escapes trocados por `<ESC>`:
  `<ESC>[38;5;99m⣾<ESC>[0m Baixando <ESC>[38;5;99m█████<ESC>[0m<ESC>[38;5;60m░░░░░░░░░░░░░░░<ESC>[0m 25%  5.0 / 20.0 MiB`
- Teste 1 (asserções exatas): Unicode `filled == "█████"`, `empty == "░"×15`, `tail == "25%  5.0 / 20.0 MiB"`; Ascii `Baixando [#####---------------] 25%  5.0 / 20.0 MiB`.
- Teste 2, `fake::plain` da linha do CLI (o pedaço depois de `Baixando ` é idêntico ao da TUI depois de `rust.msi `):
  `| Baixando [######--------------] 34%  9.3 / 27.0 MiB · 3.1 MiB/s · ~6 s` (3 s)
  `| Baixando [######--------------] 34%  9.3 / 27.0 MiB · sem dados há 12 s` (15 s)
- Bytes de controle crus (< 0x20 fora de tab/LF/CR): `install_progress.rs` 0, `output.rs` 0, `style.rs` 0; `cli_color.rs` 4, os mesmos 4 que já estavam na base.

Grep do brief (`stderr` em cli_offline/cli_color/cli_install): nenhuma asserção exige stderr vazio em `update`/`search` com catálogo vazio (as exatas, `cli_color.rs:255` e `cli_install.rs:586`, rodam com o catálogo já criado). Nenhuma mudou.

Arquivos fora da lista Files:
- `tests/it/update.rs`: `setup()` virou `pub` (o teste 8 usa o wiremock dele, como o brief pede).
- `tests/it/wi_install_flow.rs:909`: `LineProgress::new` ganhou o 3º argumento (`Default::default()`).
- `tests/it/snapshots/it__tui_view__queue_modal_snapshot.snap`: a barra de status da TUI agora mostra o tamanho (`50%  0.0 / 0.0 MiB`; o teste manda 50 de 100 bytes). É a mudança que o item 6 pede.

Decisions:
- `LineParts` (spinner, label, open/filled/empty/close, rest, warn) e `LineParts::plain()`; `progress_frame` corta pedaço a pedaço na ordem (o aviso é o último, então sai primeiro, como o `fit_cols`, que foi removido junto com `output::progress_line`).
- `line_progress(cfg)` usa `progress_for(cfg, tty)`, a mesma função que `TtyUi::build` chama (o build precisa do `tty` falso de `with_term`); tema e glifos em `theme_of`/`glyphs_of`.
- A TUI grava no medidor com uma chave fixa: o `ProgressThrottle` já manda a soma de todos os downloads da ação.
- O teste 8 monta um `extras.tgz` 200 com prioridade sobre o 500 do `setup()`: com o 500, a fonte nunca fica em dia e a 2ª rodada teria `sources: 1`.
- `list` passa a sua linha para `ensure_catalog_ran` (novo parâmetro `line`), assim a atualização bloqueante de um catálogo vazio aparece na mesma linha. Os demais chamadores abrem uma linha própria e fecham antes de devolver.

Concerns:
- TDD: os testes foram escritos depois da implementação, então não vi o vermelho antes. As 8 mutações acima fazem esse papel: cada uma derruba o teste dela por asserção.
- Teste 13 ao vivo (`cli_color::list_progress_plain_off_a_terminal`, `#[ignore = "live: varre o registro real"]`): não rodou. `cargo test --release` passou de 15 min compilando e o `timeout 900` cortou. O orquestrador precisa rodar: `cargo test --quiet --release --test it -- cli_color::list_progress_plain_off_a_terminal --ignored`.
- O impacto do `list` no tempo (montar o tema e uma linha de progresso a mais) não foi medido.

## Minors da revisão (commit 10c75d4)
1. Teste `install_progress::update_no_wait_busy_without_waiting_phase` (lock ocupado + `NoWait` ⇒ `Busy`, fases `[Preparing]`).
   `[OK] mutação PEGA: 'if lock.is_none() && !wait.timeout().is_zero() {' -> 'if lock.is_none() {' fez o teste falhar (exit 101).`
2. TUI `progress_text`: com `meter.finished()` mostra `{label} Instalando…` (como o CLI), sem "sem dados". Teste `tui_finished_download_is_no_stall`.
   `[OK] mutação PEGA: '    if p.meter.finished() {' -> '    if false && p.meter.finished() {' fez o teste falhar (exit 101).`
3. `RunProgress.done`/`total` removidos; `tests/it/tui_state.rs:3249` passa a ler `p.meter.done()` (arquivo fora da lista, 1 linha).
4. Doc de `progress_for` corrigida ("animated when `tty`").
Gate: `[FAIL] fmt exit 1 — Diff in src\install\executor.rs:3018` (arquivo da 219/220, em edição). Meus arquivos: `rustfmt --check` OK, `cargo clippy --all-targets` sem aviso, `cargo test --test it -- install_progress:: tui_ cli_color:: update:: wi_install_flow::` ⇒ `test result: ok. 501 passed; 0 failed; 6 ignored`.
