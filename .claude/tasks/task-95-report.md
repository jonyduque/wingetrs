Status: DONE

Commits: (ver `git log --format='%h %s' 724317a..HEAD`)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 99s
[OK]   test (733 passed, 18 ignored) 146s
[OK]   gate verde

Mutation proofs:
- list_icons fora de `newer` (anchor `.is_some_and(|l| *l != app.settings.list_icons);` -> `.is_some_and(|_| false);`, teste every_setting_joins_save_coalescing):
  [OK] mutação PEGA: '.is_some_and(|l| *l != app.settings.list_icons);' -> '.is_some_and(|_| false);' fez o teste falhar (exit 101).
  (falha: "ListIcons lost its change: []")
- sem SetMouseCapture (teste mouse_setting_toggles_capture):
  [OK] mutação PEGA: 'cmds.push(Cmd::SetMouseCapture(app.mouse_enabled));' -> '' fez o teste falhar (exit 101).

Decisions:
- `Behavior` perdeu o derive `Default` e ganhou `impl Default` manual (mouse = true).
- Err no `SettingsSaved` devolve `SetMouseCapture` só se `mouse_enabled` mudou na reversão.
- Teste extra `mouse_setting_reverts_capture_on_save_failure`.
- `theme_setting_flips_dark` ajustado de 8 para 9 Downs (Theme desceu uma linha); snapshot Ajustes: só a linha Mouse mudou.
- Início: captura só liga se `cfg.behavior.mouse`; `DisableMouseCapture` de saída/panic hook segue incondicional.

Concerns: none
