Status: DONE_WITH_CONCERNS

Commits:
b1cad6e feat(install): pure runner.ps1 generator and variable-by-hook table

Gate: (uma execução, conforme a regra nova do _common.md; rules/fmt/clippy passaram, o passo test falhou só em módulos de outros agentes)
```
  thread 'wi_select::classic_types_are_native' (6096) panicked at tests\it\wi_select.rs:24:38:
  ---- wi_select::exe_silent_rules stdout ----
  thread 'wi_select::exe_silent_rules' (12260) panicked at tests\it\wi_select.rs:24:38:
  ---- wi_uninstall_cmd::valid_arp_key_cases stdout ----
  thread 'wi_uninstall_cmd::valid_arp_key_cases' (6604) panicked at tests\it\wi_uninstall_cmd.rs:158:9:
  test result: FAILED. 1113 passed; 7 failed; 28 ignored; 0 measured; 0 filtered out; finished in 16.76s
  error: test failed, to rerun pass `--test it`
  (log completo: target/verify.log)
```
Falhas (todas alheias, arquivos não commitados de outros agentes): script_scan::hklm_in_here_string_ignored, tui_view::settings_screen_scrolls_to_the_last_row_in_24_lines, wi_manifest::explicit_upgrade_defaults_false, wi_select::{bridge_reasons_one_fixture_each, classic_types_are_native, exe_silent_rules}, wi_uninstall_cmd::valid_arp_key_cases.
Prova dos meus arquivos:
- `cargo test --quiet --test it script_runner::` ⇒ `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1110 filtered out; finished in 5.04s`
- `cargo clippy --quiet --all-targets --message-format short 2>&1 | grep runner` ⇒ nenhuma linha (o `unwrap` em script_runner.rs:223 apontado pelo coordenador foi corrigido: `run_pwsh` devolve `io::Result`).

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/install/runner.rs -Anchor 'HookKind::PostInstall => cx.current.clone(),' -Replacement 'HookKind::PostInstall => cx.version_dir.clone(),' -TestCommand 'cargo test --quiet --test it script_runner::vars_post_install_dir_is_current'`
   `[OK] mutação PEGA: 'HookKind::PostInstall => cx.current.clone(),' -> 'HookKind::PostInstall => cx.version_dir.clone(),' fez o teste falhar (exit 101).`
2. `pwsh -File scripts/mutate.ps1 -Path src/install/runner.rs -Anchor "'\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}'" -Replacement "'\u{2018}'" -TestCommand 'cargo test --quiet --test it script_runner::quotes_ascii_and_typographic_escaped'`
   `[OK] mutação PEGA: ''\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}'' -> ''\u{2018}'' fez o teste falhar (exit 101).`
3. `pwsh -File scripts/mutate.ps1 -Path src/install/runner.rs -Anchor 'ScriptOp::Update => "update",' -Replacement 'ScriptOp::Update => "install",' -TestCommand 'cargo test --quiet --test it script_runner::vars_update_install_hooks_cmd_update'`
   `[OK] mutação PEGA: 'ScriptOp::Update => "update",' -> 'ScriptOp::Update => "install",' fez o teste falhar (exit 101).`
   Mesmo comando com `script_runner::vars_update_uninstall_hooks`:
   `[OK] mutação PEGA: 'ScriptOp::Update => "update",' -> 'ScriptOp::Update => "install",' fez o teste falhar (exit 101).`
(Mutações rodadas antes da troca do `unwrap` no helper de teste; a troca não toca `src/` nem as asserções.)

Decisions:
- Tabela seguida pelo relatório de sondas 2c (linhas "variável × hook"): `$old_version` só nos hooks de desinstalação com `op: Update` (é "—" nos hooks de instalação do upgrade); `$purge` só com `op: Uninstall` em hooks de remoção.
- `$globaldir` = `<raiz>\global` (`$global` é sempre `$false`; evita `%ProgramData%\scoop` e caminho vazio que viraria `\`). `$cachedir` = `<raiz>\cache`. `$scoopdir` = `<raiz>`.
- `$bucket`/`$architecture` ausentes ⇒ `Null`. `$Name` é `StrArray` (parâmetro `[string[]]` do `Invoke-Installer`). `$old_manifest`/`$install` não gerados (não há campo no `ScriptContext`).
- `RunnerError::BadName(String)` carrega o nome recusado; mensagem em português.
- `end_to_end_pwsh` chama `pwsh` (PowerShell 7) com `-NoProfile -NonInteractive -ExecutionPolicy Bypass -File`, runner gravado em UTF-8 sem BOM numa tempdir.

Concerns:
- O runner é gravado sem BOM pelo teste; se o chamador futuro usar `powershell.exe` 5.1, precisará gravar com BOM (5.1 lê BOM-less como ANSI). Não coberto aqui.
- Gate não ficou verde por falhas alheias (lista acima); o orquestrador deve rodar o gate depois que os outros agentes commitarem.

## Revisão (c4b7187 fix(install): review of task 155)

Commits:
c4b7187 fix(install): review of task 155
b1cad6e feat(install): pure runner.ps1 generator and variable-by-hook table

1. `end_to_end_pwsh` agora `#[ignore = "live: roda pwsh real"]`; `run_pwsh` devolve `io::Error` com "pwsh nao encontrado ou nao executou: ..." (sem unwrap).
   Mutação `$ErrorActionPreference = 'Stop'` (-AnchorFile/-ReplacementFile; TestCommand `cargo test --quiet --test it script_runner::end_to_end_pwsh -- --ignored`):
   `[OK] mutação PEGA: 's.push_str("$OutputEncoding = [Console]::OutputEncoding\n");' -> 's.push_str("$OutputEncoding = [Console]::OutputEncoding\n$ErrorActionPreference = 'Stop'\n");' fez o teste falhar (exit 101).` (pânico em script_runner.rs:289, o `assert_eq!(code, 0)` do Remove-Item)
2. `$old_version`: ~/scoop/apps/scoop/current (e6aa3b36), `libexec/scoop-update.ps1:262` define `$old_version` dentro de `update()`, que chama `install_app` em :389/:393; `lib/install.ps1` não a redefine ⇒ visível por escopo dinâmico. Agora definida em todos os hooks com `op: Update` (teste 6 exige `Some("0.9")`). Isso diverge do "—" da tabela da sonda D3.
   `[OK] mutação PEGA: 'if let (ScriptOp::Update, Some(old)) = (cx.op, &cx.old_version) {' -> 'if let (ScriptOp::Update, false, Some(old)) = (cx.op, install_hook, &cx.old_version) {' fez o teste falhar (exit 101).`
3. `bad_var_name_err` também recusa `"1a"` e `""` e aceita `"_a1"`.
4. Doc de `runner_ps1`: quem grava o arquivo usa UTF-8 com BOM (o powershell 5.1 lê sem BOM como ANSI).

Gate (uma execução; falha alheia em src\install\script_scan.rs:682):
```
  error: character literal may only contain one codepoint
  error: character literal may only contain one codepoint
  error[E0762]: unterminated character literal
  error: could not compile `wingetrs` (lib) due to 3 previous errors
  error: could not compile `wingetrs` (lib test) due to 3 previous errors
  (log completo: target/verify.log)
```
Prova dos meus: `cargo test --quiet --test it script_runner:: -- --include-ignored` ⇒ `test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1166 filtered out; finished in 5.85s`; `cargo clippy --all-targets` (compilou tudo depois que a script_scan foi corrigida) ⇒ avisos só em tests\it\wi_fetch.rs (alheio), nenhum em runner/script_runner.
