Status: DONE_WITH_CONCERNS

Commits:
324f1d5 docs(armadilhas): cargo fmt -- <files> formats the whole crate
573d4d7 feat(cli): self update from the GitHub release

Gate: (uma execução; falhou só no fmt de `src/tui/app.rs`, arquivo da Task 218 em andamento; não repeti)
```
[OK]   rules 13s
[OK]   rules-selftest 44s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\tui\app.rs:1:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\tui\app.rs:11:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\tui\app.rs:4230:
  (log completo: target/verify.log)
```
Provas dos meus arquivos fora do gate:
- `git show HEAD:<f> | rustfmt --edition 2024 --check` em cada .rs do commit: todos `ok`.
- `cargo clippy --all-targets --message-format short` filtrado por `^(src|tests)`: nenhuma linha.
- `cargo test --quiet --no-fail-fast` (checkout inteiro, com a 218 no meio):
```
test result: ok. 64 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 12.84s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.77s
test result: ok. 1882 passed; 0 failed; 85 ignored; 0 measured; 0 filtered out; finished in 48.90s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
- `cargo test --quiet --test it self_update::` → `test result: ok. 12 passed; 0 failed`.
- Ao vivo (só leitura): cópia do exe de debug numa pasta do scratchpad, `self update --check` → `Conferindo versão…` / `✓ o wingetrs já está na última versão (0.2.1)`, exit 0; no repositório, `target/debug/wingetrs.exe self update --check` → `✗ build de desenvolvimento: atualize com cargo`.

Mutation proofs:
1. `latest/download` em vez da tag:
   `pwsh -File scripts/mutate.ps1 -Path src/selfupdate/mod.rs -Anchor 'let exe_url = format!("{base}/{tag}/{ASSET}");' -Replacement 'let exe_url = format!("{base}/latest/download/{ASSET}");' -TestCommand 'cargo test --quiet --test it self_update::apply_uses_pinned_tag_paths'`
   `[OK] mutação PEGA: 'let exe_url = format!("{base}/{tag}/{ASSET}");' -> 'let exe_url = format!("{base}/latest/download/{ASSET}");' fez o teste falhar (exit 101).`
2. Pular a conferência do SHA:
   `pwsh -File scripts/mutate.ps1 -Path src/selfupdate/mod.rs -Anchor 'if sha256_file(&temp.path)? != want {' -Replacement 'if false && sha256_file(&temp.path)? != want {' -TestCommand 'cargo test --quiet --test it self_update::apply_hash_mismatch_keeps_exe'`
   `[OK] mutação PEGA: 'if sha256_file(&temp.path)? != want {' -> 'if false && sha256_file(&temp.path)? != want {' fez o teste falhar (exit 101).`
3. Tag como argumento do processo elevado:
   `pwsh -File scripts/mutate.ps1 -Path src/selfupdate/mod.rs -Anchor 'params: ELEVATED_PARAMS.to_string(),' -Replacement 'params: format!("{ELEVATED_PARAMS} --tag v9.9.9"),' -TestCommand 'cargo test --quiet --test it self_update::elevated_scope_uses_runas_self_apply'`
   `[OK] mutação PEGA: 'params: ELEVATED_PARAMS.to_string(),' -> 'params: format!("{ELEVATED_PARAMS} --tag v9.9.9"),' fez o teste falhar (exit 101).`
4. Nada real: os testes usam `Launcher` falso, wiremock e tempdir; nenhum `runas`, nenhum Program Files. Sem prova de mutação (não é regra de código).

Decisions:
- Fases: nova variante `Phase::SelfUpdate(SelfUpdateStep)` em `src/install/progress.rs` (fora da lista Files; necessária para "Conferindo versão"/"Conferindo SHA-256"/"Trocando o executável" com relógio). Por ser match exaustivo, ganhou braços em `src/cli/output.rs` e uma linha em `src/tui/jobs.rs` (arquivo também da 218; o hunk era só meu na hora do commit). "Baixando" é o `ProgressEvent::Download` (barra existente). Antes do UAC: `Phase::WaitingElevation` (texto `ELEVATION_NOTICE`); depois, `SelfUpdate(Elevated)` "não pode ser cancelado".
- Erros além da lista: `ElevationRefused` (UAC recusado, texto `ELEVATION_REFUSED`), `BadTag` (tag fora de `vX.Y.Z`, recusada antes de qualquer pedido) e `Other` (HTTP/GitHub restantes, executável acima do teto de 256 MiB via `apply_capped`).
- `.sha256` lido antes do exe (com teto de 1 KiB) para não baixar nada com soma inválida; aceita `<hex>`, `<hex>  wingetrs.exe`, `*wingetrs.exe`, maiúsculas, CRLF.
- Lado elevado (`self apply-elevated`, tratado em `main.rs` antes de qualquer DataDir/log): `current_exe`, recusa build de dev, exige `hold_admin_chain` ok, `Http::new`, `latest_tag` com URL constante; nada de variável de ambiente de URL. Diretório de trabalho do `runas` = pasta do exe; janela oculta.
- `tests/it/main.rs`: a linha `mod self_update;` não existia; acrescentei e commitei só ela (`git update-index --cacheinfo`, sem o `mod admin_terminal;` da 218). Limpeza do `.old` em `main.rs` pula o `bridge-worker` (elevado não apaga nada).

Concerns:
- Rodei `cargo fmt -- <arquivos>` uma vez: ele formata o crate inteiro e tocou o `src/tui/app.rs` em andamento da 218 (só formatação). Corrigi o conselho errado no item de `docs/ARMADILHAS.md` (commit 324f1d5).
- O processo elevado usa `Http::new()`, que segue variáveis de proxy do ambiente do usuário (HKCU\Environment). O TLS continua validado de ponta a ponta e o SHA vem do mesmo release, mas a revisão de segurança deve decidir se o cliente elevado precisa de `no_proxy` (exigiria mexer em `src/http.rs`).
- Sem teste: o braço de rollback do `swap` (falha no segundo rename), `admin_scope`/`run_elevated` reais (DACL e UAC verdadeiros, prova ao vivo do usuário) e o fluxo `run_self` do CLI de ponta a ponta (depende de `current_exe` e rede). Ctrl+C no meio do download mata o processo e deixa um `wingetrs.exe.download-*` na pasta (não há limpeza deles no início).
- Medição do custo do `remove_old` no início de cada execução: não medido (uma chamada `DeleteFile` que falha).

## Rodada de revisão (CHANGES_REQUESTED)

Status: DONE_WITH_CONCERNS

Commits:
2b695b9 fix(selfupdate): elevated client without user proxy or user roots

Gate: (uma execução; falhou só no fmt de `tests/it/admin_terminal.rs`, da Task 218)
```
[OK]   rules 11s
[OK]   rules-selftest 35s
[FAIL] fmt exit 1
  Diff in \?\C:\Users\jonyd\Projetos\wingetrs\tests\it\admin_terminal.rs:939:
  Diff in \?\C:\Users\jonyd\Projetos\wingetrs\tests\it\admin_terminal.rs:970:
  (log completo: target/verify.log)
