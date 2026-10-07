Status: DONE

Commits:
17f5771 fix(installed): satisfy clippy as_chunks lint in reg_string
972103d test(installed): move registry value conversion to pure arp.rs with tests
1568cb3 fix(tui): mark offline only for network failures
d610f90 fix(update): TUI refresh never waits on the lock; JoinError is Internal, not Busy
ac264d2 refactor(cache): add DataDir::from_env shared by CLI and TUI
ad94388 fix(tui): keep stderr logging off when launching the TUI

Gate:
[OK]   clippy 5s
[OK]   test (144 passed, 2 ignored) 43s
[OK]   gate verde

Mutation proofs:
1. cargo test cli_parse: [OK] mutação PEGA: 'cli.command.is_some() && cli.verbose > 0' -> 'cli.verbose > 0' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: 'LockWait::NoWait => Duration::ZERO' -> 'LockWait::NoWait => Duration::from_millis(500)' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: 'Msg::RefreshBusy => {' -> 'Msg::RefreshBusy => { app.offline = true;' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: 'outra atualização em andamento' -> 'falha geral' fez o teste falhar (exit 101).
3. [OK] mutação PEGA: 'any_offline |= *offline;' -> 'any_offline = true;' fez o teste falhar (exit 101).
3. [OK] mutação PEGA: 'ScoopError::Http(e) if e.is_offline()' -> 'ScoopError::Tar(_)' fez o teste falhar (exit 101).
6. [OK] mutação PEGA: 'if vtype != REG_SZ && vtype != REG_EXPAND_SZ {' -> 'if false {' fez o teste falhar (exit 101).
6. [OK] mutação PEGA: '.trim_end_matches('\0')' -> '.trim_end_matches('x')' fez o teste falhar (exit 101).
Items 4 and 5: no mutation proof requested.

Decisions:
- Items 2 and 4 share one commit (d610f90): both edit run_update/UpdateError and the TUI call site had to compile together.
- TUI Busy is a new Msg::RefreshBusy (status "Aguardando: outra atualização em andamento", offline untouched).
- DataDir::from_env delegates to pure from_local_app_data(Option<OsString>) so it is testable without touching env.
- Item 6 commit ac... first landed with a clippy as_chunks error; fixed in the follow-up commit (gate green at HEAD).

Concerns: none
