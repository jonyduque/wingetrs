# Prova ao vivo consolidada (2b + 2c + 2d) — roteiro (2026-10-18)

Roteiro único para fechar a Task 131 (2b), a seção 7 da validação da 2c (Task 163) e a Task 188 (2d) numa sessão só. Preparado por um agente sem instalar nada: todos os pacotes abaixo foram conferidos com `--dry-run` em 2026-10-18 (binário `target/release` compilado às 17:03, com as edições ainda não commitadas da Task 164 no checkout). As saídas esperadas citadas aqui são as do dry-run; o que só a execução real mostra está marcado "anote".

**Antes de começar:** rode este roteiro depois que as Tasks 164 e 190–195 entrarem e recompile (`cargo build --release`). Se o texto de algum passo mudar por causa delas, vale o novo.

Convenções:
- PowerShell 7, **não elevado**, no Windows Terminal, na raiz do repositório, salvo quando o passo diz outra coisa.
- **[UAC]** = o passo abre um pedido de UAC. **[UAC-adm]** = precisa de um terminal de administrador (aberto pelo próprio usuário, não pelo wingetrs).
- Cada passo: comando, o que deve aparecer, caixa para marcar. Cole a saída real de cada passo no relatório da Task 188 (`docs/superpowers/reports/2026-10-XX-fase2d-validacao.md`) e da Task 131.

Contagem de UAC: 1 terminal de administrador (chaves de teste HKLM e restauração), 1 sessão da TUI no conhost (UAC **recusado** de propósito), 1 sessão da TUI no Windows Terminal (o UAC aceito que cobre todas as instalações/remoções de máquina e o órfão HKLM). Sem a TUI, cada comando CLI de escopo máquina pede o seu UAC (o auxiliar vive só enquanto o processo vive).

## Pacotes escolhidos (S4)

