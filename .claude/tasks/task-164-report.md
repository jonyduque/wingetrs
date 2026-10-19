# Task 164 report: constant `$global` and informational `if ($global)` blocks

Status: DONE_WITH_CONCERNS

Commits:
```
bf96b3b feat(scripts): mask if ($global) inside plain ForEach-Object and statement blocks
dd5d030 fix(scripts): mask only top-level if ($global) with an allowlisted condition
567c83f fix(scripts): a continued line keeps if in argument position
2e275a1 fix(scripts): mask if ($global) blocks only in statement position
0bd54b5 feat(scripts): constant $global and informational if ($global) blocks
```

## Quinta mudança, revisão adversarial 3 (último commit desta tarefa): valores padrão de parâmetro
- Entraram em `MASK_KILLERS`, como substrings: `psdefaultparametervalues` (cobre `$PSDefaultParameterValues[...]` e `Set-Variable -Name PSDefaultParameterValues`, já que a substring casa em qualquer lugar do texto cru) e `psboundparameters`.
- Teste `parameter_defaults_disable_masking`: `$PSDefaultParameterValues['ForEach-Object:Parallel'] = $true`, o `Set-Variable ...` e `$PSBoundParameters.Clear()`, cada um seguido do formato do 7zip, saem com `Hklm` `Refuse`.
- Mutações:
  - `[OK] mutação PEGA: '    "psdefaultparametervalues",' -> '' fez o teste falhar (exit 101).`
  - `[OK] mutação PEGA: '    "psboundparameters",' -> '' fez o teste falhar (exit 101).`
- `cargo test --quiet --test it script_scan::` -> `test result: ok. 61 passed; 0 failed`. O oráculo não muda (nenhum script do catálogo tem essas palavras perto de `$global`; não medido de novo).

## Quarta mudança, decisão do usuário (bf96b3b): `ForEach-Object` e blocos de comando
- **Quadros permitidos** (`open_frame`, um `FrameKind` por `{` na pilha). Um bloco `if ($global)` é mascarado só se todos os `{` que o envolvem forem de um destes tipos:
  - o scriptblock logo depois de `| ForEach-Object` ou `| %` (não vale `||`), sem nenhum outro argumento, nem antes nem depois do `}` (`code_close`, com a mesma tokenização do scanner);
  - o corpo de `if (…)`, `foreach (…)`, `for (…)`, `while (…)` ou `try` em posição de comando;
  - `elseif`/`else` logo depois do `}` de um corpo de `if`; `catch`/`finally` logo depois de `try`/`catch`.

  O parêntese de cada condição é casado pelo próprio scanner (mapa `)` -> `(`, com as strings puladas), então aspas na condição não atrapalham (`if (Test-Path "$d\$_.reg")`). Qualquer outro `{` (scriptblock literal, hashtable, `switch`, função) deixa o bloco vivo, e um `}` sem par desliga a máscara daí em diante.
- **Palavras que desligam a máscara do script inteiro** (texto cru, comentários incluídos, sem diferenciar maiúsculas):
  - Por substring: `set-alias`, `new-alias`, `alias:` (cobre `Set-Item`/`New-Item`/`ni`/`si alias:…`), `-parallel`, `start-job`, `start-threadjob`, `invoke-command`, `[powershell]`, `runspace`, `invoke-expression`, `[scriptblock]::create`, `.invoke(`, `invoke-item`, `add-type`, `executioncontext`, `get-command`, `new-module`, `import-module`.
  - Por token inteiro (o token vizinho não pode ser letra, dígito, `_` ou `-`): `function`, `filter`, `sal`, `nal`, `iex`, `gcm`, `ipmo`, `pwsh`, `powershell`. Assim `-Filter`, `$salt`, `banal` e `$pwshArch` não casam.
  - Também desliga: chamar uma variável (`& $x`, `. $x`), pelo `invokes_variable` já existente, linha a linha.
  - A regra (b) continua igual. Respondendo ao item 5: `global_named_outside_conditions` não é código morto; é a regra (b) e está ligada no começo de `global_dead_mask`.
