Status: DONE_WITH_CONCERNS

Commits:
b852bdc feat(install): prelude group 2 (intercepted global effects) and effects.jsonl parser

Gate (as linhas finais reais; as duas rodadas caíram só em arquivos de outros agentes):
1ª rodada:
```
[OK]   rules 6s
[OK]   rules-selftest 15s
[OK]   fmt 3s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (test "it") due to 1 previous error
```
(log: `tests\it\wi_bridge_install.rs:410:37: error: unnecessary use of `clone` ...` (2d, não é meu))
2ª rodada, depois do commit:
```
[OK]   rules 6s
[OK]   rules-selftest 16s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\winget_install\msix.rs:60:
```
Prova dos meus arquivos:
- `cargo test --quiet --test it prelude_ -- --include-ignored` -> `test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1272 filtered out; finished in 17.67s` (13 prelude_effects + 16 prelude_core, os ignorados incluídos; pwsh 7 real; prelude_core inclui a carga no Windows PowerShell 5.1)
- `cargo clippy --all-targets --message-format short -- -D warnings 2>&1 | grep "prelude\|script_effects"` -> nenhuma linha.
- `cargo fmt -- <meus arquivos>` aplicado.
- Teste de fumaça manual (scratchpad t153) das 11 funções no pwsh 7 e no 5.1: JSON idêntico nos dois (só a ordem das chaves muda), arrays de uma entrada continuam arrays.

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/install/prelude.ps1 -Anchor 'if ($script:__wr_env_overlay.ContainsKey($Name))' -Replacement 'if ($false)' -TestCommand 'cargo test --quiet --test it prelude_effects::set_env_records_and_overlay -- --include-ignored'`
   `[OK] mutação PEGA: 'if ($script:__wr_env_overlay.ContainsKey($Name))' -> 'if ($false)' fez o teste falhar (exit 101).`
2. `pwsh -File scripts/mutate.ps1 -Path src/install/prelude.ps1 -AnchorFile <scratch>/a2.txt -ReplacementFile <scratch>/r2.txt -TestCommand 'cargo test --quiet --test it prelude_effects::shim_records_without_touching_path -- --include-ignored'` (a2 = `__wr_effect @{ op = 'shim'`, r2 = `Add-Path -Path (Split-Path $path); __wr_effect @{ op = 'shim'`)
   `[OK] mutação PEGA: '__wr_effect @{ op = 'shim'' -> 'Add-Path -Path (Split-Path $path); __wr_effect @{ op = 'shim'' fez o teste falhar (exit 101).`
3. `pwsh -File scripts/mutate.ps1 -Path src/install/script_effects.rs -Anchor 'other => return Err(EffectsError::UnknownOp(n)),' -Replacement '_ => continue,' -TestCommand 'cargo test --quiet --test it prelude_effects::parser_limits_and_errors'`
   `[OK] mutação PEGA: 'other => return Err(EffectsError::UnknownOp(n)),' -> '_ => continue,' fez o teste falhar (exit 101).`
   (a 1ª tentativa deu INCONCLUSIVO porque outro agente estava com um mutante no `fetch.rs` naquela hora; repetida com o crate compilando.)
4. Gate: acima.

Decisions:
- `__wr_effect` é função interna e não entra em `PRELUDE_FUNCTIONS`; o teste `prelude_text_is_declared_ascii_and_clean` (prelude_core.rs, fora da lista Files, autorizado pelo orquestrador) agora ignora nomes `__wr_*` ao comparar as funções definidas no texto. A lista declarada foi para 35 e a lista de texto proibido não mudou.
- `-TargetEnvVar` muda a variável de sessão indicada (como o brief pede; no Scoop original só `$env:PATH` muda). `Add-Path`/`Remove-Path` sempre gravam o efeito (o executor deduplica); não atualizam a sobreposição de `Get-EnvVar` (só `Set-EnvVar` atualiza).
- `shim` mantém o `abort` do original quando o alvo não existe, grava o caminho resolvido (`Convert-Path`) e o nome em minúsculas (o Scoop cria `$name.tolower()`). `startmenu_shortcut` mantém as checagens de existência do original e não chama `shortcut_folder`.
- `Set-EnvVar` com `$null` ou `''` vira `value: null` (o Scoop apaga a variável nos dois casos). Semente inválida em `$__wr_env_seed` gera exceção (`-ErrorAction Stop`). Nome na semente é comparado sem diferenciar maiúsculas.
- Rust: o braço literal `other => ...` deixaria `other` sem uso; resolvi com `#[allow(unused_variables)]` no `let effect = match` e um comentário. Os limites contam todas as linhas de `str::lines` (vazias incluídas), e é essa mesma contagem que numera as linhas.

Concerns:
- O gate não ficou verde por causa de arquivos de outros agentes (clippy em `tests/it/wi_bridge_install.rs`, fmt em `src/winget_install/msix.rs`). O orquestrador precisa rodar o gate de novo.
- A recusa nova no parser não foi testada contra um corpus real: não existe `effects.jsonl` real antes do runner (154), e o formato é nosso.

## Rodada de revisão (42ac47a fix(install): review of task 153)

Commits: `42ac47a fix(install): review of task 153`
Gate: `[OK] rules / rules-selftest / fmt`, `[FAIL] clippy exit 101` só em `tests\it\wi_bridge_install.rs:410:37` (outro agente). Meus arquivos: `cargo test --quiet --test it prelude_ -- --include-ignored` -> `test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 1277 filtered out; finished in 16.57s` (inclui script_scan::prelude_redefinition_refused, que depois passou); clippy filtrado por prelude/script_effects sem nenhuma linha.

Alterações:
1. Teste de limite exato: linha inteira de 65 536 bytes => Ok, de 65 537 => TooLong(1).
2. Um struct por op com `deny_unknown_fields`; chaves que aceitam null são obrigatórias (`deserialize_with = "nullable"`), então `{"op":"set_env","name":"PATH"}` => Invalid. Teste com 13 linhas ruins (falta uma chave ou sobra uma, em cada op).
3. `ScriptEffect::AddPath` ganhou `force: bool`, que vale true para `-Force` e para `add_first_in_path`. Teste novo: `add_path_force_flag`.
4. Nova função interna `__wr_env_lookup` (sobreposição e depois semente, sem exceção). `Remove-Path -PassThru` devolve o que estava no valor guardado, não na sessão. `Add-Path`/`Remove-Path` atualizam a sobreposição quando a variável é conhecida; variável desconhecida continua desconhecida, e o `Get-EnvVar` depois ainda lança exceção, em vez de inventar um valor. Teste novo: `path_overlay_and_passthru`. Verifiquei à mão também no PowerShell 5.1.
5. Teste puro `shim_body_never_touches_path`: o corpo de `shim` no texto do prelúdio não contém Add-Path, add_first_in_path, env:PATH nem Set-EnvVar. O `Add-Path` que a revisão viu era só o mutante temporário da prova 2 do brief; o mutate.ps1 restaura o arquivo, e o corpo commitado tem 0 ocorrências.

Mutation proofs (saída real):
[OK] mutação PEGA: '.find(|(_, l)| l.len() > MAX_LINE_BYTES)' -> '.find(|(_, l)| l.len() > MAX_LINE_BYTES + 1)' fez o teste falhar (exit 101).
[OK] mutação PEGA: '#[serde(deserialize_with = "nullable")]\n    value: Option<String>,' -> '    value: Option<String>,' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'prepend = $true; force = [bool]$Force }' -> 'prepend = $true; force = $false }' fez o teste falhar (exit 101).
[OK] mutação PEGA: '        return $inPath' -> '        return $inSessionPath' fez o teste falhar (exit 101).
[OK] mutação PEGA: '            $script:__wr_env_overlay[$TargetEnvVar] = (@($Path)' -> '            $null = (@($Path)' fez o teste falhar (exit 101).
[OK] mutação PEGA: '__wr_effect @{ op = 'shim'' -> 'Add-Path -Path (Split-Path $path); __wr_effect @{ op = 'shim'' fez o teste falhar (exit 101).   (teste puro shim_body_never_touches_path)
Repetidas depois das mudanças: as 3 do brief deram [OK] PEGA (overlay ContainsKey, shim + Add-Path, braço `other`).
