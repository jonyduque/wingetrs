# Task 222 report

Status: DONE_WITH_CONCERNS

Commits:
```
d5d99a2 ci: GitHub Actions running the verify gate on Windows
```

Gate: rodado uma vez; falhou só em arquivo alheio (Task 216 em andamento):
```
[FAIL] rules exit 1
    src\update.rs:206: [spawn_blocking sem handle] tokio::task::spawn_blocking(move || UpdateLock::acquire_timeout(&lock_path, timeout))
  (log completo: target/verify.log)
```
Meus arquivos provados à parte:
- `cargo test --quiet --test it -- non_elevated:: script_host_win:: install_file_installer:: elevated_install::` => `test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1947 filtered out; finished in 4.48s`
- `cargo clippy --quiet --all-targets --message-format short -- -D warnings` => nenhuma linha (limpo no crate todo naquele momento)
- `rustfmt --edition 2024 --check` nos 4 `.rs` meus => limpo
- `pwsh -File scripts/check_rules_selftest.ps1` => `[OK] check_rules: 47 casos plantados`
- `pwsh -File scripts/check_rules.ps1 -Path <pasta vazia> -ItDir tests/it` => exit 0 (a regra nova vê os 10 nomes como testes e só chamadas na lista)
- `pwsh -File scripts/ci_non_elevated.ps1 -SelfTest` => `[OK] Test-RunOutput: 7 casos`
- Simulação do modo real sem `-Credential` (usuário atual, não elevado, `CI=true` no pai e removido pelo `-Environment`, mesmos argumentos): `test result: ok. 10 passed; 0 failed; ...`, veredito do `Test-RunOutput` vazio (passou).

`-DryRun` (saída real):
```
executável: C:\Users\jonyd\Projetos\wingetrs\target\debug\deps\it-157b989d55711831.exe
testes (10):
  elevated_install::win::staging_dacl_has_no_users
  install_file_installer::installer_win_runs_a_cmd_with_fixed_path_cwd_and_args
  script_host_win::args_exact
  script_host_win::cancel_after_launch_waits_for_the_hook
  script_host_win::cwd_is_workdir
  script_host_win::env_path_is_fixed_not_inherited
  script_host_win::env_scoop_set_psmodulepath_removed
  script_host_win::exit_code_and_lines
  script_host_win::policy_error_mapped
  script_host_win::workdir_removed_with_code_1
comandos:
  New-LocalUser -Name wingetrs-ci-126b7f -Password *** -AccountNeverExpires -PasswordNeverExpires -UserMayNotChangePassword
  Add-LocalGroupMember -SID S-1-5-32-545 -Member wingetrs-ci-126b7f
  icacls "C:\Users\jonyd\Projetos\wingetrs" /grant "wingetrs-ci-126b7f:(OI)(CI)RX" /Q
  New-Item -ItemType Directory C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f; icacls "C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f" /grant "wingetrs-ci-126b7f:(OI)(CI)M" /Q
  Start-Process -FilePath "C:\Users\jonyd\Projetos\wingetrs\target\debug\deps\it-157b989d55711831.exe" -ArgumentList '--exact --test-threads=1 --nocapture elevated_install::win::staging_dacl_has_no_users install_file_installer::installer_win_runs_a_cmd_with_fixed_path_cwd_and_args script_host_win::args_exact script_host_win::cancel_after_launch_waits_for_the_hook script_host_win::cwd_is_workdir script_host_win::env_path_is_fixed_not_inherited script_host_win::env_scoop_set_psmodulepath_removed script_host_win::exit_code_and_lines script_host_win::policy_error_mapped script_host_win::workdir_removed_with_code_1' -Credential wingetrs-ci-126b7f:*** -WorkingDirectory "C:\Users\jonyd\Projetos\wingetrs" -RedirectStandardOutput "C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f\stdout.txt" -RedirectStandardError "C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f\stderr.txt" -Wait -PassThru
  ambiente do filho: TEMP=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f; TMP=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f; USERPROFILE=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f; APPDATA=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f\AppData; LOCALAPPDATA=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f\LocalAppData; CI=
  finally: Remove-LocalUser wingetrs-ci-126b7f; perfil (Win32_UserProfile) e C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-126b7f removidos
```
O modo real nunca criou nada aqui: ganhou uma trava (`GITHUB_ACTIONS` diferente de `true` => `[FAIL] só roda num runner do GitHub`, exit 1, antes de gerar senha ou usuário); a trava foi exercitada uma vez e parou ali.