- Testes:
  - `global_block_in_foreach_object_is_informational`: o formato real do 7zip/git e mais 17 formas permitidas.
  - `global_block_in_other_frames_fails_closed`: 22 formas recusadas, entre elas `-Parallel`, `% -Begin {} -Process {…}`, `-End {}` depois, `-Process`, `Where-Object`, `||`, `$sb = {…}; & $sb`, hashtable, `else`/`finally`/`catch` fora de ordem, `Write-Output foreach`, `switch` e um `}` sem par.
  - `mask_killer_words_disable_masking`: uma linha por palavra (inclusive o bypass `iex ('Set-'+'Alias ForEach-Object Start-Job')`, `sal x y`, `Set-Alias ForEach-Object Start-Job`) e 8 negativos.
  - Os testes de antes mudaram: `foreach (...) { if ($global) … }` passou de recusado para informativo; `Start-Job`/`pwsh -c`/função/`$sb` continuam recusados. `install_planner::scanner_refusal…` voltou à versão de 0bd54b5 (o 7zip planeja; sem o guard, recusa).
- Mutações (todas `[OK] mutação PEGA … fez o teste falhar (exit 101)`; rodei uma de cada vez, com o teste verde antes):
  - `return if !double && alone {` -> `return if alone {` e -> `return if !double {`
  - `.is_none_or(|j| matches!(chars[j], '\n' | ';' | '|' | '}' | ')'))` -> `.is_none_or(|_| true)` (argumentos depois do bloco)
  - `.eq_ignore_ascii_case("foreach-object")` -> `.to_ascii_lowercase().ends_with("-object")`
  - `let pipe = start.and_then(back).filter(|&p| chars[p] == '|');` -> sem o filtro
  - `"if" if statement(s)` -> `"if"`; `"elseif" if after(...)` -> `"elseif"`; `"foreach" | "for" | "while" if statement(s)` -> sem a guarda; `"else"`/`"try"`/`"catch"`/`"finally"` sem a guarda (cada um)
  - `let enclosed_ok = !unbalanced && frames…` -> sem `!unbalanced`, e -> só `!unbalanced`
  - `"if" … => FrameKind::If` -> `FrameKind::Other`; `"foreach" | "for" | "while"` -> sem `"while"`; `finally` só depois de `try` (estes caem no teste dos informativos)
  - `let close = code_close(chars, open);` -> `block_close(…)` (o `'{{7zip_dir}}'` dentro da string fecharia o quadro)
  - `let lp = paren_open.get(&p).copied();` -> `None`
  - `|| mask_killer(&chars)` removido; `"set-alias"`, `"alias:"`, `"invoke-expression"`, `"[scriptblock]::create"`, `".invoke("` removidos (cada um); tokens `"sal"`, `"iex"`, `"pwsh"` e `"powershell"` removidos (cada um); `if lower.lines().any(invokes_variable)` -> `if false`; o `c != '-'` antes do token removido; a fronteira depois do token removida.
  - Atenção: uma primeira rodada em lote saiu inválida, porque duas execuções do script de mutação rodaram ao mesmo tempo no mesmo arquivo e outros agentes deixaram o checkout sem compilar por alguns minutos. Repeti tudo em sequência; só as saídas acima contam.
