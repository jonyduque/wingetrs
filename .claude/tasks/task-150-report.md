# Task 150 — relatório

Status: DONE_WITH_CONCERNS

Commits:
```
b514ecc feat(install): static gate for manifest scripts (script_scan)
```
(`git log --format='%h %s' d4b584a..HEAD -- <arquivos>` também lista `7b60e1b fix(install): review of task 142`, que é da Task 142, que mexeu em `examples/oracle_2c.rs`; não é meu.)

Gate (uma execução, conforme a regra nova do `_common.md`):
```
[OK]   rules 5s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (lib) due to 1 previous error
  error: could not compile `wingetrs` (lib test) due to 1 previous error
  (log completo: target/verify.log)
```
Causa (target/verify.log): `src\winget_install\switches.rs:91:5: unused variable: upgrade` e `tests\it\wi_switches.rs:12:5: used unwrap()`, arquivos ainda não commitados da 2d (outro agente). Prova dos meus arquivos:
- `cargo clippy --quiet --all-targets --message-format short -- -D warnings`, filtrado por `^(src|tests|examples)`: única linha `tests\it\wi_switches.rs:12:5: error: used unwrap() on a Result value` (alheio); nada em `script_scan.rs`, `tests/it/script_scan.rs`, `examples/oracle_2c.rs`.
- `cargo test --quiet --test it script_scan::` → `test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 1131 filtered out`.
- `cargo fmt --check` sem diff nos meus três arquivos (os diffs mostrados eram de `src/winget_install/select.rs`/`switches.rs`, alheios).

Mutation proofs (`pwsh -File scripts/mutate.ps1 -Path src/install/script_scan.rs ...`):
1. `-Anchor 'let text = strip_here_strings(&strip_comments(script));' -Replacement 'let text = strip_comments(script);' -TestCommand 'cargo test --quiet --test it script_scan::hklm_in_here_string_ignored'`
   `[OK] mutação PEGA: 'let text = strip_here_strings(&strip_comments(script));' -> 'let text = strip_comments(script);' fez o teste falhar (exit 101).`
2. `-Anchor 'let text = strip_here_strings(&strip_comments(script));' -Replacement 'let text = strip_here_strings(script);' -TestCommand 'cargo test --quiet --test it script_scan::hklm_in_comment_ignored'`
   `[OK] mutação PEGA: 'let text = strip_here_strings(&strip_comments(script));' -> 'let text = strip_here_strings(script);' fez o teste falhar (exit 101).`
3. `-Anchor '!supported.iter().any(|s| s.eq_ignore_ascii_case(name))' -Replacement 'true' -TestCommand 'cargo test --quiet --test it script_scan::supported_scoop_function_ok'`
   `[OK] mutação PEGA: '!supported.iter().any(|s| s.eq_ignore_ascii_case(name))' -> 'true' fez o teste falhar (exit 101).`
4. `-Anchor '.max().unwrap_or(Verdict::Confirm)' -Replacement '.min().unwrap_or(Verdict::Confirm)' -TestCommand 'cargo test --quiet --test it script_scan::worst_verdict_wins'`
   `[OK] mutação PEGA: '.max().unwrap_or(Verdict::Confirm)' -> '.min().unwrap_or(Verdict::Confirm)' fez o teste falhar (exit 101).`
5. Corpus real: `cargo run --quiet --release --example oracle_2c -- %LOCALAPPDATA%\wingetrs\catalog.db` (só leitura, nada executado), seção nova:

```
## Veredito do portão estático (`script_scan`, Task 150)

1293 manifestos S-hook; 8 não resolvem para x64 e ficam de fora. Veredito do manifesto = o pior entre os seus scripts. Nada foi executado.

| funções do Scoop suportadas | Confirm | ConfirmStrong | Refuse |
|---|---:|---:|---:|
| nenhuma (`&[]`) | 672 (52.3%) | 50 (3.9%) | 563 (43.8%) |
| todas (262) | 1084 (84.4%) | 125 (9.7%) | 76 (5.9%) |

| flag (com todas as funções suportadas) | manifestos |
|---|---:|
| AbsolutePath | 107 (8.3%) |
| RegistryUser | 50 (3.9%) |
| Hklm | 46 (3.6%) |
| Elevation | 27 (2.1%) |
| ScheduledTaskOrService | 8 (0.6%) |
| Download | 4 (0.3%) |
| DynamicCode | 4 (0.3%) |
| ScoopCommand | 4 (0.3%) |
| Msiexec | 3 (0.2%) |
| GlobalSwitch | 1 (0.1%) |
| SystemWrite | 1 (0.1%) |
```
Na primeira rodada (regra `-Global` literal), GlobalSwitch dava 21 manifestos e Refuse 96 (7.5%) com todas as funções suportadas; ver Decisions.

Decisions:
- `-Global:$global` (repassa a flag de escopo do próprio Scoop, que é `$false` na instalação por usuário) **não** conta como `GlobalSwitch`: o corpus tinha 84 linhas nessa forma (`Add-Path ... -Global:$global`, `Get-EnvVar -Name X -Global:$global`) e elas eram falsos positivos de recusa (21 → 1 manifesto). `-Global`, `-Global:$true` e uma linha com as duas formas continuam recusados (testes `alternate_spellings_flagged`, `near_misses_not_flagged`).
- `Hklm` também pega `HKLM\` (ex.: `reg add HKLM\...`), não só `HKLM:`/`HKEY_LOCAL_MACHINE`. Detalhes de `DynamicCode`/`Download`/`RegistryUser`/`ScheduledTaskOrService`, que o brief não fixou, ficaram em português, sem "segur".
- Palavras das regras de linha: `$var`, caminhos e nomes com ponto contam como uma palavra só (`$dir\curl.exe` não é `curl`, `$iex` não é `iex`). `#` só abre comentário no início de um token, como no PowerShell. Comandos vêm de `commands_in(script)`; a linha de um flag de comando é a primeira linha em que o nome aparece.
- `#[rustfmt::skip]` no `let verdict = ...` para manter a linha literal exigida (o rustfmt quebrava a cadeia em 5 linhas). Em `env_var_names` o texto limpo se chama `cleaned` para que a âncora da mutação seja única.
- Adicionei testes além dos 24 do brief: grafias alternativas de cada regra, quase-acertos que não devem gerar flag, números de linha depois de comentários de bloco e here-strings, e `variables`.

Concerns:
- Os 46 manifestos com `Hklm` incluem muitos `if ($global) { $content = $content.Replace('HKEY_CURRENT_USER', 'HKEY_LOCAL_MACHINE') }` (amostra do corpus), um ramo que nunca roda na instalação por usuário. Separar esse caso pede análise de fluxo; o orquestrador decide se a recusa continua assim.
- `AbsolutePath` (107) vem quase todo de `$env:APPDATA`/`$env:USERPROFILE`, mais alguns `C:\...` literais (ex.: `C:\rtools45`). É `ConfirmStrong`, não recusa.
- Gate vermelho só por arquivos de outro agente (2d, `winget_install/switches.rs`, `tests/it/wi_switches.rs`); não repeti a execução, seguindo a regra nova.

## Revisão (ce3af24 fix(install): review of task 150)

