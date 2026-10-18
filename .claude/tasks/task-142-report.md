Status: DONE

Commits:
455413f feat(install): phase 2c oracle (sub-cases and Scoop functions used by manifest scripts)

Gate:
[OK]   rules 5s
[OK]   rules-selftest 14s
[OK]   fmt 3s
[OK]   clippy 35s
[OK]   test (1095 passed, 27 ignored) 57s
[OK]   gate verde
(rodado com o trabalho não commitado da Task 140 presente no checkout; não interferiu.)

Mutation proofs:
1. Ignorar variantes de `architecture` (teste 3):
   pwsh -File scripts/mutate.ps1 -Path src/install/classify_2c.rs -Anchor 'if let Some(arch) = r.get("architecture").and_then(Value::as_object) {' -Replacement 'if let Some(arch) = r.get("no-architecture").and_then(Value::as_object) {' -TestCommand 'cargo test --quiet --test it classify_2c::variant_only_key_counts'
   [OK] mutação PEGA: 'if let Some(arch) = r.get("architecture").and_then(Value::as_object) {' -> 'if let Some(arch) = r.get("no-architecture").and_then(Value::as_object) {' fez o teste falhar (exit 101).
2. Não pular here-strings (teste 6):
   pwsh -File scripts/mutate.ps1 -Path src/install/classify_2c.rs -Anchor "'@' if here_string_start(&chars, i) => {" -Replacement "'@' if false && here_string_start(&chars, i) => {" -TestCommand 'cargo test --quiet --test it classify_2c::commands_skip_comments_strings_herestrings'
   [OK] mutação PEGA: ''@' if here_string_start(&chars, i) => {' -> ''@' if false && here_string_start(&chars, i) => {' fez o teste falhar (exit 101).
   (O here-string do teste contém `don't`: sem o tratamento, o `'` abre uma string comum que fecha no apóstrofo e `ensure` vira comando. Sem esse apóstrofo a mutação não seria pega, porque `'@`...`'@` também fecha como string simples.)
3. Oráculo rodado: `target/release/examples/oracle_2c.exe %LOCALAPPDATA%\wingetrs\catalog.db` (0,3 s); saída abaixo e em docs/superpowers/reports/2026-10-18-oraculo-2c.md.

Decisions:
- Assinatura: `classify(&str) -> Result<Option<Classification>, serde_json::Error>` (o brief pede `None` e `Err` além de `Classification`); `SubCase` com `Ord` na ordem de risco; `worst` = maior do conjunto.
- `installer`/`uninstaller` só com `args` (sem `file`) conta S-file (o Scoop roda o arquivo baixado); `script` + `file` juntos dão os dois sub-casos. `innosetup`/`psmodule` contam também em variantes.
- Léxico (`commands_in`, documentado no código): separadores também `}` e `&`; palavras seguidas de `=` (chave de hashtable/atribuição), palavras-chave do PowerShell e o nome após `function` não contam; dedup case-insensitive por manifesto.
- Funções do Scoop: lidas de `~/scoop/apps/scoop/current/lib/*.ps1` local (commit e6aa3b366bdee8ed138c1e0f7b85192ebdd35d0f), 262 nomes, tabela fixa no exemplo.
- Cobertura: S-ps1 nunca conta como coberto (o .ps1 não está no manifesto); acrescentei a coluna "N comandos mais usados de qualquer tipo" e N=50/100, porque só funções do Scoop param em 3,4% (cmdlets dominam).

Concerns:
- Funções definidas no próprio script (ex.: `CreateFile`) aparecem como "cmdlet/outro"; comandos dentro de `"$(...)"` não são vistos. O catálogo local só tem main e extras (sem linha "demais").
- Achado para a decisão da 2c: implementar só funções do Scoop cobre 3,4% dos S-hook/S-ps1; cobrir 82% exige os 30 comandos mais usados, quase todos cmdlets (`Remove-Item`, `Test-Path`, `Out-Null`, `New-Item`, `Set-Content`...).
- Teste extra `commands_cover_all_script_places_and_separators` além dos 7 pedidos.

Saída do oráculo:

# Oráculo da fase 2c

Catálogo: `C:\Users\jonyd\AppData\Local/wingetrs/catalog.db` (somente leitura). Manifestos scoop: 4118, JSON inválido: 0.

## Manifestos 2c por bucket e pior sub-caso (`worst`)

