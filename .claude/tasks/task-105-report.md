# Task 105 — Sonda da ponte de atualização (winget e scoop)

Status: DONE_WITH_CONCERNS

Commits:
- um único commit `chore(tasks): upgrade bridge probe report for task 105` contendo este arquivo; o SHA não pode constar no próprio arquivo — confira com `git log --format='%h %s' dc58ed5..HEAD` (o SHA vai na mensagem ao orquestrador).

Gate:
```
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 4s
[OK]   test (801 passed, 20 ignored) 45s
[OK]   gate verde
```
(Sem mudança de código; o gate foi rodado só para registrar a base verde.)

Mutation proofs: no mutation proof — tarefa de sonda, só leitura; os dois "Checks beyond the steps" (`git status`, `leftovers.ps1`) são conferências, não regras de código. Saídas coladas no fim.

Decisions:
- Medi o spawn com um crate de sondagem em Rust no scratchpad (sem dependências, fora do repo) para ver o que o `std::process::Command` da Task 106 vai enxergar; Python serviu só para contar bytes de controle.
- Não rodei `winget error`, `scoop status` (faz `git fetch`, ver 2.3) nem `scoop update` algum; os códigos do scoop vêm da leitura do código, com arquivo e linha citados.
- Fontes do winget: tag `v1.29.380` (a versão instalada) para comportamento, `master` só para a tabela `returnCodes.md` (idêntica nos dois commits conferidos).
- Recomendo chamar o `scoop.ps1` direto por `pwsh -Command` com UTF-8 forçado em vez de `scoop.cmd` (motivo em 2.4/4.1).

Concerns:
- **`scoop update <app>` sai com 0 em quase toda falha "branda"**: app inexistente, já atualizado, segurado (hold), processo em execução — todos `exit 0` (ver 2.2). A Task 106 não pode confiar no código de saída; tem de reler a versão instalada depois.
- **`scoop update <app>` faz `git pull` de Scoop e de todos os buckets se `LAST_UPDATE` tem ≥ 3 h**, sem flag para desligar (2.3). O tempo de um update de scoop varia por isso; não medido.
- Códigos "nenhuma atualização aplicável", "fixado" e "instalador falhou" **não medidos** ao vivo (exigiriam `winget upgrade --id <pacote instalado>`, proibido); vêm da documentação e do código-fonte da tag.
- Progresso real de download/instalação do winget redirecionado: **não medido** ao vivo (exigiria atualizar algo); o código da tag `v1.29.380` diz que não há barra nem spinner quando a saída é redirecionada (1.2).
- Matar o `winget.exe` (timeout/cancelar) não mata o instalador lançado por `ShellExecuteEx` (3). Não medido.

---

## 1. winget

### 1.1 Versão e flags que tiram a interatividade

```
$ winget --version
v1.29.380
```

Trecho de `winget upgrade --help` (pt-BR, colado da saída real):
```
  -e,--exact                           Localizar pacote usando uma correspondência exata
  -h,--silent                          Solicitar uma instalação silenciosa
  --accept-package-agreements          Aceitar todos os contratos de licença para pacotes
  --accept-source-agreements           Aceitar todos os acordos de origem durante as operações de origem
  -u,--unknown,--include-unknown       Atualizar pacotes mesmo que a versão atual não seja determinada
  --pinned,--include-pinned            Atualizar pacotes mesmo que eles tenham um marcador sem bloqueio
  --disable-interactivity              Desabilitar os prompts interativos
```

