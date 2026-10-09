# wingetrs

Gerenciador de pacotes para Windows em Rust (TUI ratatui + CLI) sobre o catálogo do winget (CDN `source2.msix`) e buckets do Scoop. Fase atual: **fase 1, somente leitura** (`update`, `search`, `show`, `list`).

- Spec: `docs/superpowers/specs/2026-10-07-wingetrs-fase1-design.md`
- Plano: `docs/superpowers/plans/2026-10-07-wingetrs-fase1.md`
- Ledger de tarefas: `.claude/tasks/progress.md`
- Referência portada do winget-cli (MIT, commit em `SOURCE_COMMIT.txt`): `docs/reference/winget-cli/`

## Comandos

| O quê | Comando |
|---|---|
| Gate completo (fmt + clippy + testes) | `pwsh -File scripts/verify.ps1` |
| Um teste | `cargo test --quiet <nome>` |
| Prova de mutação | `pwsh -File scripts/mutate.ps1 -Path <arquivo> -Anchor '<texto exato>' -Replacement '<texto>' -TestCommand 'cargo test --quiet <nome>'` |
| Extrair brief de tarefa | `pwsh -File scripts/brief.ps1 <N>` |
| Checar briefs | `pwsh -File scripts/check_briefs.ps1 -Dir .claude/tasks <de> <até>` |
| Testes ao vivo (rede/máquina real) | `cargo test --quiet -- --ignored` |
| Regenerar fixtures | `python -I scripts/make_fixtures.py ...` (ver docstring) |

`verify.ps1` imprime só um resumo; o log completo fica em `target/verify.log`. Leia o log com `Select-String`/`grep` focado, nunca inteiro.

## LSP / navegação de código

Use o LSP (rust-analyzer) para diagnósticos e navegação em vez de ler arquivos inteiros:

- `rust-analyzer symbols < src/x.rs` — outline do arquivo (structs, fns, linhas) sem ler o corpo. Use antes de abrir um arquivo grande e depois leia só o trecho.
- `rust-analyzer diagnostics .` — diagnósticos do projeto todo.
- `rust-analyzer unresolved-references .` — referências quebradas após renomear.
- `cargo check --message-format short` — erros compactos (uma linha cada).
- Para "quem chama X": `grep -rn "X(" src tests` com contagem antes de afirmar.

## Regras do projeto (cada uma com o motivo)

- **Dado externo nunca causa pânico.** Sem `unwrap()`/`expect()` em caminhos que tocam rede, disco, registro ou manifestos; use `?` com erro tipado. Motivo: o catálogo muda sem aviso e um manifesto ruim não pode derrubar a TUI.
- **Erros:** `thiserror` por módulo; `anyhow` só nas bordas (`main.rs`, `cli/`, `tui::run`). Motivo: módulos precisam de erros que o chamador distingue (offline vs. integridade).
- **Nada escreve em stdout fora de `main.rs`/`cli/`.** Logs vão por `tracing` (arquivo). Motivo: a TUI ocupa o terminal; um `println!` perdido corrompe a tela.
- **Código específico de Windows** (`winreg`, `windows`) fica em arquivos próprios com `#[cfg(windows)]` no `mod`, nunca `if cfg!(windows)` no meio da lógica. Motivo: a lógica de correlação é testável em qualquer plataforma com dados sintéticos.
- **Testes unitários não acessam rede nem o registro real.** Usam `tests/fixtures/` e `wiremock`. Testes que precisam da máquina/rede reais levam `#[ignore = "live: <motivo>"]`.
- **Fixtures são binárias ou grandes: nunca `cat`/`Read` inteiras.** Os fatos que os testes precisam já estão nos briefs.
- **Não portar mais do que o brief pede** do winget-cli (YAGNI). Cada função portada cita o arquivo de origem em comentário de uma linha.
- **Versões de dependências:** só as do `Cargo.toml`; não adicione crate sem o brief mandar. Motivo: o conjunto foi verificado num crate de sondagem.
- Texto de UI e mensagens ao usuário em português; identificadores e comentários de código em inglês.

## Comandos proibidos para agentes

- `git checkout`/`restore`/`stash`/`clean`/`reset` em arquivos que você não criou.
- `git push` (não há remoto; integração é do orquestrador).
- `cargo update` (versões travadas no `Cargo.lock`).
- `find /`, `Get-ChildItem C:\ -Recurse` ou qualquer varredura do disco inteiro: um `find / -name oracle.py` esquecido por um subagente ficou horas consumindo CPU e invalidou os benchmarks. Procure só dentro do repositório, do scratchpad ou de caminhos conhecidos.
- Deixar processos em segundo plano vivos ao terminar (servidores, watchers, buscas): encerre tudo que você iniciou antes de responder.
- Editar `docs/superpowers/specs/*`, `docs/superpowers/plans/*`, `.claude/tasks/progress.md` (são do orquestrador).

## Economia de contexto (subagentes)

- O implementador recebe **somente** o caminho do brief (`.claude/tasks/task-N-brief.md`). O brief é autocontido; não leia o plano nem a spec inteiros, só seções citadas pelo brief.
- Saída de comandos: rode com `--quiet`, filtre com `grep`/`Select-String`; nunca despeje logs longos.
- Relatório da tarefa vai para `.claude/tasks/task-N-report.md` (formato no brief). A mensagem final ao orquestrador tem no máximo 10 linhas: status, SHAs, caminho do relatório.
- O revisor recebe brief + `git diff <base>..HEAD`, não o repositório inteiro.
- Espere ativamente comandos longos (laço conferindo o arquivo de saída até aparecer o exit) antes de encerrar o turno — não encerre esperando notificação.
