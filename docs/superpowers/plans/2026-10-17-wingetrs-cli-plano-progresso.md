# wingetrs — plano enxuto e progresso por fases no CLI; Ctrl+C durante o msiexec (Tasks 139–140)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-17-wingetrs-cli-plano-progresso.md`.

**Goal:** o CLI mostra só as etapas que o usuário precisa considerar, com numeração em negrito e glifos, e um progresso de uma linha por fase parecido com o da TUI; um Ctrl+C durante o `msiexec /a` não mata o processo nem deixa lixo.

**Origem (prova ao vivo da 2b, 2026-10-17):**
- `wingetrs install main/rust --yes --allow-msiexec` ficou 7 min em `Baixando: 0.0 MiB`. O MSI estava em cache (nenhum progresso de download) e `cli/run_win.rs` passa todo `progress(label, done, total)` para `LineProgress::update(url, ...)`: o rótulo `executor::INSTALLER_RUNNING` virou mais uma "URL" com 0 bytes. O usuário fechou o processo; o `msiexec` (serviço) terminou sozinho às 20:30 e sobrou `apps\rust\.staging-1.99.1.msiexec` (sem `job.end` no log).
- Pedido do usuário: "O progresso quando está baixando por CLI deve ser melhor, com aspecto parecido com do TUI. Na descrição das etapas do CLI, deve formatar o texto que tenham informações, a numeração da lista pode ser negrito. Usar glyph para seta e outras coisas que forem possíveis. Pode ser mais objetivo também. Não precisa indicar a URL de download, o nome dos arquivos, etc. … só precisa indicar as etapas que forem relevantes e precisem efetivamente ser consideradas pelo usuário, como registrar no path, mexer em registros, etc."
- `which fastfetch` logo depois de instalar não achou nada: o terminal aberto não vê o PATH novo.

## Decisões do orquestrador

- O plano completo continua existindo: `impl Display for InstallPlan`/`UninstallPlan` não muda (snapshots do planejador dependem dele) e é o que `--dry-run` imprime. Instalar/atualizar/desinstalar sem `--dry-run` mostram o plano **resumido**.
- Glifos vêm de `tui::theme::Glyphs` conforme `--glyphs`/config (já carregados em `TtyUi`); estilos só de `cli/style.rs` (respeitam `--color`). Campo novo em `Glyphs` (ex.: `arrow`) precisa existir nos três conjuntos (nerd, unicode, ascii: `->`).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- Nada escreve em stdout fora de `cli/`; progresso vai para stderr e só quando stderr é TTY (como hoje).
- Sem crate nova. Texto ao usuário em português.
- Thread de animação (se houver) é juntada em todos os caminhos (guard com `Drop`), regra ⚙ do `CLAUDE.md`.
- Testes nunca rodam `msiexec` nem tocam PATH/registro reais.

## Review Focus

1. Saída redirecionada (`> arquivo`) ou `--color never`: nenhum código ANSI, nenhum `\r`, plano resumido legível.
2. Download de vários arquivos ao mesmo tempo, com e sem tamanho conhecido, e arquivo já em cache (zero bytes baixados).
3. Ctrl+C antes do `msiexec` começar: cancela e desfaz como uma falha; Ctrl+C durante o `msiexec`: não cancela, avisa, espera e limpa.
4. Atualização (`upgrade`) e desinstalação usam o mesmo resumo; etapas que mexem fora da pasta do app nunca somem do resumo.
5. Conjunto `ascii` de glifos num console sem Nerd Font.

---

