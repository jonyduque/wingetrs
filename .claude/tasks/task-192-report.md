# Task 192 report

Status: DONE_WITH_CONCERNS

Commits:
```
833f96d fix(scripts): gate catches computed calls, bare paths and launchers
89b35bd fix(scripts): package's own .exe/.com call only informs
6c6eb5a fix(scripts): gate follows called script files and flags aliases
```
(`git log --format='%h %s' 5984441c..HEAD -- <12 arquivos da tarefa>`)

Gate:
```
[OK]   rules 10s
[OK]   rules-selftest 24s
[OK]   fmt 3s
[OK]   clippy 21s
[OK]   test (1842 passed, 87 ignored) 87s
[OK]   gate verde
```
(gate depois de 89b35bd; o de 6c6eb5a também ficou verde, com 1839 testes)

Gate da revisão (833f96d): o gate não fechou por causa de arquivos de outros agentes que estavam em edição (`[FAIL] fmt`: `src/buckets.rs:25`; antes disso `src/tui/app.rs`, `src/winget_install/engine.rs`, `tests/it/install_progress.rs`). Rodei uma vez, conforme a regra, e provei só os meus arquivos:
```
rustfmt --check --edition 2024 src/install/script_scan.rs tests/it/script_scan.rs   -> fmt-ok
cargo clippy --all-targets (filtrado por script_scan)                                -> 0 avisos
script_scan: test result: ok. 74 passed; 0 failed
install_planner: test result: ok. 26 passed; 0 failed
cli_scripts: test result: ok. 27 passed; 0 failed   (1 falha transitória numa execução, sem repetir em 3)
install_ps1: test result: ok. 17 passed; 0 failed
tui_scripts: test result: ok. 19 passed; 0 failed; 1 ignored
```
Na suíte inteira, as únicas falhas são de `elevated_*`/`wi_*` (Tasks 200/201, em andamento).

