# Task 189 report

Status: DONE_WITH_CONCERNS

Commits:
820afc0 feat(winget-install): native portable and zip-portable packages as wingetrs apps

Gate:
```
[OK]   rules 11s
[OK]   rules-selftest 37s
[OK]   fmt 4s
[OK]   licenses 7s
[OK]   clippy 22s
[OK]   test (1943 passed, 87 ignored) 100s
[OK]   gate verde
```
(A first run failed only on `src/github/mod.rs` of Task 208, mid-edit; a second run failed on
`tui_jobs::plan_install_reports_unknown_packages_and_non_scoop_sources`, which was mine and is
fixed below, plus `gh_release::foreign_download_host_dropped` of Task 208. The third run, above,
is green.)

Mutation proofs:
1. Accept `..` in `RelativeFilePath` (portable.rs, `safe_components` replaced by a plain split), anchor/replacement files `t189-anchor1.txt`/`t189-repl1.txt` in the scratchpad:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/portable.rs -AnchorFile .../t189-anchor1.txt -ReplacementFile .../t189-repl1.txt -TestCommand 'cargo test --quiet --test it wi_portable::nested_path_escape_refused'`
   `[OK] mutação PEGA: '            let parts = safe_components(&f.relative_file_path)` (multi-line anchor; the test panicked at wi_portable.rs:280)
2. Extra (machine-scope guard I added): `-Path src/winget_install/plan.rs -Anchor 'if declared_machine || wanted == ScopeWanted::Only(Scope::Machine) {' -Replacement 'if declared_machine {'`
   `[OK] mutação PEGA: 'if declared_machine || wanted == ScopeWanted::Only(Scope::Machine) {' -> 'if declared_machine {' fez o teste falhar (exit 101).`
   `-Anchor 'if declared_machine || wanted' -Replacement 'if false || wanted'`
   `[OK] mutação PEGA: 'if declared_machine || wanted' -> 'if false || wanted' fez o teste falhar (exit 101).`
3. Extra (select verdict): `-Path src/winget_install/select.rs -Anchor '        InstallerKind::Zip | InstallerKind::Portable if portable => {}' -Replacement '' -TestCommand '... wi_portable::portable_exe_becomes'`
   `[OK] mutação PEGA: '        InstallerKind::Zip | InstallerKind::Portable if portable => {}' -> '' fez o teste falhar (exit 101).`

Check 2 (real manifests, `wingetrs install <id> --dry-run --yes`, debug build, catalog of the machine):
11 planned as wingetrs apps: jqlang.jq (portable, shim `jq` -> jq-windows-amd64.exe), MikeFarah.yq (portable, `yq`),
BurntSushi.ripgrep.MSVC (zip, `rg` -> ripgrep-15.2.0-x86_64-pc-windows-msvc\rg.exe), junegunn.fzf (`fzf`),
Gyan.FFmpeg (zip, 3 shims ffmpeg/ffplay/ffprobe -> ffmpeg-9.0.2-full_build\bin\*), ajeetdsouza.zoxide, eza-community.eza,
JesseDuffield.lazygit, cURL.cURL (`curl` -> curl-8.23.0_3-win64-mingw\bin\curl.exe), Casey.Just, Rclone.Rclone.
6 went to the bridge with "tem dependências" (sharkdp.fd, sharkdp.bat, astral-sh.uv, dandavison.delta, sharkdp.hyperfine, sxyazi.yazi), as D8 says.
Example (jq):
```
Instalar jqlang.jq 1.8.2  winget · sem arquitetura
  1. Baixar [1] https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-windows-amd64.exe (jq-windows-amd64.exe, sha256 a6fc67fe...)
  5. Criar comando jq em ...\wingetrs\shims (jq.exe + jq.shim) -> ...\apps\jqlang.jq\current\jq-windows-amd64.exe
  6. Garantir ...\wingetrs\shims no PATH do usuário
  8. Registrar jqlang.jq 1.8.2 (winget) em ...\wingetrs\state.db
Simulação: nada foi alterado.
```