1. `#` abre comentário quando o caractere anterior não é de palavra (o mesmo conjunto do lexer do `classify_2c`), tanto no `script_scan` quanto no `commands_in`. O script do achado (`$x =#@'` / RunAs / `'@'`) agora dá Refuse.
2. As crases são removidas das linhas antes de casar: `Run`As`, `HK`LM:`, `Inv`oke-Expression` dão Refuse, Refuse e ConfirmStrong.
3. `-Global:$global` só é ignorado se o caractere seguinte não for de palavra; `-Glo`/`-Glob`/`-Globa` são recusados.
4. HKLM pelo .NET é recusado: `Registry]::LocalMachine`, `RegistryHive]::LocalMachine`, e `OpenBaseKey` + `LocalMachine` na mesma linha.
5. Escrita em Program Files: entram os aliases (`mkdir md ni cp cpi copy mv mi move rm ri rd rmdir del erase ren rni sc ac`), `robocopy`/`xcopy`, `[IO.File]::Write*`, `$env:ProgramW6432` e `$env:CommonProgramFiles`.

Testes: `cargo test --quiet --test it -- script_scan:: classify_2c::` → `test result: ok. 46 passed; 0 failed; 0 ignored; 0 measured; 1150 filtered out`.
Mutações:
- `[OK] mutação PEGA: ''#' if prev.is_none_or(|p| !continues_word(p))' -> ''#' if prev.is_none_or(|p| p.is_whitespace())' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '.map(|l| l.replace('`', "").to_ascii_lowercase())' -> '.map(|l| l.to_ascii_lowercase())' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: ''#' if i > 0 && is_word_char(chars[i - 1]) =>' -> ''#' if false =>' fez o teste falhar (exit 101).`
Gate (uma vez): `[OK] rules`, `[OK] rules-selftest`, `[OK] fmt`, `[FAIL] clippy exit 101`, só por arquivo alheio (`src\winget_install\verified_win.rs:27:13: variable does not need to be mutable`). Depois disso, o clippy com `--all-targets -D warnings` filtrado não aponta nada em `src/`. O binário `it` não compilou por `tests\it\wi_launch.rs` (alheio), então essa rodada não passou o clippy nos meus testes; eles compilam e passam no `cargo test` acima.

## Fail closed (d2c5691 fix(install): script gate fails closed on ambiguous comments)

- As regras de linha passam para `line_rules(l, calls_reg)` e rodam também no texto cru, linha a linha (só sem crases). Uma regra de Refuse que casa no cru e não no limpo da mesma linha gera `FlagKind::HiddenCode` (novo), ConfirmStrong, com o detalhe `"possível código escondido em comentário ou here-string: <detalhe da regra>"`. Se a regra casa no limpo, continua Refuse.
- Mudança de asserção pedida pelo coordenador: o teste 5 do brief (`hklm_in_comment_ignored`) passa de `Confirm`/sem flags para `ConfirmStrong` com só `HiddenCode`. O teste 6 continua "nenhum `Hklm`".
- Testes novos:
  - `refusal_hidden_in_here_string_is_strong_not_refuse`;
  - `hash_bypass_still_refused_without_hidden_flag`: `$x =#@'`…RunAs…`'@'` continua Refuse; regra no limpo mais comentário dá um flag só;
  - `ambiguous_comment_start_fails_closed`: `Get-Thing a<#b` / RunAs. O limpo esconde, o cru acha, e o resultado é ConfirmStrong.
- Corpus (oracle_2c, catalog.db real só leitura): **0 manifestos mudam de veredito por HiddenCode**, nos dois conjuntos de funções suportadas:

```
| funções do Scoop suportadas | Confirm | ConfirmStrong | Refuse | mudam por HiddenCode |
|---|---:|---:|---:|---:|
| nenhuma (`&[]`) | 672 (52.3%) | 50 (3.9%) | 563 (43.8%) | 0 (0.0%) |
| todas (262) | 1084 (84.4%) | 124 (9.6%) | 77 (6.0%) | 0 (0.0%) |
```
  Refuse foi de 76 para 77 (Hklm de 46 para 47) por causa de ce3af24 (.NET/`#`), não desta mudança.
- O oracle tinha quebras de linha literais em duas strings do `Gate::print`, de um heredoc meu; agora usam `\n`. A saída não mudou.
- Mutações:
  - `[OK] mutação PEGA: 'if verdict == Verdict::Refuse && !clean.iter().any(|c| c.0 == kind) {' -> 'if false {' fez o teste falhar (exit 101).`
  - `[OK] mutação PEGA: 'if verdict == Verdict::Refuse && !clean.iter().any(|c| c.0 == kind) {' -> 'if verdict == Verdict::Refuse {' fez o teste falhar (exit 101).`
  - Repetidas as duas do brief com os testes 5 e 6 novos: `[OK] mutação PEGA: ... -> 'let text = strip_comments(script);'` e `[OK] mutação PEGA: ... -> 'let text = strip_here_strings(script);'`.
