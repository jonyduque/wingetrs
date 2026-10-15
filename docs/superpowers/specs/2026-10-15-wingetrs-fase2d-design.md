# wingetrs — Fase 2d: instaladores do winget (design)

Data: 2026-10-15
Status: **rascunho; as decisões D1–D8 são do usuário.** Rascunho feito por um agente opus, só leitura, a partir do winget-cli (`ManifestCommon.cpp`, `ShellExecuteInstallerHandler`, `MsiInstallFlow`, `MSIXInstallerHandler`), da sonda da Task 105 e das notas da 2d na spec da 2a.

## (a) Objetivo e escopo

Instalar, remover e atualizar pacotes do catálogo winget com o próprio wingetrs, e limpar registros ARP velhos ou órfãos. Motivação do usuário: aprendizado e velocidade.

**Entra:**
- instaladores silenciosos `msi`, `wix`, `burn`, `inno`, `nullsoft`, `msix`/`appx`, e `exe` com switch `Silent`/`SilentWithProgress`;
- UAC e códigos de saída (sucesso, esperados, reinício);
- SHA-256 do instalador sempre;
- desinstalação pelo ARP (QuietUninstallString, UninstallString, `msiexec /x`) e do MSIX;
- limpeza de "sobra provável" (lote G) com backup `.reg`;
- aviso de dados compartilhados (Kindle).

**Fica fora** (vai para a ponte do winget ou para depois):
- `zip` (com qualquer `NestedInstallerType`), `portable`, `font`, `pwa` e `msstore`;
- `Dependencies`;
- versão antiga (`--version`);
- reparo;
- MSIX em escopo de máquina;
- fontes que não sejam `winget`.

**Já existe:**
- `src/bridge/` (`winget upgrade`);
- `installed::arp::missing_uninstaller`;
- `RegistryScan::export` (`reg.exe`) e `delete_key`;
- `download.rs` com cache por sha256;
- `sources/winget/manifest.rs`, que hoje só lê `Architecture`, `InstallerType`, `Scope`, `InstallerUrl` e `InstallerSha256`;
- a cadeia de confiança: catálogo → versionData → manifesto → `InstallerSha256`.

**Defeito achado:** `download.rs:92` carrega o arquivo baixado inteiro na memória antes do hash. A P5 corrige isso com hash em streaming.

## (b) Decisões em aberto

