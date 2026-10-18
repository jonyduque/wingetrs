# wingetrs — Fase 2c: scripts PowerShell, innosetup e installer.file/args (design)

Data: 2026-10-15
Status: **decidida pelo usuário em 2026-10-18** (ver "Decisões do usuário" ao fim da seção b). Depende da 2b.
Rascunho escrito por um agente opus em modo só leitura, a partir do fonte do Scoop (master em 2026-10-15). Nada foi executado.

## (a) Objetivo e escopo

Instalar, remover e atualizar na raiz própria (`%LOCALAPPDATA%\wingetrs\apps`) os manifestos que hoje caem em `NãoSuportado` por causa de `pre_install`, `post_install`, `installer`, `uninstaller`, `pre_uninstall`, `post_uninstall`, `psmodule` ou `innosetup`.

As chaves de script estão em `SCRIPT_KEYS`, em `src/install/manifest.rs:204`; o innosetup é tratado em `:226`.

Sub-casos, em ordem de risco:

| Sub-caso | Roda código de terceiros? | Precisa de PowerShell? |
|---|---|---|
| S-psm: `psmodule` | não | não (junction + `PSModulePath` em `HKCU\Environment`) |
| S-inno: `innosetup: true` | só o innounp, com hash conferido; o instalador não roda | não |
| S-file: `installer.file`/`args` com `.exe`, `.msi` ou `.cmd` | sim, com os privilégios do usuário | não |
| S-ps1: `installer.file` terminando em `.ps1` | sim | sim |
| S-hook: os hooks de script | sim | sim |

**Fora do escopo:**
- `$global`/instalação de máquina e qualquer coisa que exija admin;
- instalar no `~/scoop`;
- pacotes winget (são da 2d).

**Teto de cobertura** (spec 2a, catálogo de 2026-10-09): 17,4% do Main e 45,6% do Extras. A divisão por sub-caso **não foi medida**; `examples/oracle_2c.rs` (P1) vai medi-la.

**Fatos do fonte do Scoop:**
- **Ordem em `install_app`:** download → extração (o innounp roda aqui) → `pre_install` → `Invoke-Installer` (file/args, depois script) → `link_current` → shims → atalhos → psmodule → `env_add_path` → `env_set` → persist → `post_install` → grava `manifest.json`/`install.json`.
- **`$dir` muda no caminho:** `pre_install` vê a pasta da versão e `post_install` vê o `current` (a confirmar na sonda D3).
- **`Invoke-HookScript`:** roda o script com `Invoke-Command ([scriptblock]::Create(...))` no escopo de `install_app`, então o script vê todas as variáveis locais dali.
- **`Invoke-Installer`:**
  - recusa arquivo fora do app;
  - substitui só `$dir`, `$global` e `$version` em `args`;
  - roda `.ps1` dentro do próprio PowerShell e o resto via `Invoke-ExternalCommand`;
  - apaga o arquivo salvo, a menos que `keep`.
- **`$cmd`:** é o subcomando do `scoop.ps1`. O `vscode` usa `if ($cmd -eq 'uninstall')`.
- **`core.ps1`:**
  - tem 78 funções;
  - `$scoopdir` vem de `$env:SCOOP` antes de qualquer outra fonte;
  - carregar o arquivo não grava em disco;
  - `shim` chama `Add-Path`, que grava o PATH do usuário.
- **innosetup:** `Expand-InnoArchive` roda `innounp -x -d<dest> <arq> -y -c{app}[\<extract_dir>]`. O innounp está no Main 2.71.2 e é **GPL-3.0-only**. A URL aponta para um branch, então o hash muda junto com o manifesto.
- **psmodule:** cria a junction `modulesdir\<name>` e põe `modulesdir` na frente do `PSModulePath`. Na remoção, não mexe no `PSModulePath`.
- **Lacuna nossa:** `src/sources/scoop/bucket.rs` guarda só `bucket/*.json`. Os scripts usam `$bucketsdir\<bucket>\scripts\...` (casos `python` e `vscode`).

## (b) Decisões em aberto

