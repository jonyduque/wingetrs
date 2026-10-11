Status: DONE

Commits: (see git log d487f11..HEAD; task-47 commit is the one titled "fix(update): back off background refresh...")

Gate:
[OK]   fmt 1s
[OK]   clippy 1s
[OK]   test (444 passed, 16 ignored) 21s
[OK]   gate verde
(an earlier run failed only on tests/install_ops.rs, Task 46's in-progress file; re-run was green)

Mutation proofs (all "[OK] mutação PEGA"):
- 'now - t >= RETRY_SECS' -> 'now - t > RETRY_SECS' (test background_refresh_backs_off): PEGA
- 't > now || ' -> '' : PEGA
- remove fs::write of refresh.attempt (claim_records): PEGA
- needs_refresh(...) -> true in background_due: PEGA
- unreadable file read as Some(now) (unreadable_attempt): PEGA
- --background back to LockWait::from_env() (cli_offline background_update_gives_up): PEGA
- NoWait for all updates (foreground_update_still_waits): PEGA

Decisions: claim_background_refresh calls needs_refresh before reading the attempt file (reviewer focus b); removed now-unused imports (HttpState, needs_refresh) in cli/mod.rs.

Concerns: none. leftovers.ps1 listed only pre-existing unrelated processes (bash/python from 10/10).

Follow-up (review Minor): pinned boundary at attempt instant. Mutation: [OK] mutação PEGA: 't > now || now - t' -> 't >= now || now - t' fez o teste falhar (exit 101).
