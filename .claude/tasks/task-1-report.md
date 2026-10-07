Status: DONE_WITH_CONCERNS

Commits:
9220586 feat(cli): scaffold crate, core types and command parser

Gate:
[OK]   fmt 1s
[OK]   clippy 88s
[OK]   test (7 passed, 0 ignored) 2s
[OK]   gate verde

Mutation proofs:
Regra: --source e case-insensitive.
Comando: pwsh -File scripts/mutate.ps1 -Path src/core/mod.rs -Anchor 's.to_ascii_lowercase().as_str()' -Replacement 's' -TestCommand '$o = cargo test --quiet --test cli_parse 2>&1; $c = $LASTEXITCODE; $o | Where-Object { "$_" -notmatch "^error: test failed" }; cmd /c exit $c'
Saida: [OK] mutação PEGA: 's.to_ascii_lowercase().as_str()' -> 's' fez o teste falhar (exit 101).
  search_with_source_and_limit --- FAILED
(Com o TestCommand simples do brief o script devolve INCONCLUSIVO, ver Concerns.)

cargo tree -d --depth 1 | grep crossterm: sem saida (0 duplicatas). `cargo tree -i crossterm`: unica versao crossterm v0.29.0, usada por ratatui-crossterm e pelo crate. (Duplicatas de outros crates, ex. base64 0.22/0.23, existem; so crossterm foi pedido.)

Decisions:
- Arquivos criados com a ferramenta Write (heredoc grande falhou no parse do bash); conteudo identico ao brief; um hook formatou tests/cli_parse.rs e src/cli/mod.rs (rustfmt).
- parse_source adicionado em cli/mod.rs e usado via value_parser nos 3 subcomandos com --source, como o brief decide.
- run(): sem subcomando -> bail "TUI ainda não implementada"; demais -> "comando ainda não implementado: <nome>" via Command::name().
- Passo TDD "ver falhar" nao executado separadamente: teste e implementacao foram escritos antes da primeira execucao do cargo.

Concerns:
- scripts/mutate.ps1 classifica como "nao compila" qualquer saida com a linha `error: test failed` (padrao '^error: '), logo toda prova de mutacao com `cargo test` simples sai INCONCLUSIVO mesmo com o mutante compilando e o teste falhando. Contornei filtrando essa linha no TestCommand; o script nao foi editado. Sugestao ao orquestrador: ajustar o regex do script (excluir 'error: test failed') antes das proximas tarefas.
- .claude/tasks/task-1-base.txt ficou untracked (nao e meu, nao commitado).

Fix (revisão): teste do --background agora inspeciona o help do subcomando update.
Commit: test(cli): pin hidden --background flag on subcommand help (ver git log)
Comando: pwsh -File scripts/mutate.ps1 -Path src/cli/mod.rs -Anchor 'hide = true' -Replacement 'hide = false' -TestCommand 'cargo test --quiet --test cli_parse'
Saida: [OK] mutação PEGA: 'hide = true' -> 'hide = false' fez o teste falhar (exit 101).
Gate: [OK]   gate verde (7 passed)
