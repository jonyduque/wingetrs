Status: DONE_WITH_CONCERNS

Commits:
7511c23 feat(winget-install): MSIX add and remove through PackageManager

Gate:
```
[OK]   rules 6s
[OK]   rules-selftest 15s
[OK]   fmt 2s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (test "it") due to 1 previous error
  (log completo: target/verify.log)
```
A falha é só em arquivos de outros agentes (target/verify.log):
`src\install\executor.rs:1463:9: error: unused variable: others` e
`tests\it\wi_bridge_install.rs:410:37: error: unnecessary use of clone ...`.
Prova dos meus arquivos:
- `cargo clippy --quiet --all-targets --message-format short -- -D warnings 2>&1 | grep -E "msix|^error: could"` ⇒ só `error: could not compile wingetrs (test "it") due to 1 previous error` (o erro único é o do wi_bridge_install; nenhuma linha com msix).
- `cargo test --quiet --test it wi_msix::` ⇒ `test result: ok. 5 passed; 0 failed; 1 ignored; 0 measured; 1284 filtered out; finished in 0.00s`

Mutation proofs:
1. Espaço não codificado em `file_uri`:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/msix.rs -Anchor "b.is_ascii_alphanumeric() || b\"-._~" -Replacement "b == b' ' || b.is_ascii_alphanumeric() || b\"-._~" -TestCommand 'cargo test --quiet --test it wi_msix::file_uri_encodes'`
   `[OK] mutação PEGA: 'b.is_ascii_alphanumeric() || b"-._~' -> 'b == b' ' || b.is_ascii_alphanumeric() || b"-._~' fez o teste falhar (exit 101).`
2. Remover só o primeiro full name: o laço foi extraído para a função pura `msix::remove_each(full_names, remove)`, que `remove_package` usa; testes `remove_each_removes_every_full_name` e `remove_each_stops_at_first_error`.
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/msix.rs -Anchor "    for name in full_names {" -Replacement "    for name in full_names.iter().take(1) {" -TestCommand 'cargo test --quiet --test it wi_msix::remove_each'`
   `[OK] mutação PEGA: '    for name in full_names {' -> '    for name in full_names.iter().take(1) {' fez o teste falhar (exit 101).`
   Limite: uma regressão em `msix_win.rs` que montasse `full_names` com só o primeiro pacote da família só a prova ao vivo (`live_add_and_remove`, com 2ª remoção esperando 0) alcança.
3. Gate: ver acima (vermelho só por arquivos alheios).