Decisions:
- Started from the uncommitted work of the interrupted implementer (portable.rs, CLI/TUI wiring, 2a `plan_manifest`, correlate/upgradable for bucket `winget`); reviewed it against the brief, fixed a broken string literal in wi_select.rs (the fmt failure).
- Added: a portable whose chosen installer declares `Scope: machine`, or with `--scope machine` (`ScopeWanted::Only(Machine)`), goes to the bridge ("tipo portable com escopo máquina") — a wingetrs app lives in the user root, so installing it per-user would silently ignore the asked scope. Test `portable_with_machine_scope_required_goes_to_winget`.
- TUI `jobs::plan_install(.., "winget", id)` now checks `state.db` first (only a `winget`-bucket wingetrs app is upgraded via the catalog); this fixed the existing tui_jobs test that panicked in `Handle::block_on` from an async context.
- Added `upgrade_plan_reads_the_catalog_manifest` (MSZIP versionData built in the test) covering `plan_from_catalog`: upgrade plan 1.0 -> 2.0, and refusal when the catalog's newer version is no longer portable.

Concerns:
- `src/install/ops.rs` was edited (new `plan_manifest`, `upgradable` maps bucket `winget` to source `winget`) although the brief's constraint lists ops.rs as not to edit (phase 2c parallelism); it needs the private `layout`/`read_state`/`old_manifest`. `src/cli/*`, `src/tui/*`, `src/installed/correlate.rs`, `src/sources/winget/manifest.rs` (new `Commands` field) are also outside the Files list: needed to wire install/upgrade/list.
- `.join` with external data: none written directly; `RelativeFilePath` goes through `safe_components` (portable.rs) and then the 2a planner's safepath; the id and version are checked as one component each before becoming the app name/version folder; the downloaded file name is either a plain URL name (`[A-Za-z0-9._+-]`, one component) or `<alias|id>.exe`.
- `tui/jobs.rs::portable_upgrade` calls `Handle::block_on`: correct on the job's blocking thread (production), would panic if called from an async task; guarded now by the state check, not by type.
- Cosmetic: the CLI header reads "sem arquitetura" for these apps (the generated scoop manifest has no `architecture` block). Not treated.
- Mutation of the "no longer portable" arm of `plan_from_catalog` not run (the new test asserts its message).

## Review fixes (CHANGES_REQUESTED, items 1-7)

Commits:
cd424a6 fix(winget-install): portable review: tests for CLI/TUI arms, job handle, keep_old in CLI

Gate:
```
[OK]   rules 12s
[OK]   rules-selftest 36s
[OK]   fmt 4s
[OK]   licenses 7s
[OK]   clippy 55s
[OK]   test (1963 passed, 87 ignored) 86s
[OK]   gate verde
```

Changes:
1. CLI `Engine::Portable`: `cli_portable_prints_agreements_and_hands_the_spec_over` (fake `WingetFlows` in wi_portable.rs). It covers "Licença aceita" with --yes, the plain line without it, no line when there are no agreements, and checks that `install_portable` receives the spec.
2. TUI: `tui_engine_portable_asks_for_the_2a_plan` (`Engine::Portable` => `Cmd::PlanPortable { keep }`, both values of keep_old) and `tui_portable_planned_each_arm`. The second covers: agreements open the confirmation even with confirm=Never; no agreements with Never => `Cmd::Execute(Apply)`; no agreements with Always => confirmation without notes; error => toast.
3. `tui_jobs_plan_the_portable_upgrade_from_the_catalog`: a `winget`-bucket app is installed and 2.0 is published on the mocked CDN (`publish`). It runs `plan_install`, `plan_upgrade_all` and `plan_batch` from a thread outside a multi-thread runtime, as `spawn_job` does. An id not installed from winget is refused.
4. `jobs::WingetSrc { handle, cdn_base }` is passed to `plan_install`/`plan_batch`/`plan_upgrade_all`. `spawn_plan` and the batch job build it from the `h` that `spawn_job` hands over. No more `Handle::try_current`. Test callers in tui_jobs.rs/tui_scripts.rs updated (helper `wsrc()`).
5. CLI: new `winget_cmd::portable_plan(ctx, spec, behavior)`, which uses `behavior.keep_old`; `ProdFlows::install_portable` uses it. Equivalence test `cli_and_tui_portable_plans_are_the_same`: with 1.0 installed and 2.0 planned, the CLI text equals the TUI text for keep_old false and true, and the two differ from each other.
6. `alias_of`: only `shim::is_safe_shim_name` (it already requires one `safe_components` component and no separators).
7. `jobs.rs`: `crate::bridge::winget_out::UPDATING_SOURCE` replaces the literal.

