Status: DONE_WITH_CONCERNS

Commits:
```
761ce77 fix(tui): navigation never waits for details; one frame per batch
30a4279 feat(tui): count frames, keys, ticks and draw time in the event loop
```
(`git log 3a398a8..HEAD` também lista 0651046 e 89845c0, que são do agente paralelo de testes, não desta tarefa.)

Gate (rodado num worktree isolado em HEAD + só os arquivos desta tarefa, porque o checkout compartilhado tinha `src/cli/mod.rs` em edição pelo outro agente, que quebrava o clippy):

Commit 1 (contadores):
```
[OK]   rules 3s
[OK]   rules-selftest 12s
[OK]   fmt 2s
[OK]   clippy 3s
[OK]   test (467 passed, 16 ignored) 159s
[OK]   gate verde
```
Commit 2:
```
[OK]   rules 3s
[OK]   rules-selftest 11s
[OK]   fmt 3s
[OK]   clippy 80s
[OK]   test (477 passed, 16 ignored) 381s
[OK]   gate verde
```

Mutation proofs (rodadas no mesmo worktree isolado, com `pwsh -File scripts/mutate.ps1 -Path <p> -Anchor <a> -Replacement <r> -TestCommand <t>`):
1. app.rs, `fn move_to(&mut self, target: usize) -> Vec<Cmd> {` -> `... { if self.details_pending.is_some() { return Vec::new(); }`, test `cargo test --quiet --test tui_state selection_moves_while_details_are_pending`
   `[OK] mutação PEGA: 'fn move_to(&mut self, target: usize) -> Vec<Cmd> {' -> 'fn move_to(&mut self, target: usize) -> Vec<Cmd> { if self.details_pending.is_some() { return Vec::new(); }' fez o teste falhar (exit 101).`
2. app.rs (set_tab), `cmds.extend(self.details_cmd());` -> ``, test `every_view_change_requests_details_for_the_new_selection`
   `[OK] mutação PEGA: 'cmds.extend(self.details_cmd());' -> '' fez o teste falhar (exit 101).`
3. app.rs, `if app.details_pending.as_deref() == Some(key.as_str()) {` -> `if true {`, test `stale_details_keep_the_spinner_for_the_current_item`
   `[OK] mutação PEGA: 'if app.details_pending.as_deref() == Some(key.as_str()) {' -> 'if true {' fez o teste falhar (exit 101).`
4. run.rs, `MissedTickBehavior::Skip` -> `MissedTickBehavior::Burst`, test `--test tui_loop spinner_ticker_skips_missed_ticks`
   `[OK] mutação PEGA: 'MissedTickBehavior::Skip' -> 'MissedTickBehavior::Burst' fez o teste falhar (exit 101).`
5. run.rs, `while batch.len() < cap {` -> `while batch.len() < usize::MAX {`, test `drain_ready_takes_what_is_queued_up_to_the_cap`
   `[OK] mutação PEGA: 'while batch.len() < cap {' -> 'while batch.len() < usize::MAX {' fez o teste falhar (exit 101).`
6. run.rs, `.rposition(|c| matches!(c, Cmd::LoadDetails { .. }));` -> `.position(...)`, test `last_details_only_keeps_the_newest_request`
   `[OK] mutação PEGA: '.rposition(|c| matches!(c, Cmd::LoadDetails { .. }));' -> '.position(|c| matches!(c, Cmd::LoadDetails { .. }));' fez o teste falhar (exit 101).`
7. run.rs, `batch.iter().any(|m| !matches!(m, Msg::Tick) || animating)` -> `{ let _ = (batch, animating); true }`, test `idle_ticks_do_not_draw`
   `[OK] mutação PEGA: 'batch.iter().any(|m| !matches!(m, Msg::Tick) || animating)' -> '{ let _ = (batch, animating); true }' fez o teste falhar (exit 101).`
8. run.rs, `Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),` -> `Err(e) if e.is_panic() => Msg::DetailsLoaded { key, result: Err(e.to_string()) },`, test `details_worker_panic_reaches_the_loop`
   `[OK] mutação PEGA: 'Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),' -> 'Err(e) if e.is_panic() => Msg::DetailsLoaded { key, result: Err(e.to_string()) },' fez o teste falhar (exit 101).`
