Status: DONE_WITH_CONCERNS

Commits:
47ff7d2 feat(bridge): elevated helper runs a hash-verified installer copied to an admin-only folder

Gate (one run; it stopped at the build because another agent's work in progress (Task 147/148, `src/install/prod_win.rs`) did not compile at that moment):
```
[OK]   rules 6s
[OK]   rules-selftest 14s
[OK]   fmt 3s
[OK]   clippy 70s
[FAIL] test exit 101
  error[E0063]: missing field `installer` in initializer of `Runners`
  error: could not compile `wingetrs` (lib) due to 1 previous error
  error: could not compile `wingetrs` (lib test) due to 1 previous error
  (log completo: target/verify.log)
```
`src/install/prod_win.rs:75` belongs to the parallel task, not this one. My own tests, run again once the tree compiled:
```
cargo test --quiet --test it elevated_   -> test result: ok. 38 passed; 0 failed; 2 ignored (the two live tests)
cargo test --quiet --lib bridge          -> test result: ok. 12 passed; 0 failed; 1 ignored
cargo clippy --quiet --all-targets       -> no warnings in bridge/ or elevated_install
```
The orchestrator still needs to run the full gate.

Step 0 probes (none needed elevation):
- `icacls "C:\Program Files"`: `BUILTIN\Usuários:(RX)` and `(OI)(CI)(IO)(GR,GE)`. No write or create right for Users. `PROPRIETÁRIO CRIADOR:(OI)(CI)(IO)(F)`, so a folder an admin creates has BA as owner. The premise holds.
- std `library/std/src/sys/fs/windows.rs` `open_native`: `c::CreateFileW(path, access, opts.share_mode, ...)`. `share_mode` goes straight through.
- `.cmd` opened by `VerifiedFile`, then launched by `ShellLauncher`: it works (exit code 3, marker written, and opening it for writing fails while the handle is held). This is kept as the test `win::verified_handle_still_launches`. The real run also uses `CreateProcessW` (through std `Command`) on a copy held the same way, and `win::staged_installer_runs_the_copy_with_its_own_temp` proves that works too.

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor '"/quiet" | "/passive"' -Replacement '"/quiet" | "/x" | "/passive"' -TestCommand 'cargo test --quiet --test it elevated_install::msi_params_allowlist'`
   `[OK] mutação PEGA: '"/quiet" | "/passive"' -> '"/quiet" | "/x" | "/passive"' fez o teste falhar (exit 101).`
   The first try SURVIVED because `/x {G}` is also refused for the `{G}` token. I added the bare switches `/x`, `/i`, `/a`, `/p`, `/package`, `/uninstall`, `/update`, `/jm`, `/l*v`.
2. `..` is no longer a separate check. `is_windows_unfriendly` already refuses any component that ends in a dot, so a separate `c == ".."` was an equivalent mutation (it SURVIVED) and I removed it. This mutation targets exactly "do not refuse `..`":
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor '.any(|c| !c.is_empty() && crate::' -Replacement '.any(|c| !c.is_empty() && c != ".." && crate::' -TestCommand 'cargo test --quiet --test it elevated_install::bad_fields_never_reach_ops'`
   `[OK] mutação PEGA: '.any(|c| !c.is_empty() && crate::' -> '.any(|c| !c.is_empty() && c != ".." && crate::' fez o teste falhar (exit 101).`
3. Run the source instead of the copy. This is caught without elevation by `win::staged_installer_runs_the_copy_with_its_own_temp`: a copy of `cmd.exe` writes `!CMDCMDLINE!` to the log.
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/staging_win.rs -Anchor '(staged.file().to_path_buf(), params)' -Replacement '(PathBuf::from(&req.source), params)' -TestCommand 'cargo test --quiet --test it elevated_install::win::staged_installer_runs_the_copy_with_its_own_temp'`
   `[OK] mutação PEGA: '(staged.file().to_path_buf(), params)' -> '(PathBuf::from(&req.source), params)' fez o teste falhar (exit 101).`
Extra:
- User TEMP passed through: `.envs(installer_env(std::env::vars_os(), &run.join(TMP_NAME)))` -> `.envs(std::env::vars_os())` gives `[OK] mutação PEGA` (same test).
- Reparse check removed (`        plain_dir(dir)?;` -> ``) gives `[OK] mutação PEGA` (`win::staging_root_refuses_reparse_points`).
- Worker line cap: `            tail.pop_front();` -> `` gives `[OK] mutação PEGA`, and `tail.push_back(clip(text).to_string());` -> `tail.push_back(text.to_string());` gives `[OK] mutação PEGA` (`installer_lines_are_the_last_forty_and_clipped`).
- `RUN_SDDL` without `P` (`"D:(A;..."`) SURVIVED. It is an equivalent mutation at creation: a probe showed that `CreateDirectoryW` with an explicit DACL reads back as `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)` either way. The `P` is kept because it still matters if the ACL of the folder above is ever propagated again.

Places where external data is joined to a path:
- `run.path().join(name)`: `name` is `req.file_name` after the worker's regex check and `install::safepath::safe_components` (exactly one component, so `CON.exe` is refused too).
- `Path::new(&req.source)` is absolute, so it cannot go through safepath. `InstallerRequest::check` validates it instead: `X:\`, no `\\?\` or UNC, no second `:`, no `..` or device-name component, at most 32 767 bytes.
- The other joins (`program_files().join("wingetrs").join("staging")`, `<root>.join(random_hex)`, `.join("tmp")`, `.join("install.log")`) use fixed or random names only.

Decisions:
- The installer is launched with std `Command` (`CreateProcessW`: absolute path, `raw_arg`, `current_dir = <run>`, `env_clear().envs(installer_env(..))`, null stdio), not `ShellLauncher`. `ShellExecuteExW` has no environment parameter, and the TEMP/TMP note requires one. The other way, `set_var` in the process, would change TEMP for every test running at the same time and for the helper's winget runs.
- `serve` takes `&mut dyn WorkerOps`. A blanket `impl WorkerOps for F: FnMut(..)` (installers refused with `Unsupported`) keeps the closures in `tests/it/elevated_worker.rs` and `tests/it/wi_bridge_install.rs` compiling unchanged, so neither file was edited. Production uses `SystemOps` in `elevate_win`.
- The 40-line cap is enforced in the worker, which buffers the last 40 lines and clips each to `INSTALLER_LINE_MAX = LINE_CAP/8` bytes. `staging_win::log_lines` also takes only 40. A log with a FF FE BOM goes through `tail_text`; any other log (Inno) is read as lossy UTF-8. When the 8 KiB window cuts the first line, that line is dropped.
- Validation goes beyond the brief in a few places: device names (`C:\x\CON` would block the elevated read), an extra `:` (ADS) and NUL in `source`, and PROPERTY values limited to a bare value or a single quoted span with no quote inside (`A=x"y"` and `A="x"y"z"` are refused).
- `program_files()` and `random_hex()` in `elevate_win.rs` became `pub(super)` so they can be reused. Unit tests pass an SDDL with an extra `(A;OICI;FA;;;OW)`, because unelevated they cannot write into a SY/BA-only folder. `staging_dacl_has_no_users` uses the real `RUN_SDDL`, shows that the creator gets `PermissionDenied` writing inside, and runs `icacls /reset` only to clean up.

Concerns:
- "`job.end` of these jobs carries `elevated=true`": no job uses `ElevatedRunner` yet (the engine and CLI come in later tasks), so there is nothing to log here. The task that wires the runner must do it.
- The rest of the user environment still reaches the elevated installer, as the orchestrator decided. That includes the user part of `PATH`, which the user can write and which is the last step of the DLL search order.
- `source` is read as admin from a user path. A junction in that path can point at a file only admins can read; it is only run if the hash matches, which needs its content. "Não confere" versus "ilegível" works as a hash oracle on admin-only files. The risk is low; opening the file while impersonating the user would close it.
- An installer that opens itself with share mode 0 would fail while the `VerifiedFile` handle is held. This is not measured; it is in the live script below.
- TDD order was not literal: tests and code were written in the same pass. Each rule was then shown to bite with the mutations above (two first tries survived and led to the fixes described).

Live script (for the user, elevated via UAC, release build):
```powershell
$env:WINGETRS_TEST_MSI = 'C:\caminho\para\um-msi-pequeno.msi'
cargo test --quiet --release --test it elevated_install::win::live_run_installer_once -- --ignored --nocapture
# one UAC prompt; expect Ok(0) and log lines printed
Get-ChildItem 'C:\Program Files\wingetrs\staging'          # must be empty afterwards
icacls 'C:\Program Files\wingetrs\staging'                 # inherited from Program Files: Users only (RX)/(GR,GE)
# With a slow MSI, while it runs:
icacls "C:\Program Files\wingetrs\staging\<32 hex>"        # only SISTEMA and Administradores, no inheritance
icacls "C:\Program Files\wingetrs\staging\<32 hex>\tmp"    # same
# Also check: an Inno/Nullsoft exe installer runs while its copy is held open (share-read).
```

## Revisão de segurança (fix 42564e2)

Commits: `42564e2 fix(bridge): review of task 180` (on top of `47ff7d2`).

Gate (one run): `[OK] rules`, `[OK] rules-selftest`, `[FAIL] fmt exit 1`. The fmt failure is a diff in `src/install/executor.rs:1865`/`:1883`, which is another agent's file. Checks on my files:
- `rustfmt --check` on the 5 files: ok.
- `cargo clippy --all-targets -- -D warnings`: exit 0.
- `check_rules.ps1`: exit 0.
- `cargo test --test it elevated_`: 40 passed, 2 live ignored.
- `--lib bridge`: 12 passed.

Changes:
1. **Environment.** The installer environment is now a closed list. `SystemEnv::query()` takes every value from KnownFolders or `GetNativeSystemInfo`, and `installer_env(sys, tmp)` builds the list from it plus constants (`PATHEXT`, `OS`). The process is started with `env_clear().envs(..)`.
   - The pure test `installer_env_is_a_closed_list` checks the exact key list and the exact PATH, and confirms that no variable of the test process gets in.
   - The real `cmd.exe` copy shows no `USERNAME=` and only the system PATH.
   - Mutations `.env_clear()` -> `` and `.envs(installer_env(..))` -> `.envs(std::env::vars_os())`: both `[OK] mutação PEGA`.
2. **MSI properties.** Only these names are accepted:
   - `ALLUSERS` (empty/1/2), `MSIINSTALLPERUSER` (empty/1), `REBOOT` (ReallySuppress/Suppress/Force/R/S/F), `ADDLOCAL` (`[A-Za-z0-9_.,]+`);
   - `INSTALLDIR`/`TARGETDIR` only strictly inside one of the folders from `WorkerOps::program_files()`. That method returns an empty list by default; `SystemOps` fills it from KnownFolders.
   - `TRANSFORMS=C:\x.mst`, `PATCH=C:\p.msp`, `ACTION=ADMIN`, `REINSTALL=ALL` and `TRANSFORMS=:x` are refused.
   - Tab now separates tokens, and `\"` is refused in every `params`.
   - Mutations `_ => false` -> `_ => true` and INSTALLDIR -> `true`: both `[OK] mutação PEGA`.