| # | Pergunta | Opções | Recomendação |
|---|---|---|---|
| D1 | Arquitetura | **A** motor nativo: mais aprendizado, sem os 2–3,7 s de partida do winget, plano exato no dry-run; contras: muito código, cobertura inicial menor, risco real na máquina. **B** ponte `winget install/uninstall`: pouco código, cobertura total, risco baixo; contras: aprende pouco, é lento, não resolve órfãos. **C** híbrido: motor nativo para os tipos do escopo, a ponte para o que o motor recusa (com motivo) e reserva em Ajustes ("Motor do winget: wingetrs / winget"); desinstalação ARP e órfãos sempre nativos | **C**. As tarefas puras P1–P4 vêm primeiro e a ponte de install/uninstall (P13a) vem cedo |
| D2 | Escopo quando o manifesto tem `user` e `machine` | A `user` primeiro; B ordem do manifesto; C perguntar | **A** + `--scope machine` na CLI. "pede UAC" aparece na confirmação |
| D3 | Elevação | A `runas` por operação (N prompts num lote); B ajudante elevado próprio, um por lote; C exigir o wingetrs elevado | **A** para instaladores. **B** só para apagar chaves HKLM, com argumentos na linha de comando e revalidação de cada chave |
| D4 | `exe` sem silencioso, ou desinstalador de tipo desconhecido | A recusar; B rodar interativo com aviso; C heurística | Instalar: **B**. Desinstalar: heurística só com sinal forte (`WindowsInstaller=1` ⇒ msiexec; `*_is1` + `Inno Setup: App Path` ⇒ `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`; `BundleCachePath`/`BundleUpgradeCode` ⇒ `/quiet /norestart`); senão **B**. NSIS `/S` só se o manifesto correlacionado disser `nullsoft` |
| D5 | Backup `.reg` | A `reg.exe export`; B escritor próprio; C A agora, B depois | **A**, com `/reg:32` ou `/reg:64` |
| D6 | Kindle (instalação antiga junto com a nova) | A avisar e oferecer cópia das pastas conhecidas para `backups\<ts>\<app>\`; B só avisar; C "remover só o registro" | **A + C**. C só quando o registro não é o principal do grupo. Pastas conferidas só por existência: `%LOCALAPPDATA%\<Publisher>`, `%APPDATA%\<Publisher>`, `%LOCALAPPDATA%\<Nome>`, `%APPDATA%\<Nome>`, `Documents\<Nome>` |
| D7 | Instalador falso nos testes | A `.cmd` em tempdir; B `examples/fake_installer.rs`; C `[[bin]]` de teste | **A**. Hash calculado no teste e servido pelo wiremock; `Launcher` injetado; nunca `runas` em teste automático |
| D8 | Agreements / Dependencies | A mostrar Agreements, e "Confirmar" aceita (`--yes` exige `--accept-package-agreements`); B Dependencies ⇒ ponte com o motivo; C recusar os dois | **A + B** |

## (c) Design

```
src/sources/winget/manifest.rs   + campos do instalador com herança da raiz (P1)
src/install/winget/
  select.rs     escolha do instalador: arquitetura, escopo, locale, tipo, motivo da recusa (P2)
  switches.rs   switches padrão por tipo (porte de ManifestCommon.cpp) + linha de comando (P3)
  codes.rs      códigos ⇒ Outcome {Ok, OkReboot, Failed(categoria)} (P4)
  plan.rs       WingetPlan (Display = dry-run = confirmação)
  ops.rs        baixar → conferir → lançar → classificar → conferir ARP/MSIX (P8, P9, P11)
  launch_win.rs ShellExecuteExW + espera + código + is_elevated (P6)
  msix_win.rs   PackageManager Add/RemovePackageAsync (P7)