9. Medição na TUI real (30 s parado + 5 s segurando seta): não medido (sem TUI aberta). Fica para o orquestrador/usuário comparar a linha `tui stats` de 30a4279 com a de 761ce77.

Decisions:
- O laço drena teclas e `rx` juntos com `futures_util::stream::select(filter_map(EventStream), poll_fn(rx.poll_recv))` numa única chamada a `drain_ready` (cap 256); um erro do `EventStream` durante a drenagem é guardado e encerra o laço com `Err` logo após.
- `load_details`: o `spawn_blocking` interno antes convertia pânico em erro com `??`; agora o pânico é relançado (`resume_unwind`) para que a tarefa de trabalho entre em pânico e `join_details` entregue `Msg::Panicked`.
- Contadores ficam numa struct `LoopStats` cujo `Drop` emite `tui stats`, então todo caminho de saída (Quit, Err, pânico) registra.
- Ordem dos comandos em `Booted`: `ScanInstalled`, `LoadDetails`, depois `Refresh`.

Concerns:
- Três testes existentes de `tests/tui_state.rs` afirmavam o comportamento antigo e foram ajustados: `escape_ladder_and_quit` (Esc que limpa a busca agora devolve `LoadDetails` da nova seleção), `boot_with_a_populated_fresh_catalog_loads_it_and_does_not_refresh` e `boot_refreshes_when_the_catalog_is_empty_or_stale` (Booted com catálogo não vazio agora inclui `LoadDetails`). O brief exige essas mudanças.
- Teste 2 (tui_view): as linhas de `TestBackend` vêm entre aspas, então o teste remove a `"` inicial antes de testar `starts_with("│> ")`.
- Os testes 1 e 2 passam também no código antigo, como o brief prevê (são guardas).

## Review round 1 (CHANGES_REQUESTED: drain waker, refresh join)

Status: DONE

Commits:
```
469266e fix(tui): drain input with the task's waker; await the refresh task
```

Gate (worktree isolado em 761ce77 + `src/tui/run.rs` e `tests/tui_loop.rs`; o checkout compartilhado tem a Task 48 em andamento em tui_state/tui_jobs/jobs.rs):
```
[OK]   rules 8s
[OK]   rules-selftest 12s
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (479 passed, 16 ignored) 77s
[OK]   gate verde
```

Mutation proofs:
9. Drenagem com o waker da tarefa: `-Anchor 'match std::future::poll_fn(|cx| Poll::Ready(more.poll_next_unpin(cx))).await {' -Replacement 'match futures_util::FutureExt::now_or_never(more.next()).map_or(Poll::Pending, Poll::Ready) {' -TestCommand 'cargo test --quiet --test tui_loop drain_ready_leaves_the_task_waker_with_a_pending_stream'`
   `[OK] mutação PEGA: 'match std::future::poll_fn(|cx| Poll::Ready(more.poll_next_unpin(cx))).await {' -> 'match futures_util::FutureExt::now_or_never(more.next()).map_or(Poll::Pending, Poll::Ready) {' fez o teste falhar (exit 101).`
   (Conferido à mão: a mutação compila e falha na asserção `recorded.will_wake(&task)`, tests/tui_loop.rs:96.)
10. Pânico do refresh chega ao laço: anchor de duas linhas `Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),\n        Err(e) => Msg::RefreshFinished` -> `Err(e) if e.is_panic() => Msg::RefreshFinished(vec![]),\n        Err(e) => Msg::RefreshFinished`, test `refresh_worker_panic_reaches_the_loop`
   `[OK] mutação PEGA: 'Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),` (o resto da linha foi cortado pela quebra de linha da âncora; exit 101)

Decisions:
- `drain_ready` agora é `async` e faz o poll com `std::future::poll_fn(|cx| Poll::Ready(more.poll_next_unpin(cx)))`. O `rx` continua dentro do mesmo `stream::select` (via `poll_fn(rx.poll_recv)`), que agora também recebe o waker real. Teste 7 virou `#[tokio::test]` e ganhou `.await`; as asserções não mudaram.
- `spawn_refresh`: o trabalho devolve `Msg` (`RefreshBusy`/`RefreshFinished`) e é aguardado por `pub async fn join_refresh`: pânico vira `Msg::Panicked`, cancelamento vira `RefreshFinished` com falha "tarefa cancelada".

Concerns: none
