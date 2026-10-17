# wingetrs — log completo dos jobs e conferência do winget (Task 134)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 134 -Plan docs/superpowers/plans/2026-10-17-wingetrs-log-jobs.md`.

**Goal:** cada job (ponte winget/scoop e instalação/remoção/atualização própria, na TUI e na CLI) deixa no log o início e o fim com tudo que explica o resultado; e um `winget upgrade` que sai 0 sem instalar a versão esperada deixa de aparecer como "Atualizado".

**Pedido do usuário (2026-10-17):** "Os logs precisam ser mais completos." — o usuário atualizou 9 apps pela TUI e o log do wingetrs só tinha as 3 falhas (sem início, sem sucesso, sem versões, sem duração).

**O que a sessão de 2026-10-17 mostrou (logs reais do winget em `DiagOutputDir`):**
- `Google.Antigravity`: o wingetrs ofereceu 2.23.1 ⇒ 2.24.0. O winget instalou **2.23.1** de novo e saiu 0, porque a fonte local dele (`Microsoft.Winget.Source` 2026.1016.1226) era 15 h mais velha que o catálogo do wingetrs. A TUI tratou como sucesso; a linha continua "atualizável".
- `Google.AndroidStudio`: `0x8A150114` ("O pacote não pode ser atualizado usando WinGet. Use o método fornecido pelo editor"), sem mensagem própria em `winget_message`.
- `Anki.Anki`: `0x8A15002B` (versão nova existe mas não se aplica ao sistema).
- `Git.Git`: `0x8A150006`, instalador saiu 1 ("Defaulting to Cancel for suppressed message box (Retry/Cancel)", provavelmente arquivo em uso); o caminho do log do instalador estava só nas linhas de saída.
- Sucessos (Adobe Reader, CrystalDiskInfo, Docker Desktop, Chrome, gum) não deixaram nenhuma linha.

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- Log só por `tracing` (arquivo); nada em stdout fora de `main.rs`/`cli/`. Mensagens ao usuário em português.
- Sem crate nova. Testes não rodam winget nem scoop: o executor da ponte é injetado (`BridgeExec`).
- Arquivos do winget são só leitura; esta tarefa não lê nem grava nada do winget, só a saída do processo.

## Review Focus

1. Saída do winget em outro idioma (inglês `Found Git [Git.Git] Version 2.55.0.5`, português `Encontrado Git [Git.Git] Versão 2.55.0.5`) e com códigos ANSI ou acentos quebrados pelo pipe.
2. Versão esperada desconhecida (linha sem "Disponível") ⇒ nunca vira falha só por isso.
3. Linhas de saída com dados longos: o log de fim guarda no máximo as últimas 40 linhas.
4. Job cancelado ou com pânico: ainda há uma linha de fim.

---

