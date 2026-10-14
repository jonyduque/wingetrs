# Task 106 — Ponte de atualização (winget e scoop)

Status: DONE_WITH_CONCERNS

## Commits

```
86f7e36 feat: upgrade apps installed by winget or scoop through their own CLI
```
(`git log --format='%h %s' 6c1bfcb..HEAD`; `git show --stat 86f7e36`: src/bridge/mod.rs, src/bridge/spawn_win.rs, src/lib.rs, src/tui/app.rs, src/tui/jobs.rs, src/tui/run.rs, tests/bridge.rs, tests/tui_jobs.rs, tests/tui_state.rs)

## Gate

```
Carregado em 252ms
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 2s
[OK]   clippy 22s
[OK]   test (815 passed, 20 ignored) 287s
[OK]   gate verde
```

## Mutation proofs

1. `scoop_message` aceita 0 sem comparar versões:
   `mutate.ps1 -Path src/bridge/mod.rs -Anchor '&& after != before' -Replacement '&& true' -TestCommand 'cargo test --quiet --test bridge scoop_exit_zero_needs_new_version'`
   `[OK] mutação PEGA: '&& after != before' -> '&& true' fez o teste falhar (exit 101).` (panicked at tests\bridge.rs:204)
2. `valid_target` aceita `-` no início:
   `-Anchor 'bytes.next().is_some_and(|c| c.is_ascii_alphanumeric())' -Replacement "bytes.next().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'-')" -TestCommand 'cargo test --quiet --test bridge invalid_targets_refused'`
   `[OK] mutação PEGA: 'bytes.next().is_some_and(|c| c.is_ascii_alphanumeric())' -> 'bytes.next().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'-')' fez o teste falhar (exit 101).` (tests\bridge.rs:116)
3. Tirar `--disable-interactivity`:
   `-Anchor '"--disable-interactivity",' -Replacement '' -TestCommand 'cargo test --quiet --test bridge command_lines_match_the_probe'`
   `[OK] mutação PEGA: '"--disable-interactivity",' -> '' fez o teste falhar (exit 101).` (tests\bridge.rs:71)
4. Thread de stderr não juntada (join ⇒ drop). **Quem pega: o teste 7**, não a regra mecânica (o `check_rules.ps1` só conhece `.join().ok()`/`unwrap_or`/`if let Ok`):
   `-Path src/bridge/spawn_win.rs -Anchor 'let err_rest = join(&mut readers.err)?;' -Replacement 'let err_rest: Option<String> = readers.err.take().and_then(|h| { drop(h); None });' -TestCommand 'cargo test --quiet --test bridge spawn_runs_fake_tool'`
   `[OK] mutação PEGA: 'let err_rest = join(&mut readers.err)?;' -> 'let err_rest: Option<String> = readers.err.take().and_then(|h| { drop(h); None });' fez o teste falhar (exit 101).` (tests\bridge.rs:319, `["erro", "um", "dois"]` sem o `fim` do stderr)
5. Nada ao vivo: nenhum `winget upgrade`/`scoop update` real foi executado; testes usam `cmd.exe` e closures falsas. O único processo real dos testes é `cmd.exe /d /c ...`.
   ```
   $ git grep -n "Command::new" src/bridge
   src/bridge/spawn_win.rs:73:    let mut child = Command::new(program)
   ```
   `leftovers.ps1`: `[OK] nenhum processo de agente sobrando (> 2 min)`

## Decisions

- `PINNED_NOTE`/`NOT_OURS_NOTE` passaram a ser `pub` em `bridge` (o teste 1 os compara); `app.rs` os importa de lá.
- Progresso da ponte: `Msg::BridgeLine` reescreve `Running.label` como `"<rótulo> · <linha>"` (e `BRIDGE_WAITING` após `BRIDGE_QUIET` = 600 s, conferido no `Msg::Tick`), para não editar `view.rs`, cujo `progress_text` imprimiria "N bytes".
- Cada leitor devolve a última linha sem `\n` pelo `join`, o que torna o `join` necessário para o resultado (é por isso que a mutação 4 é pega pelo teste 7).
- A confirmação segue `ConfirmMode` (`Removals`/`Never` não confirmam a ponte). `scoop_root` e `pwsh` (`find_pwsh`: `pwsh.exe` no PATH, senão `powershell`) são calculados uma vez no início do `event_loop` (depois do 1º quadro), e o mesmo valor vai para o `App` (texto) e para o `Ctx` (execução).
- `upgrade_target(app, r)` (antes `(r, pins)`) devolve `Target::{Ours, Bridge}`; ferramenta com `Some(false)` ⇒ `Err("winget/scoop não está instalado")`. `U` com um `Err` que não seja pin nem NOT_OURS mostra a própria nota como toast.

## Concerns

- Três testes antigos de `tests/tui_state.rs` usavam uma linha `arp` + `winget` + `Available` como "não é nosso" (`upper_u_upgrades_selected_or_all`, `enter_in_installed_opens_actions_menu`, `upgrade_marked_plans_batch_and_one_confirm`). Pela regra nova, essa linha agora passa pela ponte. Troquei a fonte para `scoop:main`/`scoop:extras`, que continua `NOT_OURS_NOTE`, e assim a intenção dos testes ficou a mesma.
- winget código 0 + "aviso de reinício na saída" (§4.4) não foi implementado: exigiria casar texto localizado do winget, e `winget_message(code)` só recebe o código.
- Sem tempo-limite (como pede o brief): se um instalador herdar os handles do pipe e continuar rodando, os leitores esperam por ele. Não medido.
- O menu de ações aberto quando chega o `ToolsDetected` só reflete o resultado ao ser reaberto.
- TDD: os testes foram escritos antes do código, mas a 1ª execução já tinha a implementação, então não vi o vermelho separado. As 4 mutações acima cobrem as regras.

## Review fixes (5e0dfb1)

- `spawn_win`: com erro de I/O num leitor, o processo é aguardado (`child.wait()`) antes de propagar o erro.
- `-Command`: dobra `'` e as aspas tipográficas U+2018..U+201B; teste com `C:\Users\D’Angelo`. `[OK] mutação PEGA: ''\'' | '\u{2018}' | '\u{2019}' |' -> ''\'' | '\u{2018}' |' fez o teste falhar (exit 101).`
- `run_bridge(scoop_root: Option<&Path>)`: scoop com `None` falha com "scoop não encontrado" antes de rodar. `[OK] mutação PEGA: '(Tool::Scoop, None) => return fail' -> '(Tool::Scoop, Some(_)) => return fail' fez o teste falhar (exit 101).`
- `start_batch` com plano em andamento: os itens da ponte não são enfileirados; um toast de aviso diz isso e as marcas ficam. `[OK] mutação PEGA: 'if !bridges.is_empty() {' -> 'if false {' fez o teste falhar (exit 101).`
- Gate: `[OK]   test (817 passed, 20 ignored) 245s` / `[OK]   gate verde`. O travamento quando um processo neto segura o pipe ficou pendente, como pedido.
