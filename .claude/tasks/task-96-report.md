# Task 96 report

Status: DONE

Commits:
(see `git log --format='%h %s' fdd9d42..HEAD` — the single commit of this task, SHA in the orchestrator message)

Gate:
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 2s
[OK]   clippy 16s
[OK]   test (745 passed, 18 ignored) 165s
[OK]   gate verde
```

Mutation proofs (CARGO_TARGET_DIR=target/t96; the failing test name is in each output):

1. Second action runs at once, ignoring `running`:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'app.running.is_none() && !app.waiting_lock && !residues_pending(app)' -Replacement '!app.waiting_lock && !residues_pending(app)' -TestCommand 'cargo test --quiet --test tui_state second_action_waits_in_queue'`
   `[OK] mutação PEGA: 'app.running.is_none() && !app.waiting_lock && !residues_pending(app)' -> '!app.waiting_lock && !residues_pending(app)' fez o teste falhar (exit 101).` (`second_action_waits_in_queue --- FAILED`)
2. No pause with `Residues` open:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'app.running.is_none() && !app.waiting_lock && !residues_pending(app)' -Replacement 'app.running.is_none() && !app.waiting_lock' -TestCommand 'cargo test --quiet --test tui_state residues_dialog_pauses_the_queue'`
   `[OK] mutação PEGA: 'app.running.is_none() && !app.waiting_lock && !residues_pending(app)' -> 'app.running.is_none() && !app.waiting_lock' fez o teste falhar (exit 101).` (`residues_dialog_pauses_the_queue --- FAILED`)
3. `Busy` puts the action at the back:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'app.queue.push_front(Queued { id, label, action });' -Replacement 'app.queue.push_back(Queued { id, label, action });' -TestCommand 'cargo test --quiet --test tui_state busy_requeues_at_front_and_retries'`
   `[OK] mutação PEGA: 'app.queue.push_front(Queued { id, label, action });' -> 'app.queue.push_back(Queued { id, label, action });' fez o teste falhar (exit 101).` (`busy_requeues_at_front_and_retries --- FAILED`)
4. `q` quits at once with `running`:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor "KeyCode::Char('q') if empty => request_quit(app)," -Replacement "KeyCode::Char('q') if empty => vec![Cmd::Quit]," -TestCommand 'cargo test --quiet --test tui_state quit_waits_for_running'`
   `[OK] mutação PEGA: 'KeyCode::Char('q') if empty => request_quit(app),' -> 'KeyCode::Char('q') if empty => vec![Cmd::Quit],' fez o teste falhar (exit 101).` (`quit_waits_for_running --- FAILED`)
5. `Cmd::RetryQueue` handle rule (`src/tui/run.rs:1038`): no worker is spawned; the sleep is awaited inside the task itself, same shape as `spawn_details`' debounce:
   ```
   fn spawn_retry(ctx: &Arc<Ctx>) {
       let ctx = Arc::clone(ctx);
       tokio::spawn(async move {
           tokio::time::sleep(crate::tui::app::BUSY_RETRY).await;
           let _ = ctx.tx.send(Msg::RetryQueue);
       });
   }
   ```

Decisions:
- `Running` has an extra `action: Option<Action>` field (the brief lists `{id, label, progress}`): `Busy` must put the action back at the front, and `Cmd::Execute` consumes it. `None` = residue cleanup.
- Residue cleanup (chosen in the dialog or auto in mode `high`) becomes the `running` operation ("Limpando resíduos de X") instead of the removed `Modal::Running`; the queue continues on `ResiduesCleaned`. Running it beside the next install could delete a shortcut/folder that a reinstall of the same app just created. `Esc`/empty choice continues the queue at once.
- Quit dialog reuses `Modal::Confirm` with `action: Option<Action>` (`None` = quit). Ctrl+C goes through the same check as `q`/`Esc` (it used to quit at once, which would abandon the job). With nothing running but a queue (lock wait / residues pause) the text is "Há N operações na fila. Sair cancela as pendentes."
- Dialogs no longer replace each other: a plan's Confirm/Picking or a Residues dialog that arrives while another dialog is open waits in `pending_modals` and opens when it closes (test `a_dialog_arriving_meanwhile_waits_for_the_open_one`); the queue counts a waiting Residues dialog as a pause.
- `Modal::Queue { selected }` stores the op id under the cursor (not an index), so rows shifting when an op finishes never make `x` remove another item; `f` also works on the Settings tab; the help dialog lists `f`.

Concerns:
- `second_upgrade_while_running_does_nothing` contradicted the brief (planning during an execution is now allowed): renamed to `second_upgrade_while_running_plans_again` and inverted. `running_swallows_keys_and_shows_progress` was deleted (covered by `navigation_works_while_running` and `esc_and_ctrl_c_also_ask_while_running`); `busy_shows_status_and_rescans` now expects `[ScanInstalled, RetryQueue]`.
- A plan confirmed while `quit_after_running` is set is queued and then dropped by the quit; not guarded.
