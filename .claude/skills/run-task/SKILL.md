---
name: run-task
description: Orquestra UMA tarefa do plano do wingetrs com subagentes de baixo custo - registra a base, extrai o brief, despacha o rs-implementer, valida o gate, despacha o rs-reviewer, corrige Critical/Important e atualiza o ledger. Use quando o usuário pedir para executar/continuar tarefas do plano ("roda a task 5", "continua a implementação", "próxima tarefa").
---

# run-task — laço por tarefa com contexto mínimo

O orquestrador (sessão principal) **não implementa**. Ele despacha, audita e integra. O objetivo é que cada subagente receba só o que precisa e devolva só o que o orquestrador precisa.

## Laço (uma tarefa N por vez; tarefas tocam os mesmos arquivos)

1. **Base:** `git rev-parse HEAD > .claude/tasks/task-N-base.txt` (deixe o arquivo sujo de propósito — o primeiro commit da tarefa o inclui).
2. **Brief:** `pwsh -File scripts/brief.ps1 N` e `pwsh -File scripts/check_briefs.ps1 -Dir .claude/tasks N N`. Exit ≠ 0 → corrija o plano, não o brief.
3. **Modelo:** consulte `.claude/delegation.json` → `designTasks`. Tarefa de design → `model: "opus"`; demais → `rs-implementer` (sonnet).
4. **Despacho do implementador** (Agent tool, `subagent_type: "rs-implementer"`), prompt mínimo:
   ```
   Brief: .claude/tasks/task-N-brief.md
   Base: <sha>
   Implemente conforme o brief e o CLAUDE.md. Responda em ≤10 linhas.
   ```
   Não cole o brief no prompt (o agente lê o arquivo). Não cole contexto da conversa.
5. **Gate independente:** rode você mesmo `pwsh -File scripts/verify.ps1`. Leia o relatório `.claude/tasks/task-N-report.md` (não o transcript do agente).
6. **Auditoria barata do relatório:** cada SHA → `git cat-file -t <sha>` = `commit`; linhas de mutação com `[OK] mutação PEGA`; nada de "tests pass" sem saída.
7. **Revisão** (`subagent_type: "rs-reviewer"`), prompt mínimo:
   ```
   Brief: .claude/tasks/task-N-brief.md
   Diff: <base>..HEAD
   Relatório: .claude/tasks/task-N-report.md
   ```
8. **Correção:** só Critical/Important. Reenvie ao mesmo implementador (SendMessage, mantém o contexto dele) a lista de achados, não o review inteiro. Achado que não se sustenta: recuse com a evidência.
9. **Ledger:** acrescente uma linha em `.claude/tasks/progress.md`: `| N | título | base..head | DONE | modelo | notas |`. Commit do ledger + base: `chore(tasks): record task N`.

## Regras de economia

- Nunca leia o arquivo de saída/transcript do subagente; leia o relatório.
- Saída de comandos sempre filtrada (`verify.ps1` já resume).
- Uma tarefa por vez; não paralelize tarefas que compartilham `Cargo.toml`/`lib.rs`.
- Revisão final do branch (após a última tarefa): um único revisor com `model: "opus"` sobre `git diff <base da task 1>..HEAD --stat` + os hunks que ele pedir.

## Quando parar e perguntar ao usuário

- Implementador devolve `BLOCKED` por contradição entre brief e realidade (API não existe, fixture diferente).
- Gate vermelho após duas rodadas de correção.
- Qualquer mudança no escopo da spec.
