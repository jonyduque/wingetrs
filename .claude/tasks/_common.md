#### Execution rules

- Leia `CLAUDE.md` (regras do projeto, comandos proibidos, economia de contexto). Não leia o plano nem a spec inteiros; este brief é autocontido.
- TDD: escreva o teste, rode e veja falhar pelo motivo certo, implemente, rode e veja passar.
- Gate antes de cada commit: `pwsh -File scripts/verify.ps1` deve terminar com `[OK]   gate verde`. Se demorar, espere ativamente (um laço que confere o arquivo de saída até aparecer o exit) antes de encerrar o turno — não encerre esperando notificação.
- Commits pequenos, mensagem em inglês no formato `feat(<módulo>): ...`/`test(...)`/`fix(...)`, terminando com a linha `Co-Authored-By: Claude <noreply@anthropic.com>`. Nunca `git push`.
- Não adicione dependências fora das listadas no brief. Não edite arquivos fora da lista **Files** sem registrar o motivo no relatório.
- **Cada regra listada em "Checks beyond the steps" ou "The decision this task must get right" precisa da sua prova de mutação colada.** Se você pular uma, diga qual e por quê — o orquestrador vai rodá-la.
- **Desempenho:** spans antes de otimizar; benchmark sempre com `-Baseline target/release/examples/noop.exe`; número decisivo ⇒ compare A/B contra a base (worktree temporário, removido no fim). Otimização sem ganho medido é revertida.
- **Threads/pânico:** toda thread que você criar é juntada em todos os caminhos; `join()` nunca termina em `.ok()`/`if let Ok`/`unwrap_or_default` (o gate barra). Veja `docs/ARMADILHAS.md`.
- Nunca varra o disco inteiro (`find /`, `Get-ChildItem C:\ -Recurse`); procure só no repositório e em caminhos conhecidos. Encerre todo processo que você iniciou em segundo plano (inclusive laços `until … sleep`) antes de responder; confira com `pwsh -File scripts/leftovers.ps1`.
- Se algo do brief não existir ou contradizer o código real: pare, registre `BLOCKED: <motivo com evidência>` no relatório. Não invente uma alternativa silenciosa.

#### Report contract

Grave `.claude/tasks/task-{N}-report.md` com, nesta ordem:

1. `Status: DONE | DONE_WITH_CONCERNS | BLOCKED`
2. `Commits:` lista de SHAs (`git log --format='%h %s' <base>..HEAD`), colados da saída real.
3. `Gate:` as linhas finais reais de `scripts/verify.ps1` (copiadas, não resumidas).
4. `Mutation proofs:` para cada regra listada em "Checks beyond the steps", o comando `scripts/mutate.ps1` executado e a linha de saída real (`[OK] mutação PEGA...`). Uma regra sem saída colada conta como não verificada. Se a tarefa declarar "no mutation proof", escreva isso explicitamente.
5. `Decisions:` escolhas que o brief não fixou, com o motivo (máx. 5 linhas).
6. `Concerns:` o que ficou de fora ou parece errado (ou `none`).

Não conta como entrega: "tests pass" sem saída colada; prova de mutação sem a linha `[OK] mutação PEGA`; número que não foi medido (escreva "não medido").

A mensagem final ao orquestrador: no máximo 10 linhas — status, SHAs, caminho do relatório.