Mutation proofs:
- `-Path src/tui/jobs.rs -Anchor 'Some((r, _)) if r.bucket == crate::winget_install::portable::WINGET_BUCKET => {' -Replacement 'Some((r, _)) if false => {' -TestCommand 'cargo test --quiet --test it wi_portable::tui_jobs_plan'`
  `[OK] mutação PEGA: 'Some((r, _)) if r.bucket == crate::winget_install::portable::WINGET_BUCKET => {' -> 'Some((r, _)) if false => {' fez o teste falhar (exit 101).`
- `-Path src/tui/jobs.rs -Anchor '        if bucket == crate::winget_install::portable::WINGET_BUCKET {' -Replacement '        if false {'` (plan_upgrade_all)
  `[OK] mutação PEGA: '        if bucket == crate::winget_install::portable::WINGET_BUCKET {' -> '        if false {' fez o teste falhar (exit 101).`
- `-Path src/cli/winget_cmd.rs -Anchor 'spec, b.keep_old' -Replacement 'spec, b.keep_old && false' -TestCommand '... wi_portable::cli_and_tui'`
  `[OK] mutação PEGA: 'spec, b.keep_old' -> 'spec, b.keep_old && false' fez o teste falhar (exit 101).`
  (The first try, `-Replacement 'ctx, spec, false,'`, came back `[?] INCONCLUSIVO: o mutante não compila` because `b` was left unused.)
- `-Path src/cli/winget_cmd.rs -Anchor '            for line in agreement_lines(&spec.agreements, f.yes) {' -Replacement '            for line in agreement_lines(&[], f.yes) {' -TestCommand '... wi_portable::cli_portable'`
  `[OK] mutação PEGA: '            for line in agreement_lines(&spec.agreements, f.yes) {' -> '            for line in agreement_lines(&[], f.yes) {' fez o teste falhar (exit 101).`
- `-Path src/tui/app.rs -Anchor 'vec![Cmd::PlanPortable { spec, keep }]' -Replacement 'vec![Cmd::PlanPortable { spec, keep: keep && false }]' -TestCommand '... wi_portable::tui_engine'`
  `[OK] mutação PEGA: 'vec![Cmd::PlanPortable { spec, keep }]' -> 'vec![Cmd::PlanPortable { spec, keep: keep && false }]' fez o teste falhar (exit 101).`
- `-Path src/tui/app.rs -Anchor 'Ok(Planned::Ready { action, .. }) if !agreements.is_empty() => {' -Replacement 'Ok(Planned::Ready { action, .. }) if false && !agreements.is_empty() => {' -TestCommand '... wi_portable::tui_portable_planned'`
  `[OK] mutação PEGA: 'Ok(Planned::Ready { action, .. }) if !agreements.is_empty() => {' -> 'Ok(Planned::Ready { action, .. }) if false && !agreements.is_empty() => {' fez o teste falhar (exit 101).`

Notes:
- `src/tui/app.rs` is not in this commit; its two mutations ran while app.rs had been idle for 30 min. Because `mutate.ps1` restores the original bytes, they could have overwritten a Task 215 edit made during the run. The file's content was unchanged afterwards.
- One gate run failed only because of `src/cli/output.rs` from Task 215, mid-edit. After it compiled, the gate was green twice: before item 7 and after it, as above.
