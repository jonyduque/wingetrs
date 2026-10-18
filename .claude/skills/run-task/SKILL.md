---
name: run-task
description: Orquestra UMA tarefa do plano do wingetrs com subagentes de baixo custo - registra a base, extrai o brief, despacha o rs-implementer, valida o gate, despacha o rs-reviewer, corrige Critical/Important e atualiza o ledger. Use quando o usuário pedir para executar/continuar tarefas do plano ("roda a task 5", "continua a implementação", "próxima tarefa").
---

# run-task — laço por tarefa com contexto mínimo

O orquestrador (sessão principal) **não implementa**. Ele despacha, audita e integra. Cada subagente recebe só o que precisa e devolve só o que o orquestrador precisa. Antes de escrever briefs ou otimizar, leia `docs/ARMADILHAS.md`.

## Laço (uma tarefa N)

1. **Base:** `git rev-parse HEAD > .claude/tasks/task-N-base.txt`.
2. **Brief:** `pwsh -File scripts/brief.ps1 N [-Plan <plano>]` e `pwsh -File scripts/check_briefs.ps1 -Dir .claude/tasks N N`. Exit ≠ 0 → corrija o plano, não o brief.
3. **Modelo:** `.claude/delegation.json` → `designTasks` vão para `model: "opus"`; demais para `rs-implementer` (sonnet). Revisor: sonnet; **opus** para tarefas de design e para revisões finais de lote.
4. **Despacho do implementador** (`subagent_type: "rs-implementer"`), prompt mínimo: caminho do brief, SHA base, e só o que mudou desde o plano (ex.: "outro agente edita X em paralelo: só `git add` dos seus arquivos"). Não cole o brief.
5. **Auditoria barata** (você mesmo, sem ler o transcript):
   - `git cat-file -t <sha>` = `commit` para cada SHA do relatório;
   - `pwsh -File scripts/verify.ps1` (gate independente);
   - **toda regra do brief sem `[OK] mutação PEGA` colado ⇒ rode você a mutação** (duas vezes nesta fase isso achou regra descoberta);
   - mudança de desempenho ⇒ reexecute um bench com `-Baseline` e compare;
   - mudança de comportamento visível ⇒ rode o binário real (`target/release/wingetrs.exe …`) e olhe a saída;
   - `pwsh -File scripts/leftovers.ps1` — processos de agente sobrando (só reporte; encerrar exige pedido do usuário).
6. **Revisão** (`subagent_type: "rs-reviewer"`): brief + `git show <sha>` + relatório + um "foco extra" escrito por você com os riscos específicos da tarefa (concorrência, arquivos de terceiros, equivalência entre caminhos, dados reais).
7. **Correção:** só Critical/Important, mais Minors baratos que protegem tarefa futura. Reenvie ao **mesmo** implementador (SendMessage) a lista exata, com o teste literal quando possível. Achado que não se sustenta: recuse com a evidência (ex.: CRLF "faltando" que estava lá).
8. **Ledger:** uma linha em `.claude/tasks/progress.md` com o que a revisão achou (é o que alimenta `docs/ARMADILHAS.md`). Commit do ledger + base + relatório com `git add` dos dois novos (base e relatório nunca foram rastreados; sem isso o `git commit -- <arquivos>` falha com "pathspec did not match") e `git commit -- <arquivos>`.
9. **Lição nova?** Se um defeito escapou do implementador e só a revisão pegou, pergunte: qual instrumento deixaria isso impossível? (regra em `check_rules.ps1`, item em `ARMADILHAS.md`, regra no `CLAUDE.md`, seção no `_common.md`). Faça isso **antes** da próxima tarefa e prove o instrumento (plante a violação, veja o gate falhar, restaure).

## Paralelismo

- **`cargo test --test it` aceita um filtro só** (dois filtros saem com erro 1). Nos prompts de revisor e implementador, dê um prefixo comum (`install_`) ou um comando por módulo; nunca `cargo test ... a:: b::`.
- Tarefas que tocam arquivos disjuntos podem rodar em paralelo; avise cada agente dos arquivos do outro e proíba `git add -A`.
- Teste novo em `tests/it/<modulo>.rs`: antes de despachar, crie o arquivo com `#![allow(clippy::unwrap_used, clippy::expect_used)]` e uma linha `//! <assunto> (Task N).` (sem o allow, todo agente tropeça no clippy com `unwrap` em teste) (vazio, o `cargo fmt` acrescenta um CRLF e o gate falha) e a linha `mod <modulo>;` em `tests/it/main.rs` e faça commit; senão duas tarefas em paralelo dividem o `main.rs`. Para registrar `mod`, insira depois de uma âncora; nunca reordene um bloco que tem atributos (`#[cfg(windows)]`, `#[allow]` do `spans`, `pub(crate) mod`): o atributo passa para a linha errada (aconteceu duas vezes em 2026-10-18). Com agentes rodando, a lista de arquivos de um commit do orquestrador é escrita à mão: nunca `git diff --name-only`/`git status` como lista (pegou o trabalho não commitado de 4 agentes em 2026-10-18). Duas tarefas (ou uma correção de revisão e uma tarefa nova) nunca ao mesmo tempo no mesmo arquivo: o `git commit -- <arquivo>` de uma leva os trechos não commitados da outra (Tasks 144-fix e 145 em `state.rs`, 2026-10-18). Antes de despachar, cruze a lista Files com a das tarefas e correções em andamento.
- **Tarefas que medem tempo rodam sozinhas** — e nenhuma revisão com `cargo test` em paralelo a um benchmark.
- Revisor é read-only e pode rodar junto com a próxima implementação se os arquivos não se cruzam.

## Quando parar e perguntar ao usuário

- Implementador devolve `BLOCKED` por contradição entre brief e realidade.
- Gate vermelho após duas rodadas de correção.
- Qualquer mudança de escopo da spec, dependência nova não prevista, ou meta que não fecha (apresente opções com vantagens, desvantagens e recomendação).
- Encerrar processo que não foi iniciado pela tarefa corrente.
