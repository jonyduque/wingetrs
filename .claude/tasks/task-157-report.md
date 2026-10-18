Status: DONE_WITH_CONCERNS

Commits:
e6f7e85 feat(install): apply script effects through the phase 2a paths

Gate:
1st run (failed on my own clippy warning `bool_assert_comparison` in the test, fixed):
```
[OK]   rules 9s
[OK]   rules-selftest 16s
[OK]   fmt 3s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (test "it") due to 3 previous errors
```
2nd run (after the fix; the failure is not mine: the linker could not write a test exe another agent was running):
```
[OK]   rules 9s
[OK]   rules-selftest 16s
[OK]   fmt 3s
[OK]   clippy 27s
[FAIL] test exit 101
  error: linking with `link.exe` failed: exit code: 1104
  error: could not compile `wingetrs` (lib test) due to 1 previous error
```
Proof of my files instead (same tree, after the 2nd gate run):
- `cargo test --quiet --test it install_` -> `test result: ok. 488 passed; 0 failed; 19 ignored; 0 measured; 1019 filtered out; finished in 15.75s`
- `cargo test --quiet --lib install::` -> `test result: ok. 16 passed; 0 failed; ...`
- `cargo clippy --all-targets --quiet -- -D warnings` -> no output
The orchestrator should run the gate again.

Mutation proofs:
- `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if !is_inside(&entry, &app_dir) && !is_inside(&entry, &persist_dir) {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it install_script_effects::add_path_outside_app_rolls_back'`
  `[OK] mutação PEGA: 'if !is_inside(&entry, &app_dir) && !is_inside(&entry, &persist_dir) {' -> 'if false {' fez o teste falhar (exit 101).`
- SetEnv Undo (`-AnchorFile scratchpad/t157-anchor.txt` = the 4 lines `run.push(Undo::RestoreEnv { name: name.to_string(), previous: before, });` in `fn set_env`; replacement `let _ = ( name.to_string(), before, );`), test `install_script_effects::set_env_applied_restored_on_rollback`:
  `[OK] mutação PEGA: '    run.push(Undo::RestoreEnv {` (the script prints the first line of a multi-line anchor)
- Extra: the root check in Remove-Path, `-Anchor 'if !is_inside(seg, layout.root()) || !own_path_entry(run, seg) {' -Replacement 'if !own_path_entry(run, seg) {'`, test `remove_path_own_entry_outside_root_ignored`:
  `[OK] mutação PEGA: 'if !is_inside(seg, layout.root()) || !own_path_entry(run, seg) {' -> 'if !own_path_entry(run, seg) {' fez o teste falhar (exit 101).`
