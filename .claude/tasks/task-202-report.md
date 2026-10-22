# Task 202 report

Status: DONE_WITH_CONCERNS

Commits:
```
ebba7af feat(bridge): elevated side derives the installer hash from the signed catalog and runs it from admin-only staging
```

Gate:
```
[OK]   rules 24s
[OK]   rules-selftest 82s
[OK]   fmt 6s
[OK]   licenses 8s
[OK]   clippy 3s
[OK]   test (2064 passed, 90 ignored) 79s
[OK]   gate verde
```
(The first run failed only on the `cli_color` snapshots, which carried the old plan text; snapshots updated, second run above.)

Mutation proofs:
1. Skip the msix signature check:
   `scripts/mutate.ps1 -Path src/bridge/signed_install.rs -Anchor 'let signer = ctx.verifier.verify(&msix).map_err(SignedError::Signature)?;' -Replacement 'let signer = Signer { common_name: SIGNER_NAME.into(), organization: SIGNER_NAME.into(), not_before: ctx.now };' -TestCommand 'cargo test --quiet --test it elevated_install::signed::refused_signature_downloads_no_installer'`
   `[OK] mutação PEGA: 'let signer = ctx.verifier.verify(&msix).map_err(SignedError::Signature)?;' -> 'let signer = Signer { common_name: SIGNER_NAME.into(), organization: SIGNER_NAME.into(), not_before: ctx.now };' fez o teste falhar (exit 101).`
2. SHA-256 not from the manifest. The new request has no SHA field, so two forms:
   a. Accept the old `sha256` field in the request (drop `deny_unknown_fields`):
   `scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor '#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]' -Replacement '#[serde(tag = "op", rename_all = "snake_case")]' -TestCommand 'cargo test --quiet --test it elevated_install::signed::install_package_protocol'`
   `[OK] mutação PEGA: '#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]' -> '#[serde(tag = "op", rename_all = "snake_case")]' fez o teste falhar (exit 101).`
   b. Stop comparing the download with the manifest's SHA-256 (the served installer differs from it):
   `scripts/mutate.ps1 -Path src/bridge/signed_install.rs -Anchor 'if file_sha256(&installer)? != sha {' -Replacement 'if file_sha256(&installer)?.is_empty() {' -TestCommand 'cargo test --quiet --test it elevated_install::signed::installer_not_matching_the_manifest_is_refused'`
   `[OK] mutação PEGA: 'if file_sha256(&installer)? != sha {' -> 'if file_sha256(&installer)?.is_empty() {' fez o teste falhar (exit 101).`
3. Run the installer outside the staging folder:
   a. `scripts/mutate.ps1 -Path src/bridge/signed_install.rs -Anchor 'dir: run_dir.to_path_buf(),' -Replacement 'dir: run_dir.parent().unwrap_or(run_dir).to_path_buf(),' -TestCommand 'cargo test --quiet --test it elevated_install::signed::signed_catalog_stages_the_manifest_installer'`
   `[OK] mutação PEGA: 'dir: run_dir.to_path_buf(),' -> 'dir: run_dir.parent().unwrap_or(run_dir).to_path_buf(),' fez o teste falhar (exit 101).`
   b. Real runner (`.cmd` that writes `%CD%`/`%TEMP%`): `scripts/mutate.ps1 -Path src/bridge/staging_win.rs -Anchor '        &staged.dir,' -Replacement '        &sys.system32,' -TestCommand 'cargo test --quiet --test it elevated_install::win::staged_installer_runs_in_its_run_folder'`
   `[OK] mutação PEGA: '        &staged.dir,' -> '        &sys.system32,' fez o teste falhar (exit 101).`
4. Extra:
   `[OK] mutação PEGA: '            ctx.gate.next_plan();' -> '' fez o teste falhar (exit 101).` (wi_install_flow::no_helper_falls_back_to_runas)
   `[OK] mutação PEGA: '        check_identity(&bytes)?;' -> '' fez o teste falhar (exit 101).` (elevated_install::signed::foreign_identity_is_refused)
   `[OK] mutação PEGA: '    elevation_allowed(plan, cmd, &sha)?;' -> '' fez o teste falhar (exit 101).` (wi_install_flow::runas_refuses_params_outside_the_allowlist)

