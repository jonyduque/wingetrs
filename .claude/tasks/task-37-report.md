Status: DONE_WITH_CONCERNS

Commits:
c4e68c8 fix(install): ops revalidates under lock; pin cross-bucket deps and list wiring
f6ea10c feat(install): operations with dependencies, lock, repair and list integration

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 6s
[OK]   test (360 passed, 15 ignored) 65s
[OK]   gate verde

Mutation proofs:
Checks beyond the steps:
1. Ciclo detectado
   pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'chain.push(id);' -Replacement 'drop(id);' -TestCommand 'cargo test --quiet --test install_ops dependency_cycle'
   [OK] mutação PEGA: 'chain.push(id);' -> 'drop(id);' fez o teste falhar (exit 101).
   pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'if let Some(pos) = self.stack.iter().position(|s| s.eq_ignore_ascii_case(&id)) {' -Replacement 'if let Some(pos) = None::<usize> {' -TestCommand 'cargo test --quiet --test install_ops dependency_cycle'
   [OK] mutação PEGA: 'if let Some(pos) = self.stack.iter().position(|s| s.eq_ignore_ascii_case(&id)) {' -> 'if let Some(pos) = None::<usize> {' fez o teste falhar (exit -1073741571).   (stack overflow sem a detecção)
2. Dependência não suportada aborta antes do download (mock expect(0))
   pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'walk.visit(&bucket, d)?;' -Replacement 'let _ = walk.visit(&bucket, d);' -TestCommand 'cargo test --quiet --test install_ops unsupported_dependency_fails_before_any_download'
   [OK] mutação PEGA: 'walk.visit(&bucket, d)?;' -> 'let _ = walk.visit(&bucket, d);' fez o teste falhar (exit 101).
3. HasDependents
   pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'if who.is_empty() {' -Replacement 'if true {' -TestCommand 'cargo test --quiet --test install_ops uninstall_refuses_while_another_app_depends_on_it'
   [OK] mutação PEGA: 'if who.is_empty() {' -> 'if true {' fez o teste falhar (exit 101).

What binds (uma por regra; T='cargo test --quiet --test install_ops'):
- broadcast uma vez / zero sem mudança (env_broadcast_happens_once_for_two_plans_with_env, no_broadcast_when_nothing_in_the_environment_changes):
  [OK] mutação PEGA: '    if changed {' -> '    if true {' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '    let changed = match env_snapshot(env, names) {' -> '    let _c = env.broadcast(); let changed = match env_snapshot(env, names) {' fez o teste falhar (exit 101).
- dependência já instalada é pulada (dependency_installed_by_wingetrs_is_skipped):
  [OK] mutação PEGA: 'if self.state.get(&id)?.is_some() {' -> 'if false {' fez o teste falhar (exit 101).
- ordem topológica deps + alvo (dependencies_come_first_in_topological_order_without_duplicates):
  [OK] mutação PEGA: '    nodes.push(Node {' -> '    nodes.insert(0, Node {' fez o teste falhar (exit 101).
- manifest_sha256 gravado (manifest_sha256_is_recorded), src/install/executor.rs:
  [OK] mutação PEGA: 'manifest_sha256: plan.manifest_sha256.clone(),' -> 'manifest_sha256: String::new(),' fez o teste falhar (exit 101).
- lock ocupado => Busy; NoWait não espera (busy_lock_with_nowait_is_busy, busy_lock_blocks_uninstall_and_repair_too):
  [OK] mutação PEGA: '.ok_or(OpsError::Busy)' -> '.ok_or(OpsError::Lock(String::new()))' fez o teste falhar (exit 101).
  [OK] mutação PEGA: 'LockWait::NoWait => Duration::ZERO,' -> 'LockWait::NoWait => Duration::from_secs(5),' fez o teste falhar (exit 101).   (sobreviveu na 1ª versão; teste ganhou limite de tempo)
- upgrade --all compara com core::version (upgradable_lists_apps_with_a_newer_catalog_version, 1.9 -> 1.10):
  [OK] mutação PEGA: 'if Version::parse(&pkg.entry.version) > Version::parse(&rec.version) {' -> 'if pkg.entry.version > rec.version {' fez o teste falhar (exit 101).   (sobreviveu com 1.0 -> 1.10; teste trocado para 1.9)