```
Meus arquivos: `git show HEAD:<f> | rustfmt --edition 2024 --check` `ok` nos 6 .rs; clippy `--all-targets` sem linha nos meus caminhos; `self_update::` → `test result: ok. 16 passed; 0 failed; 1 ignored`; o teste ao vivo `live_elevated_client_reaches_github` (`--ignored`) → `test result: ok. 1 passed` (as raízes da máquina chegam ao github.com; só a raiz de teste dá `UntrustedRoot`). `cargo test --no-fail-fast`: só falhou `admin_terminal::install_ignores_code_that_runs_only_at_removal` (Task 218); `it` → `1889 passed; 1 failed`.

Mutation proofs:
1. Tirar `.no_proxy()`:
   `pwsh -File scripts/mutate.ps1 -Path src/http.rs -Anchor 'Some(r) => b.no_proxy().tls_certs_only(r.iter().cloned()),' -Replacement 'Some(r) => b.tls_certs_only(r.iter().cloned()),' -TestCommand 'cargo test --quiet --test it self_update::elevated_http_has_no_proxy_and_only_given_roots'`
   `[OK] mutação PEGA: 'Some(r) => b.no_proxy().tls_certs_only(r.iter().cloned()),' -> 'Some(r) => b.tls_certs_only(r.iter().cloned()),' fez o teste falhar (exit 101).`
2. Incluir o repositório do usuário (`CERT_SYSTEM_STORE_CURRENT_USER` = 0x10000):
   `pwsh -File scripts/mutate.ps1 -Path src/selfupdate/apply_win.rs -Anchor 'let mut last_err = None;' -Replacement 'let mut last_err = None; roots.extend(read(0x0001_0000, "Root")?);' -TestCommand 'cargo test --quiet --test it self_update::machine_roots_read_only_local_machine_stores'`
   `[OK] mutação PEGA: 'let mut last_err = None;' -> 'let mut last_err = None; roots.extend(read(0x0001_0000, "Root")?);' fez o teste falhar (exit 101).`
3. Pular a volta do exe antigo:
   `pwsh -File scripts/mutate.ps1 -Path src/selfupdate/mod.rs -Anchor 'if let Err(back) = rename(&old, exe) {' -Replacement 'if let Err(back) = Ok::<(), std::io::Error>(()) {' -TestCommand 'cargo test --quiet --test it self_update::swap_failure_puts_old_exe_back'`
   `[OK] mutação PEGA: 'if let Err(back) = rename(&old, exe) {' -> 'if let Err(back) = Ok::<(), std::io::Error>(()) {' fez o teste falhar (exit 101).`
   (a 1ª tentativa deu INCONCLUSIVO porque o crate de teste não compilava por um erro da 218, `Modal` sem `Debug`; repeti quando voltou a compilar.)

Decisions:
- `Http::for_elevated(roots: &[Vec<u8>])` em `src/http.rs`: `no_proxy()` + `tls_certs_only` nos dois clientes e na cópia de `with_strict_download_redirects` (campo `pinned_roots`), com as mesmas políticas de redirecionamento e os mesmos tempos. Raiz que o rustls recusa é pulada (cada uma testada num build de cliente), porque um único certificado ruim faria o build inteiro falhar; nenhuma raiz utilizável vira erro. Prova estrutural de que não há proxy: `may_use_proxy()` olha o `Debug` do `reqwest::Client`, que lista `proxies` exatamente quando há algum (o proxy do sistema incluído; controle: `Http::new()` dá `true`).
- `apply_win::machine_roots_with(reader)`: só `LOCAL_MACHINE` (`CERT_SYSTEM_STORE_LOCAL_MACHINE`) × `["Root", "AuthRoot"]`, sem duplicatas; o leitor real usa `CertOpenStore` em modo somente leitura e `OPEN_EXISTING`.
- Erro claro: o `map_err` do `http.rs` reconhece `invalid peer certificate` do rustls (pelo texto, porque o rustls não é dependência direta) e devolve `HttpError::UntrustedCertificate`, que vai para `GhError::UntrustedCertificate` (em `src/github/mod.rs`, fora da lista Files) e daí para `SelfUpdateError::UntrustedRoot`: "o Windows não tem a raiz do GitHub no repositório da máquina (…)". Isso também muda o caminho comum: erro de certificado deixa de contar como "sem conexão".
- `swap_with(temp, exe, rename)` é público e recebe o rename injetado; em qualquer falha apaga o temporário.
- A limpeza no início (`startup_cleanup`) apaga `<exe>.old` e os `wingetrs.exe.download-*` da pasta do exe que são arquivo comum (`DirEntry::metadata`, que não segue link) e estão parados há 1 h ou mais. Assim um download em andamento noutro processo não perde o arquivo.
- Fixture: `tests/fixtures/selfupdate/test-root.der` (CA P-256 autoassinada de teste, gerada com openssl; a chave foi apagada).

Concerns:
- O verificador `tls_certs_only` do rustls (webpki) não consulta revogação nem o repositório `LocalMachine\Disallowed`. Uma raiz que a Microsoft desconfiou e que ainda esteja em LM\Root ou LM\AuthRoot continua valendo para o cliente elevado.
- O caminho que mostra o erro de certificado só tem o teste ao vivo (ignorado no gate), porque o wiremock não serve TLS.

## Rodada 3 (Minors depois do APPROVE)

Commits:
7b53626 fix(selfupdate): drop machine-disallowed roots, pin rustls error text

Gate:
```
[OK]   rules 9s
[OK]   rules-selftest 33s
[OK]   fmt 3s
[OK]   licenses 5s
[OK]   clippy 26s
[OK]   test (1995 passed, 88 ignored) 63s
[OK]   gate verde
```
Mutation proofs:
- Ignorar Disallowed: `pwsh -File scripts/mutate.ps1 -Path src/selfupdate/apply_win.rs -Anchor 'roots.retain(|c| !disallowed.contains(c));' -Replacement 'let _ = &disallowed;' -TestCommand 'cargo test --quiet --test it self_update::machine_roots_read_only_local_machine_stores'`
  `[OK] mutação PEGA: 'roots.retain(|c| !disallowed.contains(c));' -> 'let _ = &disallowed;' fez o teste falhar (exit 101).`
- Texto do rustls: `pwsh -File scripts/mutate.ps1 -Path src/http.rs -Anchor 'if text.contains("invalid peer certificate") {' -Replacement 'if text.contains("invalid peer certificate!") {' -TestCommand 'cargo test --quiet --test it self_update::untrusted_certificate_is_classified_from_rustls_text'`
  `[OK] mutação PEGA: 'if text.contains("invalid peer certificate") {' -> 'if text.contains("invalid peer certificate!") {' fez o teste falhar (exit 101).`
- O teste ao vivo `live_elevated_client_reaches_github` (`--ignored`) continua passando com o Disallowed real: `test result: ok. 1 passed`.

Decisions: se o repositório Disallowed não puder ser lido, a montagem das raízes falha e a atualização elevada não roda. A classificação foi extraída para `http::classify_causes(&dyn Error)`, que percorre só as causas; o teste usa uma causa `io::Error` com o texto do `Display` do rustls 0.23.45 (`invalid peer certificate: {err}`, `error.rs:986`).

Concerns: o webpki continua sem conferir revogação (CRL/OCSP).