| # | Pergunta | Opções | Recomendação |
|---|---|---|---|
| D1 | Como executar os scripts | **A** pwsh + lib real do Scoop, fixada num commit e baixada para `scoop-lib\<commit>\` com SHA-256 por arquivo; `SCOOP`, `SCOOP_GLOBAL`, `SCOOP_CACHE` e `XDG_CONFIG_HOME` apontam para a nossa raiz; um prelúdio substitui as funções de efeito global. **A'** a lib do `~/scoop` do usuário. **B** prelúdio próprio, reimplementando 78+ funções. **C** análise estática só para recusar. **D** ponte `scoop install` (instala no `~/scoop`) | **A**, com **C** como portão e **D** como alternativa quando o portão recusar e houver Scoop |
| D2 | Regra "nunca escrever fora de `%LOCALAPPDATA%\wingetrs`" | 1 exceção explícita e delimitada (texto abaixo). 2 recusar scripts que a análise mostre escrevendo fora (garantia falsa). 3 token restrito (complexo, não medido) | **1** |
| D3 | Confirmação | 1 script nunca é aceito só com `--yes`: a CLI exige `--allow-scripts` e a TUI sempre mostra o script. 2 lembrar a confirmação por `manifest_sha256`. 3 confiar no main e no extras | **1** |
| D4 | Política de execução | 1 `-ExecutionPolicy Bypass -File runner.ps1` no processo filho, detectando a GPO. 2 `RemoteSigned`. 3 `-Command` | **1**, com `-NoProfile -NonInteractive` e saída UTF-8 |
| D5 | Rollback dos efeitos do script | 1 desfazer os nossos passos e listar os efeitos detectados (diff de `HKCU\Environment` + varredura de resíduos). 2 rodar o `uninstaller.script` ao falhar. 3 snapshot amplo (0,9–3 s, Task 33) | **1** |
| D6 | `$bucketsdir` e `scripts/` | 1 o `update` guarda `<repo>/scripts/**` (com `safepath` e troca atômica). 2 baixar na hora. 3 recusar manifestos que usam `scripts/` | **1** |
| D7 | Análise estática | 1 léxico mínimo em Rust; o que não for analisável vira "desconhecido" com confirmação reforçada. 2 AST do pwsh (custo por manifesto; seria um segundo caminho) | **1**; o AST serve só para validar o léxico (sonda) |
| D8 | Elevação | 1 recusar com o wingetrs elevado; o portão recusa `RunAs`, `sudo`, `HKLM:`, `-Global` e escrita em `ProgramFiles`/`windir`. 2 permitir com confirmação | **1** (o UAC fica para a 2d) |

### Decisões do usuário (2026-10-18)

- **D1 = B: prelúdio próprio.** Nada da lib do Scoop é baixado nem carregado. O runner dot-source um prelúdio do wingetrs que implementa em PowerShell (ou intercepta como efeito em `effects.jsonl`) **só as funções do Scoop que os manifestos reais chamam**, medidas pelo oráculo (P1). Função do Scoop que o prelúdio não tem ⇒ o portão (P9) recusa com o nome dela e o motivo "função do Scoop ainda não suportada". Consequências: a sonda D2 e a P10 ("Lib do Scoop fixada") saem; entra a **P10' Prelúdio próprio** (lista medida, uma função por vez, teste por função com pwsh falso ou script sintético); o risco "substituições desatualizadas" vira "manifesto usa função que não temos" (recusa clara + contagem no oráculo); a variável `$scoopdir` e afins são definidas pelo prelúdio apontando para a nossa raiz.
- **`if ($global)` (2026-10-18, depois da Task 163): opção E.** Instalação para todos os usuários (`scoop install -g`) continua fora do escopo: exigiria rodar script de terceiros como administrador e gravar em `C:\ProgramData` (grupo Usuários cria arquivos lá). O script roda **sem alteração**; o runner fixa `$global` como constante `$false` (`Constant,AllScope`, sonda em pwsh 7 e 5.1: 14 formas de reatribuir falham), e o portão trata o código dentro de `if ($global) { ... }` como informativo ("só roda em instalação global"); bloco que o portão não delimita com certeza segue as regras normais. Rejeitada a opção C (apagar o bloco do texto que roda): alterar código de terceiros quebra scripts futuros de formas imprevistas. Task 164.
- **D2–D8:** como recomendado (D2 = 1, D3 = 1, D4 = 1, D5 = 1, D6 = 1, D7 = 1, D8 = 1).

### Emenda pós-sondas (2026-10-18, `docs/superpowers/reports/2026-10-18-sondas-2c.md`)

- **Prelúdio (D1 = B):** a lib do Scoop (commit `e6aa3b3`) tem 263 funções; os manifestos chamam **42** (3270 hooks medidos). O prelúdio implementa essas 42 (ou intercepta como efeito); o resto é recusado pelo portão. `scoop` chamado como comando externo dentro de hook (367 manifestos do nerd-fonts, 18 do Main, 34 do Extras) é tratado à parte: recusado com motivo claro, ou mapeado para operações do wingetrs só se o oráculo mostrar que vale.
- **`$cmd`** vem por escopo dinâmico do `exec`; no upgrade, os hooks de **instalação da versão nova** também veem `'update'`.
- **`$dir`:** no `post_install` é a junção `current`, e `$original_dir` é a pasta da versão. Nos hooks de desinstalação durante o upgrade, `$version` é a versão **nova** e `$manifest` o **antigo**. O runner reproduz isso (teste por hook).
- **`installer`:** o Scoop roda `installer.file/args` **e depois** `installer.script` (não um ou outro).
- **`$modulesdir`** só existe como função (`modulesdir`).
- **innounp:** sai com código 0 e **zero arquivos** quando o filtro `-c{app}\<sub>` não casa ⇒ `ExtractInno` conta os arquivos extraídos e trata zero como erro (`MissingExtractDir`). Saída em OEM CP850 com CRLF.
- **Política de execução:** bloqueia `.ps1` em disco (`-File` e dot-source), não `-Command`/`iex`; `-ExecutionPolicy Bypass` no processo filho resolve, exceto sob GPO (não reproduzido; mensagem documentada).
- **`scripts/` dos buckets (D6):** Main 15 arquivos / 11,9 KB; Extras 98 / 195 KB. Custo no `update` não medido (medir na tarefa da P8).

**Texto proposto para o CLAUDE.md (D2-1):**

> **Exceção da fase 2c (código de terceiros).** Os scripts de manifesto e os instaladores de `installer.file` são código de terceiros que roda com os privilégios do usuário e pode gravar em qualquer lugar que o usuário pode. O **código do wingetrs** continua limitado aos lugares da 2a. As funções do Scoop que mexem em PATH, variáveis, shims, atalhos e `PSModulePath` são interceptadas e aplicadas pelo wingetrs, que as registra na state.db. O wingetrs nunca roda script sem a confirmação que mostra o script e a análise, nunca roda script com o processo elevado e nunca roda script de manifesto real em teste. Os testes usam pwsh falso ou scripts sintéticos numa raiz temporária. Agentes nunca executam scripts de manifesto reais nem o innounp sobre instaladores reais sem pedido do usuário.

## (c) Design

- **Manifesto.** `Resolved` ganha:
  - `hooks`;
  - `installer` e `uninstaller` (`InstallerSpec{file, args, keep, script}`);
  - `psmodule`;
  - `innosetup`;
  - `helpers` (`Innounp | SevenZip | Lessmsi | Dark`, pela mesma regra do `Get-InstallationHelper`).

  A variante de arquitetura sobrepõe a raiz chave a chave. `installer.file` passa pelo `safepath`.
- **Passos do install:**
  ```
  EnsureLauncher, EnsureHelper*, Download*, Extract*|ExtractInno, Promote,
  Hook{PreInstall}, RunInstaller|Hook{InstallerScript}, LinkCurrent, Shim*, Shortcut*,
  PsModule, EnvAddPath*, EnvSet*, EnsureShimsOnPath, Persist*, Hook{PostInstall},
  WriteManifest, RecordState, RemoveOldVersion?
  ```
  Os hooks rodam **depois do `Promote`**, para que o `$dir` gravado em arquivos não seja o do staging. `WriteManifest` grava `manifest.json` na pasta da versão, e a remoção usa os scripts desse arquivo.
- **Uninstall:** `PreUninstall` → `RunUninstaller|UninstallerScript` → as remoções da 2a → `PostUninstall`, com `$cmd='uninstall'`. No upgrade, `$cmd='update'`.
- **innounp e auxiliares:**
  - os auxiliares são apps instalados do bucket main e marcados `helper=1`;
  - `ExtractInno` roda em Rust via `spawn_win::run`, que passa para `src/proc/`;
  - o programa é injetável;
  - código ≠ 0 ⇒ rollback.
- **RunInstaller (sem `.ps1`):**
  - substitui só `$dir`, `$global` (sempre `False`) e `$version`;
  - apaga o arquivo, a menos que `keep`;
  - código ≠ 0 ⇒ rollback e o aviso da D5.
- **Portão estático (`script_scan.rs`, puro)** ⇒ `ScriptReport{commands, variables, flags, verdict}`:
  - `Refuse`: elevação, HKLM, `-Global`, escrita em `ProgramFiles`/`windir`, função que não está na lib fixada;
  - `ConfirmStrong`: `iex`, `& $var`, download, `reg import`, `Set-ItemProperty HKCU:`, tarefas agendadas, serviços, caminho absoluto fora de `$dir`/`$persist_dir`;
  - `Confirm`: o resto.

  O mesmo relatório alimenta o dry-run, a confirmação, o `show` e o oráculo.
- **Execução (`ScriptHost`):**
  - **Trait:** `run(&ScriptJob) -> ScriptOutcome{code, lines, effects}`.
  - **`runner_ps1` (puro), na ordem:**
    1. `$ErrorActionPreference='Stop'`;
    2. dot-source da lib fixada;
    3. variáveis como literais (`ps_single_quoted`);
    4. substituições **depois** do dot-source: `shim`, `Add-Path`, `Remove-Path`, `Set-EnvVar`, `env`, `ensure_in_psmodulepath` e `startmenu_shortcut` só gravam uma linha em `effects.jsonl`, e `abort` vira `throw`;
    5. o hook é lido de `hook.ps1` e nunca interpolado.
  - **Processo filho:** `SCOOP=<raiz>`, `SCOOP_GLOBAL` desativado, `SCOOP_CACHE`, `XDG_CONFIG_HOME` com um `config.json` mínimo.
  - **Depois de código 0:** o executor aplica `effects.jsonl` pela via da 2a (`EnvStore`, `shim.rs`, `Shortcuts`), com `Undo` e registro na state.db. Efeito ilegível ou fora do app ⇒ rollback.
  - **No Windows:** `workdir` em `<raiz>\tmp\script-…` com guard que apaga; recusa se o processo estiver elevado; carência de 2 s para processo neto; sem tempo-limite.
  - **Lib fixada:** o tarball do commit é baixado e **cada arquivo** é conferido contra uma tabela SHA-256 no código.
- **psmodule (Rust):**
  - junction `<raiz>\modules\<name>` → `current`;
  - `<raiz>\modules` entra no `PSModulePath` via `EnvStore`;
  - a entrada sai quando o último módulo é removido.
- **Confirmação:**
  - o cabeçalho diz "Script de terceiro (<hook>), roda com os seus privilégios e pode alterar qualquer arquivo seu";
  - mostra os flags e o script numerado;
  - na TUI, uma seção rolável (geometria em `regions`);
  - na CLI, `--allow-scripts` é obrigatório.

## (d) Sondas

| # | Sonda | Autorização |
|---|---|---|
| D1 | Fixar o commit da lib e listar os arquivos e as funções que gravam fora de `$scoopdir`. Descobrir onde `$bucketsdir` é definido | não (só leitura) |
| D2 | Carregar a lib numa raiz temporária. Medir que nada é gravado (árvore e `HKCU\Environment`), o custo de carga com `-Baseline` (pwsh 7 e 5.1) e a lista de funções | **sim** |
| D3 | Tabela variável × hook × comando e a ordem dos hooks em `scoop-update`/`scoop-uninstall` | não |
| D4 | innounp sobre o menor `innosetup:true` do Main: argumentos, código, codificação, `{app}`, `extract_dir` com e sem `{`, tempo | **sim** |
| D5 | `oracle_2c` sobre o `catalog.db`; léxico × AST do pwsh numa amostra fixa de 200 hooks | sim, para o AST |
| D6 | Tamanho e número de arquivos de `scripts/` nos tarballs; tempo a mais no `update` | não |
| D7 | `Get-ExecutionPolicy -List` e a mensagem de GPO | não |

## (e) Tarefas (a numerar depois da 2b)

| P | Tarefa | Modelo impl/rev | Testes e mutação principais |
|---|---|---|---|
| P1 | Oráculo `oracle_2c` + `classify()` | sonnet/sonnet | uma fixture por camada; chave só na variante `64bit`; precedência. Mutação: ignorar as variantes |
| P2 | Modelo tipado + fixtures `python`/`vscode`/innosetup/psmodule + mensagem de `:229` | sonnet/opus | variante sobrepõe raiz; `file` com `..`; psmodule sem `name`; regra dos auxiliares. Mutação: `variant.or(root)` invertido |
| P3 | `WriteManifest` e leitura na remoção | sonnet/sonnet | uninstall com o catálogo trocado usa o manifesto gravado |
| P4 | psmodule nativo | sonnet/opus | `PSModulePath` preservado; a entrada sai só no último módulo; `..` recusado |
| P5 | `src/proc/spawn_win.rs` + `ExtractInno` | opus/opus | argumentos exatos por forma de `extract_dir`; `.cmd` falso; código ≠ 0 ⇒ rollback |
| P6 | Auxiliares como dependências ocultas | sonnet/sonnet | remover auxiliar com dependente ⇒ recusa |
| P7 | `RunInstaller`/`RunUninstaller` sem `.ps1` | sonnet/opus | substitui só `$dir`/`$global`/`$version`; `keep`; código 3 ⇒ rollback |
| P8 | Guardar `scripts/` dos buckets | sonnet/opus | `scripts/../x` e symlink recusados sem abortar o resto; troca atômica |
| P9 | `script_scan` | opus/opus | uma fixture por flag e por veredito; comentário com `HKLM:` não conta; here-string. Mutação: ignorar here-strings |
| P10 | Lib do Scoop fixada | sonnet/opus | arquivo com hash errado ⇒ nada gravado; arquivo faltando ⇒ erro |
| P11 | Gerador do runner (puro) | opus/opus | aspas ASCII e tipográficas escapadas; hook nunca interpolado; substituições depois do dot-source |
| P12 | `ScriptHost` no executor + `effects.jsonl` | opus/opus | ordem dos hooks; `$dir` por hook; `AddPath` fora do app ⇒ rollback; injeção de falha em cada `k` |
| P13 | Remoção e upgrade com hooks e `$cmd` | sonnet/opus | `$cmd` = uninstall/update; ordem |
| P14 | `ScriptHost` real no Windows | opus/opus | `fake_pwsh.cmd` confere os argumentos e o ambiente; `workdir` some com código 1; ao vivo só com script sintético |
| P15 | Confirmação na CLI e na TUI | sonnet/opus | `--yes` sem `--allow-scripts` ⇒ erro e host sem chamadas; sem TTY ⇒ erro |
| P16 | Validação: oráculo antes e depois, bench, relatório, emendas; prova ao vivo só com autorização | opus/orquestrador | — |

Paralelismo:
1. {P1, P2};
2. {P4, P5, P8, P9, P10};
3. P3 e P7 em série com P4/P5 (todos tocam o `executor.rs`);
4. P11 → P12 → P13 → P14 → P15 → P16.

Entregar S-psm e S-inno primeiro.

## (f) Riscos

| Risco | Mitigação |
|---|---|
| Script destrutivo ou malicioso | confirmação obrigatória com o script e os flags; recusa de elevado, HKLM e `-Global`; hash dos downloads; aviso explícito |
| Análise estática passa falsa segurança | é portão e informação, nunca garantia; `ConfirmStrong` para construções dinâmicas; a UI nunca diz "seguro" |
| Manifesto usa função do Scoop que o prelúdio não tem (D1 = B) | o portão recusa com o nome da função; o oráculo mede (0 manifestos de `main`/`extras` na Task 163) |
| Script grava no `~/scoop` | `SCOOP` aponta para a raiz do wingetrs; o `~/scoop` é comparado antes e depois na prova ao vivo |
| `$dir` do staging gravado em arquivos | hooks depois do `Promote`; teste do `$dir` por hook |
| Rollback incompleto | D5 + aviso na confirmação |
| GPO bloqueia o pwsh | detecção, mensagem clara e a ponte como alternativa |
| Hash do innounp muda (a URL aponta para um branch) | vem pelo manifesto; hash diferente ⇒ `HashDivergente` |
| GPL-3.0 do innounp | não é empacotado; o usuário baixa pelo manifesto; nota no README |
| pwsh lento | medir na D2; um runner por fase, se valer a pena |
| Teste roda script real | regra no CLAUDE.md + regra em `check_rules.ps1` proibindo o `ScriptHost` real fora dos testes ignorados (provada com violação plantada) |
