Status: DONE_WITH_CONCERNS

Commits:
ebe8484 fix(bridge): start a new elevated helper when the old one exited idle
f90554f fix(bridge): elevated helper runs only Windows' own winget and never touches the disk
05fced3 feat(bridge): one elevated helper per session for machine-scope winget upgrades

Gate (depois das correções):
```
[OK]   rules 4s
[OK]   rules-selftest 13s
[OK]   fmt 2s
[OK]   clippy 9s
[OK]   test (1049 passed, 27 ignored) 42s
[OK]   gate verde
```

Mutation proofs:
1. serve sem `valid_target` (teste 2):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor 'if !super::valid_target(Tool::Winget, &target) {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it elevated_worker::serve_refuses_bad_targets'`
   `[OK] mutação PEGA: 'if !super::valid_target(Tool::Winget, &target) {' -> 'if false {' fez o teste falhar (exit 101).` (falhou `elevated_worker::serve_refuses_bad_targets`)
2. aceitar campo desconhecido (teste 3):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor '#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]' -Replacement '#[serde(tag = "op", rename_all = "snake_case")]' -TestCommand 'cargo test --quiet --test it elevated_worker::decode_refuses_unknown_op_extra_fields_and_long_lines'`
   `[OK] mutação PEGA: '#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]' -> '#[serde(tag = "op", rename_all = "snake_case")]' fez o teste falhar (exit 101).`
