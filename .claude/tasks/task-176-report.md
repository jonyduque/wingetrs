# Task 176 report

Status: DONE_WITH_CONCERNS

Commits:
```
56e4874 feat(bridge): winget install through the bridge (CLI and TUI), --scope, accepted agreements in the log
```

Gate (`pwsh -File scripts/verify.ps1`, final run, linhas finais reais):
```
[OK]   clippy 20s
[FAIL] test exit 101
  test result: ok. 62 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 10.02s
  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
  test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.56s
  manifest_2c::planner_refuses_each_2c_key_with_old_text --- FAILED
  ---- manifest_2c::planner_refuses_each_2c_key_with_old_text stdout ----
  thread 'manifest_2c::planner_refuses_each_2c_key_with_old_text' (12284) panicked at tests\it\manifest_2c.rs:204:30:
  test result: FAILED. 1253 passed; 1 failed; 57 ignored; 0 measured; 0 filtered out; finished in 17.95s
  error: test failed, to rerun pass `--test it`
  (log completo: target/verify.log)
```
A única falha é de outro agente (2c, Task 144: o planejador aceita `psmodule`; `tests/it/manifest_2c.rs` e `src/install/planner.rs` não são desta tarefa). rules, rules-selftest, fmt e clippy passaram nessa mesma execução. Prova dos meus arquivos (`cargo test --quiet --test it -- wi_bridge_install:: bridge elevated_worker job_log tui_ cli_parse install_target`):
```
test result: ok. 438 passed; 0 failed; 4 ignored; 0 measured; 869 filtered out; finished in 4.22s
```
`wi_bridge_install::` sozinho: `test result: ok. 13 passed; 0 failed`.

Mutation proofs:
1. serve sem `valid_target` no braço Install (teste 2):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor '(target, _) if !super::valid_target(Tool::Winget, &target) => {' -Replacement '(target, BridgeOp::Upgrade) if !super::valid_target(Tool::Winget, &target) => {' -TestCommand 'cargo test --quiet --test it wi_bridge_install::serve_runs_install_only_with_valid_target'`
   `[OK] mutação PEGA: '(target, _) if !super::valid_target(Tool::Winget, &target) => {' -> '(target, BridgeOp::Upgrade) if !super::valid_target(Tool::Winget, &target) => {' fez o teste falhar (exit 101).`
2. `--scope` mesmo com `scope: None` (teste 1):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/mod.rs -Anchor 'if let BridgeOp::Install { scope: Some(s) } = job.op {' -Replacement 'if let BridgeOp::Install { scope } = job.op { let s = scope.unwrap_or(InstallScope::Machine);' -TestCommand 'cargo test --quiet --test it wi_bridge_install::install_command_exact'`
   `[OK] mutação PEGA: 'if let BridgeOp::Install { scope: Some(s) } = job.op {' -> 'if let BridgeOp::Install { scope } = job.op { let s = scope.unwrap_or(InstallScope::Machine);' fez o teste falhar (exit 101).`
3. elevar quando algum (não todos) instalador é machine (teste 5):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/mod.rs -Anchor '.all(|i| parse_scope(i.scope.as_deref()) == Some(InstallScope::Machine));' -Replacement '.any(|i| parse_scope(i.scope.as_deref()) == Some(InstallScope::Machine));' -TestCommand 'cargo test --quiet --test it wi_bridge_install::machine_only_manifest_goes_elevated_user_never'`
   `[OK] mutação PEGA: '.all(|i| parse_scope(i.scope.as_deref()) == Some(InstallScope::Machine));' -> '.any(|i| parse_scope(i.scope.as_deref()) == Some(InstallScope::Machine));' fez o teste falhar (exit 101).`
4. (extra) a nova tentativa sem `--scope` vale só para o escopo vindo do Ajustes:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/jobs.rs -Anchor ') && scope_fallback;' -Replacement ');' -TestCommand 'cargo test --quiet --test it wi_bridge_install::settings_machine_scope_falls_back_to_no_scope_once'`
   `[OK] mutação PEGA: ') && scope_fallback;' -> ');' fez o teste falhar (exit 101).`
5. Check 4 (testes da 137 `elevated_worker` e da 134/136 `tui_jobs` verdes): estão dentro do `438 passed` acima.
6. Check 5, binário real (`target/release/wingetrs.exe`, só `--dry-run`):
```
$ wingetrs install Git.Git --dry-run
Git 2.55.0.5 (winget) via winget
winget install --id Git.Git --exact --source winget --silent --disable-interactivity --accept-package-agreements --accept-source-agreements
Simulação: nada foi alterado.
$ wingetrs install Microsoft.PowerToys --dry-run
PowerToys 0.102.2862.0 (winget) via winget
winget install --id Microsoft.PowerToys --exact --source winget --silent --disable-interactivity --accept-package-agreements --accept-source-agreements
Simulação: nada foi alterado.
$ wingetrs install Git.Git --scope machine --dry-run
... --accept-source-agreements --scope machine
$ wingetrs install ripgrep --bucket main --scope user --dry-run
erro: --scope só vale para pacotes do winget
$ wingetrs install Git.Git --scope admin
error: invalid value 'admin' for '--scope <ESCOPO>': escopo inválido 'admin': use user ou machine
```
(Os dois manifestos não têm `Agreements`, então nenhuma linha `Licença:` aparece; a formatação é coberta pelos testes 8 e 10.)

