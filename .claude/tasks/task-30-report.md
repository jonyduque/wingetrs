Status: DONE_WITH_CONCERNS

Commits:
8c62970 feat(install): shims with kiennq launcher, PE subsystem and script wrappers

Gate: NAO ficou verde por arquivos de outros agentes em andamento. Ultimas linhas reais:
  [OK]   rules
  [FAIL] fmt exit 1  (Diff em env_win.rs e tests/install_win_scan.rs, nao meus)
  clippy --all-targets: erro unico em src/install/registry_win.rs:150 (chunks_exact_to_as_chunks), nao meu; 0 ocorrencias em shim.
  rustfmt --check nos meus 2 arquivos: ok. cargo test --quiet: todos os binarios ok (7/7 em install_shim).
  Orquestrador deve rodar verify.ps1 quando os outros agentes terminarem.

Mutation proofs:
[OK] mutação PEGA: 'lfanew.checked_add(24 + 68)' -> 'lfanew.checked_add(24 + 66)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'fs::write(&exe, bytes).map_err(io)?;' -> 'fs::write(&launcher, &bytes).map_err(io)?; fs::write(&exe, bytes).map_err(io)?;' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !verify(&spec, &bytes) {' -> 'if false && !verify(&spec, &bytes) {' fez o teste falhar (exit 101).

Decisions:
- Launcher fica em layout.launcher() (shims/.launcher/shim.exe); zip/stage temporarios ao lado, removidos sempre.
- Alvo GUI detectado lendo o alvo; se ilegivel/invalido, trata como console (hardlink).
- Wrapper .ps1.cmd escolhe pwsh/powershell em tempo de execucao (where /q pwsh.exe), como o scoop.
- ensure_launcher_from ignora _data (parametro mantido pela assinatura do brief).

Concerns: texto exato do .cmd do .ps1 nao fixado pelo brief (testado por trechos). Teste red-first nao separado (teste e impl escritos juntos).

## Correcoes da revisao (commit 9e037ef)
Itens 1-6 feitos (testes: remove_shim isolado, recriacao/flip GUI-console, lancador invalido, PE alem de 4 KiB = console, nomes inseguros, texto exato ps1/jar com args). Erro ao remover .exe antigo agora e ShimError::Io. Gate: fmt/test de outros agentes falham (registry_win.rs), meus arquivos ok (13/13).
[OK] mutação PEGA: '["exe", "shim", "cmd", "ps1"]' -> '["cmd", "ps1"]' fez o teste falhar (exit 101).
