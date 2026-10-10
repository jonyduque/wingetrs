Status: DONE_WITH_CONCERNS

Commits:
5fc4c1c feat(cli): install, uninstall, upgrade, bucket and repair commands

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 7s
[OK]   test (395 passed, 16 ignored) 103s
[OK]   gate verde

Mutation proofs:
1. `--dry-run` não toca:
   pwsh -File scripts/mutate.ps1 -Path src/cli/install_cmd.rs -Anchor '    print_plans(&plans);' -Replacement '    print_plans(&plans); if f.dry_run { ops::apply(ctx, &plans).await?; }' -TestCommand 'cargo test --quiet --test cli_install'
   [OK] mutação PEGA: '    print_plans(&plans);' -> '    print_plans(&plans); if f.dry_run { ops::apply(ctx, &plans).await?; }' fez o teste falhar (exit 101).  (dry_run_prints_the_plan_and_leaves_the_root_untouched)
2. `--yes` sem TTY não escolhe sozinho:
   pwsh -File scripts/mutate.ps1 -Path src/cli/install_cmd.rs -Anchor 'bail!("{}", output::ambiguous_text(target, &names));' -Replacement 'return Ok(Some(found.swap_remove(0)));' -TestCommand 'cargo test --quiet --test cli_install'
   [OK] mutação PEGA: 'bail!("{}", output::ambiguous_text(target, &names));' -> 'return Ok(Some(found.swap_remove(0)));' fez o teste falhar (exit 101).  (ambiguous_target_without_a_terminal_fails_listing_bucket_ids)

Decisions:
- Arquivos novos fora da lista do brief: `src/cli/install_cmd.rs` (lógica dos comandos, trait `Ui`, `TtyUi`, `LineProgress`; neutra de plataforma, testável) e `src/cli/prod_win.rs` (efeitos reais Windows: RegEnvStore, ComShortcuts, ShellTrash, RegScanner, `files_in_use`). Motivo: manter `mod.rs` enxuto e o código Windows em `*_win.rs`.
- `--yes` limpa resíduos com `confidence == High && preselected` (um High não pré-marcado, ex. "pode ser de outra instalação", não é removido sem escolha). Sem TTY e sem `--yes`: lista os resíduos e não remove nada.
- Com `WINGETRS_TEST_ENV_KEY` presente, `prod_win` usa `ResidueScan::none()` (teste nunca lê Desktop/Menu Iniciar/AppData reais).
- `upgrade` sem alvo e sem `--all` lista as atualizações (e avisa os pulados no stderr) e sai; `upgrade <app>` já atualizado ⇒ mensagem, exit 0; app pulado/quebrado ⇒ erro com o motivo. Confirmação sem TTY e sem `--yes` ⇒ erro pedindo `--yes`.
- `picker.rs`: `pick_inline_multi` adicionado (reaproveita `Picker::multi` de `tui::picker`); `pick_inline` passou a usar o mesmo laço.

Concerns:
- O teste do ciclo install -> uninstall (`install_then_uninstall_restores_the_root`) é `#[ignore = "live: ..."]` porque grava na chave de teste do HKCU (apagada no Drop da própria chave); rodei com `--include-ignored` e passou. Os demais 6 testes de `cli_install` rodam no gate (não escrevem no registro).
- A "raiz igual à inicial" precisou de exceções além das do brief: `install.lock` (arquivo de lock permanece, por desenho) e a pasta `apps` vazia (uninstall não a remove); `catalog.db-shm/-wal` são ignorados como parte do catalog.db (criados pela abertura do catálogo, não pelo plano). Conteúdo dentro de `apps` deve sumir (assertado).
- Seletor com setas e confirmação interativa (`Continuar? [s/N]`, `pick_inline`, resíduos em modo múltiplo) não têm teste automatizado (exigem TTY); só a parte pura (`is_yes`, `progress_line`, `Picker`) é testada.
- Progresso em `\r` só em stderr-TTY: sem teste (não medido/observado em terminal real).
- `cargo fmt` rodou no checkout inteiro durante o trabalho (tocou só arquivos meus, conforme `git status`); arquivos de `src/tui/*` do outro agente não foram adicionados ao commit.

## Revisão (CHANGES_REQUESTED) - correções

Commit extra: ver `git log` (fix(cli): read-only planning never creates state.db; single production ctx builder).
Gate: [OK] rules / fmt / clippy / test (407 passed, 16 ignored) / gate verde.

1. `StateDb::open_existing(path) -> Result<Option<StateDb>>` (sem criar arquivo/pasta/schema) e `StateDb::empty_in_memory()` em `state.rs`; `ops::read_state(layout)` (pub) usa os dois e substitui `open_state` em `plan_for`, `plan_uninstall`, `upgradable` e no `upgrade` da CLI. `open` (cria) só em apply/uninstall/repair. `Lab::new` não pré-cria mais o state.db; teste de dry-run em raiz limpa afirma raiz idêntica e sem `state.db`. Mutação:
   [OK] mutação PEGA: 'match StateDb::open_existing(&layout.state_db())? {' -> 'match Some(StateDb::open(&layout.state_db())?) {' fez o teste falhar (exit 101). (dry_run_prints_the_plan_and_leaves_the_root_untouched)
2. Teste unitário `yes_cleans_only_high_confidence_preselected_residues` (Ui falso, FakeTrash): só o High+preselected é limpo; sem --yes e sem TTY nada. Mutação:
   [OK] mutação PEGA: 'r.confidence == Confidence::High && r.preselected' -> 'r.preselected' fez o teste falhar (exit 101).
3. `uninstall_dry_run_leaves_the_root_untouched_and_takes_no_lock`: raiz idêntica e sem `install.lock`.
4. `src/install/prod_win.rs` (movido de `src/cli/`): 
   `pub struct ProdEffects` com `ProdEffects::new() -> Result<ProdEffects, EffectError>` e `.effects() -> Effects<'_>`;
   `pub fn build_ctx<'a>(prod: &'a ProdEffects, data: &'a DataDir, http: &'a Http, catalog: &'a Catalog, wait: LockWait, progress: Progress<'a>) -> OpsCtx<'a>` (in_use = `fs_win::files_in_use`; `ResidueScan::none()` quando `WINGETRS_TEST_ENV_KEY` está definida, senão `user_profile`). A CLI usa via `src/cli/run_win.rs`. A TUI deve passar `LockWait::NoWait` e seu progress.
Arquivos extras editados: src/install/ops.rs, state.rs, mod.rs (permitido).