### Task 134: Log de início e fim de cada job; winget que não muda a versão

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/bridge/mod.rs`: `BridgeJob { tool, target, name }` (~:38), montado em `bridge_target` (~:48–70) a partir da `ListRow`; `winget_updated` (~:178); `winget_message` (~:249); `strip` de ANSI/acentos perto de `REBOOT_MARKERS` (~:185–220).
- `src/tui/jobs.rs`: `run_bridge` (~:308–382) já lê a versão antes/depois só para scoop e só loga a falha (`tracing::warn!` ~:372); `run_action` (~:271) para os jobs próprios.
- `src/tui/run.rs` (~:1275): onde a TUI dispara `run_bridge` e recebe `Msg::InstallFinished`.
- `src/cli/install_cmd.rs`: install/uninstall/upgrade pela CLI.
- Captura de log em teste: `crate::spans::Log` (`tests/it/spans/mod.rs` ~:185, criado na 133b; grava eventos DEBUG+ formatados por thread).
- Módulo de teste novo **já criado pelo orquestrador**: `tests/it/job_log.rs` (vazio) com `mod job_log;` em `tests/it/main.rs`. Não edite o `main.rs`.

#### What binds this task

1. **`BridgeJob` ganha `installed: Option<String>` e `available: Option<String>`**, preenchidos em `bridge_target` pelas colunas da linha (a versão instalada principal e a disponível). Atualize as construções em testes.
2. **Linha de início** (`INFO`, alvo `wingetrs::job`, mensagem `job.start`) para todo job, com campos: `kind` (`bridge`/`install`/`remove`/`upgrade`), `tool` (`winget`/`scoop`/`wingetrs`), `target`, `name`, `from`, `to`, e para a ponte `cmd` (o `command_text` exato).
3. **Linha de fim** (`job.end`; `INFO` quando ok, `WARN` quando não) para todo job, inclusive cancelado/pânico (guard com `Drop` ou o ponto único onde a TUI recebe o resultado), com: `outcome` (`ok`/`sem_efeito`/`falhou`/`cancelado`), `code` (ponte), `ms` (duração), `after` (versão depois, quando conhecida), `message`, e nas falhas e em `sem_efeito` as **últimas 40 linhas** da saída. Na ponte winget, extraia também o caminho do log do instalador quando a saída o trouxer (linha que termina em `.log` com caminho absoluto) no campo `installer_log`. A linha `bridge upgrade failed` atual é substituída pela de fim (não duplique).
4. **Conferência do winget:** função pura `bridge::winget_chosen_version(target, lines) -> Option<String>` que acha a versão que o winget escolheu na linha `<nome> [<target>] <palavra> <versão>` (qualquer idioma: o id entre colchetes e a última palavra da linha), depois de `strip`. Em `run_bridge`, quando o código diz atualizado e `chosen` existe e `available` existe e `chosen != available`: resultado **não** é sucesso; `outcome=sem_efeito`, mensagem `"O winget instalou {chosen}, não {available}: o catálogo local do winget está mais velho que o do wingetrs. Tente de novo mais tarde."`. Quando `chosen` ou `available` falta, mantém o comportamento atual.
5. **`winget_message`:** `0x8A15_0114` ⇒ `"O editor não permite atualizar este pacote pelo winget; use o atualizador do próprio app."`.
6. **CLI:** install/uninstall/upgrade da CLI emitem as mesmas linhas `job.start`/`job.end` (mesma função auxiliar, sem duplicar o formato).

**Files:**
- Modify: `src/bridge/mod.rs`, `src/tui/jobs.rs`, `src/tui/run.rs`, `src/cli/install_cmd.rs`, testes que constroem `BridgeJob`
- Test: `tests/it/job_log.rs` (já existe vazio)

Testes (`tests/it/job_log.rs`, capturando com `crate::spans::Log`):
1. `chosen_version_pt_and_en`: `Encontrado Git [Git.Git] Versão 2.55.0.5` e `Found Git [Git.Git] Version 2.55.0.5` ⇒ `2.55.0.5`; com `ESC[32m` em volta, igual; linha de outro id ⇒ `None`.
2. `stale_winget_source_is_not_success`: exec falso devolve 0 e a linha `Encontrado Antigravity [Google.Antigravity] Versão 2.23.1`; job com `available = 2.24.0` ⇒ `Err` com a mensagem exata do item 4 e log com `outcome=sem_efeito`.
3. `same_version_ok_when_available_unknown`: mesmo exec, `available = None` ⇒ sucesso.
4. `start_and_end_logged_on_success`: exec 0 com a versão certa ⇒ log tem `job.start` com `from=`/`to=`/`cmd=` e `job.end` com `outcome=ok` e `ms=`.
5. `failure_keeps_last_40_lines_and_installer_log`: exec devolve `0x8A150006` e 60 linhas, uma delas `O registro do instalador está disponível em: C:\x\Git.log` ⇒ `job.end` `WARN`, só as linhas 21–60, `installer_log=C:\x\Git.log`; e não há mais a linha `bridge upgrade failed`.
6. `upgrade_not_allowed_message`: `winget_message(0x8A150114 as i32)` = texto exato do item 5.
7. `cli_job_logged`: um install pela CLI no cenário offline existente deixa `job.start` e `job.end` (`kind=install`).

#### Checks beyond the steps

1. Mutação: tirar a comparação `chosen != available` ⇒ teste 2 **PEGA**.
2. Mutação: guardar todas as linhas em vez das últimas 40 ⇒ teste 5 **PEGA**.
3. Mutação: não emitir `job.end` no sucesso ⇒ teste 4 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(log): start and end of every job; winget run that keeps the old version is not a success`

---

## Próximas (a detalhar depois da 134)

- **Task 135 — aplicabilidade antes de oferecer a ponte:** `UpgradeBehavior: deny` no manifesto ⇒ "só pelo editor" (Android Studio, `0x8A150114`); escopo instalado (HKLM × HKCU) sem instalador do mesmo escopo ⇒ "não se aplica" (Anki, `0x8A15002B`).
- **Task 136 — fonte do winget atrasada (decisão do usuário 2026-10-17: opção 1):** quando a conferência da 134 acusar `sem_efeito` porque o winget escolheu versão mais velha, rodar `winget source update --name winget` uma vez e repetir o upgrade uma vez; logar as duas tentativas.
- **Elevação única por lote (aguarda decisão do usuário):** hoje cada instalador de escopo máquina pede UAC separado (Docker, Acrobat, CrystalDiskInfo em 2026-10-17).
