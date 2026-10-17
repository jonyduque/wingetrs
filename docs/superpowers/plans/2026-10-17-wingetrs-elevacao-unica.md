# wingetrs — um pedido de elevação por lote na ponte do winget (Task 137)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 137 -Plan docs/superpowers/plans/2026-10-17-wingetrs-elevacao-unica.md`.

**Goal:** atualizar vários apps de escopo máquina pela ponte do winget pede o UAC **uma vez**, não uma por instalador.

**Decisão do usuário (2026-10-17):** opção 1, "auxiliar elevado por lote". Origem: Docker Desktop, Adobe Acrobat Reader e CrystalDiskInfo pediram elevação cada um na mesma sessão.

## Desenho (decidido pelo orquestrador; o implementador não muda sem perguntar)

- **Subcomando oculto** `wingetrs bridge-worker --pipe <nome> --parent <pid>` (`#[command(hide = true)]`). É o único código que roda elevado.
- **Quem eleva:** a TUI, na primeira vez que vai rodar um job de ponte **winget** cuja linha tem `ListRow.scope == Some(InstallScope::Machine)` (campo da Task 135) e não há auxiliar vivo. `ShellExecuteExW` com verbo `runas`, `SEE_MASK_NOCLOSEPROCESS`, janela oculta. Jobs de escopo usuário, desconhecido e scoop continuam como hoje (sem elevação).
- **UAC recusado** (`ERROR_CANCELLED`, 1223) ou auxiliar que não conecta em 30 s: o job roda como hoje (o instalador pede o UAC sozinho) e o progresso mostra `"elevação recusada; o instalador pode pedir de novo"`. Nenhuma nova tentativa de elevar até a TUI reiniciar.
- **Canal:** pipe nomeado criado pela TUI **antes** de elevar, nome `\\.\pipe\wingetrs-<128 bits aleatórios em hex>`, `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, 1 instância, DACL só com o SID do usuário atual (SDDL `D:P(A;;GA;;;<SID>)`). Ao conectar, a TUI confere `GetNamedPipeClientProcessId` = PID do processo que o `ShellExecuteExW` devolveu; o auxiliar confere `GetNamedPipeServerProcessId` = `--parent`. Divergiu ⇒ fecha e trata como recusa.
- **Protocolo:** linhas JSON (`serde_json`), uma mensagem por linha, cada linha ≤ 64 KiB (maior ⇒ protocolo inválido, fecha). TUI ⇒ auxiliar: `{"op":"upgrade","job":<u64>,"target":"<id>"}` e `{"op":"quit"}`. Auxiliar ⇒ TUI: `{"job":n,"line":"..."}`, `{"job":n,"end":{"code":i32,"lines":[...]}}` ou `{"job":n,"error":"..."}`. Nenhuma outra operação existe.
- **O auxiliar só executa** `bridge::command` para `Tool::Winget` com o `target` que passa em `bridge::valid_target` (o mesmo comando fixo de hoje: `winget upgrade --id <id> --exact --source winget --silent ...`), e o `winget source update` da Task 136 se pedido como `{"op":"source_update","job":n}`. Alvo inválido ⇒ `error`, não executa. Um job por vez.
- **`msi.lock`:** continua com a TUI (ela segura o lock enquanto o job roda no auxiliar, como hoje).
- **Vida do auxiliar:** fica vivo enquanto a TUI vive; sai quando o pipe fecha, quando recebe `quit`, quando o processo `--parent` morre (espera no handle do pai) ou depois de 10 min sem job. A TUI manda `quit` ao sair.
- **Encaixe:** `jobs::run_bridge` não muda de contrato: recebe um `BridgeExec` que, para jobs elevados, manda pelo pipe e devolve `(code, lines)` com as linhas chegando por `on_line`. Toda a conferência da 134/136 continua valendo.
- **Log:** a TUI registra `job.start`/`job.end` como hoje, com o campo novo `elevated=true|false`; o auxiliar loga no mesmo arquivo com `role=worker` (alvo `wingetrs::worker`), uma linha por mensagem recebida e por execução.

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- Código Win32 (`ShellExecuteExW`, pipe, DACL, PIDs, espera no pai) só em `*_win.rs` com `#[cfg(windows)]` no `mod`; a lógica (protocolo, laço do auxiliar, decisão de elevar) testável sem Windows com transporte falso.
- Features novas do crate `windows` já travado (0.62.2), sem crate nova e sem mudar o `Cargo.lock` de pacote: `Win32_System_Pipes`, `Win32_Security`, `Win32_Security_Authorization` (as que faltarem; diga quais).
- **Testes nunca elevam nem rodam winget**: o teste live do UAC é `#[ignore = "live: pede UAC"]` e só roda com o usuário.
- Dado do pipe é externo: sem `unwrap`/`expect`, sem pânico; linha inválida fecha o canal.

## Review Focus

1. Outro processo do mesmo usuário tentando conectar no pipe antes do auxiliar, ou com o nome adivinhado.
2. Auxiliar recebendo `target` com espaço, aspas, `&`, `--override`, caminho, ou JSON com campos extras.
3. TUI fechada no meio de um job elevado: o auxiliar termina o winget em curso (não mata o instalador) e sai.
4. UAC recusado: os jobs seguem como hoje, um aviso, nenhuma nova janela de UAC na mesma sessão.
5. Duas TUIs abertas: cada uma com seu pipe e seu auxiliar; nada compartilhado.

---