Decisions:
- `MSIXInstallFlow.cpp` não existe na referência; o fluxo MSIX está em `AppInstallerCLICore_Workflows_InstallFlow.cpp::MsixInstall` + `Deployment.cpp::AddPackageWithDeferredFallback`. O winget **não fecha o app à força** (sem `ForceApplicationShutdown`), mas em `ERROR_PACKAGES_IN_USE` ele faz `StagePackageAsync` + `RegisterPackageByFullNameAsync` e, se o registro ainda der "em uso", marca "registro adiado" (o app novo entra quando o antigo fechar) e trata como sucesso. Nós devolvemos o erro (`PackageInUse`, "feche-o e tente de novo"). **Divergência para levar ao usuário.** O winget também usa `RequestAddPackageAsync` (SmartScreen) e `RetainFilesOnFailure`; não portado.
- `file_uri` aceita só caminho com letra de unidade (`X:\`/`X:/`), tira o prefixo `\\?\`; UNC, `C:x`, `\d\x` e não-UTF-8 ⇒ `None`. Mantém literais só `A-Za-z0-9-._~!$&'()*+,;=:@/`; o resto vira `%XX` maiúsculo (inclui espaço, `#`, `%`, `?`, `[`, `]`, `^` etc.).
- `wait` trata as duas formas de falha: `join()` com `Err` (HRESULT + mensagem) e `DeploymentResult.ExtendedErrorCode` falho (código + `ErrorText`), como `WaitForDeployment`.
- `remove_each` para no primeiro erro e devolve o erro (como o `THROW` do winget); `MsixError` deriva `thiserror::Error`, `Default`, `Clone`, `PartialEq`.
- O teste ao vivo exige também `WINGETRS_TEST_MSIX_FAMILY` (a família não é lida do arquivo; ler o AppxManifest seria porte além do brief).

Concerns:
- `outcome()` usa a mensagem genérica de `codes::message` ("o pacote está em uso; feche-o e tente de novo"); o texto com o nome do app ("feche <app> e tente de novo") precisa ser montado por quem chama, que conhece o nome.
- Não verificado ao vivo (`live_add_and_remove` não rodado: precisa de um `.msix` assinado indicado pelo usuário). A inferência `None` no `dependencyPackageUris` compila (o crate `windows` 0.62 não reexporta `IIterable`/`IAsyncOperationWithProgress`; nenhum crate novo).
- Sem nova feature do `windows`.

---

## Follow-up (orchestrator decision: port deferred registration)

Status: DONE_WITH_CONCERNS

Commits:
176ffb9 feat(winget_install): deferred MSIX registration when the app is in use

Gate (red only in other agents' files):
```
[OK]   rules 7s
[OK]   rules-selftest 18s
[OK]   fmt 3s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (test "it") due to 5 previous errors
  (log completo: target/verify.log)
```
The errors are all in `tests\it\prelude_effects.rs` (missing field `force`). My files: `cargo clippy --all-targets --message-format short -- -D warnings` (run just before) showed only `tests\it\wi_bridge_install.rs:410:37` and nothing in msix; `cargo test --quiet --test it wi_msix::` ⇒ `test result: ok. 14 passed; 0 failed; 1 ignored; 0 measured; 1286 filtered out; finished in 0.00s`.

Mutation proofs (all `-Path src/winget_install/msix.rs`, `-TestCommand 'cargo test --quiet --test it wi_msix::<filter>'`):
- `&& b[1] == b':'` -> `&& true` (filter file_uri_query): `[OK] mutação PEGA: '&& b[1] == b':'' -> '&& true' fez o teste falhar (exit 101).`
- cleanup guard `&& !was_installed` -> `&& true` (deferred_no_cleanup_when_already): `[OK] mutação PEGA: '        && !was_installed' -> '        && true' fez o teste falhar (exit 101).`
- unknown install state => `true` changed to `false` (deferred_no_cleanup_when_install_state): `[OK] mutação PEGA: '        true` (multiline anchor; the script printed only the first line)
- register in use => Deferred disabled (`if false`) (deferred_register_in_use): `[OK] mutação PEGA: '        Err(e) if e.hresult == ERROR_PACKAGES_IN_USE => Ok(AddResult::Deferred),' -> '        Err(e) if false => Ok(AddResult::Deferred),' fez o teste falhar (exit 101).`
- cleanup removed (`if result.is_err()` -> `if false`) (deferred_): `[OK] mutação PEGA: '    if result.is_err()' -> '    if false' fez o teste falhar (exit 101).`
- add in use no longer triggers the fallback (deferred_in_use_stages): `[OK] mutação PEGA: '        Err(e) if e.hresult == ERROR_PACKAGES_IN_USE => e,' -> '        Err(e) if false => e,' fez o teste falhar (exit 101).`

Decisions:
- The pure decision lives in `msix::add_with_deferred_fallback(&impl Deployer)` (steps add/full_name/is_installed/stage/register/remove); `msix_win::WinDeployer` implements it with `PackageManager`. `MsixOps::add` now returns `Result<AddResult{Installed|Deferred}, MsixError>`, and `AddResult::user_text()` gives "o app será atualizado quando for fechado" for `Deferred`.
- The full name is read only on the in-use path (winget reads it up front). If it can't be read, the original in-use error comes back (the user closes the app and retries) and a `tracing::warn` is logged.
- Bundle: `file_full_name` tries `IAppxFactory::CreatePackageReader` first, then `IAppxBundleFactory::CreateBundleReader`. For a bundle it uses the bundle's own full name (`IAppxBundleManifestReader::GetPackageId`), as winget's `MsixInfo` does. Not tested live.
- Cleanup guard: `FindPackageByUserSecurityIdPackageFullName` before the stage. Ok ⇒ already installed; a null result (windows-rs error with S_OK) ⇒ not installed; any other error ⇒ unknown, treated as installed (no cleanup). Unlike winget, there is no cleanup when the first `AddPackageAsync` fails (as the decision asked: only stage/register failures).
- COM for the Appx factories is MTA (`ComInit` with `RPC_E_CHANGED_MODE` tolerated, since the factories are free-threaded). Stream via `SHCreateStreamOnFileEx` (`STGM_READ|STGM_SHARE_DENY_WRITE`). No new crate or feature.

Concerns:
- Nothing tested live: Appx readers, `FindPackageByUserSecurityIdPackageFullName` returning null, stage+register and bundle registration by bundle full name. `live_add_and_remove` only covers the normal path.
- `RequestAddPackageAsync` (SmartScreen) and `RetainFilesOnFailure` are still not ported.