src/installed/uninstall.rs   comando de desinstalação a partir do ARP (P10)
src/installed/orphans.rs     export .reg ⇒ apagar (P12)
src/install/elevated_win.rs  ajudante `__elevated arp-delete` (P12)
```

**Manifesto (P1).**
- Campos novos:
  - tipo e plataforma: `InstallerType`, `NestedInstallerType`, `Scope`, `InstallerLocale`, `Platform`, `MinimumOSVersion`;
  - modos e switches: `InstallModes`, `InstallerSwitches{Silent, SilentWithProgress, Interactive, InstallLocation, Log, Upgrade, Custom}`;
  - códigos: `InstallerSuccessCodes`, `ExpectedReturnCodes`;
  - atualização: `UpgradeBehavior`, `RequireExplicitUpgrade`, `ElevationRequirement`;
  - identificação: `ProductCode`, `PackageFamilyName`, `AppsAndFeaturesEntries`;
  - `Dependencies`, `Agreements`.
- Cada campo do instalador cai para o da raiz quando falta. `InstallerSwitches` é mesclado chave por chave, e o do instalador vence.

**Escolha do instalador (P2).**
- Arquitetura nativa. Em x64, x86 é a reserva; em arm64, x64 e depois x86.
- Escopo conforme D2. Locale do sistema, depois qualquer um.
- Recusas com motivo:
  - tipo fora do escopo (`zip` incluído);
  - `Dependencies`;
  - `MinimumOSVersion` acima do sistema;
  - `exe` sem silencioso (D4).

**Linha de comando (P3).**

| Tipo | Silent | SilentWithProgress | Log | InstallLocation |
|---|---|---|---|---|
| msi, wix, burn | `/quiet /norestart` | `/passive /norestart` | `/log "<LOGPATH>"` | `TARGETDIR="<INSTALLPATH>"` |
| nullsoft | `/S` | `/S` | — | `/D=<INSTALLPATH>` |
| inno | `/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART` | `/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART` | `/LOG="<LOGPATH>"` | `/DIR="<INSTALLPATH>"` |
| exe | só o que o manifesto traz | | | |

- Ordem dos argumentos: modo, Log, Custom, Upgrade (só na atualização), InstallLocation (só com `--location`).
- `<LOGPATH>` = `%LOCALAPPDATA%\wingetrs\log\installers\<id>-<ver>-<ts>.log`.
- msi e wix rodam `msiexec.exe /i "<arquivo>" …`; a forma exata é conferida no `MsiInstallFlow` durante o porte.
- Args com NUL ou quebra de linha, ou com mais de 32 KiB, são recusados.

**Códigos (P4).**
- Sucesso: 0 e os `InstallerSuccessCodes`.
- `ExpectedReturnCodes` do manifesto vencem a tabela padrão do tipo:
  - MSI, cerca de 19 códigos: 1618 outra instalação, 1602 cancelado, 1625 política, 112 disco cheio, 3010 reiniciar para concluir, 1641 reinício iniciado, 1638 já instalado;
  - Inno: 2 e 5 cancelado, 8 reiniciar;
  - MSIX: cerca de 14 HRESULTs.
- 1223 no `runas` = "Permissão de administrador recusada."
- A comparação é sempre `as u32`.
- Desinstalação: só 0 é sucesso (mais 3010/1641 no msiexec). A prova é a chave ARP sumir.

**Download (P5).**
- O hash é calculado em streaming.
- O blob vai para `blobs\` e é ligado em `installers\<id>\<ver>\<nome>.<ext>`. A extensão vem do tipo.
- Antes de lançar: abrir com compartilhamento só de leitura, conferir o SHA de novo e manter o handle aberto até o fim.
- `InstallerUrl` precisa ser https; vale a emenda de redirecionamento de 2026-10-13.

**Lançamento e UAC (P6).**
- `ShellExecuteExW` com `SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC`, numa thread de trabalho STA que é sempre juntada.
- Nunca CreateProcess: um exe `requireAdministrator` daria 740.
- `SW_SHOW` em produção (o winget documenta que `SW_HIDE` trava instaladores). `SW_HIDE` só nos testes.
- `WaitForSingleObject` e depois `GetExitCodeProcess`. O código 1223 ⇒ `Cancelled`.
- `needs_runas` (regra pura):
  - `elevationRequired` ⇒ sim;
  - msi/wix/burn com escopo de máquina ou desconhecido ⇒ sim (hipótese; a S1 confirma);
  - `elevatesSelf` ⇒ não;
  - escopo `user` ⇒ não;
  - desinstalação de chave HKLM ⇒ só para msiexec;
  - desinstalação de chave HKCU ⇒ nunca.
- Só se eleva um arquivo com hash conferido ou um comando vindo de uma chave HKLM.
- Barra de status: "Aguardando o UAC; se não aparecer, clique no escudo piscando na barra de tarefas".
- Sem tempo-limite e sem cancelar depois de lançado.
- `is_elevated` usa `TokenElevation`.
- Features novas do `windows`: `Win32_System_Threading`, `Win32_Security`, `Win32_System_Console` (sem crate novo).

**MSIX (P7).**
- `AddPackageAsync(Uri, None, ForceTargetApplicationShutdown)`; a opção está em aberto.
- `RemovePackageAsync(fullName)`, com o full name obtido pelo PFN no `appx_win`.
- Só escopo de usuário.

**Conferência depois da instalação (P8).**
- Ler de novo o ARP e o MSIX, procurando por `AppsAndFeaturesEntries` (ProductCode/UpgradeCode), `ProductCode`, PFN e, por último, nome + publisher.
- Saiu 0 mas o app não apareceu ⇒ aviso, nunca um falso "Instalado" (Squirrel e afins).
- Tabela `winget_ops(id, version, installer_type, scope, sha256, arp_key, at)` no `state.db`, só como pista: o ARP continua sendo o registro de verdade.

**Atualização (P9).**
- `UpgradeBehavior`:
  - `install`: usa o switch `Upgrade`;
  - `uninstallPrevious`: P11 e depois P8;
  - `deny`: recusa.
- `RequireExplicitUpgrade` fica fora do "atualizar tudo". O pin é respeitado.
- `Target::Native`; a escolha entre o motor e a ponte vem do ajuste D1.

**Desinstalação (P10, P11).**
- Ordem de escolha:
  1. MSIX ⇒ P7;
  2. `WindowsInstaller=1` ⇒ `msiexec /x{KeyName} /quiet /norestart`;
  3. `QuietUninstallString`;
  4. heurística D4;
  5. `UninstallString` interativo com aviso.
- O parser de programa/argumentos é o mesmo de `missing_uninstaller`. Há um teste ao vivo de equivalência com `SHEvaluateSystemCommandTemplate`.
- Depois de rodar, espera a chave sumir: consulta a cada 500 ms, por até 120 s (o NSIS se copia para o `%TEMP%` e sai). O relógio é injetado nos testes.

**Órfãos (P12).**
- A confirmação:
  - mostra o caminho que falta;
  - avisa que o app pode ter mudado de lugar;
  - mostra o destino do `.reg`;
  - nunca vem pré-marcada.
- `--yes` sozinho não basta: precisa de `--orphans`.
- Execução:
  1. `reg.exe export … /y /reg:32|64` para `backups\<ts>\<local>-<sub>.reg`;
  2. confere que o arquivo existe, não está vazio e tem o cabeçalho e o nome da chave; senão, não apaga;
  3. HKCU: `delete_key` no próprio processo;
  4. HKLM: `wingetrs.exe __elevated arp-delete hklm32:<sub> …`.
- O ajudante:
  - aceita só a raiz fixa `SOFTWARE\[WOW6432Node\]Microsoft\Windows\CurrentVersion\Uninstall`;
  - recusa `\`, vazio ou mais de 255 caracteres;
  - reavalia `missing_uninstaller` em cada chave;
  - usa a view explícita;
  - escreve o resultado em `elevate\<nonce>.json`.

## Emenda de segurança proposta

Fora de `%LOCALAPPDATA%\wingetrs`, a 2d autoriza:
- (1) os efeitos do próprio instalador ou desinstalador, só depois de confirmação e só para um instalador com SHA-256 conferido ou um desinstalador lido do ARP;
- (2) implantar e remover MSIX pelo PackageManager;
- (3) apagar chaves `…\Uninstall\<sub>` (HKCU, HKLM 32/64) uma a uma, confirmadas, depois do export `.reg`.

Nenhum arquivo do winget é tocado (o `installed.db` continua só leitura).

Testes nunca rodam instalador real, nunca usam `runas` e nunca tocam o ARP real. Usam `.cmd` falso, `Launcher` falso, `FakeRegistry` e `WINGETRS_TEST_*`.

## (d) Sondas

- **S1 — UAC (opus; o usuário clica).** `ShellExecuteExW runas` de `cmd /c exit 7` a partir do Windows Terminal e do conhost, com `hwnd` nulo, `GetConsoleWindow()` e `GetForegroundWindow()`. Medir:
  - se o prompt aparece ou só pisca;
  - o código 7;
  - o 1223 ao recusar;
  - `is_elevated`;
  - um MSI de máquina pequeno com `/quiet` sem `runas`: falha? com qual código?
- **S2 — Cobertura do catálogo (sonnet; só manifestos).** Amostra fixa de 300 pacotes. Medir a distribuição de `InstallerType`, exe com ou sem Silent, `ElevationRequirement`, escopo, `Dependencies`, `Agreements`, `UpgradeBehavior` e os códigos. Resultado: a fração que o motor nativo cobre.
- **S3 — ARP desta máquina (sonnet; só leitura).** Por entrada: Quiet, `WindowsInstaller`, `_is1`, `BundleCachePath` e NSIS. Também:
  - hive e view das 3 sobras e do Kindle;
  - se o desinstalador do 2.8.3 existe;
  - `reg export /reg:32` de uma chave HKLM sem admin, para o scratchpad.
- **S4 — Validação ao vivo (no fim, com autorização).** Instalar e remover um pacote pequeno de cada tipo; um órfão HKCU e um HKLM com o `.reg` restaurado.

## (e) Tarefas (numeração a partir da Task 116)

| P | Tarefa | Modelo impl/rev | Testes e mutação principais |
|---|---|---|---|
| P1 | Manifesto completo | sonnet/sonnet | herança por campo; mescla de switches; mutação `i.x.or(root.x)` → `root.x.or(i.x)` |
| P2 | Escolha do instalador | sonnet/opus | x86 listado antes de x64; user depois de machine; fixture por recusa; mutação: tirar a reserva x86 |
| P3 | Switches e linha de comando | sonnet/sonnet | `insta` por tipo; Upgrade só na atualização; manifesto sobrepõe; mutação: tirar `/norestart` |
| P4 | Classificador de códigos | sonnet/sonnet | manifesto vence o padrão; 3010 ⇒ OkReboot; `as u32`; 1223; mutação: trocar a precedência |
| P5 | Download em streaming + materialização | opus/opus | 8 MiB por wiremock; último byte errado ⇒ nada em `installers\`; troca depois da conferência ⇒ recusa |
| P6 | Launcher, UAC, `is_elevated` | opus/opus | tabela-verdade de `needs_runas`; ao vivo `.cmd exit /b 7`; thread juntada no pânico; depende da S1 |
| P7 | MSIX instalar/remover | opus/opus | HRESULT ⇒ categoria; ao vivo só com autorização |
| P8 | Fluxo de instalação nativa | opus/opus | hash divergente ⇒ `Launcher` chamado 0 vezes; saída 0 sem ARP ⇒ aviso; dry-run = confirmação |
| P9 | Atualização nativa e escolha do motor | sonnet/opus | `uninstallPrevious` na ordem certa; `deny`; equivalência do instalador escolhido nos dois motores |
| P10 | Comando de desinstalação a partir do ARP | sonnet/opus | uma entrada por regra; `runas` nunca em HKCU; equivalência do parser ao vivo |
| P11 | Fluxo de desinstalação + Kindle | opus/opus | chave some ⇒ Removido; chave fica ⇒ aviso; não principal ⇒ pastas; relógio injetado |
| P12 | Órfãos + ajudante elevado | opus/opus | export vazio ⇒ não apaga; validação do ajudante; chave que deixou de ser sobra ⇒ não apaga; mutação: trocar a view 32/64 |
| P13 | CLI `install`/`uninstall`/`cleanup --orphans`; **P13a** ponte `winget install/uninstall` (pode vir antes) | sonnet/sonnet | parse e help |
| P14 | TUI: jobs, "aguardando UAC", "Remover registro órfão", ajuste do motor | sonnet/opus | snapshot da confirmação com "pede UAC"; mutação: órfão pré-marcado |
| P15 | Emendas: spec, CLAUDE.md, ARMADILHAS | orquestrador | — |
| P16 | Validação ao vivo (S4) | orquestrador + usuário | relatório |

**Ordem:**
1. S1–S3;
2. P1–P4 em paralelo (arquivos disjuntos);
3. P5, P6, P7;
4. P8, P10;
5. P9, P11, P12;
6. P13, P14;
7. P16.

## (f) Riscos

| Risco | Mitigação |
|---|---|
| Instalação real sem Sandbox (Windows Home) | Só `.cmd` e `Launcher` falsos nos testes; ao vivo só na S4, com pacotes escolhidos pelo usuário |
| Instalador ignora o silencioso ou abre janela | `SW_SHOW`, aviso, sem tempo-limite |
| UAC escondido na barra de tarefas | S1 + texto na barra de status |
| MSI de máquina com `/quiet` falha sem elevação | `runas` para msi/wix/burn de máquina (S1) |
| NSIS sai antes de terminar | a prova é a chave sumir |
| Apagar a chave de um app que só mudou de lugar (Steam) | nunca pré-marcado; caminho mostrado; `.reg` conferido; o ajudante reavalia |
| Kindle antigo apaga dados compartilhados | D6 |
| View 32/64 errada | local na identidade da linha; mutação de troca |
| Arquivo trocado entre a conferência e a execução elevada | handle só leitura + nova conferência do SHA |
| Instalador grande estoura a memória | P5 em streaming |
| 1618 / reinício pendente | categorias P4; a fila serializa |
| Instalador que lança um filho e sai | conferência no ARP com aviso |
| Os dois motores divergem | equivalência (P9) e ajuste de reserva |
| Sem Mark-of-the-Web (`IAttachmentExecute` não portado) | o Defender continua escaneando; fica registrado como não portado |