- Testes: `cargo test --quiet --test it -- script_scan:: classify_2c::` → `test result: ok. 49 passed; 0 failed`.
- Gate (uma vez): rules-selftest e fmt OK; clippy FAIL só por arquivos alheios (`src\winget_install\host_win.rs` E0432, `launch_win.rs`, `fetch.rs`, `src\install\state.rs`). Com a lib sem compilar, o clippy não chega aos meus arquivos nessa rodada.

## Internos e redefinições do prelúdio (9693c60 fix(install): gate refuses prelude internals and redefinitions)

Três `FlagKind` novos:
- `PreludeInternal` (Refuse): qualquer `__wr` na linha. Cobre `__wr_effect`, `$__wr_*`, `${__wr_*}`, `function:__wr*` e `Get-Variable __wr*`.
- `PreludeRedefinition` (Refuse): `function`/`filter <nome>` (prefixo de escopo ignorado), ou `function:<nome>` usado com `Set-Item`/`New-Item`/`Remove-Item` (ou os aliases deles) ou como `$function:`/`${function:}`. Vale para qualquer nome de `prelude::PRELUDE_FUNCTIONS`, sem distinção de caixa.
- `EnvironmentVariable`: `[Environment]::SetEnvironmentVariable(` ou `[System.Environment]::...(`. É Refuse se o 3º argumento, separado no nível de topo, contém `'user'`/`'machine'` (aspas simples ou duplas) ou `environmentvariabletarget]`; senão ConfirmStrong.

As regras ficam em `line_rules`, então também passam pela verificação de código escondido.

Testes: `prelude_internals_refused`, `prelude_redefinition_refused` e `set_environment_variable_target_decides`. Cada caso exige exatamente um flag, do tipo da regra, e há casos negativos.

Como `tests/it/prelude_effects.rs` (alheio) não compilava, rodei meu módulo num binário temporário `tests/tmp_task150_scan.rs` (`#[path]` para `it/script_scan.rs`), apagado antes do gate: `test result: ok. 39 passed; 0 failed`.

Mutações:
- `[OK] mutação PEGA: 'if l.contains("__wr") {' -> 'if false {' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'for name in redefined_prelude_functions(l) {' -> 'for name in Vec::<&str>::new() {' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '(Verdict::Refuse, D_ENV_PERSISTENT)' -> '(Verdict::ConfirmStrong, D_ENV_PERSISTENT)' fez o teste falhar (exit 101).`

Corpus (oracle_2c, catalog.db real só leitura), coluna nova:
```
| funções do Scoop suportadas | Confirm | ConfirmStrong | Refuse | mudam por HiddenCode | mudam pelas regras do prelúdio |
|---|---:|---:|---:|---:|---:|
| nenhuma (`&[]`) | 671 (52.2%) | 51 (4.0%) | 563 (43.8%) | 0 (0.0%) | 1 (0.1%) |
| todas (262) | 1079 (84.0%) | 129 (10.0%) | 77 (6.0%) | 0 (0.0%) | 5 (0.4%) |
```
5 manifestos mudam, todos de Confirm para ConfirmStrong: `EnvironmentVariable` aparece em 6 manifestos, todos no escopo do processo. Refuse continua 77. Nenhum manifesto do corpus usa `__wr` nem redefine função do prelúdio.

Gate (uma vez): `[OK] rules`, `[OK] rules-selftest`, `[OK] fmt`, `[FAIL] clippy exit 101`, só por `tests\it\wi_bridge_install.rs:410:37` (alheio). Nenhum lint em `src/` nem em `tests/it/script_scan.rs`.