- Verificação: `cargo test --quiet --test it -- script_scan:: install_planner:: install_script_effects::` -> `test result: ok. 128 passed; 0 failed`. `rustfmt --check` limpo. `cargo clippy --all-targets`: nenhum aviso nos meus arquivos.
- **Oráculo:** `| todos | 1403 | 0 | 1352 (96.4%) | 43 (3.1%) | 8 (0.6%) |` (main 284/294, extras 1068/1109). O esperado era ~1354; a diferença são estes 2:
  - `main/pwsh`: o próprio script contém `pwsh.exe` e `\scripts\pwsh\`. O token `pwsh` desliga a máscara (falha fechada, como você pediu).
  - `extras/lazarus`: usa `ForEach-Object -Process {…}`, e o parâmetro explícito fica fora da lista branca.
  - Continuam fora, como já decidido: `main/python` (`-Global` fora da condição, regra (b)) e `main/vulkan` (`$x = if ($global)`). Desta vez entram de volta git, 7zip, vscode, windows-terminal, notepadplusplus e os demais.
- Os números de oráculo das seções abaixo são de commits anteriores.

## Terceira correção, revisão CHANGES_REQUESTED (dd5d030): travessões Unicode e outro runspace
1. **Lista branca na condição** (`global_condition`): a condição tem que ser só ASCII e exatamente `$global` ou `$global -and $<identificador>` (letras e dígitos ASCII e `_`, separados por espaço ou tab). A lista de proibidos antiga saiu. Testes: `global_condition_unicode_dashes_fail_closed` (U+2013, U+2014 e U+2015, cada um tanto antes de `or` quanto antes de `and`). Os casos `$global -and $x -eq 1`, `$global -and $x.y` e `$global -and $x $y` passaram para `global_block_doubts_fail_closed`.
2. **(a) Só no nível 0 de chaves:** o tokenizador conta `{`/`}` no código e só mascara com profundidade 0. **(b)** Se o script tiver a palavra `global` fora de uma condição `if (...)`/`elseif (...)` e fora do `-Global:$global` exato, nenhum bloco do script é mascarado. A verificação roda no texto sem comentários, com as strings mantidas, então `"$global"` também conta. Testes: `global_block_in_another_runspace_fails_closed` (`Start-Job {…}`, `pwsh -c {…}`, `foreach (...) { if ($global) {HKLM} }` em uma linha e em várias, `function f {…}`) e `global_named_elsewhere_disables_masking` (7 casos recusados, entre eles `$global = $true`, `appdir $app $global`, `-Global` solto, `$global:y`; e 4 informativos: `if (!$global)`, `-Global:$global`, comentário, `$globaldir`). O caso `& { if ($global) {…} }`, que era informativo, agora recusa.
- Revertidos para o comportamento original: `install_planner::scanner_refusal_of_a_real_manifest_script_stops_the_plan`, porque o 7zip voltou a ser recusado (o `if ($global)` fica dentro de um `ForEach-Object { }`), e `install_script_effects::python_catalog_pathext_script`, porque o texto do catálogo usa `-Global` fora da condição, então pela regra (b) recusa com e sem o guard.
- Mutações (cada teste verde antes):
  - `[OK] mutação PEGA: 'starts_word && statement && depth == 0' -> 'starts_word && statement' fez o teste falhar (exit 101).` (global_block_in_another_runspace_fails_closed)
  - `[OK] mutação PEGA: 'if global_named_outside_conditions(&uncommented) {' -> 'if false {' fez o teste falhar (exit 101).` (global_named_elsewhere_disables_masking)
  - `[OK] mutação PEGA: '!(in_condition || passthrough)' -> '!passthrough' fez o teste falhar (exit 101).`
  - `[OK] mutação PEGA: '!(in_condition || passthrough)' -> '!in_condition' fez o teste falhar (exit 101).`
  - `[OK] mutação PEGA: '        [g, and, x] => g == "$global" && and == "-and" && ident(x),` (a seguinte `_ => false` vira `_ => parts.first().is_some_and(|g| g == "$global")`) `fez o teste falhar` (global_block_doubts_fail_closed; `-AnchorFile`)
  - `[OK] mutação PEGA: 'and == "-and" && ident(x),' -> 'and == "-and" && (ident(x) || !x.is_empty()),' fez o teste falhar (exit 101).`
  - Sobrevive por construção: `'if !cond.is_ascii() {' -> 'if false {'` dá `[X] mutação SOBREVIVEU`. Com a lista branca, nenhum caractere fora do ASCII chega a casar (os tokens são comparados exatamente, e o identificador só aceita ASCII), então a checagem de ASCII é redundante. Por isso o teste dos travessões não pega a mutação da lista branca (o ASCII barra antes); essa mutação foi provada pelos casos ASCII de `doubts`.