Decisions:
- Escopo do Ajustes (resposta do orquestrador): `machine` no Ajustes sem `--scope` vira `--scope machine` e eleva (igual a `--scope machine`); com `0x8A150010` (`APPINSTALLER_CLI_ERROR_NO_APPLICABLE_INSTALLER`, levantado em `InstallFlow.cpp:310`; o valor vem do `AppInstallerErrors.h`, que não está em `docs/reference`) roda mais uma vez sem `--scope` e sem elevar, com o progresso `sem instalador para máquina; instalando para o usuário`. Isso fica em `jobs::run_bridge_install` (por cima de `run_bridge_elevated`); cada execução grava seu próprio `job.start`/`job.end`.
- A origem do escopo e os agreements não cabem no `BridgeJob`/`Action::Bridge` fixados no brief. Por isso a CLI passa os dois para `run_bridge_install`. Na TUI, `machine` sempre vem do Ajustes (não há `--scope` lá), e o `run.rs` guarda os agreements `<label> <url>` por id no `Ctx` entre o plano e a execução.
- `Msg::WingetPlanned` leva as notas prontas (`Licença: <label> — <url>`). Assim o rótulo não precisa ser separado do URL de novo. Com agreements, a confirmação sempre abre; sem eles, segue o `confirm` do Ajustes. Na TUI, o Ajustes é relido do `config.toml` a cada plano, porque a aba Ajustes pode ter mudado o padrão.
- `request_for` virou `pub` (o teste 4 o chama) e só aceita job `Tool::Winget`. `serve` valida o alvo num único ponto para `Upgrade` e `Install`.
- Lote marcado na Pesquisa: os pacotes winget continuam fora (a ponte instala um por vez). Agora aparecem pelo nome no toast `pulados: …` em vez de serem contados em `N itens ficaram de fora`.

Concerns:
- Arquivos fora da lista Files, com motivo: `tests/it/cli_parse.rs` (`scope: None` em 2 literais de `Command::Install`); `tests/it/install_target.rs` (`winget_only_candidate_is_not_selectable` virou `..._is_selectable`); `tests/it/tui_state.rs` (além das 2 linhas `op:`, `i_on_winget_...` agora espera `Cmd::PlanWingetInstall` e `marked_winget_hits_are_skipped_and_named` espera `pulados: PowerToys, Zeal`); snapshots `help_root`, `help_install` e 6 do `tui_view` (o botão `Instalar (enter)` aparece em linha winget; no `offline_snapshot`, de 12 linhas, o botão ocupa a linha da descrição).
- `REFUSED_NOTE` diz "atualizando sem elevar" também quando uma instalação tem a elevação recusada (texto da 137, que não mexi).
- O gate completo ficou vermelho só por causa de `manifest_2c` (outro agente). Durante as mutações, o checkout passou vários minutos sem compilar por causa de mutantes e arquivos de outros agentes (msix, state, prelude_effects); repeti até compilar.
- Nenhuma instalação real foi feita (nem winget nem runas); `run_bridge_install` pelo `ElevatedBridge` real não foi provado ao vivo.

## Revisão (commit df160cc `fix(bridge): review of task 176`)
1. Teste `settings_fallback_runs_unelevated`: o transporte aceita (helper que roda `serve`); exige `starts == 1`, a 1ª execução no pipe com `--scope machine` e a 2ª pelo exec local, sem `--scope` e sem requisição nova.
   `mutate.ps1 -Path src/tui/jobs.rs -AnchorFile t176-a.txt ("        scope: None,\n        ..job.clone()") -ReplacementFile t176-r.txt ("        ..job.clone()") -TestCommand 'cargo test --quiet --test it wi_bridge_install::settings_fallback_runs_unelevated'`
   `[OK] mutação PEGA: '        scope: None, ..job.clone()' -> '        ..job.clone()' fez o teste falhar (exit 101).` (pânico em wi_bridge_install.rs:772)
2. Caso `[("arm64","machine")]` num host x64 ⇒ `scope == None`, sem elevar.
   `[OK] mutação PEGA: 'let machine_only = !fit.is_empty()' -> 'let machine_only = true' fez o teste falhar (exit 101).` (sem a guarda: pega no caso arm64, :267)
   `[OK] mutação PEGA: 'let machine_only = !fit.is_empty()' -> 'let machine_only = true || !fit.is_empty()' fez o teste falhar (exit 101).`
3. `tui::jobs::plan_winget_install` + `WingetPlan` são usados pela CLI e pela TUI. Config que não abre ⇒ aviso e as configurações já carregadas (na TUI, `ctx.cfg.behavior`; na CLI, `Behavior::default()`, porque `install_cmd`/`run_win` são da 147 e o startup já recusa um config ruim). Equivalência no `plan_is_one_sequence_for_cli_and_tui` (wiremock + fixture do PowerToys).
4. Os agreements vão em `BridgeOp::Install { scope, agreements }`. O mapa por id saiu de `run.rs`. Teste `agreements_travel_with_the_job`: plano cancelado + 2º plano do mesmo id ⇒ só os do 2º rodam e chegam ao `job.start`.
5. O teste 10 confere também a nota de UAC.
Testes: wi_bridge_install 16 ok; tui_ 344, elevated_worker 17, job_log 20, bridge 47, cli_parse 16, todos ok; clippy --all-targets 0 avisos; fmt ok.
