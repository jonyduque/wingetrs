Status: DONE_WITH_CONCERNS

Commits: (SHA preenchido abaixo pelo git log)

Gate: `pwsh -File scripts/verify.ps1` parou em `[FAIL] fmt exit 1` por `src/install/script_scan.rs:1610` (Task 164, arquivo alheio). rules e rules-selftest OK. Provado o meu: `cargo test --quiet --test it -- wi_ elevated_install config` => `test result: ok. 316 passed; 0 failed; 5 ignored`; `cargo clippy --all-targets` sem avisos nos meus arquivos.

Mutation proofs: `mutate.ps1 -Path src/winget_install/switches.rs -Anchor 'if setting == InstallerMode::Progress' -Replacement 'if false' -TestCommand 'cargo test --quiet --test it -- default_mode_is_silent_with_progress'` => teste falha sob a mutação (`assertion left == right failed ... 1 failed`), arquivo restaurado (0 ocorrências de `if false`). A linha `[OK] mutação PEGA` ficou fora do `tail -3` que usei; o orquestrador pode reexecutar. A âncora do brief (`mode_for` ignora o ajuste) foi escrita conforme o texto real após rustfmt.

Decisions:
- `plan()` manteve a assinatura (usa o padrão `progress`); novo `plan_with_mode(.., InstallerMode)`. Fiado só em `cli/run_win.rs::plan_install` (hunk de 1 chamada).
- Lista branca do worker (`msi_params_allowed`, `exe_params_allowed`) já aceitava `/passive` e `/SILENT` exatos: nenhuma mudança em `worker.rs`, só o teste `worker_allows_progress_switches_exactly`.
- Nova linha do plano: "mostra o progresso do instalador" / "sem janela".
- Ajuste novo na TUI (`SettingId::InstallerMode`, ORDER passou a 13); o teste existente `machine_msi_goes_through_helper` agora espera `/passive /norestart` (novo padrão).
- Arquivos compartilhados (plan.rs, run_win.rs, elevated_install.rs, wi_install_flow.rs) foram indexados a partir do blob HEAD + só meus trechos, para não levar edições da 196/197.

Concerns:
- O ajuste NÃO está ligado em: `engine::plan_upgrade` (engine.rs, 196), `tui/run.rs::native_install`/`native_upgrade` (197), `cli/run_win.rs::plan_upgrade`. Esses caminhos usam o padrão `progress` e ignoram `installer_mode=silent` até o orquestrador passar `behavior.installer_mode` a `plan_with_mode`.
- A cópia de trabalho de `wi_install_flow.rs` (edição da 196) não contém mais o teste msi que ajustei; o commit leva só a troca de 1 linha sobre o HEAD.