Mutation proofs:
- Item 1, a regra tirada (brief): `mutate.ps1 -Path src/install/script_scan.rs -AnchorFile t192-a1.txt ('for target in called_files(&l.replace('`', "")) {') -ReplacementFile t192-r1.txt ('... .into_iter().take(0) {') -TestCommand 'cargo test --quiet --test it script_scan::call_quoted_path_is_confirm_strong'`
  `[OK] mutação PEGA: 'for target in called_files(&l.replace('`', "")) {' -> 'for target in called_files(&l.replace('`', "")).into_iter().take(0) {' fez o teste falhar (exit 101).`
- Check 1 (máscara da 164 não enfraquece; arquivo chamado também a desliga): `... -Path src/install/script_scan.rs ... -TestCommand 'cargo test --quiet --test it script_scan::called_file_disables_global_masking'`
  `[OK] mutação PEGA: '.any(|l| invokes_variable(l) || !called_files(l).is_empty())' -> '.any(|l| invokes_variable(l))' fez o teste falhar (exit 101).`
- Check 2 (só a cópia guardada da 149, pelo marcador): `... -TestCommand 'cargo test --quiet --test it script_scan::unresolvable_target_confirm_strong'`
  `[OK] mutação PEGA: 'if !bucket_dir.join(".scripts-ok").is_file() {' -> 'if false {' fez o teste falhar (exit 101).`
- Check 2 (caminho por `safepath`): mesmo teste
  `[OK] mutação PEGA: 'let parts = super::safepath::safe_components(rel)?;' -> 'let parts: Vec<String> = rel.split('\\').map(String::from).collect();' fez o teste falhar (exit 101).`
- Item 3 (apelidos): `... -TestCommand 'cargo test --quiet --test it script_scan::alias_redefinition_confirm_strong'`
  `[OK] mutação PEGA: 'if any(&["set-alias", "new-alias", "sal", "nal"]) || l.contains("alias:") {' -> 'if false {' fez o teste falhar (exit 101).`
- Item 2, pior veredito vence: `... -TestCommand 'cargo test --quiet --test it script_scan::dot_source_bucket_script_scanned'`
  `[OK] mutação PEGA: 'report.flags.push(flag);' -> 'let _ = flag;' fez o teste falhar (exit 101).`
- Caminho planejador (recusa no plano): `-Path src/install/planner.rs ... -TestCommand 'cargo test --quiet --test it install_planner::bucket_script_called_by_any_hook_is_refused'`
  `[OK] mutação PEGA: 'let (report, _) = script_scan::scan_called(script, PRELUDE_FUNCTIONS, roots);' -> 'let report = script_scan::scan(script, PRELUDE_FUNCTIONS);' fez o teste falhar (exit 101).`
- Caminho executor (hook com `$dir` já extraído): `-Path src/install/executor.rs ... -TestCommand 'cargo test --quiet --test it install_ps1::hook_called_dir_file_is_scanned_before_running'`
  `[OK] mutação PEGA: 'refuse_called(kind, s, &roots)?;' -> 'let _ = (kind, &roots);' fez o teste falhar (exit 101).`
- Caminho confirmação (texto mostrado, CLI e TUI): `-Path src/cli/output.rs ... -TestCommand 'cargo test --quiet --test it cli_scripts::consent_shows_called_bucket_script'`
  `[OK] mutação PEGA: 'std::iter::once(first).chain(files).collect()' -> 'std::iter::once(first).chain(files.take(0)).collect()' fez o teste falhar (exit 101).`
- Divisão por tipo de alvo (pedido do orquestrador, 89b35bd):
  `mutate.ps1 -Path src/install/script_scan.rs -Anchor 'matches!(ext.as_str(), "exe" | "com").then_some(name)' -Replacement '... .then_some(name).filter(|_| false)' -TestCommand 'cargo test --quiet --test it script_scan::package_program_call_is_confirm'`
  `[OK] mutação PEGA: 'matches!(ext.as_str(), "exe" | "com").then_some(name)' -> 'matches!(ext.as_str(), "exe" | "com").then_some(name).filter(|_| false)' fez o teste falhar (exit 101).`
  Mesma mutação com `script_scan::near_misses_not_flagged`: `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
  `... -AnchorFile ('matches!(ext.as_str(), "exe" | "com").then_some(name)') -ReplacementFile ('Some(name)') -TestCommand 'cargo test --quiet --test it script_scan::script_or_interpreter_call_stays_confirm_strong'`
  `[OK] mutação PEGA: 'matches!(ext.as_str(), "exe" | "com").then_some(name)' -> 'Some(name)' fez o teste falhar (exit 101).`
  `... -AnchorFile (lista de prefixos) -ReplacementFile (a lista com "$bucketsdir\\") -TestCommand 'cargo test --quiet --test it script_scan::program_outside_package_stays_confirm_strong'`
  `[OK] mutação PEGA: 'let rel = ["$dir\\", "${dir}\\", "$persist_dir\\", "${persist_dir}\\"]' -> 'let rel = ["$dir\\", "${dir}\\", "$bucketsdir\\", "$persist_dir\\", "${persist_dir}\\"]' fez o teste falhar (exit 101).`
