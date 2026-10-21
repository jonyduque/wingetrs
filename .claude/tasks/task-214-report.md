Status: DONE

Commits:
37d0157 feat(tui): notice and Ctrl+U for a new wingetrs version

Gate:
[OK]   clippy 30s
[OK]   test (2003 passed, 88 ignored) 100s
[OK]   gate verde

Mutation proofs:
1. Ignore the cache: `mutate.ps1 -Path src/selfupdate/mod.rs -Anchor '&& (0..CHECK_EVERY).contains(&(now - c.checked_at))' -Replacement '&& false' -TestCommand 'cargo test --quiet --test it tui_self_update::check_skipped_within_24h'`
   [OK] mutação PEGA: '&& (0..CHECK_EVERY).contains(&(now - c.checked_at))' -> '&& false' fez o teste falhar (exit 101).
2. Network error surfaces (the check returns a value instead of silence), on `tui_self_update::offline_check_is_silent`:
   [OK] mutação PEGA: 'tracing::debug!(error = %e, "version check failed"); None' -> '... Some(format!("v99.0.0 {e}"))'
   (check_latest has no toast; silence is "returns None, writes no cache, app gets no Msg". The toast mutation was expressed as the Err branch returning something.)

Decisions:
- Item 4: Ctrl+U while an operation runs or waits is refused with the toast "espere a operação atual" (tested); the confirmation itself is the existing `Modal::Confirm` (buttons Continuar/Cancelar) carrying a new `Action::SelfUpdate { tag }`, queued via `enqueue` only after Continuar. The brief's Sim/Não buttons (Task 212) do not exist in this checkout; used CONFIRM_BUTTONS.
- Shared update: new `selfupdate::update_to` (direct or one-UAC path, with the WaitingElevation notice); the CLI `run_self` now calls it and its private `elevated_scope`/`elevated_self_apply` were removed (src/cli/mod.rs touched, outside the Files list, to avoid duplicating the logic).
- Cache and clock injection: `selfupdate::check_latest(http, web_base, data_root, now)`; cache in the future or with a bad tag counts as stale; dev builds never check. Check runs in a `tokio::spawn` after the first frame (never before it).
- `tests/it/main.rs` did not have `mod tui_self_update;`; I added the line.
- Help line added; no status hint (the pill already shows the key).

Concerns: none. Not run live: the real UAC/self-swap from the TUI (needs the user's machine).
