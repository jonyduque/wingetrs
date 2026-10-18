Status: DONE

Commits:
c819118 feat(install): innosetup extraction through innounp (S-inno) and injectable runners

Gate:
[OK]   rules 6s
[OK]   rules-selftest 13s
[OK]   fmt 2s
[OK]   clippy 38s
[OK]   test (1368 passed, 60 ignored) 83s
[OK]   gate verde

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/install/inno.rs -Anchor 'if files == 0 {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it install_inno::'`
   [OK] mutação PEGA: 'if files == 0 {' -> 'if false {' fez o teste falhar (exit 101).
     install_inno::exit_zero_with_zero_files_is_missing_extract_dir --- FAILED
     install_inno::fake_exit_zero_no_files_rolls_back --- FAILED
2. `pwsh -File scripts/mutate.ps1 -Path src/install/inno.rs -Anchor "if dir.starts_with('{') {" -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it install_inno::args_with_brace_dir'`
   [OK] mutação PEGA: 'if dir.starts_with('{') {' -> 'if false {' fez o teste falhar (exit 101).
   (a linha real é `let (root, rest) = if dir.starts_with('{') {`; a âncora casa como substring, sem moldar o código)
3. `pwsh -File scripts/mutate.ps1 -Path src/install/planner.rs -Anchor '.ends_with(".exe")' -Replacement '.ends_with("")' -TestCommand 'cargo test --quiet --test it install_inno::plan_uses_extract_inno_only_for_exe'`
   [OK] mutação PEGA: '.ends_with(".exe")' -> '.ends_with("")' fez o teste falhar (exit 101).
4. Prova ao vivo contra instalador real: `install_inno::real_innounp_over_real_installer` está escrito com `#[ignore = "live: innounp sobre instalador real"]` (lê `WINGETRS_LIVE_INNOUNP` e `WINGETRS_LIVE_INNO_SETUP`). **NÃO rodou**: precisa da autorização do usuário e do xming da sonda D4.
Extra (recusa nova minha): `-Path src/install/executor.rs -Anchor 'c.eq_ignore_ascii_case(tmp_name)' -Replacement 'false && c.is_empty()' -TestCommand '... install_inno::extract_to_into_the_temp_folder_is_refused'`
   [OK] mutação PEGA: 'c.eq_ignore_ascii_case(tmp_name)' -> 'false && c.is_empty()' fez o teste falhar (exit 101).

Decisions:
- Ambiente do filho (nota da revisão da 146): `inno::child_env(system32, exe)` monta só `PATH=<System32 por FOLDERID_System>;<pasta do innounp>`, aplicado por cima do herdado pelo `run_spec` (a chave PATH do std no Windows é case-insensitive, então substitui o PATH do usuário). Teste puro `child_env_is_a_fixed_path_only` + teste ignorado `fake_cmd_runner_sees_fixed_path` (um `.cmd` falso num temp `t147-*` imprime `%PATH%`, sai 3 e escreve CP850): rodei uma vez com `--ignored`, passou (PATH com um `;` só, terminando na pasta do auxiliar). Nenhum innounp real rodou.
- `extract_dir` com `{nome}`: o nome tem que ser `[A-Za-z0-9_]+`; `{`/`}` em qualquer outro componente é recusado (UnsafePath). `extract_to` igual (sem caixa) ao nome do temporário `.inno-<n>` é recusado (senão `merge_into` mesclaria a pasta nela mesma).
- O teste de "innounp ausente" é feito antes de criar o staging (nada de `apps\<app>` é criado). Saída do corredor é lida dentro do `catch_decoder` (pânico vira `ExtractError::Decoder`); `count_files` pula links/junções (`is_symlink`, que no std cobre symlink e mount point).
- Refatorei a preparação do staging e a busca da fonte do braço `Extract` em `own_staging`/`extract_source` (mesmo código, compartilhado pelos dois braços).
- `.join` com dado externo: `extract_to` → `extract::safe_components` antes de `dest.extend` (executor) e `safepath::safe_components` em `plan.rs` (texto do resumo); `extract_dir` → `safepath::safe_components` em `inno_filter`. Os outros joins usam constantes/índice interno (`current("innounp").join("innounp.exe")`, `staging.join(".inno-<index>")`); `merge_into` (já existente) junta nomes lidos do disco dentro do temporário.

Concerns:
- Arquivos fora da lista Files, tocados por necessidade de compilação/semântica: `tests/it/install_psmodule.rs` e `tests/it/install_written_manifest.rs` (literais `ExecCtx` que o brief não listou: só a linha `runners`), `tests/it/install_planner.rs` (braço `ExtractInno` num `match` exaustivo), `tests/it/manifest_2c.rs` e `tests/it/install_manifest.rs` (afirmavam a recusa antiga do `innosetup`; agora afirmam que planeja / não é recusado).
- Literais nos testes usam o caminho completo `wingetrs::install::executor::Runners::disabled()` para não mexer nos `use`.
- Prova contra instalador real (check 4) pendente de autorização do usuário.

## Review fix
Commit: see `git log --format='%h %s' -1 -- src/install/inno.rs` (fix(install): review of task 147).
- `args_unsafe_dir_refused` gained `"{a b}"` and `"{..}"`; mutation `c.is_ascii_alphanumeric() || c == '_'` -> `true`: [OK] mutação PEGA (exit 101).
- `inno::find_link` + refusal (UnsafePath, rollback) before `merge_into`; test `link_in_extracted_tree_is_refused` (junction to an outside folder: install fails, app dir gone, target intact). Mutation `return Ok(Some(entry.path()));` -> `continue;`: [OK] mutação PEGA (exit 101).
- Gate: failed only in other agents' files (`elevated_install::msi_params_allowlist`, `install_helpers::helper_of_dependency_planned_before_it`, both uncommitted work of Tasks 154/180); mine: `install_inno::` 20 passed, fmt ok, clippy clean on my files.