- in_use injetado chega ao executor (uninstall_with_file_in_use_changes_nothing):
  [OK] mutação PEGA: '        in_use: ctx.in_use,' -> '        in_use: &|_: &Path| Vec::new(),' fez o teste falhar (exit 101).
- resíduos classificados (uninstall_returns_classified_residues, residue_scan_finds_shortcuts_and_folders_from_the_injected_dirs):
  [OK] mutação PEGA: '        .values_containing(RESIDUE_REG_ROOTS, &app_dir.display().to_string())' -> '        .values_containing(&[], &app_dir.display().to_string())' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '        other_installs: (scan.other_installs)(app),' -> '        other_installs: false,' fez o teste falhar (exit 101).
- repair (repair_rebuilds_a_deleted_state_db, repair_keeps_intact_records_and_drops_orphans):
  [OK] mutação PEGA: '        if shims_on_path {' -> '        if false {' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '.filter(|(_, t)| is_under(t, &dir_text))' -> '.filter(|(_, t)| !t.is_empty())' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '            state.remove(&rec.app)?;' -> '' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '        if state.get(app)?.is_some() {' -> '        if false {' fez o teste falhar (exit 101).
- list: correlação wingetrs, origem, quebrado (list_shows_wingetrs_origin_and_broken_apps):
  [OK] mutação PEGA: 'let (InstallSource::Scoop | InstallSource::Wingetrs, Some(bucket)) =' -> 'let (InstallSource::Scoop, Some(bucket)) =' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '(InstallSource::Wingetrs, None) => UpdateState::Broken,' -> '(InstallSource::Wingetrs, None) => state,' fez o teste falhar (exit 101).
  [OK] mutação PEGA: 'InstallSource::Wingetrs => "wingetrs",' -> 'InstallSource::Wingetrs => "scoop",' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '                    r.origin.clone(),' -> '                    String::new(),' fez o teste falhar (exit 101).   (src/cli/output.rs)

Review round (CHANGES_REQUESTED -> c4e68c8), T='cargo test --quiet --test install_ops':
1. Stale plan revalidated under the lock (apply_refuses_a_plan_made_stale_by_an_outside_install, apply_skips_a_dependency_installed_meanwhile, uninstall_refuses_a_plan_made_stale_by_an_outside_change):
  [OK] mutação PEGA: '    let plans = revalidate(&state, plans)?;' -> '    let plans: Vec<&InstallPlan> = plans.iter().collect();' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '        } else if i < last && plan.upgrade_from.is_none() {' -> '        } else if false {' fez o teste falhar (exit 101).
  [OK] mutação PEGA: '        Some((rec, _)) if rec.version == plan.version => homepage_of(ctx.catalog, &rec)?,' -> '        Some((rec, _)) => homepage_of(ctx.catalog, &rec)?,' fez o teste falhar (exit 101).
2. Dependency from another bucket (dependency_with_explicit_bucket_comes_from_that_bucket):
  [OK] mutação PEGA: 'Some(b) => in_bucket(b),' -> 'Some(_) => in_bucket(parent_bucket),' fez o teste falhar (exit 101).
3. list wiring: scan_all_in / collectors (unit test scan_all_in_runs_the_wingetrs_collector in src/installed/mod.rs; cargo test --quiet --lib scan_all_in_runs_the_wingetrs_collector):
  [OK] mutação PEGA: '        (
            "wingetrs",
            Box::new(move || match data {...}),
        ),' -> '' fez o teste falhar   (the whole tuple removed from the list; multi-line anchor passed from a file)
  [OK] mutação PEGA: '            "wingetrs",' -> '            "removed",' fez o teste falhar (exit 101).
  [OK] mutação PEGA: 'Some(d) => collect_wingetrs_in(d),' -> 'Some(_) => Ok(Vec::new()),' fez o teste falhar (exit 101).
4. upgradable skipped (upgradable_skips_broken_and_uncatalogued_apps_with_a_reason):
  [OK] mutação PEGA: '        if read_json(&layout.current(&rec.app).join("install.json")).is_none() {' -> '        if false {' fez o teste falhar (exit 101).
  [OK] mutação PEGA: 'format!("não está mais no catálogo ({source})")' -> 'String::new()' fez o teste falhar (exit 101).