| bucket | total | 2c | S-psm | S-inno | S-file | S-ps1 | S-hook | só S-psm/S-inno/S-file (dos 2c) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| main | 1692 | 294 (17.4%) | 5 | 17 | 3 | 1 | 268 | 25 (8.5%) |
| extras | 2426 | 1109 (45.7%) | 5 | 76 | 1 | 2 | 1025 | 82 (7.4%) |
| todos | 4118 | 1403 (34.1%) | 10 | 93 | 4 | 3 | 1293 | 107 (7.6%) |

## Presença de cada sub-caso (% dos 2c; um manifesto pode ter vários)

| bucket | S-psm | S-inno | S-file | S-ps1 | S-hook |
|---|---:|---:|---:|---:|---:|
| main | 11 (3.7%) | 23 (7.8%) | 6 (2.0%) | 1 (0.3%) | 268 (91.2%) |
| extras | 17 (1.5%) | 124 (11.2%) | 4 (0.4%) | 3 (0.3%) | 1025 (92.4%) |
| todos | 28 (2.0%) | 147 (10.5%) | 10 (0.7%) | 4 (0.3%) | 1293 (92.2%) |

## Comandos chamados pelos scripts

1296 manifestos S-hook/S-ps1; 158 comandos distintos, 36 deles funções do Scoop (lista de 262 funções).

| # | comando | manifestos | tipo |
|---:|---|---:|---|
| 1 | `Remove-Item` | 633 | cmdlet/outro |
| 2 | `Test-Path` | 505 | cmdlet/outro |
| 3 | `Out-Null` | 449 | cmdlet/outro |
| 4 | `New-Item` | 317 | cmdlet/outro |
| 5 | `Set-Content` | 247 | cmdlet/outro |
| 6 | `Expand-7ZipArchive` | 236 | função do Scoop |
| 7 | `Copy-Item` | 222 | cmdlet/outro |
| 8 | `ForEach-Object` | 193 | cmdlet/outro |
| 9 | `Get-ChildItem` | 156 | cmdlet/outro |
| 10 | `Rename-Item` | 136 | cmdlet/outro |
| 11 | `Get-Content` | 108 | cmdlet/outro |
| 12 | `Move-Item` | 106 | cmdlet/outro |
| 13 | `ensure` | 76 | função do Scoop |
| 14 | `Write-Host` | 57 | cmdlet/outro |
| 15 | `info` | 53 | função do Scoop |
| 16 | `Select-Object` | 53 | cmdlet/outro |
| 17 | `Get-Item` | 44 | cmdlet/outro |
| 18 | `error` | 43 | função do Scoop |
| 19 | `Start-Process` | 42 | cmdlet/outro |
| 20 | `Invoke-ExternalCommand` | 41 | função do Scoop |
| 21 | `reg` | 39 | cmdlet/outro |
| 22 | `is_admin` | 34 | função do Scoop |
| 23 | `Join-Path` | 32 | cmdlet/outro |
| 24 | `Expand-MsiArchive` | 29 | função do Scoop |
| 25 | `Stop-Process` | 25 | cmdlet/outro |
| 26 | `warn` | 23 | função do Scoop |
| 27 | `Out-File` | 21 | cmdlet/outro |
| 28 | `Where-Object` | 19 | cmdlet/outro |
| 29 | `Add-Content` | 18 | cmdlet/outro |
| 30 | `get_config` | 18 | função do Scoop |
| 31 | `Expand-DarkArchive` | 15 | função do Scoop |
| 32 | `Start-Sleep` | 15 | cmdlet/outro |
| 33 | `Expand-InnoArchive` | 14 | função do Scoop |
| 34 | `Get-Command` | 12 | cmdlet/outro |
| 35 | `Set-EnvVar` | 11 | função do Scoop |
| 36 | `appdir` | 10 | função do Scoop |
| 37 | `Get-EnvVar` | 10 | função do Scoop |
| 38 | `Split-Path` | 10 | cmdlet/outro |
| 39 | `Add-Path` | 9 | função do Scoop |
| 40 | `New-ItemProperty` | 9 | cmdlet/outro |
| 41 | `Remove-Path` | 9 | função do Scoop |
| 42 | `ConvertTo-Json` | 8 | cmdlet/outro |
| 43 | `New-Object` | 8 | cmdlet/outro |
| 44 | `shortcut_folder` | 8 | função do Scoop |
| 45 | `versiondir` | 8 | função do Scoop |
| 46 | `echo` | 7 | cmdlet/outro |
| 47 | `Get-ItemProperty` | 7 | cmdlet/outro |
| 48 | `Split-PathLikeEnvVar` | 7 | função do Scoop |
| 49 | `Get-Service` | 6 | cmdlet/outro |
| 50 | `Write-Output` | 6 | cmdlet/outro |
| 51 | `abort` | 5 | função do Scoop |
| 52 | `mkdir` | 5 | cmdlet/outro |
| 53 | `mv` | 5 | cmdlet/outro |
| 54 | `python` | 5 | cmdlet/outro |
| 55 | `Remove-ItemProperty` | 5 | cmdlet/outro |
| 56 | `rm_shim` | 5 | função do Scoop |
| 57 | `sc.exe` | 5 | cmdlet/outro |
| 58 | `shim` | 5 | função do Scoop |
| 59 | `Sort-Object` | 5 | cmdlet/outro |
| 60 | `startmenu_shortcut` | 5 | função do Scoop |
| 61 | `Stop-Service` | 5 | cmdlet/outro |
| 62 | `ConvertFrom-Json` | 4 | cmdlet/outro |
| 63 | `CreateFile` | 4 | cmdlet/outro |
| 64 | `currentdir` | 4 | função do Scoop |
| 65 | `gc` | 4 | cmdlet/outro |
| 66 | `Get-Process` | 4 | cmdlet/outro |
| 67 | `installed` | 4 | função do Scoop |
| 68 | `Invoke-WebRequest` | 4 | cmdlet/outro |
| 69 | `Pop-Location` | 4 | cmdlet/outro |
| 70 | `Push-Location` | 4 | cmdlet/outro |
| 71 | `scoop` | 4 | cmdlet/outro |
| 72 | `deactivate` | 3 | cmdlet/outro |
| 73 | `git` | 3 | cmdlet/outro |
| 74 | `movedir` | 3 | função do Scoop |
| 75 | `Select` | 3 | cmdlet/outro |
| 76 | `Add-Type` | 2 | cmdlet/outro |
| 77 | `clink` | 2 | cmdlet/outro |
| 78 | `ConvertToPrettyJson` | 2 | função do Scoop |
| 79 | `Expand-ZipArchive` | 2 | função do Scoop |
| 80 | `firefox` | 2 | cmdlet/outro |