### Task 139: Plano resumido e progresso por fases no CLI

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/install/plan.rs`: `enum Step` (~:53), `InstallPlan::step_line` (~:200–295) e `impl Display for InstallPlan` (~:299); `UninstallPlan` (~:85).
- `src/cli/install_cmd.rs`: `print_plans` (só `println!("{p}")`), `install`, `uninstall`, upgrade; `LineProgress` (~:93–135, soma bytes por URL e desenha `output::progress_line`).
- `src/cli/output.rs`: `progress_line` (~:340, `"Baixando: x / y (z%)"`), `uninstall_plan_text` (~:265).
- `src/cli/run_win.rs`: liga `progress` em `LineProgress::update` sem olhar o rótulo.
- `src/install/executor.rs`: `INSTALLER_RUNNING` (~:868), emitido a cada lançamento do `msiexec /a`; download chama `progress(url, done, total)` (`download.rs` ~:215). Nenhum outro rótulo existe hoje.
- TUI: `tui/view.rs` `progress_text` (~:1405) desenha `label [#####-----] 45%`; `tui/jobs.rs` `progress_msg` separa `INSTALLER_RUNNING`.
- `src/tui/theme.rs`: `Glyphs` (~:18) com `spinner`, `ok`, `toast_warn` etc.

#### What binds this task

1. **Plano resumido** (função pura nova em `cli/output.rs`, ex.: `plan_summary(plan, layout, glyphs) -> String`), usado por `install`/`upgrade` sem `--dry-run`:
   - cabeçalho: `Instalar <app> <versão>` (ou `Atualizar <app> <de> <seta> <para>`), com bucket e arquitetura em estilo apagado;
   - só estas etapas, numeradas com o número em negrito, na ordem do plano:
     - `Shim`: agrupados numa linha `Comandos: a, b, c`;
     - `Shortcut`: `Atalho no Menu Iniciar: <nome do .lnk sem extensão>`;
     - `EnvSet`: `Variável de usuário NOME = valor`;
     - `EnvAddPath` e `EnsureShimsOnPath`: `PATH do usuário <seta> <caminho>`;
     - `Persist`: `Dados mantidos entre versões: <in_version>` (agrupados);
     - `RemoveOldVersion`: `Remove a versão <v>`;
   - escondidas: `EnsureLauncher`, `Download`, `Extract`, `Promote`, `LinkCurrent`, `RecordState`. Plano sem nenhuma etapa visível imprime só o cabeçalho.
   - Se alguma fonte for MSI (`ArchiveKind` MSI), uma linha final apagada: `Se o extrator não suportar este MSI, pede --allow-msiexec`. (Não promete que o msiexec vai rodar.)
2. **Desinstalação resumida:** `uninstall_plan_text` passa ao mesmo estilo: número em negrito, `Apaga a pasta do app`, `Comandos: …`, `Atalho no Menu Iniciar: …`, `PATH do usuário <seta> remove <caminho>` (só quando o plano realmente remove), dados de `persist` mantidos ou apagados (`--purge`).
3. **Progresso por fase** (`LineProgress` vira um desenhista de fases; nome livre):
   - download: `<spinner> Baixando [#####-----] 45%  12.3 / 27.0 MiB` (barra igual à da TUI, 20 colunas); sem total: `<spinner> Baixando 12.3 MiB`;
   - rótulo `INSTALLER_RUNNING`: nunca soma como URL; troca a linha para `<spinner> Windows Installer (msiexec /a) · não pode ser cancelado · <mm:ss>`;
   - depois que todos os downloads terminam (todos com `done == total`) e antes do fim: `<spinner> Instalando…`;
   - o spinner e o relógio avançam com uma thread de animação (~120 ms) só enquanto há linha desenhada, encerrada e juntada em `finish` e no `Drop`;
   - nada disso quando stderr não é TTY (como hoje).
4. **Fim:** depois de `Instalado:`, se o plano tem `EnsureShimsOnPath`/`EnvAddPath` e a pasta não estava no PATH do processo atual, uma linha: `Abra um terminal novo para usar: a, b` (comandos do plano).
5. `--dry-run` continua imprimindo o plano completo (`Display`), sem mudança.

**Files:**
- Modify: `src/cli/output.rs`, `src/cli/install_cmd.rs`, `src/cli/run_win.rs`, `src/tui/theme.rs` (só o glifo novo), testes que fixam a saída do CLI (insta, se houver)
- Test: `tests/it/cli_plan_summary.rs` (**já criado vazio e registrado pelo orquestrador**; não edite `tests/it/main.rs`)

Testes (`tests/it/cli_plan_summary.rs`, com `style::with_enabled(false, …)` para texto puro e um com cores):
1. `summary_hides_internal_steps`: plano com Download, Extract, Promote, LinkCurrent, RecordState, 2 Shims, EnsureShimsOnPath ⇒ duas linhas numeradas (`Comandos: a, b` e `PATH do usuário`), sem URL, sem nome de arquivo, sem `state.db`.
2. `summary_keeps_every_outside_effect`: Shortcut, EnvSet, EnvAddPath, Persist, RemoveOldVersion ⇒ cada um aparece.
3. `summary_upgrade_header_uses_arrow`: `upgrade_from = Some("1.0")` com glifos ascii ⇒ cabeçalho contém `1.0 -> 2.0`.
4. `summary_msi_note`: fonte `.msi` ⇒ nota do `--allow-msiexec`; fonte `.zip` ⇒ sem nota.
5. `bold_numbers_when_colored`: com cores ligadas, o número vem envolto no estilo negrito de `style.rs`; desligadas, nenhum `\x1b`.
6. `installer_label_is_not_a_download`: o formatador de fase recebe `(url, 5 MiB, Some(10 MiB))` e depois `(INSTALLER_RUNNING, 0, None)` ⇒ a linha passa a ser a do Windows Installer, sem "Baixando".
7. `cached_download_shows_no_zero_line`: só `INSTALLER_RUNNING` (MSI em cache) ⇒ nunca `Baixando: 0.0 MiB`.
8. `uninstall_summary`: plano com pasta, 2 comandos, atalho e remoção de PATH ⇒ as linhas do item 2.

#### Checks beyond the steps

1. Mutação: tratar `INSTALLER_RUNNING` como URL de novo ⇒ teste 6 ou 7 **PEGA**.
2. Mutação: esconder `EnvAddPath` ⇒ teste 2 **PEGA**.
3. Rode `target/release/wingetrs.exe install main/fastfetch --dry-run` e um `install`/`uninstall` real de `main/fastfetch` (raiz real autorizada para este app pequeno; desinstale no fim) e cole a saída no relatório; diga se a animação aparece (TTY) e cole também a saída com `2>&1 | cat` (não TTY).
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): short install plans and phase progress like the TUI`

---

### Task 140: Ctrl+C no CLI durante a instalação; o msiexec nunca fica órfão

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/cli/mod.rs`/`src/cli/run_win.rs`: o CLI não trata Ctrl+C hoje (sem `tokio::signal`); o Windows mata o processo. Durante o `msiexec /a` isso deixa `apps\<app>\.staging-<v>.msiexec` (o `RemoveWork` e o `AwaitWorker::drop` não rodam) e o `job.end` não é gravado.
- `src/install/executor.rs`: `AwaitWorker` (~:957; o `Drop` espera o worker e só então apaga o staging), `extract_with_msiexec` (~:891, `RemoveWork`), o `started` que vira `INSTALLER_RUNNING`.
- TUI: a política já existe (`tui/run.rs`): o job de reserva só fica não cancelável a partir do instante em que o `msiexec` arranca (Task 129, `657e155`).
- `tokio` tem `features = ["full"]` (inclui `signal`).
- **Desde a Task 141:** o rótulo `INSTALLER_RUNNING` não existe mais. O progresso é `install::progress::ProgressEvent` (`Download` | `Phase(Phase)`), e o `msiexec` começa em `Phase::Installer { log }` (antes dela vêm `WaitingInstaller` e, depois, `Placing`). "Depois do `INSTALLER_RUNNING`" abaixo quer dizer "depois do primeiro `Phase::Installer`". O uninstall emite `Removing`/`Scanning`; Ctrl+C no uninstall segue a regra de cancelar antes de apagar e não interromper a remoção já começada (diga como ficou).

