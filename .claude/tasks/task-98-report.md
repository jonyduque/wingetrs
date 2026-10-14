Status: DONE

Commits: (ver `git log --format='%h %s' 1ef0a94..HEAD`)

Gate:
[OK]   rules 3s / rules-selftest 9s / fmt 1s / clippy 2s
[OK]   test (768 passed, 18 ignored) 47s
[OK]   gate verde

Mutation proofs:
1. upgrade_target ignora o pin (`if false {`), teste `pinned_app_is_not_upgraded`: PEGA (teste falhou; linha `[OK] mutação PEGA` colada abaixo).
2. filtro de atualizar tudo ignora o pin (`.partition(|(_, _, _)| false)`), `upgrade_all_skips_pinned`:
   [OK] mutação PEGA: '.partition(|(app, _, _)| crate::pins::is_pinned(pins, "wingetrs", app));' -> '.partition(|(_, _, _)| false);' fez o teste falhar (exit 101).
3. chave da CLI sem origem (`is_pinned(&pins, "", t)`), `cli_and_tui_use_the_same_key`:
   [OK] mutação PEGA: 'crate::pins::is_pinned(&pins, "wingetrs", t)' -> 'crate::pins::is_pinned(&pins, "", t)' fez o teste falhar (exit 101).
4. Testes sem %LOCALAPPDATA% real: `tests/pins.rs:8: let dir = tempfile::tempdir().unwrap();` e `tests/cli_install.rs:67: let t = tempfile::tempdir().unwrap();` (Lab, LOCALAPPDATA aponta para o tempdir).

Decisions:
- Filtro de pinados em `UpgradeList::without_pinned` (src/install/ops.rs), chamado por `plan_upgrade_all` (lê `pins::load(c.data)`) e pela CLI; regra única `pins::is_pinned`.
- Com marcas, o menu mostra só "Atualizar/Remover marcados"; o pin é omitido (`pin: None`).
- Menu de 1 app: 3º item sempre selecionável (também para apps não-wingetrs, chave `<origin>/<name>`).
- Atualizadas as expectativas de `enter_in_installed_opens_actions_menu` (3 itens; removido o Down+Enter em item desabilitado, pois o Down agora cai no pin selecionável) e o snapshot `actions_modal_snapshot` (novo item).
- `Msg::PinsLoaded` carrega pins no boot (job); `app.pins.extend`.

Concerns: CLI `upgrade <app>` compara a chave com `t` exato (maiúsculas/minúsculas). `pinned` na CLI sem argumento imprime `<app> (fixado)`.

Follow-up (fix commit): pins carregados de forma síncrona antes do laço (PinsLoaded removido); open_actions usa is_pinned; `fs_retry::write_json_atomic` compartilhado por icons.rs e pins.rs. Gate: test (769 passed, 18 ignored), gate verde. Nota: `upgrade Ripgrep` (caixa diferente) dá NotInstalled; mantido, nada é instalado.