3. **Source file.**
   - (a) The last component must equal the sha in lowercase and its parent must be `blobs`. Mutations on the sha and on the `blobs` check: both `[OK] mutação PEGA`.
   - (b) `open_source` opens with `FILE_FLAG_OPEN_REPARSE_POINT` and `security_qos_flags(SECURITY_IDENTIFICATION)`, then refuses the REPARSE_POINT attribute (`[OK] mutação PEGA`, using a real `symlink_file`, which works on this machine) and `GetFileType != FILE_TYPE_DISK`. The `GetFileType` mutation SURVIVED: no device path both passes `valid_source` and opens (`\.\NUL` already fails at the open with "Função incorreta"). The check stays as defense in depth.
   - (c) `DuplicateHandle` was not done. It needs more than a `handle: u64` field: the parent would have to be opened with `PROCESS_DUP_HANDLE` in `worker_main`, `SystemOps` would need that handle, the client would need a Win32 open in a `_win` file outside `ElevatedRunner`'s cross-platform path, and the handle would have to stay alive across the request. That is a new design, so I left it for a future task.
4. **Exe parameters** (`exe_params_allowed`). Refused:
   - tokens with a drive (a lone letter before `:`, or `:\`/`:/` anywhere), `\`, `..`, a leading `\`, or `=\`/`=/`;
   - `/LOADINF` and `/SAVEINF` (with `/` or `-`);
   - `/DIR=` outside Program Files;
   - `/D=` (Nullsoft) outside Program Files, taking the rest of the line as the folder, unquoted with its spaces.

   `<LOGPATH>` is removed before the check. Mutations on the config switches, the path check, `/D=` and `/DIR=`: all `[OK] mutação PEGA`.
5. **Copy and verify by the same handle: not done.** For the loader to start the copy, no handle with write access can be open on it (the share mode would refuse). `VerifiedFile` reopens the copy with share-read and hashes it again through that handle, so anything changed between the write and the reopen is caught. The gap is also inside a folder only SY/BA can write.

Live script, added items:
- During a slow MSI, look at the msiexec process's environment with Process Explorer: no user variable (`USERNAME`, the user's PATH entries, proxies) and TEMP set to `<run>\tmp`.
- Install an Inno installer with `/DIR="C:\Program Files\X"` and confirm the folder.

Concerns:
- Detecting a drive as "lone letter + `:`" refuses switches like `/v:on`.
- `PROCESSOR_IDENTIFIER` is not set; it would need the registry.
- `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` point at the user's profile, as you specified.

## Segunda revisão: allowlist de chaves por tipo de instalador

The second review is commit `b64193a` (`fix(bridge): review of task 180 (per-kind switch allowlist)`).

- `names_path`, the path denylist, was removed. `exe_params_allowed(kind, params, program_files)` is now an allowlist per installer kind:
  - inno: `/VERYSILENT /SILENT /SUPPRESSMSGBOXES /NORESTART /SP- /CLOSEAPPLICATIONS /NOCANCEL /ALLUSERS /CURRENTUSER`, `/LOG=<LOGPATH>`, `/DIR=<inside ProgramFiles>`;
  - nullsoft: `/S`, and `/D=<inside ProgramFiles>` (last, the rest of the line);
  - burn: `quiet`, `passive`, `norestart` (with `/` or `-`) and `/log <LOGPATH>`;
  - exe: `/key`, `-key`, `--key`, `/key=v` or `/key:v`. The value may contain no `\`, no `/`, no `:`, no `..` and no inner quote. A key may be followed by one `<LOGPATH>` token. `/LOADINF` and `/SAVEINF` are always refused.
- Any other token, including a loose positional argument, makes the worker reply `Reply::Error` without running anything.
- The `cmd.exe` staging tests no longer call `check()`, because they need free parameters. Validation has its own tests.
- Tests now cover `//server/share/x`, `/LOG=//s/x`, `/DIR=C:/Users/x`, an unknown key per kind, and a loose positional argument per kind.

Mutations (`exe_params_allowlist`):
- inno unknown key (`INNO_FLAGS.contains(..)` -> `true`): `[OK] mutação PEGA`.
- nullsoft (`t == "/S"` -> `true`): `[OK] mutação PEGA`.
- burn (`BURN_FLAGS.contains(&key)` -> `... || !key.is_empty()`): `[OK] mutação PEGA`.
- exe loose positional argument (`return false` -> `return true`): `[OK] mutação PEGA`.
- exe free value (`Some(v) => is_log(v) || plain_value(v)` -> `Some(_) => true`): `[OK] mutação PEGA`.
- exe config switch (CONFIG_SWITCHES check removed): `[OK] mutação PEGA`.
- I added a `/` refusal in `under_program_files`; it SURVIVED its mutation, so I removed it. A `/` already fails both `valid_source` and the `\`-after-the-root check.