- Extra (coordinator's note): prefix families `-Anchor '    DENIED_ENV_PREFIXES.iter().any(|p| upper.starts_with(p))' -Replacement '    false'`, test `set_env_denied_families_refused`:
  `[OK] mutação PEGA: '    DENIED_ENV_PREFIXES.iter().any(|p| upper.starts_with(p))' -> '    false' fez o teste falhar (exit 101).`
- Extra: exceptions `-Anchor '    if ALLOWED_ENV.iter().any(|a| a.eq_ignore_ascii_case(name)) || upper.ends_with("_HOME") {' -Replacement '    if false {'`, test `set_env_allowed_exceptions_applied`:
  `[OK] mutação PEGA: '    if ALLOWED_ENV.iter().any(|a| a.eq_ignore_ascii_case(name)) || upper.ends_with("_HOME") {' -> '    if false {' fez o teste falhar (exit 101).`

Env name corpus (coordinator's request, measured): `python -I t157-envscan.py ~/scoop/buckets %LOCALAPPDATA%\wingetrs\buckets` (read-only) found 4705 manifests, 197 distinct variable names (277 uses) across `env_set` (base and architecture variants) and `Set-EnvVar`/`env` in hooks. Under the new rule 4 names / 8 uses fall in the refusal: `GIT_INSTALL_ROOT` (4, env_set), `PATHEXT` (2, python's script), `DOTNET_ROOT` (1, env_set), `NODE_PATH` (1, env_set). The rule applies only to script effects, so the 2a `env_set` ones are not affected. Of the 6 names set by scripts, 1 is refused: `PATHEXT` (the `python` manifest's post_install, which now fails with "a variável PATHEXT não pode ser gravada por script").

Decisions:
- Env names: an exact denylist (`DENIED_ENV`) plus prefix families (`DENIED_ENV_PREFIXES`) plus exceptions (`ALLOWED_ENV` and `*_HOME`). An exact deny always wins; an exception beats a family. The name must match `[A-Za-z_][A-Za-z0-9_]*` (max 255). I used a denylist rather than a strict allowlist because 197 names are in real use.
- An effect path is accepted only if it is `app_dir`/`persist_dir` itself or a path under it made of `safepath` components, compared by `norm`. `;`, `%`, `"` and control characters are refused. The entry is stored as written once the check passes.
- PATH `Add-Path` appends through the 2a `add_path` (it ignores `prepend`/`force`). `PSModulePath` puts the entry first with `RestoreEnv`, through `psmodulepath_prepend`, which is now shared with the 2a `psmodule`. `Remove-Path` on any variable other than PATH is ignored with a warning. Patterns longer than 1024 characters are refused.
- I pulled `set_env`, `make_shortcut`, `saved_shim_files` and `push_artifact` out of the 2a arms and use them for both. A duplicate artifact is not recorded twice, so an Env artifact keeps the oldest `previous`. Because `set_env` is shared, the `RestoreEnv` mutation also covers the 2a step.
- `tests/it/install_scripts.rs` (not in Files): the old test `effects_not_yet_supported_fail` became `unknown_effect_op_fails_unreadable` (it now asserts "efeitos do script ilegíveis"), and it holds the variant-hook refusal test. That test is also `variant_hook_refused_at_planning` in install_script_effects.rs.

Concerns:
- `PSModulePath` entries added by a script get no artifact, so uninstall does not remove them. That would need a new `ArtifactKind` in state.rs, which is outside Files. The 2a psmodule step has no Env artifact either.
- I ran `cargo fmt` once on the whole crate by mistake (it was meant for my files only). The gate's fmt passed, but if another agent's file was mid-edit and unformatted, fmt may have touched it.
- A script can `Remove-Path` the `<root>\shims` entry. It is recorded by this app and sits inside the root, as the brief allows. It is kept on the PATH while another app still records it.
- The shortcut link is not checked again against the root after `shortcut_link`. The name already went through `safe_components` (no `..`, drive or ADS), so a post-check could never fail and no test could reach it.
- External-data `.join`s: `PathBuf::from(path/target/icon)` only after `is_inside`; `planner::shortcut_link(root, name)` only after `safe_components`; `shims.join(format!("{name}.{ext}"))` only after `check_component("comando")`.

## Review fix (21487e4 fix(install): review of task 157)

Status: DONE_WITH_CONCERNS (starting point: HEAD 82cd322, which includes Task 160)

Changes:
1. `set_env` now uses an allowlist (`ALLOWED_ENV`) built from the measured corpus: 4705 manifests, 197 names, minus `variables` (a regex false positive), minus everything refused, plus `JAVA_HOME`/`GOPATH`/`ANDROID_SDK_ROOT` (your examples, whose buckets are not in my sample). Always refused, before the allowlist is checked: exact names (`DENIED_ENV`, now also proxy, CA bundle, `PHPRC`, `CLASSPATH`, `CARGO_HOME`, `RUSTUP_HOME`, `GRADLE_USER_HOME`) and prefixes (`DENIED_ENV_PREFIXES`, now also `WINGETRS_`, `SCOOP`, `SSL_CERT_`, `PIP_`, `NPM_CONFIG_`, `LUA_INIT`). A name not on the list fails with "a variável X não está entre as que um script pode gravar". `add_path.var` accepts only PATH.
2. A path-valued variable must stay inside the app. That covers names ending in `_HOME/_ROOT/_PATH/_DIR/_DIRECTORY/_PREFIX`, the names `GOPATH/GOROOT/SDKROOT/INCLUDE/LIB/HAXEPATH/HASHLINKPATH`, and any value containing `\`, `/` or `:`. Every `;`-separated part must pass `is_inside` against app_dir or persist_dir, otherwise the install fails with "fora da pasta do app" and rolls back.
3. `PSModulePath` in `Add-Path` is refused ("Add-Path em PSModulePath não suportado"). The test is now `psmodulepath_add_path_refused`.
4. The old `ALLOWED_ENV`/`*_HOME` exception is gone; `ALLOWED_ENV` is now the allowlist from item 1.
5. The cap test is renamed `effects_over_64_mib_refused_by_size_check_and_bounded_read`.
Tests: `set_env_allowed_names_applied` (JAVA_HOME=$dir\jdk, a PKG_CONFIG_PATH list, OPENSSL_CONF in persist, plain flags), `set_env_unlisted_name_refused`, `set_env_path_value_outside_app_refused` (JAVA_HOME=C:\qualquer, a relative value, a list with one part outside, a path in a flag-like variable, UNC), and the families/exact-name tests extended to the new entries. Tests that used `X`/`TOOL_HOME` now use variables from the list.

Corpus lists (measured, `python -I t157-envscan.py` + `t157-patch2.py`, read-only over ~/scoop/buckets and %LOCALAPPDATA%\wingetrs\buckets):
accepted (192): AIDER_DESK_DATA_DIR ALLURE_HOME ANDROID_HOME ANDROID_SDK_ROOT ANDROID_USER_HOME ANT_HOME ARCADEDB_HOME ATAC_MAIN_DIR AzureCLIPath AZURE_CLI_PATH BAT_CONFIG_DIR BAZEL_SH Boost_INCLUDE_DIR BOOST_ROOT BUN_INSTALL BUN_INSTALL_BIN BUN_INSTALL_GLOBAL_DIR CABAL_DIR CANGJIE_HOME CATALINA_BASE CATALINA_HOME CC65_HOME CCNET_CONF_DIR CHEAT_CONFIG_PATH CHROME_EXECUTABLE CLINK_DIR CMAKE_PREFIX_PATH CMDER_ROOT CODER_CONFIG_DIR CODEX_CLI_PATH CODEX_SPARKLE_ENABLED COMPOSER_CACHE_DIR COMPOSER_HOME ConEmuDir COQBIN CORMANLISP_HOME COURSIER_BIN_DIR COURSIER_CACHE CPLUS_INCLUDE_PATH CUDA_PATH CURSOR_CONFIG_DIR CURSOR_DATA_DIR C_INCLUDE_PATH DBX_DATA_DIR DEEPSTREAM_CONFIG_DIRECTORY EC2_HOME ELAN_HOME ERLANG_HOME ESPEAK_DATA_PATH FFMPEG_DIR FLUTTER_ROOT FNM_DIR FOLDER_MARKER_DIR FORKGITINSTANCE FVM_CACHE_PATH GAUGE_HOME GEM_HOME GEM_PATH GHCUP_INSTALL_BASE_PREFIX GHCUP_MSYS2 GHIDRA_INSTALL_DIR GLFW_ROOT GONAVI_DATA_ROOT GONAVI_LOG_DIR GOOGLE_WORKSPACE_CLI_CONFIG_DIR GOPATH GOROOT GRAILS_HOME GRAMPSHOME GRAMPS_DIR GROOVY_HOME G_EXPERIMENTAL G_HOME HADOOP_HOME HAPI_HOME HASHLINKPATH HAXEPATH IBMCLOUD_HOME INBUCKET_SMTP_TLSCERT INBUCKET_SMTP_TLSPRIVKEY INBUCKET_WEB_GREETINGFILE INBUCKET_WEB_UIDIR INCLUDE IPOPTWINDIR JABBA_HOME JANET_PATH JAVA_HOME JJUI_CONFIG_DIR JMETER_HOME JRUBY_HOME K9S_CONFIG_DIR K9S_LOGS_DIR KOTLIN_HOME KREW_ROOT KURRENTDB_DB LARKSUITE_CLI_CONFIG_DIR LazarusDir LazDir LD65_CFG LD65_LIB LIB LIBCLANG_PATH LLVM_LIB_DIR LUAROCKS_CONFIG LUA_DEV LUNARVIM_BASE_DIR LUNARVIM_CACHE_DIR LUNARVIM_CONFIG_DIR LUNARVIM_RUNTIME_DIR MAESTRO_HOME MAGICK_CODER_MODULE_PATH MAGICK_CONFIGURE_PATH MAGICK_HOME MICRONAUT_HOME MSBuildSDKsPath MSB_HOME NAVI_CONFIG NEKO_INSTPATH NGINX_HOME NO_MISTAKES_NO_UPDATE_CHECK NRFUTIL_HOME NSISDIR NULLCLAW_HOME NVM_HOME NVS_HOME OCI_LIB64 OPENCV_DIR OPENSSL_CONF OPENSSL_INCLUDE_DIR OPENSSL_LIB_DIR OPENSSL_MODULES OPENSSL_ROOT_DIR PGDATA PG_INCLUDE_DIR PG_LIB_DIR PHP_INI_SCAN_DIR PICOCLAW_HOME PKG_CONFIG_PATH PNPM_HOME PODMAN_CONNECTIONS_CONF PODMAN_DESKTOP_HOME_DIR POSH_THEMES_PATH PROTO_HOME PROTO_VERSION_CHECK PROXYCHAINS_CONF_FILE PSCOMPLETIONS_DATA_DIR PURPLEHOME PYENV PYENV_HOME PYENV_ROOT RABBITMQ_SERVER RIPCORD_ALLOW_UPDATES RUSTIC_HOME RYE_HOME SBCL_HOME SBT_HOME SCALA_HOME SCROLLVIEW_PATH SDKROOT SNDCPY_APK SPARK_HOME STEAMGUARD_CLI_MAFILES SUMO_HOME TABBY_ROOT TEALDEER_CONFIG_DIR TELEGRAF_CONFIG_PATH TESSDATA_PREFIX THINKRAIL_DATA_DIR THONNY_USER_DIR TLRC_CONFIG TL_PATH TNS_ADMIN TRILIUM_DATA_DIR TurboJPEG_ROOT UV_CACHE_DIR UV_PYTHON_BIN_DIR UV_PYTHON_INSTALL_DIR UV_TOOL_BIN_DIR UV_TOOL_DIR VCPKG_ROOT VK_SDK_PATH VLC VOLTA_HOME VULKAN_SDK WASMER_CACHE_DIR WASMER_DIR WEBVIEW2_BROWSER_EXECUTABLE_FOLDER WINSSHTERM_EXE_DISALLOW_AUTO_UPDATE WIRESHARK_CONFIG_DIR WIRESHARK_DATA_DIR YARR_DB YARR_LOGFILE
refused (7): CARGO_HOME DOTNET_ROOT GIT_INSTALL_ROOT GRADLE_USER_HOME NODE_PATH PATHEXT RUSTUP_HOME

Mutation proofs:
- `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if !own_shim(run, &name) {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it install_script_effects'`
  `[OK] mutação PEGA: 'if !own_shim(run, &name) {' -> 'if false {' fez o teste falhar (exit 101).`
- `-Anchor 'if !ALLOWED_ENV.iter().any(|a| a.eq_ignore_ascii_case(name)) {' -Replacement 'if false {'`, test `set_env_unlisted_name_refused`:
  `[OK] mutação PEGA: 'if !ALLOWED_ENV.iter().any(|a| a.eq_ignore_ascii_case(name)) {' -> 'if false {' fez o teste falhar (exit 101).`
- `-Anchor '&& (env_is_path(&name) || v.contains([' -Replacement '&& (env_is_path(&name) && false || v.contains(['`, test `set_env_path_value_outside_app_refused`:
  `[OK] mutação PEGA: '&& (env_is_path(&name) || v.contains([' -> '&& (env_is_path(&name) && false || v.contains([' fez o teste falhar (exit 101).`
- `-Anchor 'if !is_inside(part, &app_dir) && !is_inside(part, &persist_dir) {' -Replacement '... && false {'`, same test:
  `[OK] mutação PEGA: 'if !is_inside(part, &app_dir) && !is_inside(part, &persist_dir) {' -> 'if !is_inside(part, &app_dir) && !is_inside(part, &persist_dir) && false {' fez o teste falhar (exit 101).`
- Exact denylist (`DENIED_ENV.iter()... ||` -> `false ||`), test `set_env_denied_exact_names_refused`: `[OK] mutação PEGA: '    DENIED_ENV.iter().any(|d| d.eq_ignore_ascii_case(name))`
(My first tries at mutations 3 and 4 were INCONCLUSIVO: the mutant did not compile because it left an unused variable or function. I reran them with replacements that compile.)

Gate (one run): the fmt step failed in another agent's file, not mine:
```
[OK]   rules 11s
[OK]   rules-selftest 33s
[FAIL] fmt exit 1
  Diff in \?\C:\Users\jonyd\Projetos\wingetrs\src\bridge\worker.rs:7:
```
My files: `rustfmt --check` on executor.rs and install_script_effects.rs is ok; `cargo clippy --all-targets -- -D warnings` shows nothing in install/; `cargo test --quiet --test it install_` -> `test result: ok. 489 passed; 0 failed; 19 ignored; 0 measured; 1047 filtered out; finished in 33.36s`.

Concerns: python's real post_install (`PATHEXT`) and env_set names like `CARGO_HOME`/`DOTNET_ROOT` are now refused if a script sets them; 2a `env_set` does not go through this check. The "value looks like a path" rule also refuses URLs (`http://...`) in variables on the list.

## Adjustment 2 (6a8b438 fix(install): PATHEXT append-only and https values in script effects)

Status: DONE_WITH_CONCERNS. Gate: `[OK]   test (1597 passed, 85 ignored) 61s` / `[OK]   gate verde`.

- PATHEXT: a script may now only append. The new value must be the current user value (same extensions, same order) followed by extensions matching `\.[A-Za-z]{2,5}` that are not already present and not repeated. Any other form is refused with "o script só pode acrescentar extensões no fim do PATHEXT". That includes a removal, a reorder, re-adding `.EXE`, `None`, a literal `%PATHEXT%;...`, and a machine with no user PATHEXT. The last case is refused because a user value without `.EXE` would hide the system one. Undo restores the previous value and uninstall restores it from the Env artifact.
- Python from the catalog (measured, treated as text only): its PATHEXT lines are in `installer.script` (not post_install), behind `if ($global)` and with `-Global`. The planning gate already refuses them ("usa -Global"). In user mode Scoop does not touch PATHEXT for python. The test `python_catalog_pathext_script` proves both: the catalog text is refused, and the same lines without `-Global` pass the gate with the effect accepted.
- Address variables: the corpus has no `*_URL`/`*_MIRROR` names and no `env_set` value containing `://` (measured: 0). So `URL_ENV` is an explicit list with only `ELECTRON_MIRROR`, which is not from the corpus; trim it if you prefer. Its value must be `https://` plus a non-empty rest with no `;` `%` `"` `\`, whitespace or control characters; `http://`, `file://` and paths are refused. Other `*_URL` names fall back to the allowlist and are refused.
- New refused families: `UV_`, `POETRY_`, `CONDA_`, `BUN_`, `YARN_`, `GOPROXY`, `GOPRIVATE`, `GONOSUMDB`, `GOFLAGS`, `RUSTUP_DIST_`, `ELECTRON_GET_`. `npm_config_` was already covered by `NPM_CONFIG_` (case-insensitive). These families remove 8 corpus names from the allowlist: `BUN_INSTALL`, `BUN_INSTALL_BIN`, `BUN_INSTALL_GLOBAL_DIR`, `UV_CACHE_DIR`, `UV_PYTHON_BIN_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_TOOL_BIN_DIR`, `UV_TOOL_DIR`. All are from `env_set` (the 2a path), which this check does not affect.

Mutation proofs (all `[OK] mutação PEGA`):
- `if new.len() < cur.len() || new[..cur.len()] != cur[..] {` -> `if new.len() < cur.len() {` (pathext_other_forms_refused)
- `&& !cur.contains(ext)` -> `&& true` (same test)
- `&& !added[..i].contains(ext);` -> `&& i < usize::MAX;` (same test)
- `let valid = (2..=5).contains(&letters.len())` -> `(1..=9)` (same test)
- `&& !is_https_url(v)` -> `&& !is_https_url(v) && false` (url_var_not_https_refused)
- `if env_is_url(name) { return Ok(());` -> `if false {` (url_var_https_accepted)
- removed `"UV_",` from the prefixes (set_env_denied_families_refused)
Earlier, 3 attempts were INCONCLUSIVO (the build was broken by another agent at the time). I reran them and they are PEGA.