#### What binds this task

1. Operações do CLI que mudam o disco (`install`, `upgrade`, `uninstall`) rodam com um tratador de Ctrl+C (`tokio::signal::ctrl_c`):
   - antes do `msiexec` começar: o primeiro Ctrl+C cancela a operação soltando o future (o executor já desfaz e apaga o staging), imprime `Cancelado.` e sai com código ≠ 0; o `job.end` sai com `outcome=cancelado`;
   - depois do `INSTALLER_RUNNING`: Ctrl+C **não** cancela; a linha de progresso mostra `o Windows Installer não pode ser cancelado; esperando terminar` e a instalação segue até o fim normal (sucesso ou falha), limpando `.msiexec` como sempre.
2. Lixo de execução morta: ao começar a instalar um app, `.staging-*.msiexec` e `.wingetrs-extract-*` daquele app já são apagados (confira; se não forem, apague-os como "lixo nosso", igual ao `.staging`). Uma raiz de app sem versão registrada e só com lixo desses nomes é apagada pelo `wingetrs repair` (diga se o `repair` já faz).
3. Fechar a janela do console (`CTRL_CLOSE_EVENT`) não tem como esperar 7 min: diga no relatório o que acontece e deixe o item 2 cobrir o lixo.

**Files:**
- Modify: `src/cli/run_win.rs`, `src/cli/install_cmd.rs`, `src/install/executor.rs` (se a fase precisar ser exposta), `src/cli/mod.rs` (se preciso), `src/install/ops.rs` (se o `repair` mudar)
- Test: `tests/it/cli_cancel.rs` (**já criado vazio e registrado pelo orquestrador**; não edite `tests/it/main.rs`)