Todas as funções do Scoop usadas: `Expand-7ZipArchive` (236), `ensure` (76), `info` (53), `error` (43), `Invoke-ExternalCommand` (41), `is_admin` (34), `Expand-MsiArchive` (29), `warn` (23), `get_config` (18), `Expand-DarkArchive` (15), `Expand-InnoArchive` (14), `Set-EnvVar` (11), `appdir` (10), `Get-EnvVar` (10), `Add-Path` (9), `Remove-Path` (9), `shortcut_folder` (8), `versiondir` (8), `Split-PathLikeEnvVar` (7), `abort` (5), `rm_shim` (5), `shim` (5), `startmenu_shortcut` (5), `currentdir` (4), `installed` (4), `movedir` (3), `ConvertToPrettyJson` (2), `Expand-ZipArchive` (2), `Get-CommandPath` (2), `Get-HelperPath` (2), `persist_data` (2), `persist_permission` (2), `Select-CurrentVersion` (2), `shimdir` (2), `unlink_persist_data` (2), `Find-BucketDirectory` (1).

## Cobertura acumulada

Manifesto coberto = sem S-ps1 (o conteúdo do `.ps1` não está no manifesto) e todos os comandos dos seus scripts no conjunto implementado. Denominador: 1296 manifestos S-hook/S-ps1 (4 com S-ps1). Coluna 2: as N funções do Scoop mais usadas. Coluna 3: os N comandos mais usados de qualquer tipo (funções do Scoop + cmdlets e programas).

| N | só funções do Scoop | qualquer comando |
|---|---:|---:|
| 5 | 33 (2.5%) | 369 (28.5%) |
| 10 | 37 (2.9%) | 706 (54.5%) |
| 20 | 44 (3.4%) | 959 (74.0%) |
| 30 | 44 (3.4%) | 1066 (82.3%) |
| 50 | 44 (3.4%) | 1161 (89.6%) |
| 100 | 44 (3.4%) | 1242 (95.8%) |
| todas | 44 (3.4%) | 1292 (99.7%) |
## Revisão (fix 7b60e1b)

Status: DONE_WITH_CONCERNS (o gate não fechou verde por causa de arquivos de outros agentes, ver abaixo)

Commits:
7b60e1b fix(install): review of task 142
455413f feat(install): phase 2c oracle (sub-cases and Scoop functions used by manifest scripts)