Decisions:
- `WTHelperProvDataFromStateData`/`WTHelperGetProvSignerFromChain` need the features `Win32_Security_Cryptography_Catalog` and `_Sip` (not authorized). So: `WinVerifyTrust` (only `Win32_Security_WinTrust`) checks the whole package; the signer certificate comes from `AppxSignature.p7x` (read through `zip`, decoded by `CryptQueryObject`, no DER parsing of ours) and is chained with `CertGetCertificateChain` + `CERT_CHAIN_POLICY_MICROSOFT_ROOT` (application-root flag), as of its `NotBefore`. No revocation and cache-only URL retrieval (a CryptoAPI fetch would follow the user's proxy). `Cargo.lock` unchanged.
- Recency floor: the signer's `NotBefore` at most 7 days old (`CATALOG_MAX_AGE`), checked in the platform-neutral `check_signer` (testable). `Identity Name` checked in `AppxManifest.xml`; `CN` and `O` must be `Microsoft Corporation`.
- Request = `{id, version, installer, scope, mode, upgrade}`: the brief's four fields plus `mode` (closed enum of `switches::Mode`) and `upgrade` (bool), because the helper rebuilds the parameters with `install_args_unlogged`. The unelevated side sends the `upgrade` flag that rebuilds the plan's own command line and refuses a plan whose command line differs (equivalence by construction, plus `helper_command_line_equals_the_plan`).
- Reserve (step 4) used only when the helper cannot exist this session (`ElevatedError::NotAdminFolder`: wingetrs outside an administrator-only folder, Task 197). Without it, users with a per-user wingetrs could no longer install machine-scope packages. The launch gate is freed again before the fallback's download. A refused helper UAC is final (`UacRefused`), no second prompt.
- No local download on the helper path (the helper downloads its own copy); the uninstall-first upgrade still downloads and verifies locally before removing anything (the "failed download removes nothing" rule), so it downloads twice. New phase `HelperPreparing` ("Baixando e conferindo o instalador de X como administrador", cannot be cancelled), handled by every `Phase` consumer (CLI 3 matches, TUI 2). Plan text: one `ADMIN_PROMPT` ("Roda como administrador (pede permissão de administrador)") for msi and exe; `ADMIN_PROMPT_MSI` removed.

Concerns:
- The real `WinTrustVerifier` path was NOT run against the real catalog (asked the orchestrator; no answer). Run once: `cargo test --release --test it elevated_install::win::live_real_catalog_verifies -- --ignored` (downloads source2.msix, verifies signature, signer, age, identity, and a 1-byte change). The non-ignored test only proves refusal of unsigned files. If it fails in production, elevated installs fail with an error (they do not fall back to runas).
- Files outside the brief's list: `Cargo.toml` (feature), `scripts/check_rules*.ps1` (asked), `src/bridge/mod.rs`, `src/bridge/elevated.rs`, `src/winget_install/plan.rs`, `launch.rs` (docs), `src/install/progress.rs`, `src/cli/output.rs`, `src/tui/app.rs`, `src/tui/jobs.rs` (new phase), tests `wi_install_flow`, `wi_upgrade`, `wi_tui`, `install_progress`, two `cli_color` snapshots.
- `docs/cli-textos.md` still lists the old prompt texts; it has someone else's uncommitted edit, so I did not touch it. CLAUDE.md's 2d exception still says machine-scope native installers ask for their own UAC; now that is only the fallback.
- The installer download on the elevated side has no size cap (as on the unelevated side); a hostile download host could fill the disk under Program Files before the SHA check.
- Each 1618 retry is a new request: the helper downloads the catalog and the installer again.
- `.join` with external data: none on disk (run-folder names are constants); `rel_path` from `versionData` goes into a URL only, after `valid_rel_path`; `id` is `valid_target` and must exist in the signed index.