5. is_under refuses ./.. (repair_ignores_shims_that_escape_the_app_folder):
  [OK] mutação PEGA: '    if p.split(' -> '    if false && p.split(' fez o teste falhar (exit 101).
6. PartlyApplied names what was installed (failure_after_a_dependency_names_what_was_installed):
  [OK] mutação PEGA: '        applied.push(plan.app.clone());' -> '' fez o teste falhar (exit 101).

Decisions:
- Broadcast = diferença líquida: ops lê `Path` + variáveis de `env_set`/artefatos Env antes e depois e chama `broadcast()` uma vez se algo mudou (todo plano tem `EnsureShimsOnPath`, então "plano mexe em env" pelos passos daria sempre 1; rollback que restaurou tudo não anuncia nada).
- `OpsCtx` ganhou, além de `in_use`, `residue: ResidueScan` (pastas de atalhos/pastas candidatas, leitor de `.lnk` e "instalado também pelo scoop" injetados; `ResidueScan::none()` para testes, `ResidueScan::user_profile(..)` + `installed_by_scoop` para produção). Sem isso o uninstall leria o perfil real nos testes.
- Dependência sem bucket: bucket do dependente, senão o único bucket que a tem, senão `main`, senão `DependencyUnsupported("ambígua entre buckets")`. `HasDependents` olha o `depends` do manifesto do catálogo de cada app instalado (por nome, qualquer bucket; manifesto ausente => aviso e ignora).
- `apply` para no primeiro plano que falha (esse é desfeito pelo executor; dependências já instaladas ficam). O lock é adquirido via `spawn_blocking` aguardado (pânico propagado com `resume_unwind`); `uninstall`/`repair` são síncronos e bloqueiam direto.
- `repair` mantém registros intactos, reconstrói os ausentes (shims `.shim` e também `.cmd` `@rem`, entradas do PATH sob `apps\<app>\`, entrada `shims` se estiver no PATH; `manifest_sha256` vazio) e remove registros de apps sem pasta. `ListRow` ganhou `origin: String` (arp/msix/scoop/wingetrs) e `UpdateState::Broken`; `list_text` ganhou a coluna "Origem" e mostra "quebrado" no lugar da versão.

- Review: `UninstallPlan` gained `version` (filled by `from_state`), which `uninstall` compares under the lock. In `apply`, a plan whose app is now installed is skipped only if it is a dependency (not the last plan, `upgrade_from` None); every other mismatch returns `StateChanged`. `upgradable` returns `UpgradeList { upgradable, skipped }`. `OpsError::PartlyApplied { applied, error }` is used only when something was already applied.
- Review item 3: the test runs only the "wingetrs" collector (filtered from `collectors(Some(&data))`) through `scan_with`. Calling `scan_all_in` in full would read the real ARP/MSIX registry, which CLAUDE.md forbids in unit tests. `scan_all_in` is `scan_with(&collectors(data))`.

Concerns:
- Arquivos fora da lista Files (necessários): `src/install/plan.rs`/`planner.rs`/`executor.rs` (campo `manifest_sha256` no plano e no `PlanInput`, autorizado), `tests/install_executor.rs`/`tests/install_planner.rs` (literais com o campo novo), `src/install/residue.rs` (`github_owner` virou `pub` para buscar chaves com o dono do repo), `src/tui/view.rs` (braço `Broken` no `match` exaustivo), `tests/tui_state.rs` (literal `ListRow` com `origin`).
- `plan_uninstall` devolve `install::plan::UninstallPlan` (é onde o tipo vive; `executor::UninstallPlan` do brief não é reexportado).
- O uninstall não checa `in_use` por conta própria: o executor já faz isso antes de mexer em qualquer coisa (`ExecError::InUse` vira `OpsError::InUse`); uma segunda checagem seria mutação equivalente.
- Integração CLI/TUI (montar `OpsCtx` de produção com `fs_win::files_in_use`, `RegEnvStore`, `ComShortcuts`, `ResidueScan::user_profile(&|p| shortcut_win::link_target(p).ok(), &installed_by_scoop)`) fica para as tarefas de CLI/TUI.
- `leftovers.ps1` mostra bash/bun/python de 11:19 de outra sessão (dinoRig.ts); não são desta tarefa e não foram encerrados.