Gate: o checkout é compartilhado com agentes trabalhando nas Tasks 143/170–172 sem commit. Rodei 6 vezes ao longo de ~25 min:
- uma rodada com clippy verde terminou com `[FAIL] test exit 101`, com 1047 passaram e 2 falharam: `tui_view::settings_screen_snapshot` e `tui_view::turning_icons_off_clears_the_cache_and_the_indent` (src/tui/settings.rs, de outro agente);
- as últimas rodadas param em `[FAIL] clippy exit 101`, com `tests\it\script_runner.rs:223:5: error: used unwrap() on a Result value` (arquivo de outro agente).
Os meus testes: `cargo test --quiet --test it classify_2c::` dá `test result: ok. 12 passed; 0 failed`. `[OK] rules`, `[OK] fmt`. O orquestrador precisa rodar o gate de novo quando o checkout ficar estável.

Mutation proofs (todas no estado final):
[OK] mutação PEGA: 'if let Some(arch) = r.get("architecture").and_then(Value::as_object) {' -> 'if let Some(arch) = r.get("no-architecture").and_then(Value::as_object) {' fez o teste falhar (exit 101).   (variant_only_key_counts)
[OK] mutação PEGA: ''@' if here_string_start(chars, i) => {' -> ''@' if false && here_string_start(chars, i) => {' fez o teste falhar (exit 101).   (commands_skip_comments_strings_herestrings)
[OK] mutação PEGA: ''$' if chars.get(i + 1) == Some(&'(') && self.nest < MAX_NEST => {' -> ''$' if false && chars.get(i + 1) == Some(&'(') && self.nest < MAX_NEST => {' fez o teste falhar (exit 101).   (commands_inside_subexpressions, item 3)
[OK] mutação PEGA: ''$' if chars.get(i + 1) == Some(&'(') && self.nest < MAX_NEST => {' -> ''$' if chars.get(i + 1) == Some(&'(') => {' fez o teste falhar (exit -1073741571).   (limite de aninhamento: sem ele dá stack overflow)
[OK] mutação PEGA: '!self.defined.contains(&lower) && seen.insert(lower)' -> 'seen.insert(lower)' fez o teste falhar (exit 101).   (functions_defined_in_scripts_are_not_commands, item 4)
[OK] mutação PEGA: 'Some(f) if f.trim().to_ascii_lowercase()' -> 'Some(f) if f.to_ascii_lowercase()' fez o teste falhar (exit 101).   (installer_file_ps1_case_path_and_trailing_space, item 5)
[OK] mutação PEGA: ''$' if chars.get(i + 1) == Some(&'{') => {' -> ''$' if false && chars.get(i + 1) == Some(&'{') => {' fez o teste falhar (exit 101).   (braced_variables_and_while_are_not_commands)
[OK] mutação PEGA: '    "while",' -> '    "whilex",' fez o teste falhar (exit 101).   (braced_variables_and_while_are_not_commands)

Decisions:
- Item 1: tabela principal pela métrica B (`commands ∩ scoop_fns ⊆ top-N`), N = 5, 10, 20 e todas (39, não 36: com `$(...)` entraram `appdir`, `friendly_path`, `shimdir`, `appsdir`, `persistdir`). Coluna à parte sem os 4 manifestos que chamam `scoop`. A métrica antiga continua como "Piso".
- Item 2: `cmdlet` = alias ou função embutida do `pwsh -NoProfile` 7.6.7, ou Verbo-Substantivo com verbo de `Get-Verb`, ou os 4 cmdlets `Microsoft.PowerShell.*` de verbo não aprovado. O resto é `externo ou definido no script`.
- Item 3: `$(` é analisado também em here-strings `@" "@`, com teto de 32 níveis (`MAX_NEST`) para não estourar a pilha. Analisar `$(...)` em strings mudou o conjunto de comandos de **47 manifestos**. Medi com um dump temporário por manifesto, com e sem o recurso, e o exemplo temporário foi apagado.
- Além do pedido: `while` virou palavra-chave e `${var}` deixou de ser posição de comando. As duas coisas apareciam como "externo" no ranking. Cada uma tem teste.

Resultado novo (métrica B): N=5 cobre 85,4%; N=10 cobre 92,7%; N=20 cobre 96,9%; todas cobrem 99,7% (sobram só os 4 S-ps1). Sem os manifestos que usam `scoop` os números são praticamente iguais. Tabelas completas em docs/superpowers/reports/2026-10-18-oraculo-2c.md.

Concerns: gate (acima). A classificação de cmdlet usa o pwsh 7; o Windows PowerShell 5.1 tem aliases diferentes (ex.: `sc`).