Mutation proofs:
1. Helper pula sem checar `CI`:
   `pwsh -File scripts/mutate.ps1 -Path tests/it/non_elevated.rs -Anchor '    ci && elevated' -Replacement '    elevated' -TestCommand 'cargo test --quiet --test it non_elevated::'`
   `[OK] mutação PEGA: '    ci && elevated' -> '    elevated' fez o teste falhar (exit 101).`
2. Chamada do helper num teste fora da lista (`script_host_win::elevated_refused`, chamada dentro do corpo, várias linhas via `-AnchorFile`/`-ReplacementFile` em `scratchpad/t222-anchor.txt`/`t222-repl.txt`). TestCommand com `-Path <pasta vazia> -ItDir tests/it`, porque o `check_rules` completo já falha no `src/update.rs` da 216:
   `pwsh -File scripts/mutate.ps1 -Path tests/it/script_host_win.rs -AnchorFile .../t222-anchor.txt -ReplacementFile .../t222-repl.txt -TestCommand "pwsh -NoProfile -File scripts/check_rules.ps1 -Path .../t222-empty -ItDir tests/it"`
   `[OK] mutação PEGA: 'fn elevated_refused() {` (linha cortada pela âncora de várias linhas; exit 1). Os casos plantados do selftest cobrem: chamada fora da lista, nome da lista que não existe, nome que é fn mas não teste, chamada em ajudante, mod aninhado com e sem o nome do mod.
3. `grep -n "uses:" .github/workflows/ci.yml`:
   ```
   28:      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
   32:      - uses: dtolnay/rust-toolchain@b725eda525362e7cf34af9a7243cac1e3f01a7ec # stable branch, 2026-10-21
   36:      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3
   72:        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
   ```
   SHAs de `git ls-remote` (rust-cache: commit do tag anotado, `^{}`).
4. Script aceita `test result: ok` sem conferir `10 passed`:
   `pwsh -File scripts/mutate.ps1 -Path scripts/ci_non_elevated.ps1 -Anchor '"(?m)^test result: ok\. $Expected passed;"' -Replacement '"(?m)^test result: ok\."' -TestCommand 'pwsh -NoProfile -File scripts/ci_non_elevated.ps1 -SelfTest'`
   `[OK] mutação PEGA: '"(?m)^test result: ok\. $Expected passed;"' -> '"(?m)^test result: ok\."' fez o teste falhar (exit 1).`