Documentação oficial (https://learn.microsoft.com/en-us/windows/package-manager/winget/upgrade, `ms.date` 2026-07-19, `updated_at` 2026-07-21):
```
--silent => Runs the installer in silent mode. This suppresses all UI. The default experience shows installer progress.
--disable-interactivity => Disable interactive prompts.
--accept-package-agreements => Used to accept the license agreement, and avoid the prompt.
--accept-source-agreements => Used to accept the source license agreement, and avoid the prompt.
```

O que cada uma faz, conferido no código da tag `v1.29.380` (`src/AppInstallerCLICore/Workflows/PromptFlow.cpp`):
- `--disable-interactivity`: `IsInteractivityAllowed` (l. 16–42) devolve `false` se a flag estiver na linha de comando (l. 29). Sem interatividade, o prompt "o instalador vai fechar o terminal" é pulado e a instalação **segue** (l. 358–362); os prompts de contrato **não** são aceitos sozinhos: terminam com `APPINSTALLER_CLI_ERROR_SOURCE_AGREEMENTS_NOT_ACCEPTED` (l. 420, 0x8A150046) ou `..._PACKAGE_AGREEMENTS_NOT_ACCEPTED` (l. 219, 0x8A150041). Por isso as duas `--accept-*` são obrigatórias.
- `--silent`: passa ao instalador os switches silenciosos do manifesto. O instalador é lançado com `SW_SHOW` mesmo assim (`ShellExecuteInstallerHandler.cpp` l. 299–306: "Setting to SW_HIDE will hide installer UI and installation will never complete"), então um instalador sem modo silencioso ainda mostra janela.
- Com stdin nulo o winget também não lê prompts: `Reporter::InputStreamIsInteractive` só é verdadeiro se stdin for `FILE_TYPE_CHAR` (`ExecutionReporter.cpp` l. 173–176).

Flags combinadas aceitas juntas (pacote inexistente, nada instalado):
```
$ winget upgrade --id Nao.Existe --exact --silent --disable-interactivity --accept-package-agreements --accept-source-agreements
exit=2316632084 hex=0x8A150014 signed=-1978335212 ms=2311
stdout: len=83 esc=0 crlf=1 lone_cr=0 bom=False
b'Nenhum pacote instalado foi encontrado que corresponda aos crit\xc3\xa9rios de entrada.\r\n'
stderr: len=0 esc=0 crlf=0 lone_cr=0 bom=False
```

### 1.2 Saída redirecionada: codificação, controle, progresso

`cmd /c "winget upgrade > up1.txt 2> up1.err"`:
```
$ file up1.txt
up1.txt: Unicode text, UTF-8 text, with CRLF line terminators
len 2164 bom False esc 0 crlf 24 lone_cr 0 bs 0
(up1.err, up2.err, up3.err: 0 bytes; up1/up2/up3.txt idênticos — "identical")
```

`Format-Hex` das primeiras linhas:
```
0000000000000000 4E 6F 6D 65 20 20 20 20 20 20 20 20 20 20 20 20 Nome
0000000000000010 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20
0000000000000020 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20
0000000000000030 20 49 44 20 20 20 20 20 20 20 20 20 20 20 20 20  ID
0000000000000040 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20
0000000000000050 20 20 20 20 20 20 56 65 72 73 C3 A3 6F 20 20 20       Versǜo
0000000000000060 44 69 73 70 6F 6E C3 AD 76 65 6C 0D 0A 2D 2D 2D Dispon��vel??---
0000000000000070 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D 2D ----------------
```

Trecho da listagem com bytes de controle visíveis (`cat -A`; `^M$` = CRLF, `M-CM-#` = UTF-8 de `ã`):
```
Nome                                             ID                                   VersM-CM-#o   DisponM-CM--vel^M$
---------------------------------------------------------------------------------------------------------^M$
Anki                                             Anki.Anki                            26.5.0   26.09.3^M$
Git                                              Git.Git                              2.55.0.3 2.55.0.5^M$
WinScript                                        flick9000.WinScript                  2.26.0   2.27.1^M$
16 atualizaM-CM-'M-CM-5es disponM-CM--veis.^M$
^M$
Os pacotes a seguir tM-CM-*m uma atualizaM-CM-'M-CM-#o disponM-CM--vel, mas exigem uma segmentaM-CM-'M-CM-#o explM-CM--cita para atualizaM-CM-'M-CM-#o:^M$
Android Studio Google.AndroidStudio 2026.2   2026.2.1.8^M$
2 pacotes tM-CM-*m nM-CM-:meros de versM-CM-#o que nM-CM-#o podem ser determinados. Use --include-unknown para ver todos os resultados.^M$
```
(14 linhas na primeira tabela + 2 na de "segmentação explícita" = as "16 atualizações".)

Mesma coisa por pipe, sem console (`CREATE_NO_WINDOW`, stdin nulo), 3 vezes:
```
exit=0 hex=0x00000000 signed=0 ms=3340
stdout: len=2164 esc=0 crlf=24 lone_cr=0 bom=False
exit=0 hex=0x00000000 signed=0 ms=3452
stdout: len=2164 esc=0 crlf=24 lone_cr=0 bom=False
exit=0 hex=0x00000000 signed=0 ms=3335
stdout: len=2164 esc=0 crlf=24 lone_cr=0 bom=False
```

Conclusão medida: **UTF-8 sem BOM, CRLF, zero ESC, zero `\r` solto, nada em stderr** (mensagens de erro também saem em stdout, ver 1.1). O texto é localizado (pt-BR) — não dá para casar por frase.

Progresso durante download/instalação: **não medido ao vivo**. Código da tag `v1.29.380`, `src/AppInstallerCLICore/ExecutionReporter.cpp` l. 60–62:
```
        // When output is redirected to a file or pipe, suppress all progress output
        // so it does not appear in the redirected stream.
        if (GetConsoleWidth().has_value())
```
`GetConsoleWidth` (`ChannelStreams.cpp` l. 20–32) só tem valor se `GetConsoleScreenBufferInfo(STD_OUTPUT_HANDLE)` funciona — com stdout em pipe, não há spinner nem barra. Sobram as linhas de etapa (encontrado / baixando / hash verificado / iniciando instalação / sucesso ou erro), uma por linha.

### 1.3 Códigos de saída

Fonte: `doc/windows/package-manager/winget/returnCodes.md` do winget-cli ("generated by running: winget error --output <file>", `ms.date: 05/13/2026`). Baixado em 2026-10-14 de `https://raw.githubusercontent.com/microsoft/winget-cli/master/doc/windows/package-manager/winget/returnCodes.md` (master = `3d9083c062c1e7a7086e05becb414d4e005e4ab6`, 2026-10-08; último commit no arquivo `df1118d8eb`, 2026-10-01) e no commit de `SOURCE_COMMIT.txt` (`39739564a4`): `cmp` ⇒ `same`.

O código de saída do processo é o HRESULT; `ExitStatus::code()` do Rust devolve o `i32` com sinal (medido: `code=Some(-1978335212)`).

| Situação | Hex | `i32` | Símbolo | Conferido |
|---|---|---|---|---|
| Sucesso | 0x00000000 | 0 | — | medido (listagem, `pin list`) |
| Sucesso, reinício pendente | 0x00000000 | 0 | `INSTALL_REBOOT_REQUIRED_TO_FINISH` vira `S_OK` + aviso (`InstallFlow.cpp` tag l. 511–516) | código |
| Nenhuma atualização aplicável | 0x8A15002B | -1978335189 | `UPDATE_NOT_APPLICABLE` (`UpdateFlow.cpp` tag l. 67, 194, 208) | não medido |
| Versão desconhecida, sem `--include-unknown` | 0x8A15002B | -1978335189 | idem, l. 59–67 | não medido |
| Pacote não encontrado | 0x8A150014 | -1978335212 | `NO_APPLICATIONS_FOUND` | **medido** |
| Vários pacotes casam | 0x8A150016 | -1978335210 | `MULTIPLE_APPLICATIONS_FOUND` | não medido |
| Pacote fixado (pin) | 0x8A150068 | -1978335128 | `PACKAGE_IS_PINNED` (`UpdateFlow.cpp` tag l. 167) | não medido (`winget pin list`: "Não há martcadores configurados.") |
| Tecnologia de instalação diferente | 0x8A15008E | -1978335090 | `UPDATE_INSTALL_TECHNOLOGY_MISMATCH` | não medido |
| Instalador (exe) falhou | 0x8A150006 | -1978335226 | `SHELLEXEC_INSTALL_FAILED` (`InstallFlow.cpp` tag l. 177) | não medido |
| Instalador MSI falhou | 0x8A150049 | -1978335159 | `MSI_INSTALL_FAILED` | não medido |
| Portátil falhou | 0x8A150052 | -1978335150 | `PORTABLE_INSTALL_FAILED` | não medido |
| Reinício necessário para instalar | 0x8A15010A | -1978334966 | `INSTALL_REBOOT_REQUIRED_FOR_INSTALL` | não medido |
| Reinício iniciado | 0x8A15010B | -1978334965 | `INSTALL_REBOOT_INITIATED` | não medido |
| App em execução | 0x8A150101 | -1978334975 | `INSTALL_PACKAGE_IN_USE` | não medido |
| Outra instalação em andamento | 0x8A150102 | -1978334974 | `INSTALL_INSTALL_IN_PROGRESS` | não medido |
| Usuário cancelou o instalador | 0x8A15010C | -1978334964 | `INSTALL_CANCELLED_BY_USER` | não medido |
| Download falhou | 0x8A150008 | -1978335224 | `DOWNLOAD_FAILED` | não medido |
| Hash do instalador não confere | 0x8A150011 | -1978335215 | `INSTALLER_HASH_MISMATCH` | não medido |
| Contrato de pacote / fonte não aceito | 0x8A150041 / 0x8A150046 | -1978335167 / -1978335162 | `PACKAGE_/SOURCE_AGREEMENTS_NOT_ACCEPTED` | não medido |
| Instalação abandonada (cancelada) | 0x80004004 | -2147467260 | `E_ABORT` (`ShellExecuteInstallerHandler.cpp` tag l. 310–320) | não medido |
| Argumento inválido | 0x8A150002 | -1978335230 | `INVALID_CL_ARGUMENTS` | **medido** |

Medições coladas:
```
$ winget upgrade --id Nao.Existe --exact          (sem console, stdin nulo)
exit=2316632084 hex=0x8A150014 signed=-1978335212 ms=2189
$ winget upgrade --id Nao.Existe --exact --disable-interactivity
exit=2316632084 hex=0x8A150014 signed=-1978335212 ms=2169
$ winget upgrade --id Nao.Existe --exact --flag-inexistente
exit=2316632066 hex=0x8A150002 signed=-1978335230 ms=369
stdout: len=4757 ... "O nome do argumento n\xc3\xa3o foi reconhecido para o comando atual: '--flag-inexistente'" ...
$ winget pin list
exit=0 hex=0x00000000 signed=0 ms=2021
b'N\xc3\xa3o h\xc3\xa1 martcadores configurados.\r\n'
```
Rust (`Command::new("winget")`, `CREATE_NO_WINDOW`, stdin nulo, stdout/stderr em pipe):
```
winget ["--version"]: code=Some(0) ms=317 stdout_len=11 first="v1.29.380\r\n"
winget ["upgrade", "--id", "Nao.Existe", "--exact", "--disable-interactivity"]: code=Some(-1978335212) ms=2024 stdout_len=83 first="Nenhum pacote instalado foi encontrado que corresponda aos critérios de entrada.\r\n"
winget-nao-existe ["--version"]: spawn error kind=NotFound raw=None msg=program not found
```
O `Command` do Rust acha o alias de execução de app (`WindowsApps\winget.exe`) pelo PATH.

### 1.4 Tempo de `winget upgrade` (listagem)

`cmd /c` com redirecionamento para arquivo:
```
run 1 exit=0 ms=3954
run 2 exit=0 ms=3720
run 3 exit=0 ms=3684
```
**Mediana 3720 ms.** Por pipe sem console (Python `subprocess`): 3340 / 3452 / 3335 ⇒ **mediana 3340 ms**. Sem baseline de processo (não é número decisivo; só dimensiona timeout/feedback). Uma chamada que falha cedo (pacote inexistente) custa ~2,0–2,3 s; `--version` 317 ms.

## 2. scoop

### 2.1 Como os shims chamam o `scoop.ps1`

`~/scoop/shims/scoop.cmd`:
```
@rem C:\Users\jonyd\scoop\apps\scoop\current\bin\scoop.ps1
@echo off
where /q pwsh.exe
if %errorlevel% equ 0 (
    pwsh -noprofile -ex unrestricted -file "C:\Users\jonyd\scoop\apps\scoop\current\bin\scoop.ps1"  %*
) else (
    powershell -noprofile -ex unrestricted -file "C:\Users\jonyd\scoop\apps\scoop\current\bin\scoop.ps1"  %*
)
```
`~/scoop/shims/scoop` (sh) faz o mesmo com `pwsh.exe`/`powershell.exe`; `scoop.ps1` (shim) faz `& $path @args; exit $LASTEXITCODE`. Não existe `scoop.exe`.

Rust:
```
scoop ["--version"]: spawn error kind=NotFound raw=None msg=program not found
scoop.cmd ["--version"]: code=Some(0) ms=2532 stdout_len=486 first="Current Scoop version:\r\n..."
scoop.cmd ["help", "a&b"]: code=Some(0) ms=1679 stdout_len=41 first="ERROR scoop help: no such command 'a&b'\r\n"
```
`Command::new("scoop")` não acha nada (o Rust só completa `.exe`); `scoop.cmd` funciona e o escape de `.cmd` do std protegeu o `&`. Direto no script:
```
$ pwsh -NoProfile -ExecutionPolicy Bypass -File <root>\apps\scoop\current\bin\scoop.ps1 --version
exit=0 hex=0x00000000 signed=0 ms=2280
$ pwsh ... -File <scoop.ps1> naoexiste
exit=1 ... b"WARN  scoop: 'naoexiste' isn't a scoop command. See 'scoop help'.\r\n"
$ pwsh ... -File <scoop.ps1> help update     (3x)
ms=1521 / 1398 / 1382
$ pwsh -NoProfile -Command "exit 0"          (3x, piso do pwsh)
ms=779 / 702 / 688
```
Raiz: `lib/core.ps1` l. 1375 resolve `$scoopdir` por `$env:SCOOP`, `ROOT_PATH` da config, caminho do script, `~\scoop`; o wingetrs já tem `installed::scoop::scoop_root()` (`src/installed/scoop.rs:45`). `$env:SCOOP` não está definido nesta máquina.

### 2.2 Códigos de saída de `scoop update <app>` (lidos, não executados)

Arquivo: `~/scoop/apps/scoop/current/libexec/scoop-update.ps1` (Scoop v0.6.0, 2026-09-30). `abort` = `Write-Host $msg -f red; exit $exit_code` com padrão 1 (`lib/core.ps1` l. 311); `error`/`warn`/`info` só escrevem (l. 312–314).

| Caso | Caminho no código | Saída |
|---|---|---|
| Sucesso | `update` (l. 261–395) volta normalmente; script termina em `exit 0` (l. 483) | **0** |
| Já atualizado | l. 446 falso ⇒ l. 457 `Write-Host "$app`: $ver (latest version)"`; também l. 292–297 `warn "The latest version of '$app' ($version) is already installed."` | **0** |
| App inexistente | `Confirm-InstallationStatus` (`lib/core.ps1` l. 1167–1199) só faz `error "'$App' isn't installed."` e não o inclui; l. 437 `if ($apps)` falso ⇒ l. 483 | **0** |
| App segurado (hold) | l. 452 `warn "'$app' is held to version ..."` | **0** |
| Processo do app rodando | l. 307–310 `Write-Host 'Running process detected, skip updating.'; return` (`test_running_process`, `lib/install.ps1` l. 533–546) | **0** |
| Sem manifesto | l. 298–301 `error "No manifest available for '$app'."; return` | **0** |
| Hash não confere | l. 330–339 `abort` | **1** |
| Falha de download/instalação | `install_app`/`Invoke-CachedDownload` lançam ou chamam `abort` | **1** (exceção não tratada no `pwsh -File` também dá 1; não medido) |
| Flag inválida | l. 33 `error ...; exit 1` | **1** |
| `--global` sem admin | l. 411–412 `exit 1` | **1** |
| Falha do `git pull` do Scoop | l. 147–148 `abort 'Update failed.'` | **1** |

### 2.3 `git pull` antes do update

Sim, condicional. `scoop-update.ps1` l. 416–425:
```
    $updateScoop = $null -ne ($apps | Where-Object { $_ -eq 'scoop' }) -or (is_scoop_outdated)
    ...
    if ($updateScoop) {
        Sync-Scoop -Log:$show_update_log
        Sync-Bucket -Log:$show_update_log
        set_config LAST_UPDATE ([System.DateTime]::Now.ToString('o')) | Out-Null
```
`is_scoop_outdated` (`lib/core.ps1` l. 1251–1261) é verdadeiro quando `LAST_UPDATE` tem ≥ 3 h. `Sync-Scoop` faz `git pull --tags --force` (l. 139); `Sync-Bucket` faz `git pull -q` em cada bucket (l. 198/229, até 5 em paralelo no pwsh 7). **Não há flag para desligar** (getopt l. 32: só `gfiksqa`). `HOLD_UPDATE_UNTIL` (`Test-ScoopCoreOnHold`, l. 70–72) só pula o `Sync-Scoop`; os buckets sincronizam do mesmo jeito. Evitar exigiria escrever `LAST_UPDATE` na config do scoop — proibido.

`scoop status` **não foi rodado**: `libexec/scoop-status.ps1` l. 25 faz `git fetch -q origin` (salvo `-l/--local`, l. 16).

### 2.4 Codificação e progresso da saída redirecionada

- Progresso de download: `lib/download.ps1` l. 73 — `$progress = [console]::isoutputredirected -eq $false -and ...`: redirecionado ⇒ sem barra. Com aria2 ligado, as linhas do aria2 saem com `` `r `` (l. 451); aqui `aria2-enabled: False` na config.
- Cores: `Write-Host -ForegroundColor` redirecionado não emite ANSI (medido: `esc=0`; `$PSStyle.OutputRendering = Host`).
- Fim de linha **misto**: `scoop --version` tem `\r\n` e `\n` (o git escreve LF), `help update` tem um `\r` solto:
```
b"...b6c629c6ab xata: Update to version 1.17.1\n\r\n'extras' bucket:\r\n..."
help update: len=673 esc=0 crlf=13 lone_cr=1
```
- Codificação: o pwsh redirecionado usa a página OEM, **não UTF-8**:
```
$ pwsh -NoProfile -File enc.ps1        (Write-Host "acao: ação")
b'acao: a\x87\xc6o\r\nenc=ibm850 redirected=True render=Host\r\n'
$ chcp.com
Página de código ativa: 850
```
  Forçando no `-Command`:
```
$ pwsh -NoProfile -ExecutionPolicy Bypass -Command "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); & 'enc.ps1'; & '<scoop.ps1>' naoexiste; exit $LASTEXITCODE"
exit=1 hex=0x00000001 signed=1 ms=1495
b"acao: a\xc3\xa7\xc3\xa3o\r\nenc=utf-8 redirected=True render=Host\r\nWARN  scoop: 'naoexiste' isn't a scoop command. See 'scoop help'.\r\n"
```
  O código de saída passa pelo `exit $LASTEXITCODE` (1 ⇒ 1). O caminho com aria2 troca para UTF-8 no meio (`download.ps1` l. 430); forçar UTF-8 desde o início deixa o fluxo todo coerente.

## 3. Elevação (UAC)

Código da tag `v1.29.380`, `ShellExecuteInstallerHandler.cpp`:
- l. 289–297: se o manifesto diz `ElevationRequired`/`ElevatesSelf` e o winget não está elevado, ele só **avisa** (`InstallerElevationExpected`).
- l. 302–306 e 31–36: para `ElevationRequired`, chama `ShellExecuteExW` com `lpVerb = "runas"` e `SW_SHOW`; para `ElevatesSelf`, o próprio instalador pede.
- l. 38: `THROW_LAST_ERROR_IF(!ShellExecuteExW(...))` — se o usuário recusar o `runas`, a falha (`ERROR_CANCELLED`, 1223) vira o HRESULT de saída `0x800704C7` (dedução do código; **não medido**). Se o instalador se eleva sozinho e o usuário recusa, o código dele cai em `SHELLEXEC_INSTALL_FAILED` (0x8A150006) ou num `ExpectedReturnCode` do manifesto.
- l. 43–65: o winget espera o instalador em laço de 250 ms e pega o código de saída dele.

O prompt do UAC é desenhado pelo `consent.exe` (serviço AppInfo) na área de trabalho segura e não depende do console do processo pai, então funciona com `CREATE_NO_WINDOW` e saída em pipe. Comportamento documentado do Windows (não medido aqui): quando quem pede a elevação não está em primeiro plano, o prompt pode aparecer como um botão piscando na barra de tarefas em vez de tomar a tela.

**O wingetrs não precisa de mais nada além de esperar o processo**: nada de `runas` próprio, nada de manifesto elevado. Precisa só (a) avisar o usuário que pode aparecer um pedido do UAC, possivelmente piscando na barra de tarefas; (b) não ter tempo-limite curto, porque o processo fica parado enquanto o UAC espera; (c) tratar `0x800704C7` como "elevação recusada".

## 4. Recomendação para a Task 106

### 4.1 Linha de comando exata

- **winget** (ARP/MSIX com correspondência no winget):
  ```
  winget upgrade --id <id> --exact --silent --disable-interactivity --accept-package-agreements --accept-source-agreements
  ```
  Opcional: `--source winget` quando o id veio do catálogo `winget`, para não casar com `msstore`. Não adicionar `--include-unknown` (de propósito; se a versão for desconhecida, sai 0x8A15002B e a mensagem explica). `Command::new("winget")` com `stdin(Stdio::null())`, stdout/stderr em pipe, `creation_flags(CREATE_NO_WINDOW)` (medido: funciona e acha o alias). Stdin nulo é obrigatório: um stdin herdado do console competiria com o crossterm pela entrada da TUI.
- **scoop** (app instalado pelo Scoop):
  ```
  pwsh -NoProfile -ExecutionPolicy Bypass -Command "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); & '<root>\apps\scoop\current\bin\scoop.ps1' update <app>; exit $LASTEXITCODE"
  ```
  `pwsh.exe` se estiver no PATH, senão `powershell.exe` (igual ao `scoop.cmd`). `<root>` = `installed::scoop::scoop_root()`; `'` no caminho dobrado (`''`). `<app>` precisa estar validado (`^[A-Za-z0-9][A-Za-z0-9._+-]*$`, e passado por `safepath` se virar caminho) porque entra no texto do `-Command`. Alternativa mais simples: `Command::new("scoop.cmd").args(["update", app])` (medido: funciona, o std escapa `&`), mas aí a saída vem em OEM/850 e passa por `cmd` + `where`.

### 4.2 Como detectar se a ferramenta existe

- winget: spawn de `winget --version` (317 ms medido); `ErrorKind::NotFound` ⇒ "winget não encontrado". Fazer uma vez por sessão e guardar o resultado; ou não detectar antes e tratar o `NotFound` do próprio upgrade.
- scoop: `scoop_root()` + `<root>\apps\scoop\current\bin\scoop.ps1` existir (só `metadata`, sem processo); `pwsh.exe`/`powershell.exe` pelo spawn.

### 4.3 Como mostrar o progresso

Nenhuma das duas ferramentas dá percentual com a saída redirecionada (winget: `ExecutionReporter.cpp` tag l. 60–62; scoop: `download.ps1` l. 73). Ler stdout linha a linha numa thread juntada no fim (regras de thread do `CLAUDE.md`), separar por `\n`, tirar `\r` do fim **e** ficar com o trecho depois do último `\r` (o scoop com aria2 e o `help` têm `\r` solto), descartar linhas vazias, decodificar UTF-8 com `from_utf8_lossy` e mandar a última linha útil como mensagem de progresso do job (indeterminado: spinner + texto). Guardar as últimas ~20 linhas para mostrar no erro. Não casar texto do winget (localizado); no scoop, casar só o que a tabela abaixo pede.

### 4.4 Código de saída ⇒ mensagem em português

winget (`ExitStatus::code()` como `i32`; comparar `as u32`):

| Código | Mensagem |
|---|---|
| 0 | "Atualizado." (se a saída tiver aviso de reinício, acrescentar "Reinicie o computador para concluir.") |
| 0x8A15002B | "Nenhuma atualização aplicável para este pacote." |
| 0x8A150014 | "O winget não encontrou este pacote instalado." |
| 0x8A150016 | "Mais de um pacote corresponde a este id." |
| 0x8A150068 | "Pacote fixado no winget (pin); remova o pin para atualizar." |
| 0x8A15008E | "A nova versão usa outro tipo de instalador; atualize pelo winget manualmente." |
| 0x8A150006 / 0x8A150049 / 0x8A150052 | "O instalador falhou." + últimas linhas |
| 0x8A150101 / 0x8A150103 | "O aplicativo está em uso; feche-o e tente de novo." |
| 0x8A150102 | "Outra instalação está em andamento; tente mais tarde." |
| 0x8A150109 | "Atualizado; reinicie o computador para concluir." |
| 0x8A15010A / 0x8A15010B | "É preciso reiniciar o computador." |
| 0x8A15010C / 0x80004004 | "Atualização cancelada." |
| 0x800704C7 | "Permissão de administrador recusada." |
| 0x8A150008 | "Falha no download do instalador." |
| 0x8A150011 | "O hash do instalador não confere com o manifesto." |
| 0x8A150041 / 0x8A150046 | "Contrato não aceito." |
| outro | "O winget terminou com erro 0x{:08X}." + últimas linhas |

scoop: código ≠ 0 ⇒ "O scoop falhou (código N)." + últimas linhas. Código 0 **não basta**: comparar a versão instalada antes e depois (`<root>\apps\<app>\current\manifest.json` → `version`, já lido pelo `installed::scoop`). Mesma versão ⇒ olhar as linhas: `isn't installed` ⇒ "O scoop não encontrou este app."; `(latest version)` ou `is already installed` ⇒ "Já está na versão mais recente."; `is held to version` ⇒ "App segurado no scoop (hold)."; `Running process detected` ⇒ "O app está aberto; feche-o e tente de novo."; senão ⇒ "O scoop não atualizou o app." + últimas linhas. (As frases do scoop são em inglês e fixas no código citado em 2.2.)

### 4.5 Tempo-limite

Sem tempo-limite curto: o UAC e instaladores com janela deixam o processo parado esperando o usuário. Sugestão: nenhum kill automático; depois de 10 min sem linha nova, mostrar "ainda aguardando o instalador (pode haver uma janela ou pedido do UAC aberto)". O cancelamento pelo usuário mata só o `winget.exe`/`pwsh.exe`; o instalador lançado por `ShellExecuteEx` continua (não medido) — avisar isso na mensagem de cancelamento ou não oferecer cancelar depois de começar. Referências medidas: listagem 3,3–3,7 s, falha cedo ~2,1 s, piso do pwsh ~0,7 s, scoop `help` ~1,4 s; o `git pull` de buckets do scoop não foi medido.

### 4.6 O que testar com executável falso

Um binário de teste (ex.: `examples/` ou `tests/bin`) apontado por variável de ambiente de teste no lugar de `winget`/`pwsh`, que:
1. sai com cada código da tabela 4.4 (inclui negativos como `0x8A150014` ⇒ `-1978335212`) e confere a mensagem;
2. escreve UTF-8 com CRLF, LF solto, `\r` solto no meio da linha, linhas vazias e uma sequência não-UTF-8 — confere as linhas entregues ao job e que nada quebra;
3. não existe ⇒ `NotFound` vira "winget não encontrado"/"scoop não encontrado";
4. demora / não fecha stdout até sair (a thread leitora é juntada e o job termina);
5. escreve muito (≥ 1 MiB) sem `\n` — o leitor não cresce sem limite;
6. confere os argumentos recebidos (`--id <id> --exact --silent --disable-interactivity --accept-package-agreements --accept-source-agreements`; no scoop, `update <app>` e o `OutputEncoding` UTF-8) e que stdin chega fechado (lê stdin e recebe EOF imediato);
7. scoop: exit 0 com a versão do `manifest.json` igual antes/depois ⇒ não reportar "Atualizado"; versão nova ⇒ "Atualizado".
8. id/app com caracteres fora do padrão (`a&b`, `x'y`, `..\x`) é recusado antes do spawn.

---

## Checks beyond the steps

`leftovers.ps1` (antes do relatório):
```
[OK] nenhum processo de agente sobrando (> 2 min)
```

`git status --short` antes do commit (o `task-105-base.txt` é do orquestrador, já estava lá no início):
```
?? .claude/tasks/task-105-base.txt
?? .claude/tasks/task-105-report.md
```

`leftovers.ps1` antes do commit:
```
[OK] nenhum processo de agente sobrando (> 2 min)
```

Arquivos temporários (crate de sondagem, saídas, fontes baixadas) ficaram só no scratchpad da sessão, fora do repositório.