### Task 137: Auxiliar elevado da ponte do winget, um UAC por sessão

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/tui/run.rs` (~:1370–1384): `spawn_bridge` monta `on_line` e chama `jobs::run_bridge(&job, scoop_root, &msi_lock, &pwsh, &mut run_tool, &mut on_line)`; `run_tool` é o exec real (de `bridge::spawn_win::run`/`run_with_grace`, `src/bridge/spawn_win.rs` ~:158–168).
- `src/tui/jobs.rs`: `BridgeExec` (~:300), `run_bridge` (com o laço de nova tentativa da 136).
- `src/bridge/mod.rs`: `BridgeJob` (com `installed`/`available`), `command` (~:106), `valid_target`, `winget_source_update` (136).
- `src/installed/correlate.rs`: `ListRow.scope: Option<InstallScope>` (135). O `BridgeJob` precisa carregar o escopo (acrescente `scope: Option<InstallScope>` montado em `bridge_target`).
- `src/cli/mod.rs`: `enum Command` (~:85); `src/main.rs` despacha.
- `src/job_log.rs`: `JobLog` (campo novo `elevated`).
- Módulos de teste novos **já criados pelo orquestrador**: `tests/it/elevated_worker.rs` (vazio, com `mod elevated_worker;` no `tests/it/main.rs`; não edite o `main.rs`).

#### What binds this task

1. `src/bridge/worker.rs` (lógica pura): tipos do protocolo (`Request`, `Reply`) com `serde`, `encode`/`decode` de uma linha com teto de 64 KiB e recusa de campo desconhecido (`deny_unknown_fields`), e `serve(reader, writer, exec)` — o laço do auxiliar sobre `BufRead`/`Write` genéricos e um exec injetado. `serve` valida o alvo, monta o comando com `bridge::command`/`winget_source_update`, executa, repassa as linhas e o fim. Para no fim do leitor, em `quit` ou em linha inválida.
2. `src/bridge/elevate_win.rs`: criar o pipe com a DACL do usuário, gerar o nome aleatório (`getrandom` não existe no lock: use `BCryptGenRandom` ou o RNG que já houver no lock — diga qual), `ShellExecuteExW runas`, aceitar a conexão com prazo de 30 s, conferir os PIDs, e o lado do auxiliar (conectar, conferir o PID do servidor, esperar no handle do pai em outra thread juntada no fim).
3. `src/bridge/elevated.rs` (lógica): `ElevatedBridge` com estado `NotStarted | Running(conn) | Refused`, `exec_for(job) -> BridgeExec` que escolhe elevado (job winget com `scope == Machine` e estado não `Refused`) ou o exec atual; a primeira escolha elevada dispara a elevação. Um transporte injetável (trait) para o teste.
4. `wingetrs bridge-worker` oculto em `cli`, que chama `serve` sobre o pipe; logs com `role=worker`.
5. TUI: `Ctx` guarda o `ElevatedBridge`; `spawn_bridge` usa `exec_for`; ao sair, `quit` e espera o auxiliar sair (com prazo de 5 s; passou ⇒ só registra, não mata).
6. `JobLog` ganha `elevated`.

**Files:**
- Create: `src/bridge/worker.rs`, `src/bridge/elevated.rs`, `src/bridge/elevate_win.rs`
- Modify: `Cargo.toml` (features do `windows`), `src/bridge/mod.rs`, `src/cli/mod.rs`, `src/main.rs`, `src/tui/run.rs`, `src/tui/jobs.rs` (se preciso), `src/job_log.rs`, testes que constroem `BridgeJob`
- Test: `tests/it/elevated_worker.rs` (já existe vazio)

Testes (`tests/it/elevated_worker.rs`, transporte em memória, exec falso):
1. `serve_runs_only_the_fixed_winget_command`: `upgrade` com `Git.Git` ⇒ o exec recebe exatamente `bridge::command` do job winget; linhas e fim chegam na ordem.
2. `serve_refuses_bad_targets`: alvos `a b`, `"x"`, `x&y`, `--override`, `C:\x`, vazio ⇒ `error` e o exec nunca é chamado.
3. `decode_refuses_unknown_op_extra_fields_and_long_lines`: `{"op":"run","cmd":"calc"}`, campo extra, linha de 64 KiB + 1 ⇒ `serve` para sem executar.
4. `source_update_is_the_136_command`: `source_update` ⇒ exec recebe `bridge::winget_source_update()`.
5. `machine_scope_goes_elevated_others_do_not`: `exec_for` com transporte falso: winget+Machine ⇒ elevado (uma elevação para três jobs seguidos); winget+User, winget+None, scoop ⇒ exec atual, sem elevar.
6. `refused_elevation_falls_back_once`: transporte falso devolve "recusado" ⇒ o job usa o exec atual, aviso no `on_line`, e os jobs seguintes não tentam elevar de novo.
7. `job_log_marks_elevated`: `job.end` com `elevated=true` no caminho elevado.
8. `live_single_uac_for_two_jobs` `#[ignore = "live: pede UAC"]`: dois `winget upgrade` de apps de escopo máquina pelo auxiliar, um UAC.

#### Checks beyond the steps

1. Mutação: `serve` sem `valid_target` ⇒ teste 2 **PEGA**.
2. Mutação: aceitar campo desconhecido ⇒ teste 3 **PEGA**.
3. Mutação: tentar elevar de novo depois de recusado ⇒ teste 6 **PEGA**.
4. Diga no relatório como provou (ou por que não conseguiu provar) a DACL e a conferência de PID; esses pedaços Win32 só têm prova ao vivo — liste o roteiro para o usuário executar.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): one elevated helper per session for machine-scope winget upgrades`
