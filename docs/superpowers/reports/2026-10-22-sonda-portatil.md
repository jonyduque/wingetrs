# Sonda do modo portátil (Task 229) — 2026-10-22

`examples/portable_probe.rs` sobre os 25 ids da lista do brief, catálogo local de 2026-10-22 (cópia do `catalog.db`, só leitura). Cada id foi planejado como o CLI faz (`plan_with_mode`, `Prefer(User)`, modo padrão); só os `Native` com `elevation == Helper` e tipo `msi`/`wix` foram baixados (SHA-256 conferido), analisados por `portable_msi::analyze` e extraídos pelo extrator próprio numa pasta temporária. Nada foi executado (sem `msiexec`, sem instalador). Downloads: ~1,09 GB na rodada principal e ~0,39 GB na rodada de diagnóstico (Go, Audacity, Zoom e parte do Chrome, interrompida pelo prazo); tudo apagado no fim (pasta de trabalho e cópia do catálogo).

## Tabela

| id | tipo | elevação | veredito | bloqueios | perdas | per_user_capable | extração | MB |
|---|---|---|---|---|---|---|---|---|
| 7zip.7zip | exe | helper | fora | - | - | - | - | - |
| Microsoft.PowerShell | msix | never | fora | - | - | - | - | - |
| VideoLAN.VLC | nullsoft | helper | fora | - | - | - | - | - |
| Mozilla.Firefox | - | - | fora (ponte: parâmetros do instalador fora da lista permitida para elevação: /PreventRebootRequired=true) | - | - | - | - | - |
| KeePassXCTeam.KeePassXC | - | - | fora (ponte: tem dependências: instalado pelo winget) | - | - | - | - | - |
| OpenJS.NodeJS.LTS | wix | helper | recusado | ação personalizada Wix4RollbackInternetShortcuts_X64 roda código durante a instalação; ação personalizada Wix4CreateInternetShortcuts_X64 roda código durante a instalação | ação personalizada SetInstallScope não roda; ação personalizada WixBroadcastEnvironmentChange não roda; ação personalizada GetLocalizedUserNames não roda; ação personalizada Wix4QueryOsWellKnownSID_X64 não roda; ação personalizada Wix4SchedInternetShortcuts_X64 não roda; 6 valor(es) de registro não gravado(s); variáveis de ambiente: PATH | false | ok | 31.6 |
| GoLang.Go | wix | helper | recusado | arquivos espalhados fora de uma pasta principal | ação personalizada WixBroadcastEnvironmentChange não roda; 3 valor(es) de registro não gravado(s); variáveis de ambiente: GOPATH, PATH | false | - | 63.0 |
| Amazon.Corretto.21.JDK | wix | helper | aceito | - | 8 valor(es) de registro não gravado(s); variáveis de ambiente: JAVA_HOME, PATH | false | ok | 171.3 |
| EclipseAdoptium.Temurin.21.JDK | wix | helper | aceito | - | ação personalizada Wix4RemoveFoldersEx_X64 não roda; 14 valor(es) de registro não gravado(s); variáveis de ambiente: JAVA_HOME, PATH | false | ok | 171.0 |
| Inkscape.Inkscape | wix | helper | aceito | - | 12 valor(es) de registro não gravado(s) | false | ok | 162.4 |
| TheDocumentFoundation.LibreOffice | - | - | fora (ponte: tem dependências: instalado pelo winget) | - | - | - | - | - |
| Google.Chrome | wix | helper | recusado | ação personalizada ShowInstallerResultUIString roda código durante a instalação; ação personalizada DoInstall roda código durante a instalação; ação personalizada CallUninstaller roda código durante a instalação; arquivos espalhados fora de uma pasta principal | ação personalizada ExtractTagInfoFromInstaller não roda; 1 valor(es) de registro não gravado(s) | false | - | 163.1 |
| WiresharkFoundation.Wireshark | nullsoft | helper | fora | - | - | - | - | - |
| Microsoft.AzureCLI | wix | helper | aceito | - | ação personalizada WixBroadcastEnvironmentChange não roda; 2 valor(es) de registro não gravado(s); variáveis de ambiente: PATH | false | ok | 66.8 |
| PuTTY.PuTTY | wix | helper | aceito | - | 12 valor(es) de registro não gravado(s); variáveis de ambiente: PATH | false | ok | 3.7 |
| WinSCP.WinSCP | inno | never | fora | - | - | - | - | - |
| Zoom.Zoom | msi | helper | recusado | ação personalizada _41948481_CB8B_4F3A_BB35_0C01B7B1FE61 roda código durante a instalação; ação personalizada _CDC9A98E_A73B_4178_98C2_53CCD6C0A8FA roda código durante a instalação; ação personalizada _A016FB52_9BC3_428A_853A_5C6D1B907BAE roda código durante a instalação; ação personalizada _CB567D4D_F9A0_4423_A98E_E399543A9584 roda código durante a instalação; arquivos espalhados fora de uma pasta principal | 1 valor(es) de registro não gravado(s) | false | - | 210.1 |
| Notepad++.Notepad++ | nullsoft | helper | fora | - | - | - | - | - |
| GIMP.GIMP | inno | never | fora | - | - | - | - | - |
| Audacity.Audacity | wix | helper | recusado | arquivos espalhados fora de uma pasta principal | 132 valor(es) de registro não gravado(s) | false | - | 47.5 |
| DBeaver.DBeaver.Community | nullsoft | never | fora | - | - | - | - | - |
| Git.Git | inno | never | fora | - | - | - | - | - |
| Python.Python.3.12 | burn | never | fora | - | - | - | - | - |
| Docker.DockerDesktop | - | - | fora (ponte: parâmetros do instalador fora da lista permitida para elevação: install) | - | - | - | - | - |
| Microsoft.VisualStudioCode | inno | never | fora | - | - | - | - | - |

