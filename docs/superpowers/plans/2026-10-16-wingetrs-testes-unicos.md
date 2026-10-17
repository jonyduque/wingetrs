# wingetrs — um binário de testes de integração (Task 133)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 133 -Plan docs/superpowers/plans/2026-10-16-wingetrs-testes-unicos.md`.

**Goal:** os ~50 arquivos `tests/*.rs` viram módulos de um binário só (`tests/it/main.rs`), para o gate e o ciclo rápido linkarem 1 executável em vez de ~50, sem mudar nenhum teste.

**Pedido do usuário (2026-10-16):** "juntar os binários" (proposta da medição de build: cada binário de teste linka o crate inteiro; o link domina o build incremental).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- **Medir antes e depois** (regra do projeto): sem ganho medido, a mudança é revertida e só o relatório é entregue.
- Nenhum teste muda de comportamento, de nome (fora o prefixo de módulo) ou some. Nenhuma crate nova; `Cargo.lock` intocado.
- Rode com `CARGO_BUILD_JOBS=4` (o paginador estourou com mais, os error 1455). Nada de worktree ou `target` separado.

## Review Focus

1. Estado global de processo que antes era isolado por binário: `std::env::set_var` (`WINGETRS_LOCK_TIMEOUT_MS` em `tests/update.rs`, `WINGETRS_TEST_ENV_KEY` em `tests/install_win_effects.rs`), `static`/`OnceLock`/`thread_local` (`tests/install_executor.rs`, `tests/install_extract.rs`, `tests/tui_jobs.rs`), assinante global do `tracing`, hook de pânico. Junto num processo, um teste vê o valor de outro.
2. Snapshots do `insta` (`tests/snapshots/*.snap`, usados por `cli_parse`, `install_planner`, `tui_view`): o caminho e o nome do snapshot mudam com o módulo; um snapshot que some e é recriado em silêncio passa sem testar nada.
3. Testes `#[ignore = "live: ..."]` continuam ignorados e ainda rodam com `--ignored`.
4. Módulo compartilhado `tests/spans/` (`mod spans;` em `update.rs` e `install_ops.rs`) e submódulos internos (`mod limits`, `mod prefetch`, `mod shortcuts`, `mod release_on_retry`).

---

### Task 133: Testes de integração num binário só

**Modelo:** opus (revisor opus).

#### Where this fits

- `tests/*.rs`: 50 arquivos, ~26.700 linhas; hoje cada um é um binário. `cargo test --all-targets -- --list` lista 1006 testes (inclui unitários e ignorados).
- `tests/fixtures/`, `tests/snapshots/`, `tests/spans/` ficam onde estão ou mudam de lugar junto com quem os usa; fixtures são achadas por `env!("CARGO_MANIFEST_DIR")`.
- `scripts/verify.ps1:27` roda `cargo test --quiet --all-targets`.
- `.claude/tasks/_common.md:5` ("Ciclo rápido") manda rodar `cargo test --quiet --test <arquivo>`.

#### What binds this task

1. **Medição A (antes, sem mudar nada)**, máquina sem outro build: não meça build frio; meça (a) incremental: `touch src/install/extract.rs` e cronometre `cargo test --no-run --quiet --all-targets`, 3 vezes, mediana; (b) ciclo de um binário: `touch src/install/extract.rs` e `cargo test --no-run --quiet --test install_extract`, 3 vezes; (c) tempo do passo `test` do `verify.ps1`. Guarde `cargo test --all-targets -- --list` ordenado (lista A).
2. **Mudança:** `tests/it/main.rs` com um `mod <arquivo>;` por arquivo antigo, cada arquivo movido com `git mv` para `tests/it/<arquivo>.rs`. Módulo compartilhado `spans` declarado uma vez no `main.rs` e usado como `crate::spans`. Snapshots `.snap` movidos com `git mv` para onde o `insta` os procura com o nome novo, **conteúdo idêntico**.
3. **Estado global:** para cada item do Review Focus 1, prove que não há interferência ou isole. Isolamento permitido: deixar o arquivo como binário próprio (`tests/<arquivo>.rs` fica fora do `it`), dito no relatório com o motivo. Proibido corrigir a interferência mudando o que o teste verifica.
4. **Equivalência:** lista B (`--list` depois) tem o **mesmo número** de testes e os mesmos nomes da lista A, tirando o prefixo de módulo; cole o `diff` normalizado (vazio). Mesma contagem de `passed`/`ignored` no gate. `INSTA_UPDATE=no cargo test` passa e nenhum `*.snap.new` ou `.pending-snap` aparece.
5. **Medição B (depois):** as mesmas três medidas do item 1 (para (b) use `cargo test --no-run --quiet --test it`). Tabela A × B no relatório. **Se o incremental (a) não cair pelo menos 20%, reverta tudo** (`git restore`/`git mv` de volta só nos seus arquivos) e entregue só o relatório com os números.
6. **Ciclo rápido novo:** atualize `.claude/tasks/_common.md` ("Ciclo rápido") com o comando para rodar só um módulo (`cargo test --quiet --test it <modulo>::`) e os tempos medidos em B.

**Files:**
- Create: `tests/it/main.rs`, `.claude/tasks/task-133-report.md`
- Modify (mover): `tests/*.rs` para `tests/it/`, `tests/snapshots/*.snap`, `tests/spans/*`; `.claude/tasks/_common.md`

Testes: nenhum novo; os existentes são o critério (item 4).

#### Checks beyond the steps

1. Prova de que um teste quebrado ainda falha no binário novo: mutação `scripts/mutate.ps1` num `assert` de `tests/it/cli_parse.rs` (ou num snapshot) ⇒ **PEGA**.
2. `cargo test --quiet --release --test it -- --ignored --list` lista os testes `live`.
3. `pwsh -File scripts/check_rules.ps1` (a regra de relógio varre `tests/` recursivamente).
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `test: merge integration tests into a single binary`