- Revisão (833f96d), todas com `-Path src/install/script_scan.rs` e `-TestCommand 'cargo test --quiet --test it script_scan::<teste>'`:
  - junção para fora da base (`junction_out_of_base_stays_unresolved`): `[OK] mutação PEGA: '(real.starts_with(&real_base) && real.is_file()).then_some(path)' -> 'real.is_file().then_some(path)' fez o teste falhar (exit 101).`
  - `&(`/`.(` (`call_quoted_path_is_confirm_strong`): `[OK] mutação PEGA: 'out.push(Call::Computed);' -> 'let _ = 0;' fez o teste falhar (exit 101).`
  - caminho em posição de comando (`path_in_command_position_is_a_call`): `[OK] mutação PEGA: '&& let Some(word) = statement_path(&chars, i)' -> '&& let Some(word) = statement_path(&chars, i).filter(|_| false)' fez o teste falhar (exit 101).`
  - texto dentro de string não é comando (mesmo teste): `[OK] mutação PEGA: '            && !in_string[i]' -> '            && (!in_string[i] || i < usize::MAX)' fez o teste falhar (exit 101).`
  - lançadores (`file_run_by_another_program_is_confirm_strong`): `[OK] mutação PEGA: 'if let Some((launcher, file)) = launched_file(l) {' -> 'if let Some((launcher, file)) = launched_file(l).filter(|_| false) {' fez o teste falhar (exit 101).`
  - mshta/rundll32/regsvr32 sempre fortes (mesmo teste): `[OK] mutação PEGA: '            if always {` -> `            if false {` fez o teste falhar (exit 101).`
  - aspas simples não expandem (`package_program_needs_double_quotes_and_a_file_name`): `[OK] mutação PEGA: 'expands: first == '"',' -> 'expands: true,' fez o teste falhar (exit 101).`
  - `\` no fim (mesmo teste): `[OK] mutação PEGA: 'if t.ends_with('\\') {' -> 'if false {' fez o teste falhar (exit 101).`
- Check 3 (não tocar `src/winget_install`/`src/bridge`): sem mutação; `git show --stat 6c6eb5a` não lista nenhum deles.

Oracle (`cargo run --release --example oracle_2c -- %LOCALAPPDATA%\wingetrs\catalog.db`, 1403 manifestos 2c, 1293 S-hook):
- Depois de 833f96d (revisão): Confirm 1041 / ConfirmStrong 201 / Refuse 43 (todas as funções); 643 / 103 / 539 com nenhuma. `CalledFile` em 66 manifestos. Planejamento 1352/1403. Uma versão intermediária deu 44 Refuse e 1351/1403: `main/ghostscript` tem `"$dir\bin;$dir\lib;$dir\fonts"`, e o `;` dentro da string virava posição de comando, o que desligava a máscara do `if ($global)`. Corrigi ignorando texto entre aspas (contado desde o início da linha) só na regra de posição de comando; o caso virou teste.
- 62 candidatos da revisão (`t192-cands.txt`) rodados de novo com `scan(.., PRELUDE_FUNCTIONS)`: 52 ConfirmStrong e 10 Confirm. Os 10 Confirm são todos programas do pacote entre aspas duplas (`& "$dir\x.exe"`, `x.EXE`, `$dir/x.exe`, `${dir}\x.exe`, `$dir\.\x.exe`, `x .exe`, `x.com`, `x.ps1.exe`, `@args`, `$persist_dir\x.exe`). A saída está em `t192-res3.txt` no scratchpad.
- Depois de 89b35bd (divisão por tipo de alvo): Confirm 1059 / ConfirmStrong 183 / Refuse 43 (todas as funções); 654 / 92 / 539 com nenhuma. Planejamento continua 1352/1403. Dos alvos de `CalledFile` (sonda temporária, apagada): 9 chamadas `.exe` do pacote ficam `Confirm` (pnpm, rye, wix, gsudo, git-istage, alpinewsl, archwsl); continuam fortes 23 `.ps1` (16 manifestos JetBrains chamam `$bucketsdir\extras\scripts\jetbrains\portable.ps1`, que é analisado quando o bucket guardado existe), 8 `.bat` (`.\x.bat`) e 3 sem extensão (`$dir\cheat`, `$env:COMSPEC`).
- Em 6c6eb5a (antes da divisão), portão estático, todas as funções suportadas: antes Confirm 1085 / ConfirmStrong 157 / Refuse 43; depois 1052 / 190 / 43. `CalledFile` aparece em 37 manifestos (33 sobem de Confirm para ConfirmStrong; 4 já eram). `Alias` em 0. Nenhum Refuse novo.
- Planejamento: 1352/1403 antes e depois (43 recusados pelo portão, 8 outros erros), igual. Rodei também com o `Layout` do oráculo apontando para a raiz real (`%LOCALAPPDATA%\wingetrs`, buckets main/extras com `.scripts-ok`) por um patch temporário desfeito antes do commit: mesmo 1352/1403, então nenhum script de bucket alcançável hoje acrescenta recusa.

Decisions:
- Revisão: `&`/`.` seguidos de `(` ⇒ `ConfirmStrong` "roda um arquivo ou comando calculado na hora". Caminho em posição de comando (`.\`, `..\`, `./`, `X:\`, `$dir\`, `$persist_dir\`, também `${..}`, depois de início de linha, `;`, `|`, `{`, `(` ou `&&`) entra em `called_files` com as mesmas classes. Lançadores (`Start-Process`/`saps`/`start`, `powershell`/`pwsh`, `cmd`, `Invoke-Item`/`ii`, `Import-Module`/`ipmo`, `wscript`, `cscript`, `[..Diagnostics.Process]::Start(`) ⇒ `CalledFile` `ConfirmStrong` "roda um arquivo que não é mostrado por outro programa (<lançador>): <arquivo>" quando algum argumento é arquivo (tem `\`, `/` ou extensão; chaves `-x`, `/x` e variável sozinha ficam de fora), exceto `"$dir\x.exe"` entre aspas duplas. `mshta`/`rundll32`/`regsvr32` são sempre fortes, e `Process::Start` sem argumento de arquivo também. Alvo entre aspas simples nunca é resolvido nem conta como programa do pacote.
- Alvo resolvido e lido: a flag `CalledFile` daquela linha cai para `Confirm` ("roda o arquivo X, analisado e mostrado abaixo") e as flags do arquivo entram no relatório do hook na linha da chamada, com prefixo "<alvo>, linha N:". Recursão até 4 níveis, ciclo fica `ConfirmStrong`.
- O arquivo chamado entra na confirmação como mais um `ThirdParty::Script` com hook "arquivo chamado por <hook>: <alvo>" (CLI e TUI renderizam sem código novo). `$PSScriptRoot` resolve para a pasta do arquivo analisado; `$dir` só quando a pasta já existe (`CallRoots::new` testa `is_dir`): na remoção e nos hooks de desinstalação da versão antiga sim, na instalação nova não.
- O executor reanalisa (com `$dir` já extraído) cada hook antes de rodar e o `.ps1` de `installer.file` com seus dot-sources; recusa ⇒ falha antes de rodar. A decodificação de `.ps1` foi movida do executor para `script_scan::decode_script` (um só caminho).
- `& "<arquivo>"`/`. "<arquivo>"` também desligam a máscara de `if ($global)` (o arquivo chamado pode redefinir `ForEach-Object`); só acrescenta, não mudou o oráculo.
- Arquivos fora da lista Files, por necessidade da recusa: `src/install/planner.rs`, `src/install/executor.rs`, `src/install/ops.rs`, `src/cli/install_cmd.rs`, `src/tui/jobs.rs` (assinaturas `check_uninstall_scripts(m, roots)` e `removal_third_party(m, vdir, suffix, roots)`), testes em `install_planner.rs`, `install_ps1.rs`.

Concerns:
- Resolvido em 89b35bd, por decisão do orquestrador: `.exe`/`.com` dentro de `$dir`/`$persist_dir` (componentes por `safepath`, sem outra variável) ⇒ `CalledFile` `Confirm` "roda um programa do pacote: <nome>" e não desliga a máscara da 164; scripts e interpretadores (`.ps1`, `.psm1`, `.cmd`, `.bat`, `.vbs`, `.js`, `.wsf`, `.hta`, sem extensão, qualquer outra) e alvos fora ⇒ `ConfirmStrong`. O caso `& "$dir\x.exe" /S` voltou para `near_misses_not_flagged`; o filtro do teste agora ignora só a flag `CalledFile` com veredito `Confirm` (a asserção mudou, porque o caso passou a ter uma flag informativa).
- `& $dir\x.exe` (sem aspas) continua `DynamicCode` `ConfirmStrong` (regra de `& $var` da Task 150), não é tratado como programa do pacote.
- Joins com dado externo que escrevi: `bucketsdir.join(bucket)` (componente único por `safepath`), `base.join(c)` para cada componente de `safe_components(rel)` mais a checagem `canonicalize(real).starts_with(canonicalize(base))`; `layout.version_dir(&plan.app, &plan.version)` em `ops.rs` usa o registro do estado, igual ao que o CLI já fazia.
- Entre a análise e a execução o arquivo chamado pode mudar (TOCTOU), porque só o `.ps1` do instalador é copiado; os arquivos ficam em pastas do wingetrs.
- Pegadinha do ambiente (para o harness, não alterei): no Git Bash desta sessão, um heredoc `<<'EOF'` transformou `\\` em `\`, o que quebrou scripts Python e literais Rust; escrever o arquivo com Write e rodá-lo resolveu.