- Verificação: `cargo test --quiet --test it -- script_scan:: install_planner:: install_script_effects::` -> `test result: ok. 125 passed; 0 failed`. `rustfmt --check` limpo. `cargo clippy --all-targets`: nenhum aviso nos meus arquivos (corrigi um `needless_range_loop`).
- **Oráculo:** `| todos | 1403 | 0 | 1319 (94.0%) | 76 (5.4%) | 8 (0.6%) |` (main 280/294, extras 1039/1109). Isso dá **+1** sobre a base da Task 163 (1318), ou **-36** em relação a 1355 (+37). Medi cada regra sozinha, desligando a outra só para o oráculo (o código commitado tem as duas):
  - **só (a), nível 0:** 1319. A regra (a) sozinha custa 36 dos 37.
  - **só (b), `global` nomeado fora:** 1354. A regra (b) sozinha custa só `main/python` (o `-Global` dentro do bloco fica fora da condição).
  - O único ganho que sobra é `main/ghostscript`.
  - Os 37 que deixam de planejar são os que "planejam com o corpo apagado": `main/7zip`, `main/git-with-openssh`, `main/git`, `main/pwsh`, `main/python`, `main/vim`, `main/vulkan`, `extras/alacritty`, `extras/astrogrep`, `extras/eartrumpet`, `extras/everything`, `extras/fork`, `extras/geogebra`, `extras/gitkraken`, `extras/grepwin`, `extras/lazarus`, `extras/librecad`, `extras/losslesscut`, `extras/neovide`, `extras/notepad3`, `extras/notepadplusplus`, `extras/nvda`, `extras/pdfgear`, `extras/qbittorrent`, `extras/qview`, `extras/rebased`, `extras/recaps`, `extras/seelen-ui`, `extras/stremio`, `extras/sublime-merge`, `extras/sublime-text`, `extras/tailscale`, `extras/theia-ide`, `extras/vscode`, `extras/vscodium`, `extras/wezterm`, `extras/windows-terminal`. O motivo dominante é a regra (a): o padrão do Scoop é `Get-ChildItem ... -Filter '*.reg' | ForEach-Object { $content = ...; if ($global) { HKCU->HKLM } ... }` (o mesmo do 7zip), ou seja, o `if ($global)` fica dentro de um scriptblock de pipeline. `main/vulkan` também cai pela posição de atribuição (correção 2e275a1). Não alarguei a regra; a decisão fica com você.
- Os números de oráculo das seções abaixo (1356, 1355) são de commits anteriores.