"extração" é o `extract::extract(arquivo, "<id>.msi", main_dir.join("\\"), None, tmp)`; `-` quando não há `main_dir`.

## Contagens

- 25 ids: 10 MSI/WiX que pedem UAC (`helper`); 15 fora (4 Inno, 1 Burn e 1 NSIS sem UAC; 3 NSIS e 1 EXE com UAC; 1 MSIX; 4 vão para a ponte: Firefox e Docker por parâmetro fora da lista do auxiliar, KeePassXC e LibreOffice por dependências).
- Dos 10 MSI: **5 aceitos** (Corretto, Temurin, Inkscape, AzureCLI, PuTTY), 5 recusados.
- Recusas por regra (um pacote pode ter mais de uma): `DeferredAction` 3 (Node, Chrome, Zoom); `Scattered` 4 (Go, Chrome, Zoom, Audacity); `Service`, `Driver`, `Com`, `SystemRegistry` e `SystemFiles`: 0.
- Ações personalizadas de código: adiadas em 3 pacotes (9 ações: Node 2, Chrome 3, Zoom 4); imediatas em 5 pacotes (9 ações: Node 5, Go 1, Temurin 1, Chrome 1, AzureCLI 1).
- `per_user_capable`: 0 de 10.
- Extração própria: 6 de 6 com `main_dir` ok; na rodada de diagnóstico Go, Audacity e Zoom extraídos inteiros (sem `extract_dir`) também ok (9 de 9). Chrome não medido (rodada interrompida).
- Perdas mais comuns: valores de registro (10 de 10), variáveis de ambiente (6: `PATH` em 6, `JAVA_HOME` em 2, `GOPATH` em 1).

## `Scattered` com uma pasta só no topo

Rodada de diagnóstico (extração inteira, arquivos por pasta até 2 níveis):

- GoLang.Go: `Go\src: 12017, Go\test: 3514, Go\api: 30, Go\misc: 21, Go\lib: 17, Go\doc: 14, Go\pkg: 13, Go: 8, Go\bin: 2` — tudo sob `Go`.
- Audacity.Audacity: `Audacity 4\qml: 200, Audacity 4\bin: 142, Audacity 4\locale: 138, Audacity 4\licenses: 26, Audacity 4\workspaces: 3` — tudo sob `Audacity 4`.
- Zoom.Zoom: `Program Files (64-bit) Folder\Zoom: 357` e 1 arquivo na raiz do `SourceDir`.

Go e Audacity têm a pasta do app direto abaixo de `SourceDir` (a pasta de Program Files tem `DefaultDir` `.`), então o `main_dir` tem 1 componente e a regra "menos de 2 componentes ⇒ `Scattered`" recusa os dois. Com o limite em 1 componente eles passam (7 de 10 aceitos) e os testes da tarefa continuam valendo (`scattered_files_block` tem prefixo vazio). Zoom continua `Scattered` (1 arquivo na raiz).

## Números para as decisões

- **D-G2 (quais ações personalizadas bloqueiam):** com "só adiadas bloqueiam" (o código atual), 3 de 10 recusados por ação. As adiadas do Node são da biblioteca padrão do WiX (`Wix4CreateInternetShortcuts_X64`, que grava atalhos `.url`, e `Wix4RollbackInternetShortcuts_X64`, que pelo nome só roda em rollback; o `Type` não foi registrado, não medido se tem o bit `0x100`). Chrome (`DoInstall`, `CallUninstaller`) é um invólucro do instalador próprio e Zoom tem 4 ações de nome GUID: os dois são recusas corretas. Proposta: manter "adiada bloqueia", tratar como perda as de rollback (`Type & 0x100`) e as `Wix4*InternetShortcuts*` por lista fechada; isso aceitaria o Node (6 de 10, ou 8 de 10 junto com o limite de 1 componente). "Imediatas também bloqueiam" recusaria mais 2 aceitos hoje (Temurin, AzureCLI) e manteria Go recusado mesmo com o limite de 1 componente (as três são todas `WixBroadcastEnvironmentChange`/`Wix4RemoveFoldersEx`, inofensivas no modo portátil).
- **D-G3 (`msiexec /a` como reserva):** o extrator próprio aceitou 9 de 9 MSI medidos; a reserva não seria usada nesta amostra.
- **D-G5 (`SystemFiles` sempre bloqueia?):** nenhum dos 10 grava em pasta do Windows; a decisão não muda nada nesta amostra.
- **D-G1 (MSI dual-purpose por usuário):** `per_user_capable` 0 de 10; o caminho `ALLUSERS=2 MSIINSTALLPERUSER=1` não teria uso nesta amostra.