3. elevar de novo depois de recusado (teste 6):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/elevated.rs -Anchor 'State::Refused => false,' -Replacement 'State::Refused => self.transport.start().is_ok(),' -TestCommand 'cargo test --quiet --test it elevated_worker::refused_elevation_falls_back_once'`
   `[OK] mutação PEGA: 'State::Refused => false,' -> 'State::Refused => self.transport.start().is_ok(),' fez o teste falhar (exit 101).`
Extras (autorrevisão), todas `[OK] mutação PEGA`:
- `if helper == 0 || client != helper {` -> `if helper == 0 {` (lib `elevate_win::tests::pid_checks...`)
- `if server != parent {` -> `if false {` (lib `elevate_win::tests::pid_checks...`)
- `if buf.len() > LINE_CAP {` -> `if buf.len() > LINE_CAP + 1 {` (teste 3, fronteira exata de 64 KiB)
- helper perdido volta a `NotStarted` -> `Refused` (`a_lost_helper_fails_its_job_and_is_started_again`)
- `    log.elevated = elevated;` -> `` (teste 7)
- `job.scope == Some(InstallScope::Machine)` -> `job.scope != Some(InstallScope::User)` e `job.tool == Tool::Winget && ` removido (teste 5)

4. DACL e PID (check 4). Provado sem UAC, nos testes unitários de `src/bridge/elevate_win.rs` (pipe real, as duas pontas neste processo):
- `pipe_round_trip_and_dacl`: lê a DACL do pipe criado com `GetSecurityInfo` e confere `D:P(A;;0x12019f;;;<SID do usuário>)` (protegida, uma ACE, só o usuário); uma 2ª instância do mesmo nome é recusada (1 instância + `FILE_FLAG_FIRST_PIPE_INSTANCE`); ida e volta pelo I/O overlapped; servidor fechado = fim do fluxo.
- `pid_checks_accept_only_the_expected_process`: `check_client` aceita o PID certo e recusa o de outro processo (`PermissionDenied`); idem `check_server`.
- `accept_gives_up_when_the_helper_exits_or_never_connects`: processo que já saiu => "saiu sem conectar"; ninguém conecta => `TimedOut`.
- Não provado (só ao vivo): que outro usuário é de fato barrado pela DACL, o UAC, o `SW_HIDE` da janela de console do auxiliar e a espera no pai.
Roteiro ao vivo para o usuário:
  a) `$env:WINGETRS_LIVE_MACHINE_IDS='Id.Um,Id.Dois'` (dois apps de escopo máquina com atualização), depois `cargo test --release --test it elevated_worker::live_single_uac_for_two_jobs -- --ignored --nocapture`: deve aparecer **um** UAC e nenhuma janela de console; o teste confere uma elevação para dois jobs e que o auxiliar sai no `quit`.
  b) Na TUI: atualizar um app de máquina, recusar o UAC => aviso "sem permissão de administrador: atualizando sem elevar", e outro app de máquina não pede de novo.
  c) Com o auxiliar aberto (aceitar o UAC), fechar a TUI: `Get-Process wingetrs` não deve mostrar o auxiliar depois de ~5 s. Matar a TUI pelo Gerenciador de Tarefas: o auxiliar também sai (espera no pai).
  d) No log (`%LOCALAPPDATA%\wingetrs\logs`): linhas `bridge_worker{role="worker"}` do auxiliar e `job.end ... elevated=true`.

Decisions:
- RNG: `BCryptGenRandom` (`BCRYPT_USE_SYSTEM_PREFERRED_RNG`); features novas do `windows`: `Win32_System_Pipes`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_Security_Cryptography` (BCrypt) e `Win32_System_Registry` (exigida pelo `SHELLEXECUTEINFOW`). `Cargo.lock` sem mudança.
- `Request::SourceUpdate {}`/`Quit {}` são variantes struct: medido que o serde deixa passar campo extra numa variante unitária de enum com `tag` mesmo com `deny_unknown_fields`. `Reply::End` leva só o código; o cliente remonta a cauda pelas linhas (cabe no teto de 64 KiB).
- O exec elevado só traduz os dois comandos do próprio job (`bridge::command` / `winget_source_update`); qualquer outro é `PermissionDenied` antes do pipe. Pipe e cliente são overlapped (`PipeIo`), para `CancelIoEx` do vigia do pai encerrar a leitura; o cliente abre com `SECURITY_IDENTIFICATION` (servidor não personifica o elevado).
- Auxiliar que morre no meio: o job falha ("auxiliar elevado: ...", sem rodar sem elevar em silêncio) e o estado volta a `NotStarted` (o próximo job pede UAC de novo); qualquer falha do início (recusa, prazo de 30 s, PID errado) vira `Refused` na sessão.
- `src/main.rs` não mudou: o `bridge-worker` é despachado no início de `cli::run` (ver Correções 2); `BridgeExec` mudou para `bridge::BridgeExec`, reexportado em `tui::jobs`. Novo `jobs::run_bridge_elevated`; `run_bridge` segue igual (elevated=false).

Concerns:
- Usuário padrão que eleva com credencial de outro admin (over-the-shoulder): o auxiliar roda como outro usuário, a DACL o barra, ele sai sem conectar e a sessão fica `Refused` (cai no caminho atual). Não medido.
- `hwnd` nulo no `ShellExecuteExW`: o UAC pode abrir atrás do terminal. `SW_HIDE` no console do auxiliar com o Windows Terminal como terminal padrão não foi verificado (item a do roteiro).
- Ao sair com um job elevado em andamento, `shutdown` não espera (try_lock): o auxiliar termina o winget e sai quando o pipe quebra ou o pai morre.

## Correções (revisão de segurança, commit f90554f)

1. [Critical] winget do PATH/alias. Agora o worker resolve o winget sozinho (`elevate_win::winget_exe`): pega os pacotes da família `Microsoft.DesktopAppInstaller_8wekyb3d8bbwe` (`GetPackagesByPackageFamily`) e a pasta de cada um (`GetPackagePathByFullName`), com teto de 64 pacotes e 64 Ki caracteres antes de alocar. A função pura `winget_in` exige que a pasta seja um componente logo abaixo de `<FOLDERID_ProgramFiles>\WindowsApps\`, com nome `Microsoft.DesktopAppInstaller_*__8wekyb3d8bbwe`. A comparação é por componente e sem diferenciar maiúsculas. `Program Files` vem do `SHGetKnownFolderPath`, nunca de variável de ambiente. O exec do worker (`run_winget`) recusa qualquer programa que não seja `winget` e roda `<pasta>\winget.exe` por caminho absoluto, com o diretório de trabalho na pasta do pacote (`spawn_win::run_in`, novo). Se o winget não for achado ou estiver fora do lugar, o exec dá `NotFound` e o pedido volta como `Reply::Error`, sem rodar nada. Feature nova: `Win32_Storage_Packaging_Appx`; o `Cargo.lock` não mudou. `spawn_win.rs` ficou fora da lista Files: ganhou `run_in`, e `run_with_grace` agora passa por `run_command`, com o comportamento igual.
   Verificação ao vivo, sem elevar (só `--version`): `cargo test --quiet --lib elevate_win::tests::live_winget -- --ignored` deu `test result: ok. 1 passed`. O teste resolve o caminho, roda `winget.exe --version` (exit 0, linha `v...`) e faz o mesmo pelo `run_winget`. O caminho esperado, segundo `Get-AppxPackage`, é `C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1.29.380.0_x64__8wekyb3d8bbwe`.
2. [Critical] disco do worker. O `BridgeWorker` agora é despachado na primeira linha de `cli::run`, antes de `style::init`, `DataDir`, `ensure`, `init_logging` e `prune_logs`. Sem assinante, o `tracing` do worker não grava nada: os logs do worker somem. Não implementei `Reply::Log`; do lado da TUI ficam `job.start`/`job.end` com `cmd` e `elevated`. O teste `worker_never_touches_the_data_dir` roda o binário com `LOCALAPPDATA` numa pasta vazia, um nome de pipe válido e inexistente, e o próprio PID como pai. O worker falha e a pasta continua vazia.
3. [Important] saída por ociosidade. A leitura do worker (`PipeIo` com `read_limit = IDLE_LIMIT`, 10 min) cancela o próprio I/O (`CancelIoEx` + espera) e devolve `TimedOut`. O `serve` recebe o erro e termina, e o worker sai. O worker só fica parado numa leitura quando espera o próximo pedido, então um `winget` longo não conta como ócio. O prazo é injetável: o teste `idle_read_times_out_and_data_in_time_is_read` usa 100 ms e tem um detector (o servidor escreve depois de 3 s; sem o prazo a leitura receberia esse dado em vez de `TimedOut`).
4. [Important] teto de 64 KiB antes de alocar. O teste `read_line_caps_before_buffering_an_endless_line` usa uma linha infinita de `a`, como o `std::io::repeat`. Depois de 4 tetos o leitor falha com outro tipo de erro, para que a mutação falhe em vez de travar ou encher a memória. O teste espera `InvalidData` com no máximo 2 tetos consumidos.
5. [Minor] `run.rs`: a função `helper_exe()` registra um `warn` quando `current_exe` falha. O caminho vazio continua fazendo a 1ª elevação falhar, com a nota de recusa.

Mutações das correções (todas reais, coladas da saída):
- `pwsh -File scripts/mutate.ps1 -Path src/bridge/elevate_win.rs -Anchor '|| !root.iter().zip(&dir).all(|(a, b)| same(a, b))' -Replacement '|| false' -TestCommand 'cargo test --quiet --lib elevate_win::tests::winget_only'`
  `[OK] mutação PEGA: '|| !root.iter().zip(&dir).all(|(a, b)| same(a, b))' -> '|| false' fez o teste falhar (exit 101).`
- `... -Anchor '|| dir.len() != root.len() + 1' -Replacement '' ...winget_only`
  `[OK] mutação PEGA: '|| dir.len() != root.len() + 1' -> '' fez o teste falhar (exit 101).`
- `... -Anchor 'ours.then(|| package_dir.join("winget.exe"))' -Replacement 'Some(package_dir.join("winget.exe"))' ...winget_only`
  `[OK] mutação PEGA: 'ours.then(|| package_dir.join("winget.exe"))' -> 'Some(package_dir.join("winget.exe"))' fez o teste falhar (exit 101).`
- `... -Path src/cli/mod.rs -Anchor '    if let Some(Command::BridgeWorker { pipe, parent }) = &cli.command {' -Replacement '    DataDir::from_env()?.ensure()?;<LF>    if let Some(...' -TestCommand 'cargo test --quiet --test it elevated_worker::worker_never'`
  `[OK] mutação PEGA: '    if let Some(Command::BridgeWorker { pipe, parent }) = &cli.command {' -> '    DataDir::from_env()?.ensure()?;` (falhou `elevated_worker::worker_never_touches_the_data_dir`)
- `... -Path src/bridge/elevate_win.rs -Anchor 'if woke == WAIT_TIMEOUT {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --lib elevate_win::tests::idle_read'`
  `[OK] mutação PEGA: 'if woke == WAIT_TIMEOUT {' -> 'if false {' fez o teste falhar (exit 101).`
- `... -Path src/bridge/worker.rs -Anchor '.take(LINE_CAP as u64 + 1)' -Replacement '.take(u64::MAX)' -TestCommand 'cargo test --quiet --test it elevated_worker::read_line_caps'`
  `[OK] mutação PEGA: '.take(LINE_CAP as u64 + 1)' -> '.take(u64::MAX)' fez o teste falhar (exit 101).`
Não rodadas, e por quê:
- `program != "winget"` -> `false` em `run_winget`: com a mutação, o teste `worker_exec_runs_nothing_but_winget` executaria o `winget.exe` real com `/c exit` (o teste existe e passa com o código certo).
- `read_limit: Some(IDLE_LIMIT)` -> `None` no `worker_main`: esse trecho só roda no worker real, elevado. Entra no roteiro ao vivo: com o auxiliar aberto e sem jobs, ele deve sumir do `Get-Process` depois de 10 min.

## Correções da re-revisão (commit ebe8484)

Gate:
```
[OK]   rules 4s
[OK]   rules-selftest 12s
[OK]   fmt 2s
[OK]   clippy 16s
[OK]   test (1051 passed, 27 ignored) 43s
[OK]   gate verde
```

1. [Important] Auxiliar que saiu por ociosidade. `Helper` ganhou `alive()`; o real faz `WaitForSingleObject(process, 0) == WAIT_TIMEOUT`. Pus o método em `Helper` e não em `Transport` porque é o `Helper` que guarda o handle do processo. `exec_for`, pelo novo `ensure_started`, confere a vida antes de reusar o estado `Running`: auxiliar morto volta a `NotStarted` e é elevado de novo (novo UAC). Se o envio do pedido falha (`Lost::Unsent`: nada rodou), um auxiliar novo é iniciado e o pedido vai mais uma vez, nunca duas. Uma quebra depois do envio (`Lost::Midway`: o winget pode ter rodado) não é repetida: o job falha e o próximo job eleva de novo.
   Testes: `a_helper_that_exited_idle_is_started_again` (auxiliar sai => 2ª elevação e o job roda elevado) e `an_unsendable_request_restarts_the_helper_once` (1º auxiliar sem envio => uma reinicialização e o job roda; todos sem envio => exatamente 2 elevações, o job falha e nada roda sem elevar). O transporte falso recusa a partir da 6ª elevação, como detector: um laço de reinício falha o teste em vez de travar.
2. [Minor] Comentário em `winget_exe`: vale o primeiro pacote da família, na ordem do sistema, que esteja na pasta protegida e tenha `winget.exe`; as versões não são comparadas; pacote fora do lugar é pulado; nenhum no lugar => falha fechado.
3. [Minor] O teste `worker_never_touches_the_data_dir` tinha `\.\pipe\...`: uma barra se perdeu no heredoc do bash. Agora é `\\.\pipe\wingetrs-elevated-` + 32 zeros, e o teste confere que o erro **não** é "nome de pipe inválido", ou seja, o worker passa pelo `OpenProcess`/`CreateFileW`. A pasta continua vazia.

Mutações (saída real):
- `pwsh -File scripts/mutate.ps1 -Path src/bridge/elevated.rs -Anchor '&& !helper.alive()' -Replacement '&& false' -TestCommand 'cargo test --quiet --test it elevated_worker::a_helper_that_exited'`
  `[OK] mutação PEGA: '&& !helper.alive()' -> '&& false' fez o teste falhar (exit 101).`
- `... -Anchor 'Err(Lost::Unsent(e)) if !retried => e,' -Replacement 'Err(Lost::Unsent(e)) => e,' ...an_unsendable` (repetir sempre)
  `[OK] mutação PEGA: 'Err(Lost::Unsent(e)) if !retried => e,' -> 'Err(Lost::Unsent(e)) => e,' fez o teste falhar (exit 101).`
- `... -Anchor 'Err(Lost::Unsent(e)) if !retried => e,' -Replacement 'Err(Lost::Unsent(e)) if false => e,' ...an_unsendable` (nunca repetir)
  `[OK] mutação PEGA: 'Err(Lost::Unsent(e)) if !retried => e,' -> 'Err(Lost::Unsent(e)) if false => e,' fez o teste falhar (exit 101).`
- `... -Anchor 'let midway = |e: std::io::Error| Lost::Midway(lost(e));' -Replacement 'let midway = |e: std::io::Error| Lost::Unsent(lost(e));' -TestCommand 'cargo test --quiet --test it elevated_worker::a_lost_helper'` (repetir depois do envio)
  `[OK] mutação PEGA: 'let midway = |e: std::io::Error| Lost::Midway(lost(e));' -> 'let midway = |e: std::io::Error| Lost::Unsent(lost(e));' fez o teste falhar (exit 101).`