Testes (`tests/it/cli_cancel.rs`, sem sinal real: injete o "Ctrl+C" como um future/canal):
1. `cancel_before_installer_rolls_back`: execução com download falso lento, cancelamento injetado ⇒ resultado cancelado, staging apagado, nada em `apps\<app>\<v>`.
2. `cancel_during_installer_waits`: `Msiexec` falso que sinaliza `started` e espera um canal; cancelamento injetado depois do `started` ⇒ a operação continua, o `Msiexec` falso termina, a instalação conclui e `.msiexec` some.
3. `stale_msiexec_work_dir_is_removed`: `apps\<app>\.staging-<v>.msiexec\SourceDir\x` pré-criado ⇒ uma nova instalação o remove.

#### Checks beyond the steps

1. Mutação: deixar o cancelamento valer depois do `started` ⇒ teste 2 **PEGA**.
2. Mutação: não apagar o `.msiexec` velho ⇒ teste 3 **PEGA**.
3. Prova manual que o orquestrador faz com o usuário: Ctrl+C no `install main/rust --allow-msiexec` durante o Windows Installer.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(cli): Ctrl+C cancels an install before msiexec starts and waits once it runs`

---

### Task 141: Progresso tipado por fase e sinais de vida (nunca uma tela parada)

**Modelo:** opus (revisor opus). Roda **antes** da Task 140 (a 140 passa a usar a fase tipada do Windows Installer).

**Pedido do usuário (2026-10-17):** "Melhore o feedback para o usuário. Pq precisa ter um spinner, com algum aviso de demora, ou outra coisa que faça com que o usuário saiba que não travou e que está acontecendo alguma coisa."

**Medido na prova da 2b:** `install main/rust --allow-msiexec` com o MSI em cache: nenhum evento de download, a extração própria (que recusa o MSI) não emite nada, e o `msiexec /a` levou 7 min sem nenhum sinal na tela. O log `/l*v` do `msiexec` (UTF-16LE com BOM `FF FE`) cresceu até 142 MB, com uma linha `File: <caminho>\SourceDir\<relativo>;	To be installed;…` por arquivo copiado.

#### Where this fits

- `src/install/download.rs:89`: `pub type Progress<'a> = &'a (dyn Fn(&str, u64, Option<u64>) + Send + Sync)`; download chama `progress(url, done, total)` (~:215).
- `src/install/executor.rs`: `ExecCtx.progress` (~:81); passos `Download`/`Extract`/`Promote`/`Shim`/…; `INSTALLER_RUNNING` (~:868) mandado pelo mesmo callback (~:709); `extract_with_msiexec` (~:891) sabe o caminho do log (`layout.msiexec_log(app, attempt)`) e do `SourceDir`.
- `src/install/msiexec_win.rs`: `msiexec_admin` (laço de `try_wait` a cada 250 ms; `WARN_AFTER` 600 s só no log).
- Consumidores: `src/cli/run_win.rs` + `cli/install_cmd.rs` (`LineProgress`, Task 139) + `cli/output.rs` (`Phases`); `src/tui/jobs.rs` (`progress_msg`, `ProgressThrottle`, `Msg::InstallerRunning`/`InstallProgress`) + `tui/view.rs` (`progress_text`). `grep -rn "progress" src/cli src/tui src/install` antes de mudar.
- Armadilha: `docs/ARMADILHAS.md`, "Rótulo especial no callback de progresso só tratado num consumidor".

#### What binds this task

1. **Evento tipado** no lugar do rótulo texto: `Progress` passa a receber `&ProgressEvent` (`install::progress` ou `download.rs`):
   - `Download { url, done, total }`;
   - `Phase(Phase)` com `Phase::{Preparing, Extracting { file, engine }, Installer { log: PathBuf }, Placing, Linking, Recording}` (nomes livres, um por passo do executor que pode demorar); o executor emite a fase **ao começar** cada passo, inclusive quando o download veio do cache;
   - `INSTALLER_RUNNING` some (vira `Phase::Installer`). Todo consumidor faz `match` exaustivo (sem `_ =>`), para o compilador apontar o consumidor esquecido.
2. **Linha sempre viva no CLI** (TTY): do primeiro instante da operação até o fim há uma linha com spinner animado e o relógio da fase atual (`mm:ss`):
   - `⠋ Baixando [#####-----] 45%  12.3 / 27.0 MiB · 3.1 MiB/s · ~6 s` (velocidade média dos últimos ~3 s, estimativa só com total conhecido);
   - download sem bytes novos há 10 s: a linha ganha `· sem dados há 12 s` (em estilo de aviso);
   - `⠋ Extraindo rust-…msi (MSI) · 00:04`, `⠋ Colocando os arquivos · 00:01`, `⠋ Criando comandos`, `⠋ Registrando`;
   - Windows Installer: `⠋ Windows Installer · 03:12 · 18.304 arquivos · share/doc/rust/html/…/constant._MM_FROUND_FLOOR.html` — o contador e o arquivo atual vêm do **fim** do log do `msiexec` (ler só os últimos ~8 KiB a cada ~1 s, decodificar UTF-16LE alinhado em par, achar a última linha `File: …;	To be installed`; contador = aumento estimado pelo número dessas linhas desde a última leitura, ou omitido se não der para contar sem ler o arquivo todo — diga o que fez). Caminho cortado pela esquerda para caber em 100 colunas.
   - Windows Installer sem crescimento do log há 60 s: `· sem atividade há 1 min; o Windows Installer pode estar esperando outro instalador`.
   - Primeira vez que o Windows Installer passa de 30 s: uma linha fixa (não redesenhada) acima da animação: `O Windows Installer pode levar vários minutos em pacotes grandes; não feche o terminal.`
3. **TUI:** a barra de status mostra a mesma fase e relógio (reaproveitando a função pura de texto da fase do CLI, sem ANSI), com o throttle atual; o `Msg::InstallerRunning` continua bloqueando o cancelamento como hoje.
4. **Não TTY:** nenhuma animação; uma linha simples por mudança de fase (`Baixando…`, `Extraindo…`, `Windows Installer…`), para logs redirecionados mostrarem o andamento.
5. **Leitura do log é só leitura**, no processo do wingetrs (não elevado), arquivo aberto com compartilhamento de leitura/escrita (o serviço do Windows Installer continua gravando); erro de leitura ⇒ a linha mostra só fase e relógio.

**Files:**
- Modify: `src/install/download.rs`, `src/install/executor.rs`, `src/install/ops.rs`, `src/cli/run_win.rs`, `src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/tui/jobs.rs`, `src/tui/run.rs`, `src/tui/view.rs`, testes que constroem `Progress`/`ExecCtx`
- Create: `src/install/msiexec_log.rs` (tail UTF-16LE, pura sobre `Read + Seek`)
- Test: `tests/it/install_progress.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/install_progress.rs`):
1. `cached_install_emits_phases_in_order`: execução com download em cache (ctx falso) ⇒ eventos `Phase` na ordem dos passos, sem nenhum `Download`, primeiro evento antes da extração.
2. `phase_line_has_spinner_and_clock`: o formatador puro com relógio injetado (`Instant` de base + agora) ⇒ `Extraindo … · 00:04`.
3. `download_line_speed_eta_and_stall`: amostras injetadas (tempo + bytes) ⇒ velocidade e estimativa; 12 s sem bytes ⇒ `sem dados há 12 s`.
4. `msiexec_log_tail_reads_last_file`: log UTF-16LE com BOM gerado no teste (várias linhas `File: C:\x\SourceDir\a\b.txt;	To be installed;…`), lido por `Cursor` ⇒ arquivo atual `a\b.txt`; início do trecho caindo no meio de um caractere (offset ímpar) ⇒ ainda decodifica.
5. `installer_idle_warning_after_60s`: tamanho do log parado por 61 s (relógio injetado) ⇒ aviso de sem atividade.
6. `not_a_tty_prints_one_line_per_phase`: modo não TTY ⇒ uma linha por fase, sem `\r` e sem ANSI.
7. `tui_status_shows_phase_and_clock`: estado sintético da TUI com `Phase::Installer` ⇒ texto da barra de status com relógio.