5. `grep -n "password\|Password" scripts/ci_non_elevated.ps1`:
   ```
   83:    "  New-LocalUser -Name $user -Password *** -AccountNeverExpires -PasswordNeverExpires -UserMayNotChangePassword"
   112:        $sid = (New-LocalUser -Name $user -Password $secure -AccountNeverExpires -PasswordNeverExpires `
   113:                -UserMayNotChangePassword -Description 'wingetrs CI (temporary)').SID.Value
   ```
   A senha em texto vive só em `$plain` (linhas 99–103): vai para `::add-mask::` (exigido pelo brief, só com `GITHUB_ACTIONS=true`), vira `SecureString` e a variável é zerada. Nada grava em disco.
6. Gate local: ver "Gate" acima (falha alheia; não repeti).

Decisions:
- Os 10 nomes saíram da análise do código (testes que chegam ao `is_elevated` real: `PwshHost::with_program`, `installer_win::run`, DACL do staging), batendo com os 3 grupos da ARMADILHAS; não dá para rodar elevado aqui para conferir. O gate da CI dirá.
- `RUST_TEST_NOCAPTURE=1` só no passo do gate: sem isso o libtest engole o `eprintln!` de teste que passa e não há linha `pulado:` para contar. O filho do usuário comum roda com `--nocapture` pelo mesmo motivo.
- Ambiente do filho via `Start-Process -Environment` (PS 7.4+): `CI` removido; `TEMP/TMP/USERPROFILE/APPDATA/LOCALAPPDATA` apontam para a pasta de trabalho dele (o ambiente herdado apontaria para o perfil do runneradmin, que ele não grava). Usuário entra no grupo Usuários por SID (direito de logon local).
- `icacls` só na raiz do checkout: `target/` fica dentro dela e herda (um segundo `icacls` em `target` repetiria a propagação). Modo real recusa fora do GitHub (trava acima).
- Falha de `icacls` também cai na reserva (é preparação do usuário, não teste).

Concerns:
- YAML não validado por `actionlint` (não instalado); só `yaml.safe_load` do Python (parse ok, 9 passos).
- Não provado ao vivo: `Start-Process -Credential` + `-Environment` no runner, a propagação do `icacls` sobre `target/` (pode levar minutos) e a contagem de 10 `pulado` no gate. Precisa do push de teste do orquestrador.
- `RUST_TEST_NOCAPTURE` deixa o `target/verify.log` da CI mais verboso (saída de todos os testes); não muda o resumo do `verify.ps1`.
- O ACE do usuário apagado fica órfão no checkout do runner (VM descartável); não removido para não propagar de novo.

## Rodada de correção (Minors da revisão)

1. `check_rules`: acusa qualquer menção a `skip_if_elevated_in_ci` fora de `tests/it/non_elevated.rs` que não esteja num teste da lista (apelido via `use`, ponteiro); `skip_decision` ficou privado.
2. `ci_non_elevated.ps1`: reserva só para `New-LocalUser`/`seclogon` e para `Start-Process -Credential` lançar; `icacls`/`New-Item` falhando reprova o passo (`FALHOU na preparação` no resumo).
3. `::add-mask::` emitido direto (a trava já garante `GITHUB_ACTIONS=true`).
4. `ci.yml`: o passo de contagem reprova se as linhas `pulado:` diferirem das linhas de `tests/it/non_elevated.txt`.
5. `docs/ARMADILHAS.md`: entrada sobre `RUST_TEST_NOCAPTURE` e o filtro `^---- .* stdout ----` (arquivo fora da lista Files, pedido pelo orquestrador).

Saídas reais:
```
[OK] check_rules: 49 casos plantados
[OK] Test-RunOutput: 7 casos
```

Mutação do item 1 (apelido via `use` em `script_host_win::elevated_refused`, `-AnchorFile`/`-ReplacementFile`, TestCommand `check_rules.ps1 -Path <vazia> -ItDir tests/it`):
```
[OK] mutação PEGA: 'fn elevated_refused() {
```
(linha cortada pela âncora de várias linhas; exit 1)

`-DryRun` (exit 0):
```
executável: C:\Users\jonyd\Projetos\wingetrs\target\debug\deps\it-157b989d55711831.exe
testes (10):
  elevated_install::win::staging_dacl_has_no_users
  install_file_installer::installer_win_runs_a_cmd_with_fixed_path_cwd_and_args
  script_host_win::args_exact
  script_host_win::cancel_after_launch_waits_for_the_hook
  script_host_win::cwd_is_workdir
  script_host_win::env_path_is_fixed_not_inherited
  script_host_win::env_scoop_set_psmodulepath_removed
  script_host_win::exit_code_and_lines
  script_host_win::policy_error_mapped
  script_host_win::workdir_removed_with_code_1
comandos:
  New-LocalUser -Name wingetrs-ci-9c3c45 -Password *** -AccountNeverExpires -PasswordNeverExpires -UserMayNotChangePassword
  Add-LocalGroupMember -SID S-1-5-32-545 -Member wingetrs-ci-9c3c45
  icacls "C:\Users\jonyd\Projetos\wingetrs" /grant "wingetrs-ci-9c3c45:(OI)(CI)RX" /Q
  New-Item -ItemType Directory C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45; icacls "C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45" /grant "wingetrs-ci-9c3c45:(OI)(CI)M" /Q
  Start-Process -FilePath "C:\Users\jonyd\Projetos\wingetrs\target\debug\deps\it-157b989d55711831.exe" -ArgumentList '--exact --test-threads=1 --nocapture elevated_install::win::staging_dacl_has_no_users install_file_installer::installer_win_runs_a_cmd_with_fixed_path_cwd_and_args script_host_win::args_exact script_host_win::cancel_after_launch_waits_for_the_hook script_host_win::cwd_is_workdir script_host_win::env_path_is_fixed_not_inherited script_host_win::env_scoop_set_psmodulepath_removed script_host_win::exit_code_and_lines script_host_win::policy_error_mapped script_host_win::workdir_removed_with_code_1' -Credential wingetrs-ci-9c3c45:*** -WorkingDirectory "C:\Users\jonyd\Projetos\wingetrs" -RedirectStandardOutput "C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45\stdout.txt" -RedirectStandardError "C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45\stderr.txt" -Wait -PassThru
  ambiente do filho: TEMP=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45; TMP=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45; USERPROFILE=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45; APPDATA=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45\AppData; LOCALAPPDATA=C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45\LocalAppData; CI=
  finally: Remove-LocalUser wingetrs-ci-9c3c45; perfil (Win32_UserProfile) e C:\Users\jonyd\AppData\Local\Temp\wingetrs-ci-9c3c45 removidos
```