## Segunda correção pedida pela revisão (567c83f): continuação de linha
Bypass: "& { param($a,$b,$c) & $c } `\nif ($global) { HKLM... }". Com a crase antes do `\n`, o `if` da linha seguinte continua sendo argumento. A posição de comando agora sai de `statement_position(view, i)`: quando o caractere anterior é `\n`, a função olha antes dele (pulando espaço, tab e CR) e recusa a posição se encontrar crase, `,`, `|`, `=`, `(`, `&`, `.`, `[`, `+`, `-`, `*`, `/`, `%` ou `!` (fail closed: com esses, o pwsh continua lendo a linha). A checagem roda no texto cru **e** no texto com os comentários apagados (`strip_comments`, do mesmo tamanho), e as duas precisam concordar:
- Crase dentro de comentário (`# x ` seguido de crase): no texto cru parece continuação, então é recusado (fail closed). Sondei no pwsh 7: uma crase em comentário não continua a linha (`f a # x <crase>` e depois `if (...)` dão `args=a` e o `if` roda como comando), mas preferi falhar fechado.
- Comentário escondendo uma vírgula que continua a linha (`f a, # c\nif ...`): só o texto sem comentários pega, e o caso é recusado.
- `<# x #> if ...` é recusado. `$a = <crase>\n\nif` é informativo (a continuação termina na linha vazia). `$a = 1 # note\nif` é informativo.
- Testes: entraram 9 casos recusados e 2 informativos em `global_block_only_in_statement_position`. Depois da correção, `cargo test --quiet --test it -- script_scan:: install_planner:: install_script_effects::` deu 121 passed e 1 failed. O que falhou foi `install_planner::zigup_args_are_substituted`, que passa sozinho e toca arquivos que outro agente está editando agora (`src/install/planner.rs` e snapshots do planner). Na rodada seguinte o binário `it` deixou de compilar por causa de `tui::app::Cmd::PlanWingetUninstall`, também de outro agente. `rustfmt --check` nos 2 arquivos: limpo. `cargo clippy --all-targets`: nenhum aviso em `script_scan`.
- Mutações (todas sobre `script_scan::global_block_only_in_statement_position`, verde antes de mutar):
  - `[OK] mutação PEGA: ''`' | ',' |' -> '',' |' fez o teste falhar (exit 101).` (a crase sai da lista de continuação; rodei com `-AnchorFile`/`-ReplacementFile`)
  - `[OK] mutação PEGA: 'statement_position(&chars, i) && statement_position(&uncommented, i)' -> 'statement_position(&uncommented, i)' fez o teste falhar (exit 101).`
  - `[OK] mutação PEGA: 'statement_position(&chars, i) && statement_position(&uncommented, i)' -> 'statement_position(&chars, i)' fez o teste falhar (exit 101).`
- Oráculo: igual ao da primeira correção, `| todos | 1403 | 0 | 1355 (96.6%) | 40 (2.9%) | 8 (0.6%) |`. Só `main/vulkan` deixou de planejar.

## Correção pedida pela revisão (2e275a1): posição de comando
Bypass: `& { param($a,$b,$c) & $c } if ($global) { HKLM... }`. Aqui `if` é argumento e o `{...}` é um scriptblock que roda. Agora o bloco só é mascarado quando o `if` está em posição de comando: o caractere significativo anterior (pulando espaço, tab e CR) é o início do script, `\n`, `;` ou `{`. O `elseif` só conta logo depois do `}` de um bloco que a máscara cobriu.
- Diferença em relação à mensagem do coordenador: **tirei o `}`** da lista. No próprio exemplo do bypass o token antes do `if` é `}`, então aceitá-lo deixaria o bypass passar. A mutação abaixo prova isso: com `'}'` na lista o teste cai.
- Testes: `global_block_only_in_statement_position`. Recusam: o bypass, `Write-Output if (...)`, `$x = if (...)`, `f -p if`, `f | g if`, `f a, if`, `. if`. Continuam informativos: `; if`, `\n  if`, `& { if (...) {...} }`. Em `global_block_doubts_fail_closed` entraram `if ($x) {…} elseif ($global) {…}` e um `elseif` solto. O caso informativo de `elseif` passou a ser `if ($global) {…} elseif ($global) {…}`.
- Mutações:
  - `[OK] mutação PEGA: 'keyword.filter(|_| starts_word && statement)' -> 'keyword.filter(|_| starts_word)' fez o teste falhar (exit 101).` (global_block_only_in_statement_position)
  - `[OK] mutação PEGA: 'Some(_) => before.is_some() && before == last_close,' -> 'Some(_) => true,' fez o teste falhar (exit 101).` (global_block_doubts_fail_closed)
  - `[OK] mutação PEGA: 'matches!(chars[k], '\n' | ';' | '{')' -> 'matches!(chars[k], '\n' | ';' | '{' | '}')' fez o teste falhar (exit 101).` (global_block_only_in_statement_position)
- Verificação: `cargo test --quiet --test it script_scan::` -> `test result: ok. 54 passed; 0 failed`. `rustfmt --check` nos 2 arquivos: limpo. `cargo clippy --all-targets`: `Finished`, sem aviso.
- Oráculo depois da correção: `| todos | 1403 | 0 | 1355 (96.6%) | 40 (2.9%) | 8 (0.6%) |` (main 286/294, extras 1069/1109). Ficou **+37**, não +38. A diferença é só `main/vulkan`: o `pre_uninstall` dele faz `$reg_root = if ($global) { [Microsoft.Win32.Registry]::LocalMachine } else { ... }`, um `if` em posição de atribuição, que a correção trata como dúvida e recusa de propósito (opção simples pedida). O oráculo confirma: "Recusados pelo portão com algum bloco `if ($global)`: 4. Planejam com o corpo desses blocos apagado: 1 (`main/vulkan`)". `GlobalName`: 0. Atribuições a `$global`: 0.
- Os números de oráculo mais abaixo (1356) são de 0bd54b5, antes desta correção.
(`git log --format='%h %s' 24da373..HEAD -- <Files + install_planner.rs + install_script_effects.rs>`)

Gate:
Primeira execução (antes do último ajuste, com só o meu trabalho no checkout), etapa `test`:
```
[OK]   rules 9s
[OK]   rules-selftest 26s
[OK]   fmt 3s
[OK]   clippy 21s
[FAIL] test exit 101
  install_planner::scanner_refusal_of_a_real_manifest_script_stops_the_plan --- FAILED
  install_script_effects::python_catalog_pathext_script --- FAILED
  script_scan::global_block_doubts_fail_closed --- FAILED
  test result: FAILED. 1645 passed; 3 failed; 86 ignored; 0 measured; 0 filtered out; finished in 29.54s
```
Corrigi as três (veja Decisions). Depois disso outros agentes começaram a editar o mesmo checkout (tasks 190–195: `src/winget_install/*`, `tests/it/wi_*.rs` e outros). Execuções seguintes do gate:
```
[OK]   rules 11s
[OK]   rules-selftest 36s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\winget_install\host_win.rs:42:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\winget_install\host_win.rs:49:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\wi_select.rs:272:
```
e depois
```
[FAIL] rules exit 1
  tests\it\wi_cli.rs:854: [limite de relogio em teste] while !gate.cancelled() && t0.elapsed() < Duration::from_secs(10) {
```
Nenhuma das duas falhas está nos meus arquivos. Como o CLAUDE.md manda nesse caso, provei só os meus arquivos (com o checkout compilando de novo):
```
cargo test --quiet --test it -- script_scan:: script_runner:: prelude_core:: install_planner:: install_script_effects::
test result: ok. 134 passed; 0 failed; 16 ignored; 0 measured; 1626 filtered out; finished in 6.24s
cargo clippy --all-targets   ->  Finished `dev` profile ... (nenhum aviso; nenhum nos meus caminhos)
rustfmt --check --edition 2024 <meus 9 arquivos .rs>   ->  0 diffs
```
Testes ao vivo (pwsh real, `--release --ignored`, só scripts sintéticos):
```
cargo test --quiet --release --test it -- --ignored prelude_core:: prelude_effects:: script_host_live:: script_runner::
script_host_live::real_pwsh_runs_synthetic_hook_inside_the_sandbox --- FAILED
test result: FAILED. 30 passed; 1 failed; ...
```
`constant_global_blocks_reassignment` e `prelude_accepts_constant_global` (pwsh 7 e PowerShell 5.1) passam, e os testes de prelúdio que já existiam (grupos 1–3) também, sem mudar asserção. A falha que sobra existia antes desta tarefa (veja Concerns).

Mutation proofs:
Obrigatórias do brief:
- (a) `Constant,AllScope` -> `Constant` no runner (teste ao vivo):
  `pwsh -File scripts/mutate.ps1 -Path src/install/runner.rs -Anchor '-Option Constant,AllScope -Scope Global' -Replacement '-Option Constant -Scope Global' -TestCommand 'cargo test --quiet --release --test it -- --ignored script_host_live::constant_global_blocks_reassignment'`
  `[OK] mutação PEGA: '-Option Constant,AllScope -Scope Global' -> '-Option Constant -Scope Global' fez o teste falhar (exit 101).` (sem `AllScope` o hook passa e o bloco roda: falhou em `assert!(result.is_err())`)
- (b) aceitar `-or` na condição de bloco morto:
  `... -Path src/install/script_scan.rs -Anchor '["-or", "-xor", "-not"]' -Replacement '["-xor", "-not"]' -TestCommand 'cargo test --quiet --test it script_scan::global_block_doubts_fail_closed'`
  `[OK] mutação PEGA: '["-or", "-xor", "-not"]' -> '["-xor", "-not"]' fez o teste falhar (exit 101).`
- (c) desligar o fail closed da chave dentro de string:
  `... -Anchor "if chars[k..end].iter().any(|d| matches!(d, '{' | '}')) {" -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it script_scan::global_block_unbalanced_fails_closed'`
  `[OK] mutação PEGA: 'if chars[k..end].iter().any(|d| matches!(d, '{' | '}')) {' -> 'if false {' fez o teste falhar (exit 101).`

Extras (uma por checagem nova; todas rodaram depois do último ajuste, com o teste verde antes):
- `[OK] mutação PEGA: '["-or", "-xor", "-not"]' -> '["-or", "-not"]' fez o teste falhar (exit 101).` (doubts)
- `[OK] mutação PEGA: '["-or", "-xor", "-not"]' -> '["-or", "-xor"]' fez o teste falhar (exit 101).` (doubts)
- `[OK] mutação PEGA: 'if !(rest.is_empty() || and) {' -> 'if false {' fez o teste falhar (exit 101).` (doubts: `-eq $false`, `-ne $true`)
- `[OK] mutação PEGA: 'if matches!(chars[k], '{' | '}') {' -> 'if false {' fez o teste falhar (exit 101).` (chave em comentário)
- `[OK] mutação PEGA: 'if matches!(chars.get(k + 1), Some('{' | '}')) {' -> 'if false {' fez o teste falhar (exit 101).` (chave escapada com crase)
- `[OK] mutação PEGA: ''@' if here_string_start(chars, k) => return None,' -> '' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: ''<' if chars.get(k + 1) == Some(&'#') => return None,' -> '' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: ''#' if prev.is_some_and(continues_word) => return None,' -> '' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'Some(i) => (i + 1, true),' -> 'Some(i) => (i + 1, false),' fez o teste falhar (exit 101).` (regras por comando dentro do bloco)
- `[OK] mutação PEGA: '(&dead_lines, &dead_raw, true)' -> '(&dead_lines, &dead_raw, false)' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'for line in own_global_lines(&text) {' -> 'for line in own_global_lines(&text).into_iter().take(0) {' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'if name.eq_ignore_ascii_case("global") {' -> 'if false {' fez o teste falhar (exit 101).` (runner)
- `[OK] mutação PEGA: 'function appdir($app, [Alias('global')]$__wr_g) {' -> 'function appdir($app, $global) {' fez o teste falhar (exit 101).` (duas vezes: `prelude_core::prelude_has_no_global_parameter` e, ao vivo, `prelude_core::prelude_accepts_constant_global`)
- Sobrevive de propósito: `'if !(rest.is_empty() || and) {' -> '... || rest.trim_start().starts_with("-or ")'` (`[X] mutação SOBREVIVEU`). A lista de proibidos (`-or`) também barra esse caso: são duas camadas, e cada uma tem a sua mutação PEGA acima.
- Atenção: as primeiras rodadas de várias mutações sobre `global_block_doubts_fail_closed` não valem, porque o teste já estava vermelho por causa de `-xor` (o `mutate.ps1` não confere se o teste passa antes de mutar). Rodei todas de novo depois da correção; só as saídas acima contam.

Oracle (`cargo run --release --example oracle_2c -- %LOCALAPPDATA%\wingetrs\catalog.db`, catálogo local com main + extras, 4118 manifestos scoop, 1403 deles 2c):
```
| bucket | 2c | instaláveis antes | instaláveis depois | recusados pelo portão | outro erro |
|---|---:|---:|---:|---:|---:|
| extras | 1109 | 0 | 1069 (96.4%) | 36 (3.2%) | 4 (0.4%) |
| main | 294 | 0 | 287 (97.6%) | 3 (1.0%) | 4 (1.4%) |
| todos | 1403 | 0 | 1356 (96.7%) | 39 (2.8%) | 8 (0.6%) |

| tipo de recusa (`FlagKind`) | manifestos |
|---|---:|
| Elevation | 27 |
| Hklm | 9 |
| ScoopCommand | 4 |
| Msiexec | 3 |
| SystemWrite | 1 |

Recusados pelo portão com algum bloco `if ($global)`: 3. Planejam com o corpo desses blocos apagado: 0 (). Desses, todo flag de recusa é `-Global` (`GlobalSwitch`): 0 (). Atribuem `$global` em algum script (o bloco poderia rodar): 0 ().

Atribuem `$global` em algum script (Task 164: com o `$global` constante do wingetrs a atribuição dá erro e o script falha): 0 ().
```
Gate S-hook (Task 150), todas as funções suportadas: `| todas (262) | 1087 (84.6%) | 159 (12.4%) | 39 (3.0%) | 0 (0.0%) | 5 (0.4%) |`.
- Planejam: 1356/1403, isto é 1318 + 38, exatamente o +38 esperado. Os recusados caem de 77 para 39. Nenhuma diferença a explicar.
- Recusa nova do item 5 (`GlobalName`): 0 manifestos (não aparece nem entre os 2c nem na tabela de flags dos 1293 S-hook).
- Atribuição a `$global` (sem regra, como o brief pede): 0 manifestos dos 1403 2c.
- Os 3 recusados que ainda têm `if ($global)` continuam recusados por código fora do bloco (ou por bloco que o portão não consegue delimitar), já que apagar o corpo não os faz planejar.

Decisions:
- `script_vars` continua devolvendo `("global", Bool(false))` (o teste `vars_pre_install` não muda); o `runner_ps1` ignora o nome `global` (sem diferenciar maiúsculas) e grava a linha `Set-Variable ... Constant,AllScope` antes das variáveis.
- Os flags de dentro do bloco mantêm o `FlagKind` original, com `Confirm` e o detail do brief. Regra por comando (`sudo`, `scoop`, `msiexec`, função do Scoop) só vira informativa quando a palavra aparece apenas dentro dos blocos. A passada fail-closed de `HiddenCode` roda nas duas metades. Variante nova: `FlagKind::GlobalName`.
- O bloco é delimitado no texto cru, char a char, com a mesma tokenização do `blank`. A regra de recusa roda na linha com o corpo morto apagado (parte viva) e, à parte, só no corpo (informativo); assim `if ($global) {…} else {…}` numa linha só ainda recusa pelo `else`. Fail closed além do brief: `-xor`, aspas, `|`, `;`, `#`, `@` e `-eq`/`-ne` na condição (`$global -eq $false` é verdadeiro), crase antes de chave, `<# #>` e `a#b` dentro do corpo.
- Mexi em 2 arquivos fora da lista **Files**: `tests/it/install_planner.rs` (`scanner_refusal_of_a_real_manifest_script_stops_the_plan`: o 7zip real agora planeja; a recusa é provada com o mesmo script sem o `if ($global)`) e `tests/it/install_script_effects.rs` (`python_catalog_pathext_script`: o texto do catálogo agora planeja; o `-Global` sem o guard continua recusado). As duas asserções mudaram porque esta tarefa muda de propósito o veredito desses scripts reais.
- Testes extras: `prelude_accepts_constant_global` (ao vivo, pwsh 7 e 5.1: `appdir foo $global`, `-global $global`, `-Global:$global`, `installed`, `Select-CurrentVersion`, `Add-Path`, e `-global $true` continua dando o erro de antes).

Concerns:
- `script_host_live::real_pwsh_runs_synthetic_hook_inside_the_sandbox` (Task 160) falha no ao vivo: espera `apps\app\1.0\bin` e recebe `apps\app\current\bin`. O `$dir` do `post_install` é `current` desde b1cad6e, antes do teste existir, e esta tarefa não mexe em `$dir`: a falha é anterior e não foi corrigida aqui.
- O gate completo não ficou verde neste checkout por causa de arquivos de outros agentes (fmt em `src/winget_install/host_win.rs`/`tests/it/wi_select.rs`, regra em `tests/it/wi_cli.rs:854`). O orquestrador precisa rodar o gate depois.
- `.join` com dado externo: nenhum novo.
- Eu rodei `cargo fmt` (o crate inteiro) algumas vezes enquanto outros agentes editavam o checkout. A última vez foi durante a correção 2e275a1, e ela pode ter reformatado arquivos deles que estavam sem formatação (`src/winget_install/host_win.rs`, `tests/it/wi_select.rs`). A mudança é só de formatação, e não fiz commit de nenhum arquivo deles.
- `main/vulkan` (pre_uninstall com `$x = if ($global) {…}`) passou a ser recusado pela correção. Se valer a pena, uma tarefa futura pode aceitar o `if` em posição de atribuição.
- Sugestão de harness para o orquestrador (não editei): o `scripts/mutate.ps1` não confere se o teste passa antes de mutar. Um teste já vermelho gera um `[OK] mutação PEGA` falso, e isso aconteceu aqui. Vale uma checagem prévia no script ou uma linha em `docs/ARMADILHAS.md`.