#### Checks beyond the steps

1. Mutação: o executor deixar de emitir a fase quando o download vem do cache ⇒ teste 1 **PEGA**.
2. Mutação: decodificar o log sem alinhar em par ⇒ teste 4 **PEGA**.
3. Mutação: tirar o aviso de 60 s ⇒ teste 5 **PEGA**.
4. Rode `install main/fastfetch` e `uninstall` reais (raiz real autorizada para esse app) e cole a saída não TTY. A prova TTY com o `rust` é feita pelo orquestrador com o usuário.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): typed progress phases with live signs in the CLI and the TUI`


---

## Depois das fases 2c/2d (pedido do usuário em 2026-10-18)

Pedido literal: "Quero que todas as saídas do CLI sejam devidamente formatadas. Além disso, opções devem ser sempre em forma de TUI. Sempre deve ser mostrado progresso, em todas as formas de instalações, atualizações e desinstalações. Quando for abrir solicitação de elevação, deve mostrar um aviso no CLI e TUI como 'aguardando autorização para executar como administrador'." Começa quando as Tasks 164, 192, 196 e 197 fecharem (antes da prova ao vivo final).

### Task 199: Inventário e formatação de toda saída do CLI

**Modelo:** opus (revisor opus).

#### Where this fits

- Saída do CLI: `src/cli/output.rs` (formatadores puros), `src/cli/style.rs` (`choose` de cor, TTY, `NO_COLOR`/`CLICOLOR_FORCE`), `src/cli/install_cmd.rs` (29 `println!`/`eprintln!` em 2026-10-18), `src/cli/winget_cmd.rs` (9), `src/cli/mod.rs` (5); glifos em `tui::theme::Glyphs`.
- Regra do `CLAUDE.md` (UX): número em negrito, glifos, só as etapas que o usuário precisa considerar; URL/hash/staging só no `--dry-run`.

#### What binds this task

1. Inventário (no relatório) de **toda** linha que o CLI escreve, por comando (`update`, `search`, `show`, `list`, `install`, `uninstall`, `upgrade`, `cleanup`, erros do `anyhow` na borda, `--help`): onde nasce e como sai hoje.
2. Toda saída passa por formatadores de `output.rs` (nenhum `println!` com texto montado fora dele): títulos e nomes em negrito, números de lista em negrito, sucesso/aviso/erro com glifo e cor de `style.rs`, caminhos e ids em estilo próprio, tabelas alinhadas pela largura real (caracteres largos/acentos). Sem cor e sem ANSI quando não é TTY, `NO_COLOR` ou `--color never`.
3. Mensagens de erro: uma linha "✗ <o que falhou>" + uma linha "→ <o que fazer>" quando houver ação possível; cadeia de causas do `anyhow` só com `-v`.
4. Teste de regra mecânica em `scripts/check_rules.ps1`: `println!`/`eprintln!`/`print!` em `src/cli/` só dentro de `output.rs` e de um módulo de escrita único (prove plantando a violação).

#### Tests (literais)

- Snapshots (`insta` já em uso nos testes de help? se não, texto literal) de cada comando em TTY com cor, TTY sem cor e não TTY.
- `error_has_action_line`, `no_ansi_when_not_tty`, `wide_chars_align_table`.

**Files:** `src/cli/*`, `scripts/check_rules.ps1`, testes `tests/it/cli_*`.

#### Checks beyond the steps

1. Roda depois das Tasks 200 e 201 (mesmos arquivos do CLI); confira `git status` antes.
2. Snapshots novos revisados à mão (nenhum texto some sem motivo).
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): every output line goes through the formatter`

### Task 200: Escolhas do CLI sempre como seletor no estilo TUI

**Modelo:** opus (revisor opus).

#### Where this fits

- Perguntas de hoje: `install_cmd.rs` ~:24 trait com `fn confirm(&self, question)` (implementação real ~:178 lê `stdin`), `choose` ~:607, `src/cli/picker.rs` (seletor existente: veja se já é interativo), escolhas do caso Kindle (Task 193), confirmação por item do `msiexec` (2b), consentimento de scripts (Task 161), órfãos (`cleanup --orphans`).
- `ratatui`/`crossterm` já são dependências (TUI).

#### What binds this task

1. Um componente único de escolha para o CLI (lista com setas, Enter confirma, Esc/q cancela, atalho por letra, item padrão destacado, glifos e cores do tema), desenhado inline (sem tela alternativa), que limpa o que desenhou ao terminar e deixa uma linha com a escolha feita.
2. **Toda** pergunta do CLI usa esse componente: sim/não, escolha entre opções (Kindle: copiar e remover / só o registro / cancelar), seleção múltipla (órfãos, itens de lote) com espaço para marcar.
3. Sem terminal (stdin/stdout não TTY): nunca pergunta; sem `--yes` recusa com a mensagem de qual flag usar; `--yes` nunca escolhe por cima do que exige escolha do usuário (regra existente).
4. Ctrl+C dentro do seletor = cancelar, terminal restaurado (raw mode desligado em todos os caminhos, inclusive pânico: guard com `Drop`).

#### Tests (literais)

- Seletor dirigido por eventos injetados (setas, Enter, Esc, espaço, letra) ⇒ resultado; buffer desenhado comparado a texto.
- `selector_restores_terminal_on_panic` (guard), `no_tty_never_prompts`.
- Um teste por pergunta existente provando que ela usa o componente.

**Files:** `src/cli/picker.rs` (ou novo `src/cli/select.rs`), `src/cli/install_cmd.rs`, `src/cli/winget_cmd.rs`, testes `tests/it/cli_*`.

#### Checks beyond the steps

1. A Task 201 edita `src/install/progress.rs`, `src/tui/*`, `src/winget_install/*` e `src/bridge/elevate_win.rs` em paralelo; `src/cli/output.rs` é da 201 nesta rodada (só os textos de fase): não edite. Commit só dos seus trechos.
2. Nenhum teste lê o terminal real: eventos e backend injetados.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): every choice is an inline TUI selector`

### Task 201: Progresso em toda instalação, atualização e remoção; aviso de elevação

**Modelo:** opus (revisor opus).

#### Where this fits

- Fases tipadas (`install::progress`, Tasks 139/141) e as fases da 2d (`WaitingElevation`, `Uninstalling`, `CheckingRemoval`, Task 193); `winget_install::removal`; executor do Scoop; ponte do winget; MSIX (`PackageManager`); reserva `msiexec /a`; scripts (`PwshHost`); innounp; lote na TUI.
- Pontos que abrem UAC: subida do `bridge-worker` (`elevate_win.rs`), `runas` por instalador (Task 196), exe que se eleva sozinho (`ElevationRequirement: elevatesSelf`), `msiexec /a` da reserva quando pede elevação.

#### What binds this task

1. Matriz (no relatório) de cada caminho × (instalar, atualizar, remover) × (CLI, TUI): que fases aparecem, do primeiro instante ao fim. Todo caminho tem fase com spinner e relógio desde o início, aviso de lentidão e "não pode ser cancelado" quando for o caso. Falta ⇒ corrigir.
2. Antes de **todo** pedido de elevação, CLI e TUI mostram "Aguardando autorização para executar como administrador" (mesmo texto, uma constante), com o relógio correndo, até o UAC voltar; recusado ⇒ "Permissão de administrador recusada." Exe que se eleva sozinho: o aviso aparece quando o manifesto diz `elevatesSelf`/`elevationRequired`.
3. Teste por caminho da matriz com progresso falso/eventos capturados; teste de que o texto do aviso é o mesmo constante nos dois lados.

#### Tests (literais)

- `every_path_emits_first_phase_immediately` (tabela de caminhos, eventos capturados).
- `elevation_notice_before_every_uac` (worker, runas por instalador, elevatesSelf) e `elevation_notice_same_text_cli_tui`.

**Files:** `src/install/progress.rs`, `src/cli/output.rs`, `src/tui/*` (barra de status e jobs), `src/winget_install/*`, `src/bridge/elevate_win.rs`; testes existentes.

#### Checks beyond the steps

1. A Task 200 edita `src/cli/picker.rs` (ou `select.rs`), `src/cli/install_cmd.rs` e `src/cli/winget_cmd.rs` (perguntas); nesses dois últimos faça só os trechos de progresso e commit só deles.
2. Testes nunca abrem UAC real: lançador falso.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat: progress on every install path and an elevation notice before every UAC`