| Tipo | Pacote | Escopo / UAC | Linha de comando do dry-run | Observação |
|---|---|---|---|---|
| msi | `Rawns.Reddit-Wallpaper-Changer` (2 MiB) | escopo do instalador, pede UAC | `msiexec.exe /i "<instalador>" /quiet /norestart /log "<LOGPATH>"` | URL de tag móvel `release`: se der hash divergente, use `Zoom.Zoom` (msi, máquina, ~90 MiB). Não abra o app. |
| wix | `PuTTY.PuTTY` (3 MiB) | machine, pede UAC | idem msi | |
| burn | `Microsoft.DotNet.AspNetCore.8` (~9 MiB) | machine, pede UAC | `<instalador> /quiet /log "<LOGPATH>" /norestart` | runtime inofensivo; não está instalado |
| inno | `JAMSoftware.TreeSize.Free` (15 MiB) | user, sem UAC do wingetrs | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CurrentUser /LOG="<LOGPATH>" /CURRENTUSER` | manifesto diz `ElevationRequirement: elevatesSelf`: anote se o próprio instalador pede UAC |
| nullsoft | `PeterPawlowski.foobar2000` (7 MiB) | machine, pede UAC | `<instalador> /S` | |
| msix | `M2Team.NanaZip` (11 MiB) | escopo do instalador, sem UAC | (sem linha de comando) | o dry-run diz, errado, "abre a janela do instalador" (achado 1); `Microsoft.WinDbg` tem 1,1 GiB e `Microsoft.WindowsTerminal.Preview` vai para a ponte (dependências) |
| exe silencioso | `SumatraPDF.SumatraPDF` (10 MiB) | user, sem UAC | `<instalador> -s -install -with-preview` | também usado no teste de `upgrade` `install` |
| exe interativo | `Kumospace.Kumospace` (157 MiB) | user, sem UAC | `<instalador>` + "abre a janela do instalador (sem modo silencioso)" | URL `latest`: hash pode divergir; se divergir, anote e pule |
| ponte (zip) | `sharkdp.hyperfine` | — | `winget install --id sharkdp.hyperfine --exact --source winget --silent ...` | |
| upgrade `install` | `SumatraPDF.SumatraPDF` 3.6 → 3.6.1 | user | — | versão antiga instalada pelo próprio winget |
| upgrade `uninstallPrevious` | `0-don.clippy` 1.7.8 → 1.7.9 (65 MiB) | user, nullsoft | `<instalador> /S` | `UpgradeBehavior: uninstallPrevious` conferido no manifesto 1.7.9 |

Já instalados nesta máquina (não usar): Notepad++, CMake, Neovim, jq, fd, bat, ripgrep, Python 3.13, Graphviz, CrystalDiskMark, Chrome, PowerShell, Windows Terminal, 7-Zip (winget e scoop).

## Achados do dry-run (para não confundir com regressão)

1. `install M2Team.NanaZip --dry-run` (e `Microsoft.WinDbg`): mostra "abre a janela do instalador (sem modo silencioso)"; MSIX vai pelo `PackageManager`, sem janela. Causa: `mode_for` devolve `Interactive` para msix (sem switches padrão) e `plan.rs` exibe isso.
2. `install Microsoft.PowerShell.Preview --dry-run`: `erro: nenhum instalador para a plataforma Windows.Desktop` (código 1). O manifesto declara `Platform: Windows.Universal`; o `ManifestComparator` do winget não filtra por plataforma, então o winget instala.
3. `upgrade Microsoft.DotNet.DesktopRuntime.8 --dry-run` (e `.10`): escolhe `windowsdesktop-runtime-8.0.32-win-x86.exe`; o instalado é "(x64)". O registro do Burn fica em `WOW6432Node` e `engine.rs` trata `MachineX86` como arquitetura instalada x86. **Não rode esse upgrade na prova.**
4. `uninstall Graphviz.Graphviz --dry-run`: "abre a janela do desinstalador" sem `/S`, embora o manifesto seja `nullsoft`: o CLI (`run_win.rs`) passa `nsis_hint = false` sempre ("No manifest is read for a removal").
5. `install tumic0.GPXSee --dry-run` vai para a ponte por "parâmetros do instalador fora da lista permitida para elevação: /norestart"; `install Python.Python.3.12 --scope machine` idem por `InstallAllUsers=1`. A lista branca do auxiliar recusa switches inofensivos.
6. Dry-run do winget não mostra o SHA-256 (o do Scoop mostra); a confirmação da spec promete nome e SHA-256.
7. Textos: `install sharkdp.hyperfine --dry-run` ⇒ "Instalando pelo winget: tipo zip: instalado pelo winget" e "hyperfine 1.20.0 (winget) via winget" (redundante); `uninstall CrystalDewWorld.CrystalDiskMark --dry-run` ⇒ "Desinstalar CrystalDiskMark 9.0.3 9.0.3"; `uninstall jqlang.jq --dry-run` ⇒ comando `winget uninstall --product-code ...` rotulado "abre a janela do desinstalador"; `wingetrs install --help` ainda diz "ou um pacote do winget pelo próprio winget"; no plano do Scoop o passo "gravar ...manifest.json" sai em minúscula; o dry-run de `main/rust` não traz a linha "Se o extrator não suportar este MSI, pede --allow-msiexec" que o resumo da confirmação traz.
8. `--scope machine` com Inno duplica o switch (`/AllUsers ... /ALLUSERS`; no user, `/CurrentUser ... /CURRENTUSER`): inofensivo, igual ao winget.
9. **Risco da prova (não é defeito do wingetrs):** o `uninstaller.script` do `main/7zip` importa `uninstall-context.reg`, que apaga `HKCU\Software\7-Zip` — a chave do **seu** 7-Zip do winget (`Path64`, opções). O passo B4 faz backup e restaura.

---

## 0. Preparação e retratos "antes"

```powershell
cargo build --release
$W = "$PWD\target\release\wingetrs.exe"
$P = "$env:TEMP\wr-prova"; New-Item -ItemType Directory -Force $P | Out-Null
$arp = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
function Snap($tag) {
  Get-ChildItem -Recurse -Force "$env:USERPROFILE\scoop" -ErrorAction SilentlyContinue | Select-Object FullName, Length, LastWriteTime | Export-Csv "$P\scoop-$tag.csv" -NoTypeInformation
  reg export HKCU\Environment "$P\env-$tag.reg" /y | Out-Null
  & $W list --color never > "$P\list-$tag.txt"
  Get-ChildItem $arp -ErrorAction SilentlyContinue | Get-ItemProperty | Select-Object PSPath, DisplayName, DisplayVersion, UninstallString | Export-Csv "$P\arp-$tag.csv" -NoTypeInformation
  Get-AppxPackage | Select-Object PackageFullName | Export-Csv "$P\msix-$tag.csv" -NoTypeInformation
  Get-ChildItem -Recurse -Force "$env:LOCALAPPDATA\wingetrs\apps", "$env:LOCALAPPDATA\wingetrs\shims", "$env:LOCALAPPDATA\wingetrs\persist", "$env:LOCALAPPDATA\wingetrs\modules", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\wingetrs Apps" -ErrorAction SilentlyContinue | Select-Object FullName, Length | Export-Csv "$P\root-$tag.csv" -NoTypeInformation
  Get-ChildItem -Force "C:\Program Files\wingetrs\staging" -ErrorAction SilentlyContinue | Select-Object FullName | Export-Csv "$P\staging-$tag.csv" -NoTypeInformation
}
Snap antes
reg export "HKCU\Software\7-Zip" "$P\hkcu-7zip-antes.reg" /y
$log = "$env:LOCALAPPDATA\wingetrs\log\wingetrs.log.$(Get-Date -Format yyyy-MM-dd)"
$logStart = if (Test-Path $log) { (Get-Content $log).Count } else { 0 }
pwsh -File scripts/leftovers.ps1
```

- [ ] `cargo build` sem erro; `$P` criado; `hkcu-7zip-antes.reg` existe (a chave existe hoje, com `Path64 = C:\Program Files\7-Zip\`).
- [ ] `leftovers.ps1` sem processos (exit 0) — anote se houver.

## A. Task 131 — restante da 2b (reserva `msiexec /a`)

**A1. Plano do rust**
```powershell
& $W install main/rust --dry-run
```
- [ ] Esperado: "Instalar rust 1.99.1 (bucket main, 64bit)", 10 passos (baixar `.msi` com sha256, "Extrair ... (MSI, sem executar o instalador) ... (só a subpasta Rust)", junção, comandos `rustc`, `rustdoc`, `cargo`, PATH, registrar), "Simulação: nada foi alterado." (Sem menção ao msiexec: a recusa do extrator só aparece na execução — achado 7.)

**A2. Ctrl+C antes do msiexec**
```powershell
& $W install main/rust --allow-msiexec
# confirme; durante "Baixando" (spinner, MiB, velocidade) aperte Ctrl+C
$LASTEXITCODE; Test-Path "$env:LOCALAPPDATA\wingetrs\apps\rust"
```
- [ ] Esperado: a confirmação mostra "Se o extrator recusar este MSI, usa o Windows Installer (msiexec /a)"; no Ctrl+C sai "Cancelado.", `$LASTEXITCODE` = 130, `Test-Path` = False (ou só sem versão instalada; anote).

**A3. Ctrl+C durante o msiexec e instalação completa**
```powershell
& $W install main/rust --allow-msiexec --yes
# quando a linha mudar para a fase do Windows Installer (arquivo atual lido do log), aperte Ctrl+C uma vez
```
- [ ] Esperado: o extrator próprio recusa ("Number of rows is too large" ⇒ não suportado) e o wingetrs passa para `msiexec /a`; o Ctrl+C mostra **uma vez** "um passo já começou e não pode ser interrompido (...); esperando terminar"; a linha continua viva (spinner, relógio; "sem atividade há 1 min" se parar); no fim instala. Anote o tempo total e o desfecho.
- [ ] `rustc --version` num terminal novo funciona; `Get-ChildItem C:\Windows\Installer -Filter 'SourceHash*' -Force` (pode negar acesso sem admin; então pule) mostra o `SourceHash{ProductCode}` do rust, aceito pela spec.

**A4. Ponte e reserva ao mesmo tempo (não podem rodar juntas)**
```powershell
& $W uninstall rust --yes
# Terminal 1:
& $W install main/rust --allow-msiexec --yes
# Terminal 2, assim que o terminal 1 entrar na fase do Windows Installer:
& "$PWD\target\release\wingetrs.exe" install sharkdp.hyperfine --yes
```
- [ ] Esperado: o terminal 2 fica em "Aguardando o Windows Installer (outro instalador em uso)" com relógio até o terminal 1 terminar o `msiexec`; só então roda o `winget install`. Nenhum erro 1618. Anote os horários de início/fim dos dois (também no log: `job.start`/`job.end`).
- [ ] `hyperfine --version` num terminal novo (instalado pela ponte; vale como o pacote "ponte" da S4).
- [ ] Remover: `& $W uninstall rust --yes` (sem UAC). `hyperfine` fica para a fase E.

## B. 2c — scripts do Scoop, innosetup, `if ($global)`

**B1. pester (módulo PowerShell puro)**
```powershell
& $W install main/pester --dry-run
& $W install main/pester --yes
pwsh -NoProfile -Command "Get-Module -ListAvailable Pester | Select-Object Name, Version, Path"
```
- [ ] Dry-run: 8 passos (nupkg com sha256, "só a subpasta tools", "Ligar módulo PowerShell ... modules\Pester ... PSModulePath do usuário").
- [ ] Instala; o `pwsh` novo lista Pester 6.2.0 em `...\wingetrs\modules\Pester`.

**B2. xming (Inno com innounp)**
```powershell
& $W install main/xming --dry-run
& $W install main/xming --yes
Test-Path "$env:LOCALAPPDATA\wingetrs\shims\XLaunch.exe"
```
- [ ] Dry-run: dois planos — `innounp 2.71.2` (dependência) e `xming 6.9.0.31` com "Extrair ... (Inno Setup, com o innounp)" e hash md5.
- [ ] Instala; `True`. Anote se o innounp aparece em `& $W list` como app próprio ou oculto.

**B3. yori (S-hook com veredito Confirm)**
```powershell
& $W install main/yori --dry-run
& $W install main/yori --allow-scripts
```
- [ ] Dry-run: 19 passos (5 `.cab` baixados como `.7z`, persist `YoriInit.d` por hardlink, atalho `Yori.lnk`, "Rodar o script post_install do manifesto (PowerShell)") e o bloco "Script de terceiro (post_install) ... Análise: pede confirmação" com `1 | Remove-Item "$dir\pkginfo.ini" -Force`.
- [ ] Na execução a confirmação mostra o script; após aceitar, instala; `Test-Path "$env:LOCALAPPDATA\wingetrs\apps\yori\current\pkginfo.ini"` = False (o hook rodou).

**B4. 7zip (bloco `if ($global)`, Task 164)**
```powershell
& $W install main/7zip --dry-run
& $W install main/7zip --allow-scripts
Get-ChildItem "$env:LOCALAPPDATA\wingetrs\apps\7zip\current\*.reg"
```
- [ ] Dry-run (visto em 2026-10-18 com a 164 no checkout): post_install "Análise: pede confirmação" com "! só roda em instalação global; não roda no wingetrs (mexe no registro da máquina (HKLM)) (linha 4)"; `uninstaller.script` "pede atenção redobrada" por `reg import`. Se o dry-run disser "recusado", a 164 não entrou: anote e pule para B5.
- [ ] Instala; `$dir` tem `install-context.reg` e `uninstall-context.reg` com o caminho do wingetrs no lugar de `{{7zip_dir}}`. Nada importado no registro na instalação (`reg query "HKCU\Software\Classes\*\shellex\ContextMenuHandlers\7-Zip"` continua "não localizado").
- [ ] Atenção: o shim `7z` do wingetrs e o do `~/scoop` disputam o PATH; `where.exe 7z` mostra qual vem primeiro (anote).

**B5. Remoções da 2c**
```powershell
& $W uninstall pester --yes
& $W uninstall xming --yes
& $W uninstall yori --allow-scripts --purge
& $W uninstall 7zip --allow-scripts --purge
reg import "$P\hkcu-7zip-antes.reg"
reg export "HKCU\Software\7-Zip" "$P\hkcu-7zip-depois.reg" /y
Compare-Object (Get-Content "$P\hkcu-7zip-antes.reg") (Get-Content "$P\hkcu-7zip-depois.reg")
& $W list --color never | Select-String -Pattern 'innounp|pester|xming|yori|7zip'
```
- [ ] Remoções sem erro; a do 7zip mostra o `uninstaller.script` antes de rodar. Antes do `reg import`, `reg query "HKCU\Software\7-Zip"` deve dizer "não localizado" (o script apagou: esperado, anote). Depois do import, `Compare-Object` vazio.
- [ ] Anote se o `innounp` saiu junto com o xming; se ficou, `& $W uninstall innounp --yes`.
- [ ] A última linha só mostra o `7zip` do scoop (origem `scoop`), nada do wingetrs.

## C. 2d — S1 restante, S4, órfãos, Kindle

**C1. MSI de máquina sem elevação (hipótese "msi de máquina exige elevação")**
```powershell
Invoke-WebRequest https://the.earth.li/~sgtatham/putty/0.85/w64/putty-64bit-0.85-installer.msi -OutFile "$P\putty.msi"
(Start-Process msiexec.exe -ArgumentList '/i', "`"$P\putty.msi`"", '/quiet', '/norestart', '/l*v', "`"$P\putty-semadmin.log`"" -Wait -PassThru).ExitCode
```
- [ ] Esperado: código diferente de 0 (provável 1603 ou 1925); anote o código e a linha "Error" do log. Se der 0, o PuTTY foi instalado: remova com `msiexec /x "$P\putty.msi" /quiet` (pede UAC) e anote.

**C2. [UAC-adm] Chaves de teste para os órfãos** — abra um PowerShell **como administrador** (fica aberto até o fim):
```powershell
$P = "$env:TEMP\wr-prova"   # no terminal de admin o TEMP é o mesmo do usuário
reg add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /v DisplayName /t REG_SZ /d "wingetrs test orphan HKLM" /f
reg add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /v UninstallString /t REG_SZ /d '"C:\Program Files\wingetrs-test\u.exe"' /f
reg export "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" "$P\orf-hklm-antes.reg" /reg:64 /y
```
No terminal normal:
```powershell
reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /v DisplayName /t REG_SZ /d "wingetrs test orphan HKCU" /f
reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /v UninstallString /t REG_SZ /d "$env:LOCALAPPDATA\wingetrs-test\u.exe" /f
reg export "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" "$P\orf-hkcu-antes.reg" /y
& $W cleanup --orphans --dry-run
```
- [ ] O dry-run lista os dois "wingetrs test orphan" (HKCU e HKLM 64 bits, "falta ...u.exe") além dos 3 órfãos reais (Cities: Skylines, CS:GO, USB-Driver-Installer) e a pasta `backups\orphans-<n>`.

**C3. [UAC recusado] TUI no conhost: recusa vale para a sessão inteira**
```powershell
conhost.exe "$PWD\target\release\wingetrs.exe"
```
Na TUI: aba Busca, `PuTTY.PuTTY`, `i`, confirme; depois `foobar2000` (`PeterPawlowski.foobar2000`), `i`, confirme (os dois na fila).
- [ ] O UAC aparece em primeiro plano ou só pisca na barra de tarefas? (se piscar: a barra de status diz "Aguardando o UAC ou o instalador elevado; se o UAC não aparecer, clique no escudo piscando na barra de tarefas"? ajuda?) — anote.
- [ ] **Recuse** o UAC. Esperado: `Permissão de administrador recusada.` no primeiro job; o segundo **não** abre outro UAC (falha ou segue sem elevar com o mesmo aviso — anote). Saia com `q`.

**C4. [UAC] TUI no Windows Terminal: um UAC para toda a S4 de máquina**
```powershell
& $W
```
Na aba Busca, instale (`i`) nesta ordem, um de cada vez ou enfileirados (`f` mostra a fila):
1. `PuTTY.PuTTY` (wix, máquina) — **aqui aparece o único UAC; aceite.** Anote se apareceu ou piscou.
2. `Rawns.Reddit-Wallpaper-Changer` (msi) — sem novo UAC.
3. `Microsoft.DotNet.AspNetCore.8` (burn, máquina) — sem novo UAC.
4. `PeterPawlowski.foobar2000` (nullsoft, máquina) — sem novo UAC.
5. `JAMSoftware.TreeSize.Free` (inno, usuário) — sem UAC do wingetrs; anote se o instalador pede UAC sozinho (`elevatesSelf`).
6. `M2Team.NanaZip` (msix) — sem janela de instalador (o texto do plano diz o contrário: achado 1).
7. `SumatraPDF.SumatraPDF` (exe silencioso, usuário).
8. `Kumospace.Kumospace` (exe interativo) — abre a janela do instalador; conclua nela. Se der hash divergente, anote e pule.

- [ ] Um único UAC na sessão (anote se apareceu outro e em qual item).
- [ ] Cada item termina com sucesso; aba Instalados mostra os 8.
- [ ] Ajustes: `is_elevated` falso na TUI; no log, a linha do auxiliar diz elevado (confira em C7).

Ainda na mesma TUI, aba Instalados, remova (`u`) os 8 acima (os de máquina pelo auxiliar, sem novo UAC).
- [ ] Nenhum UAC novo; cada remoção some do ARP (`& $W list` depois). Anote os que abriram janela de desinstalador (esperado: nenhum além do Kumospace, se o ARP dele não tiver Quiet).

Órfãos, na mesma TUI (aba Instalados, filtre por "wingetrs test"): selecione cada "wingetrs test orphan" e use "Remover registro órfão".
- [ ] HKCU removido no processo; HKLM pelo auxiliar, sem novo UAC; toast "Registro órfão de ... removido (cópia em ...)" com o caminho do `.reg`.

Kindle, na mesma TUI: selecione "Amazon Kindle", `u`, leia o modal e **cancele** (Esc).
- [ ] O modal diz qual das duas instalações (2.9.1 do usuário / 2.8.3 da máquina) e mostra "Outra instalação de Amazon Kindle usa as mesmas pastas: copie antes de remover." com `C:\Users\jonyd\AppData\Local\Amazon`. Nada removido.

Saia com `q`.

> **Sem TUI (alternativa CLI):** os mesmos itens com `& $W install <id>` e `& $W uninstall <id>`; cada comando de máquina (PuTTY, Rawns, AspNetCore.8, foobar2000) pede **o seu** UAC. Órfãos: `& $W cleanup --orphans` e marque só os dois "wingetrs test orphan" (nenhum vem marcado) — [UAC] para o HKLM. Kindle: `& $W uninstall "Amazon Kindle" --dry-run` (esperado: comando `...\Kindle\application\uninstall.exe`, "abre a janela do desinstalador" e o aviso das pastas compartilhadas).

**C5. Restaurar os órfãos com o `.reg` do wingetrs**
```powershell
$bk = Get-ChildItem "$env:LOCALAPPDATA\wingetrs\backups" -Recurse -Filter '*wingetrs-test-orphan*.reg' | Sort-Object LastWriteTime | Select-Object -Last 2
$bk.FullName   # hkcu-wingetrs-test-orphan.reg e hklm64-wingetrs-test-orphan.reg
reg import ($bk | Where-Object Name -like 'hkcu-*').FullName
reg export "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" "$P\orf-hkcu-depois.reg" /y
Compare-Object (Get-Content "$P\orf-hkcu-antes.reg") (Get-Content "$P\orf-hkcu-depois.reg")
```
No terminal de admin (o import em HKLM exige elevação):
```powershell
reg import "<caminho do hklm64-wingetrs-test-orphan.reg listado acima>"
reg export "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" "$P\orf-hklm-depois.reg" /reg:64 /y
Compare-Object (Get-Content "$P\orf-hklm-antes.reg") (Get-Content "$P\orf-hklm-depois.reg")
```
- [ ] Os dois `Compare-Object` vazios (a chave volta idêntica). Se o nome tiver sufixo (`-2.reg`), é porque já havia backup com o mesmo nome: use o mais novo.

**C6. Upgrade `install` e `uninstallPrevious`** (versões antigas instaladas pelo próprio winget, escopo usuário, sem UAC)
```powershell
winget install --id SumatraPDF.SumatraPDF --version 3.6 --scope user --exact --silent
& $W upgrade SumatraPDF.SumatraPDF --dry-run
& $W upgrade SumatraPDF.SumatraPDF --yes
winget install --id 0-don.clippy --version 1.7.8 --exact --silent
& $W upgrade 0-don.clippy --dry-run
& $W upgrade 0-don.clippy --yes
& $W list --color never | Select-String 'Sumatra|clippy'
```
- [ ] Sumatra: dry-run "SumatraPDF 3.6.1 (exe, user)" com `-s -install -with-preview` e sem "pede UAC"; após o upgrade, uma linha só, 3.6.1.
- [ ] clippy: dry-run "(nullsoft, user)" com `/S`; na execução o wingetrs remove a 1.7.8 antes (anote o que aparece na tela) e fica uma linha só, 1.7.9.
- [ ] Remover: `& $W uninstall SumatraPDF.SumatraPDF --yes` e `& $W uninstall 0-don.clippy --yes`.

**C7. Log e staging**
```powershell
Get-Content $log | Select-Object -Skip $logStart | Select-String 'job\.(start|end)' | ForEach-Object { $_.Line.Substring(0, [Math]::Min(300, $_.Line.Length)) }
Get-ChildItem -Force "C:\Program Files\wingetrs\staging" -ErrorAction SilentlyContinue
```
- [ ] Cada job tem `job.start` e `job.end`; os de máquina com `elevated=true`, os de usuário `elevated=false`; `job.start` com `agreements=` (vazio para todos estes pacotes).
- [ ] `staging` vazio (ou inexistente).

## D. Retratos "depois" e comparação

```powershell
Snap depois
Compare-Object (Import-Csv "$P\scoop-antes.csv") (Import-Csv "$P\scoop-depois.csv") -Property FullName, Length, LastWriteTime
Compare-Object (Get-Content "$P\env-antes.reg") (Get-Content "$P\env-depois.reg")
Compare-Object (Import-Csv "$P\arp-antes.csv") (Import-Csv "$P\arp-depois.csv") -Property PSPath, DisplayVersion
Compare-Object (Import-Csv "$P\msix-antes.csv") (Import-Csv "$P\msix-depois.csv") -Property PackageFullName
Compare-Object (Import-Csv "$P\root-antes.csv") (Import-Csv "$P\root-depois.csv") -Property FullName
Compare-Object (Get-Content "$P\list-antes.txt") (Get-Content "$P\list-depois.txt")
pwsh -File scripts/leftovers.ps1
```
- [ ] `~/scoop`: nenhuma diferença.
- [ ] `HKCU\Environment`: no máximo `Path`/`PSModulePath` com as pastas `...\wingetrs\shims`/`...\wingetrs\modules` que já existiam antes (nenhuma entrada nova que fique depois das remoções). Anote o que mudou.
- [ ] ARP: só as duas chaves `wingetrs-test-orphan` restauradas (somem na fase E) e `hyperfine` (ponte). Qualquer outra diferença = sobra: anote o pacote.
- [ ] MSIX: nenhuma diferença (NanaZip removido).
- [ ] Raiz do wingetrs: nenhuma pasta de app sobrando (`apps\rust`, `apps\yori`, ...); `persist` sem `yori`/`7zip` (removidos com `--purge`).
- [ ] `leftovers.ps1` sem processos novos.

## E. Limpeza

```powershell
& $W uninstall sharkdp.hyperfine --yes
reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /f
```
No terminal de admin, depois feche-o:
```powershell
reg delete "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /f
```
- [ ] `hyperfine` removido (anote se foi pela ponte `winget uninstall` ou pelo ARP); as duas chaves de teste sumiram (`reg query` diz "não localizado").
- [ ] `Remove-Item -Recurse "$P"` só depois de colar as saídas nos relatórios. `C:\Windows\Installer\SourceHash{...}` do rust fica (permanente, aceito pela spec da 2b).
