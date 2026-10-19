# Prova ao vivo consolidada (2b + 2c + 2d) — roteiro (revisto em 2026-10-19)

Roteiro único para fechar a Task 131 (2b), a seção 7 da validação da 2c (Task 163) e a Task 188 (2d) numa sessão só. Revisto depois das Tasks 164 e 190–201 (HEAD `0bcc409`). Todos os pacotes foram conferidos de novo com `--dry-run` em 2026-10-19 (binário `target/release` compilado neste HEAD, ajustes padrão: `installer_mode = progress`, `default_scope = user`, `winget_engine = wingetrs`). As saídas citadas são as do dry-run; o que só a execução real mostra está marcado **anote**.

O que mudou desde a versão de 2026-10-18:
- O auxiliar elevado (`bridge-worker`) só sobe se o `wingetrs.exe` e **cada pasta acima dele** forem graváveis só por administradores (Task 197). Rodado de `target\release`, recusa. A prova roda da cópia em `C:\Program Files\wingetrs\` (passo 0.2).
- Cada instalador nativo de escopo máquina pede **o seu** UAC (`runas`), roda de uma pasta nova `installers\run-<hex>`, sem log (Task 196). O UAC único da sessão (auxiliar) cobre só a ponte do winget elevada, os desinstaladores HKLM e a remoção de chaves HKLM.
- Antes de **todo** UAC, CLI e TUI mostram `Aguardando autorização para executar como administrador` com relógio; recusa mostra `Permissão de administrador recusada.` (Task 201).
- Modo padrão dos instaladores: "silencioso com progresso" (`/passive`, `/SILENT`); o ajuste `installer_mode` ("Janela do instalador (winget)" nos Ajustes da TUI: "mostrar progresso" / "sem janela") troca (Task 198).
- Toda escolha do CLI é um seletor na linha (Task 200); a saída passa pelo formatador, planos em passos numerados, URL/SHA só no `--dry-run` (Task 199).
- 2c: `if ($global)` é informativo (Task 164): 7zip, git etc. planejam; python, vulkan, pwsh e lazarus continuam recusados. Arquivos chamados pelo script são mostrados ou marcados (Task 192).

## Convenções

- PowerShell 7, **não elevado**, no Windows Terminal, na raiz do repositório, salvo quando o passo diz outra coisa.
- **[UAC]** = o passo abre um pedido de UAC do wingetrs. **[ADM]** = comando no terminal de administrador aberto por você (passo 0.2), não pelo wingetrs.
- **Seletor do CLI:** a confirmação `Continuar?` mostra `Sim` / `Não` com o cursor em **Não** (padrão). Para aceitar: `s` (ou ↑ e Enter). `n`, Esc, `q` ou Ctrl+C = Não. Listas de marcar (órfãos, resíduos): rodapé `setas movem · Espaço marca · Enter confirma · Esc cancela`. Lista de escolha única: `setas movem · letra ou Enter escolhe · Esc cancela`.
- Cada passo: comando, o que deve aparecer, caixa para marcar. Cole a saída real no relatório da Task 188 (`docs/superpowers/reports/2026-10-XX-fase2d-validacao.md`) e da Task 131.

**Contagem de UAC prevista (sessão inteira):** 1 terminal de administrador (aberto por você) + 1 UAC recusado de propósito (passo D1) + 4 UAC aceitos, um por instalador nativo de máquina (D2: PuTTY, Reddit Wallpaper Changer, ASP.NET Core 8, foobar2000) + 1 UAC aceito do auxiliar na sessão da TUI que remove tudo (D3). Os UACs estão todos no bloco D; as fases A, B e C não abrem nenhum.

## Pacotes escolhidos (S4), conferidos em 2026-10-19

| Tipo | Pacote | Escopo / UAC | Linha de comando do dry-run | Observação |
|---|---|---|---|---|
| wix | `PuTTY.PuTTY` 0.85.0.0 | máquina, [UAC] próprio | `msiexec.exe /i "<instalador>" /passive /norestart` | passo 1 do plano: "⚠ Pede permissão de administrador para o Windows Installer" |
| msi | `Rawns.Reddit-Wallpaper-Changer` 1.0.13 | escopo do instalador, [UAC] próprio | idem | URL de tag móvel `release`: hash divergente ⇒ anote e pule. Não abra o app. |
| burn | `Microsoft.DotNet.AspNetCore.8` 8.0.32 | máquina, [UAC] próprio | `<instalador> /passive /norestart` | runtime inofensivo; não instalado |
| nullsoft | `PeterPawlowski.foobar2000` 2.26.1 | máquina, [UAC] próprio | `<instalador> /S` | |
| inno | `JAMSoftware.TreeSize.Free` 4.8.1.610 | usuário, sem UAC do wingetrs | `/SILENT /SUPPRESSMSGBOXES /NORESTART /CurrentUser /LOG="<LOGPATH>"` | `ElevationRequirement: elevatesSelf`: o próprio instalador pode pedir UAC; o wingetrs mostra o aviso por até 2 min |
| msix | `M2Team.NanaZip` 7.0.1843.0 | escopo do instalador, sem UAC | (nenhuma; plano sem passos) | `PackageManager`, sem janela |
| exe silencioso | `SumatraPDF.SumatraPDF` 3.6.1 | usuário, sem UAC | `<instalador> -s -install -with-preview` | também no teste de upgrade `install` |
| exe interativo | `Kumospace.Kumospace` 7.4.2 | usuário, sem UAC | `<instalador>` + "Abre a janela do instalador (sem modo silencioso)" | URL `latest`: hash divergente ⇒ anote e pule |
| ponte (zip) | `sharkdp.hyperfine` 1.20.0 | usuário, sem UAC | `winget install --id sharkdp.hyperfine --exact --source winget --silent --disable-interactivity --accept-package-agreements --accept-source-agreements` | |
| upgrade `install` | `SumatraPDF.SumatraPDF` 3.6 → 3.6.1 | usuário | — | versão antiga instalada pelo próprio winget (C4) |
| upgrade `uninstallPrevious` | `0-don.clippy` 1.7.8 → 1.7.9 | usuário, nullsoft | `<instalador> /S` | |

Já instalados nesta máquina (não usar): Notepad++, CMake, Neovim, jq, fd, bat, ripgrep, Python 3.13, Graphviz, CrystalDiskMark, Chrome, PowerShell, Windows Terminal, 7-Zip (winget e scoop), Amazon Kindle (2 instalações).

## Achados do dry-run de 2026-10-19 (para não confundir com regressão)

Corrigidos desde 2026-10-18 (conferidos): MSIX não diz mais "abre a janela"; `Microsoft.PowerShell.Preview` planeja (msix); `upgrade Microsoft.DotNet.DesktopRuntime.8` escolhe `win-x64`; `uninstall Graphviz.Graphviz` traz `/S`; GPXSee nativo (`/S /norestart`); dry-run do winget mostra SHA-256; "CrystalDiskMark 9.0.3" sem versão repetida; `install --help` atualizado; linha do msiexec no dry-run do `main/rust`; Inno sem switch duplicado.

Ainda abertos (nenhum bloqueia a prova):
1. `uninstall jqlang.jq --dry-run`: `1. Abre a janela do desinstalador` para o comando `winget uninstall --product-code jqlang.jq_Microsoft.Winget.Source_8wekyb3d8bbwe` (a ponte não abre janela).
2. `install PeterPawlowski.foobar2000 --dry-run` (nullsoft, `/S`): `3. Mostra o progresso do instalador`; o NSIS não tem modo com progresso (Task 198), roda sem janela.
3. `install M2Team.NanaZip --dry-run` (msix): plano sem nenhum passo numerado (só `Instalador:` e `SHA-256:`); falta algo como "Instala pelo Windows (PackageManager), sem janela".
4. `install sharkdp.hyperfine --dry-run` (ponte): sem passos numerados e o título diz `Instalando pelo winget: tipo zip` numa simulação.
5. `uninstall "Amazon Kindle" --dry-run`: avisos viram passos (`2. Outra instalação de Amazon Kindle usa as mesmas pastas: copie antes de remover.`, `3. Pasta compartilhada: ...`, `4. Tamanho da cópia: 800,4 MiB em 498 itens`).
6. `uninstall Graphviz.Graphviz --dry-run` (e CrystalDiskMark, Notepad++): `1. Roda o desinstalador` antes de `2. ⚠ Pede permissão de administrador (UAC)`; no install o UAC é o passo 1.
7. `install JAMSoftware.TreeSize.Free --dry-run`: `elevatesSelf` no manifesto, mas o plano não avisa que o instalador pode pedir permissão de administrador (a spec D2 quer "pede UAC" na confirmação).
8. `install main/python --dry-run`: `✗ o script uninstaller.script foi recusado: usa -Global (instalação para todos os usuários); usa -Global (instalação para todos os usuários)` (motivo repetido).
9. `install Python.Python.3.12 --scope machine --dry-run`: vai para a ponte por `parâmetros do instalador fora da lista permitida para elevação: PrependPath=1` (switch inofensivo, como `/norestart` e `InstallAllUsers=1` antes da Task 190).
10. `install tumic0.GPXSee --dry-run`: `SHA-256: da5b31...` em minúsculas; os outros saem em maiúsculas.
11. **Risco da prova (não é defeito):** o `uninstaller.script` do `main/7zip` importa `uninstall-context.reg`, que apaga `HKCU\Software\7-Zip` do **seu** 7-Zip do winget. B4/B6 fazem backup e restauram.

---

## 0. Preparação

**0.1 Compilar e retratos "antes"** (terminal normal)
```powershell
cargo build --release
$Wdev = "$PWD\target\release\wingetrs.exe"
$W = 'C:\Program Files\wingetrs\wingetrs.exe'
$P = "$env:TEMP\wr-prova"; New-Item -ItemType Directory -Force $P | Out-Null
$arp = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
function Snap($tag) {
  Get-ChildItem -Recurse -Force "$env:USERPROFILE\scoop" -ErrorAction SilentlyContinue | Select-Object FullName, Length, LastWriteTime | Export-Csv "$P\scoop-$tag.csv" -NoTypeInformation
  reg export HKCU\Environment "$P\env-$tag.reg" /y | Out-Null
  & $Wdev list --color never > "$P\list-$tag.txt"
  Get-ChildItem $arp -ErrorAction SilentlyContinue | Get-ItemProperty | Select-Object PSPath, DisplayName, DisplayVersion, UninstallString | Export-Csv "$P\arp-$tag.csv" -NoTypeInformation
  Get-AppxPackage | Select-Object PackageFullName | Export-Csv "$P\msix-$tag.csv" -NoTypeInformation
  Get-ChildItem -Recurse -Force "$env:LOCALAPPDATA\wingetrs\apps", "$env:LOCALAPPDATA\wingetrs\shims", "$env:LOCALAPPDATA\wingetrs\persist", "$env:LOCALAPPDATA\wingetrs\modules", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\wingetrs Apps" -ErrorAction SilentlyContinue | Select-Object FullName, Length | Export-Csv "$P\root-$tag.csv" -NoTypeInformation
  Get-ChildItem -Force "$env:LOCALAPPDATA\wingetrs\installers" -Filter 'run-*' -ErrorAction SilentlyContinue | Select-Object FullName | Export-Csv "$P\run-$tag.csv" -NoTypeInformation
  Get-ChildItem -Force "C:\Program Files\wingetrs\staging" -ErrorAction SilentlyContinue | Select-Object FullName | Export-Csv "$P\staging-$tag.csv" -NoTypeInformation
}
Snap antes
reg export "HKCU\Software\7-Zip" "$P\hkcu-7zip-antes.reg" /y
$log = "$env:LOCALAPPDATA\wingetrs\log\wingetrs.log.$(Get-Date -Format yyyy-MM-dd)"
$logStart = if (Test-Path $log) { (Get-Content $log).Count } else { 0 }
pwsh -File scripts/leftovers.ps1
```
- [ ] `cargo build` sem erro; `$P` criado; `hkcu-7zip-antes.reg` existe (`Path64 = C:\Program Files\7-Zip\`).
- [ ] `leftovers.ps1` sem processos (exit 0); anote se houver.

**0.2 [ADM] Copiar o binário para `C:\Program Files\wingetrs`** — abra um PowerShell 7 **como administrador** (Win+X ⇒ Terminal (Admin)); ele fica aberto até o fim (C2, D5, E).
```powershell
$repo = 'C:\Users\jonyd\Projetos\wingetrs'
$P = "C:\Users\jonyd\AppData\Local\Temp\wr-prova"
New-Item -ItemType Directory -Force 'C:\Program Files\wingetrs' | Out-Null
Copy-Item "$repo\target\release\wingetrs.exe" 'C:\Program Files\wingetrs\wingetrs.exe' -Force
(Get-Acl 'C:\Program Files\wingetrs').Owner
(Get-Acl 'C:\Program Files\wingetrs\wingetrs.exe').Owner
icacls 'C:\Program Files\wingetrs'
(Get-FileHash "$repo\target\release\wingetrs.exe").Hash -eq (Get-FileHash 'C:\Program Files\wingetrs\wingetrs.exe').Hash
```
- [ ] Donos `BUILTIN\Administradores` (ou `SISTEMA`/`TrustedInstaller`); `icacls` só com escrita para TrustedInstaller, SISTEMA, Administradores e `PROPRIETÁRIO CRIADOR` (só `(IO)`); `Usuários` só `(RX)`; hash `True`.
- [ ] Se recompilar no meio da prova, repita o `Copy-Item` (o `$W` aponta para a cópia).

**0.3 Ajuste `installer_mode` (Task 198), só dry-run**
```powershell
& $W install PuTTY.PuTTY --dry-run
& $W install Microsoft.DotNet.AspNetCore.8 --dry-run
& $W install JAMSoftware.TreeSize.Free --dry-run
```
- [ ] Padrão: PuTTY `msiexec.exe /i "<instalador>" /passive /norestart`; AspNetCore `<instalador> /passive /norestart`; TreeSize `/SILENT /SUPPRESSMSGBOXES /NORESTART /CurrentUser /LOG="<LOGPATH>"`. Os três com `Mostra o progresso do instalador`; PuTTY e AspNetCore com `1. ⚠ Pede permissão de administrador ...` e `2. Roda numa pasta temporária do wingetrs`; a linha de comando dos dois de máquina **sem** `/log`.

Abra `& $W` (TUI), Ajustes ⇒ "Janela do instalador (winget)" ⇒ "sem janela", saia com `q`, e repita os três dry-runs.
- [ ] Esperado: PuTTY e AspNetCore com `/quiet` no lugar de `/passive`; TreeSize com `/VERYSILENT`; a linha `Mostra o progresso do instalador` some (anote o texto que aparece no lugar). `config.toml` ganhou `installer_mode = "silent"` em `[behavior]`.
- [ ] Volte o ajuste para "mostrar progresso" na TUI e confira que o dry-run do PuTTY voltou a `/passive`.

## A. Task 131 — restante da 2b (reserva `msiexec /a`), sem UAC

**A1. Plano do rust**
```powershell
& $W install main/rust --dry-run
```
- [ ] Esperado: `Instalar rust 1.99.1  main · 64bit`, 10 passos (`1. Baixar [1] https://static.rust-lang.org/dist/rust-1.99.1-x86_64-pc-windows-msvc.msi (..., sha256 11d5b1d2...)`, `2. Extrair [1] ... (MSI, sem executar o instalador) ... (só a subpasta Rust)`, junção, comandos `rustc`/`rustdoc`/`cargo`, PATH, manifest, registrar), `Se o extrator não suportar este MSI, pede --allow-msiexec`, `Simulação: nada foi alterado.`

**A2. Seletor e Ctrl+C antes do msiexec**
```powershell
& $W install main/rust --allow-msiexec
# no seletor "Continuar?" (cursor em Não): aperte s. Durante "Baixando" aperte Ctrl+C.
$LASTEXITCODE; Test-Path "$env:LOCALAPPDATA\wingetrs\apps\rust"
```
- [ ] A confirmação mostra o plano **sem URL nem sha256** (só no dry-run) e `Se o extrator recusar este MSI, usa o Windows Installer (msiexec /a)`; o seletor aparece na linha, Não marcado; `s` aceita.
- [ ] Linha de progresso com spinner, fase, MiB, velocidade e relógio; Ctrl+C ⇒ `Cancelado.`, `$LASTEXITCODE` = 130, `Test-Path` = False (anote se sobrou pasta sem versão).

**A3. Ctrl+C durante o msiexec e instalação completa**
```powershell
& $W install main/rust --allow-msiexec --yes
# quando a linha mudar para a fase do Windows Installer, aperte Ctrl+C uma vez
```
- [ ] O extrator recusa e passa para `msiexec /a`; o Ctrl+C mostra **uma vez** que o passo não pode ser interrompido e espera terminar; a linha continua viva (spinner, relógio; "sem atividade há 1 min" se parar); no fim instala. Anote o tempo total.
- [ ] Nenhum UAC (o `msiexec /a` não pede).
- [ ] `rustc --version` num terminal novo funciona.

**A4. Ponte e reserva ao mesmo tempo (fila do `msi.lock`)**
```powershell
& $W uninstall rust --yes
# Terminal 1:
& $W install main/rust --allow-msiexec --yes
# Terminal 2, assim que o terminal 1 entrar na fase do Windows Installer:
& 'C:\Program Files\wingetrs\wingetrs.exe' install sharkdp.hyperfine --yes
```
- [ ] Terminal 2: fica em "Aguardando o Windows Installer (outro instalador em uso)" com relógio até o terminal 1 terminar o `msiexec`; depois roda o `winget install` (saída do winget no stdout, progresso no stderr). Nenhum erro 1618, nenhum UAC (hyperfine é zip de usuário). Anote os horários (`job.start`/`job.end` no log).
- [ ] Com `--yes` não aparece seletor. `hyperfine --version` num terminal novo funciona (pacote "ponte" da S4).
- [ ] `& $W uninstall rust --yes` (sem UAC). `hyperfine` fica para a fase E.

## B. 2c — scripts do Scoop, innosetup, `if ($global)`, sem UAC

**B1. pester (módulo PowerShell)**
```powershell
& $W install main/pester --dry-run
& $W install main/pester --yes
pwsh -NoProfile -Command "Get-Module -ListAvailable Pester | Select-Object Name, Version, Path"
```
- [ ] Dry-run: 8 passos (`pester.6.2.0.nupkg` com sha256, `(só a subpasta tools)`, `5. Ligar módulo PowerShell ...\modules\Pester ... PSModulePath do usuário`).
- [ ] Instala; o `pwsh` novo lista Pester 6.2.0 em `...\wingetrs\modules\Pester`.

**B2. xming (Inno com innounp)**
```powershell
& $W install main/xming --dry-run
& $W install main/xming --yes
Test-Path "$env:LOCALAPPDATA\wingetrs\shims\XLaunch.exe"
```
- [ ] Dry-run: dois planos, `Instalar innounp 2.71.2` e `Instalar xming 6.9.0.31` com `Extrair [1] Xming-mesa-6-9-0-31-setup.exe (Inno Setup, com o innounp)` e `md5 e580debb...`.
- [ ] Instala; `True`. Anote se o innounp aparece em `& $W list`.

**B3. yori (hook com veredito "pede confirmação")**
```powershell
& $W install main/yori --dry-run
& $W install main/yori --allow-scripts
# seletor: s
Test-Path "$env:LOCALAPPDATA\wingetrs\apps\yori\current\pkginfo.ini"
```
- [ ] Dry-run: 19 passos (5 `.cab` baixados como `.7z`, `12. Persistir arquivo ...YoriInit.d ... (hardlink)`, atalho `Yori.lnk`, `17. Rodar o script post_install do manifesto (PowerShell)`), e o bloco `Script de terceiro (post_install), roda com os seus privilégios ...` / `Análise: pede confirmação` / `1 | Remove-Item "$dir\pkginfo.ini" -Force`.
- [ ] Na execução a confirmação mostra o script e o seletor; após `s`, instala; `Test-Path` = False (o hook rodou).

**B4. 7zip (bloco `if ($global)` informativo, Task 164)**
```powershell
& $W install main/7zip --dry-run
& $W install main/7zip --allow-scripts
# seletor: s
Get-ChildItem "$env:LOCALAPPDATA\wingetrs\apps\7zip\current\*.reg"
reg query "HKCU\Software\Classes\*\shellex\ContextMenuHandlers\7-Zip"
where.exe 7z
```
- [ ] Dry-run: 15 passos; post_install `Análise: pede confirmação` com `! só roda em instalação global; não roda no wingetrs (mexe no registro da máquina (HKLM)) (linha 4)` e as 6 linhas do script; `uninstaller.script (na remoção)` com `Análise: pede atenção redobrada; revise antes de permitir` e `! mexe no registro do usuário (HKCU ou reg) (linha 1)`.
- [ ] Instala; `$dir` tem `install-context.reg` e `uninstall-context.reg` com o caminho do wingetrs no lugar de `{{7zip_dir}}`; `reg query` continua "não foi possível localizar" (nada importado).
- [ ] Anote a ordem do `where.exe 7z` (shim do wingetrs × `~/scoop` × Program Files).

**B5. git (dois `if ($global)` e quatro scripts) e recusas que continuam**
```powershell
& $W install main/git --dry-run
& $W install main/python --dry-run; $LASTEXITCODE
& $W install main/vulkan --dry-run; $LASTEXITCODE
& $W install main/pwsh --dry-run; $LASTEXITCODE
& $W install extras/lazarus --dry-run; $LASTEXITCODE
```
- [ ] git: 24 passos (inclui `4. Rodar o script pre_install`, `20. Definir variável de usuário GIT_INSTALL_ROOT`, `22. Rodar o script post_install`); post_install com `! só roda em instalação global; ... (linha 4)`; `uninstaller.script` com `! mexe no registro do usuário (HKCU ou reg)` nas linhas 2 e 3. Não instale (só plano).
- [ ] python: `✗ o script uninstaller.script foi recusado: usa -Global ...` (achado 8: motivo repetido), exit 1.
- [ ] vulkan: `✗ o script pre_uninstall foi recusado: mexe no registro da máquina (HKLM)`, exit 1.
- [ ] pwsh e lazarus: `✗ o script post_install foi recusado: mexe no registro da máquina (HKLM)`, exit 1. (O lazarus está no `extras`; `main/lazarus` diz "nenhum pacote encontrado".)

**B6. Remoções da 2c**
```powershell
& $W uninstall pester --yes
& $W uninstall xming --yes
& $W uninstall yori --allow-scripts --purge
& $W uninstall 7zip --allow-scripts --purge
reg query "HKCU\Software\7-Zip"
reg import "$P\hkcu-7zip-antes.reg"
reg export "HKCU\Software\7-Zip" "$P\hkcu-7zip-depois.reg" /y
Compare-Object (Get-Content "$P\hkcu-7zip-antes.reg") (Get-Content "$P\hkcu-7zip-depois.reg")
& $W list --color never | Select-String -Pattern 'innounp|pester|xming|yori|7zip'
```
- [ ] Remoções sem erro; as de yori e 7zip mostram o plano de remoção em passos e o seletor (`s`); a do 7zip mostra o `uninstaller.script` antes de rodar.
- [ ] `reg query` antes do import = "não foi possível localizar" (o script apagou: esperado). Depois do import, `Compare-Object` vazio.
- [ ] Anote se o `innounp` saiu junto; se ficou, `& $W uninstall innounp --yes`.
- [ ] A última linha só mostra o `7zip` do scoop (origem `scoop`).

## C. 2d sem UAC — escopo usuário, msix, ponte, upgrades, recusa da pasta

**C1. MSI de máquina sem elevação (hipótese "msi de máquina exige elevação")**
```powershell
Invoke-WebRequest https://the.earth.li/~sgtatham/putty/0.85/w64/putty-64bit-0.85-installer.msi -OutFile "$P\putty.msi"
(Start-Process msiexec.exe -ArgumentList '/i', "`"$P\putty.msi`"", '/quiet', '/norestart', '/l*v', "`"$P\putty-semadmin.log`"" -Wait -PassThru).ExitCode
```
- [ ] Esperado: código ≠ 0 (provável 1603 ou 1925); anote o código e a linha "Error" do log. Se der 0, o PuTTY foi instalado: anote e pule o item PuTTY do D2 (remova-o no D3).

**C2. Chaves de teste dos órfãos**
[ADM], no terminal de administrador:
```powershell
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
- [ ] O dry-run lista os dois `Órfão: wingetrs test orphan ...` (HKCU e `HKLM 64 bits`, `falta ...u.exe`) além dos 3 reais (`Cities: Skylines`, `Counter-Strike: Global Offensive`, `USB-Driver-Installer 1.0.3.0`), `O app pode ter mudado de lugar (ex.: Steam em outra pasta).` e `Antes de apagar, cada registro é exportado para um .reg em ...\backups\orphans-<n>`.

**C3. Recusa do auxiliar rodando de `target\release` (Task 197)**
```powershell
& $Wdev cleanup --orphans
# seletor de marcar: desça até "wingetrs test orphan HKLM", Espaço, Enter; confirme com s
reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan"
```
- [ ] Nenhum item vem marcado; rodapé `setas movem · Espaço marca · Enter confirma · Esc cancela`.
- [ ] **Nenhum UAC abre.** Aparece (uma vez) `Para usar funções de administrador, o wingetrs precisa estar numa pasta que só o administrador grava (ex.: C:\Program Files\wingetrs). Ele está em C:\Users\jonyd\Projetos\wingetrs\target\release.` A chave HKLM continua (`reg query` acha). Anote se o aviso `Aguardando autorização ...` chegou a aparecer antes da recusa e se ficou um `.reg` em `backups\orphans-<n>`.

**C4. Instalações de usuário e msix pelo CLI (sem UAC)**
```powershell
& $W install JAMSoftware.TreeSize.Free      # seletor: s
& $W install M2Team.NanaZip                 # seletor: s
& $W install SumatraPDF.SumatraPDF          # seletor: s
& $W install Kumospace.Kumospace            # seletor: s; conclua na janela do instalador
& $W list --color never | Select-String 'TreeSize|NanaZip|SumatraPDF|Kumospace'
```
- [ ] Cada confirmação mostra o plano em passos **sem** `Instalador:` (URL) nem `SHA-256:` (só no dry-run) e o seletor com Não marcado.
- [ ] TreeSize: janela de progresso do Inno (modo `/SILENT`); se o instalador pedir UAC sozinho (`elevatesSelf`), a linha diz `Aguardando autorização para executar como administrador` por até 2 min e depois `rodando o instalador (pode pedir autorização de administrador)`. Se esse UAC aparecer, aceite e anote (é do próprio instalador, não do wingetrs; fora da contagem do bloco D).
- [ ] NanaZip: nenhuma janela de instalador; termina instalado (`Get-AppxPackage *NanaZip*`).
- [ ] Sumatra: sem janela (`-s`); Kumospace: abre a janela (anote). Hash divergente no Kumospace ⇒ anote e pule.
- [ ] Os quatro aparecem no `list`. Remova agora (sem UAC): `& $W uninstall JAMSoftware.TreeSize.Free`, `M2Team.NanaZip`, `SumatraPDF.SumatraPDF`, `Kumospace.Kumospace` (cada um com o plano de remoção em passos e o seletor). Anote quais abriram janela de desinstalador.

**C5. Upgrade `install` e `uninstallPrevious`** (versões antigas pelo próprio winget, usuário)
```powershell
winget install --id SumatraPDF.SumatraPDF --version 3.6 --scope user --exact --silent
& $W upgrade SumatraPDF.SumatraPDF --dry-run
& $W upgrade SumatraPDF.SumatraPDF --yes
winget install --id 0-don.clippy --version 1.7.8 --exact --silent
& $W upgrade 0-don.clippy --dry-run
& $W upgrade 0-don.clippy --yes
& $W list --color never | Select-String 'Sumatra|clippy'
```
- [ ] Antes do `winget install`, o dry-run diz `✗ 'SumatraPDF.SumatraPDF' não está instalado (...)` (conferido em 2026-10-19); depois dele, `SumatraPDF 3.6.1 (exe, usuário)` com `-s -install -with-preview` e sem `⚠ Pede permissão`.
- [ ] clippy: dry-run `(nullsoft, usuário)` com `/S` e o passo `Remove a versão instalada antes (o desinstalador pode abrir janela ou pedir permissão de administrador)`; na execução remove a 1.7.8 antes (anote) e fica uma linha só, 1.7.9.
- [ ] Remova: `& $W uninstall SumatraPDF.SumatraPDF --yes` e `& $W uninstall 0-don.clippy --yes`.

## D. Bloco de UAC (tudo de `C:\Program Files\wingetrs`)

**D1. [UAC recusado] Instalador nativo de máquina pelo CLI**
```powershell
& $W install PuTTY.PuTTY
# seletor: s. Quando o UAC aparecer, clique Não.
$LASTEXITCODE
```
- [ ] Antes do UAC a linha mostra `Aguardando autorização para executar como administrador` com relógio. Anote se o UAC veio em primeiro plano ou piscou na barra de tarefas; o UAC nomeia o "Windows Installer" (passo `⚠ Pede permissão de administrador para o Windows Installer`).
- [ ] Depois do Não: `Permissão de administrador recusada.`; anote o código de saída. Nada instalado (`& $W list | Select-String PuTTY` vazio) e nenhuma pasta `run-*` sobrando em `%LOCALAPPDATA%\wingetrs\installers`.

**D2. [UAC ×4] Instaladores nativos de máquina, um UAC cada** — num Windows Terminal normal:
```powershell
& $W install PuTTY.PuTTY                       # s; [UAC] aceite
& $W install Rawns.Reddit-Wallpaper-Changer    # s; [UAC] aceite
& $W install Microsoft.DotNet.AspNetCore.8     # s; [UAC] aceite
& $W install PeterPawlowski.foobar2000         # s; [UAC] aceite
& $W list --color never | Select-String 'PuTTY|Reddit|ASP.NET|foobar'
```
- [ ] Cada um: `Aguardando autorização para executar como administrador` antes do UAC; o UAC de PuTTY e Rawns mostra o Windows Installer, o de AspNetCore e foobar o editor do arquivo (anote o nome que aparece).
- [ ] Modo padrão: PuTTY/Rawns/AspNetCore com janela de progresso sem perguntas (`/passive`); foobar sem janela (`/S`).
- [ ] Os quatro instalados; durante cada execução existe uma pasta `installers\run-<hex>` só com a cópia do instalador (confira com `Get-ChildItem "$env:LOCALAPPDATA\wingetrs\installers" -Filter run-*` num segundo terminal) e ela some no fim. Nenhum `.log` de instalador criado pelo wingetrs.

**D3. [UAC ×1] TUI: remoções HKLM e órfãos com o auxiliar**
```powershell
& $W
```
Aba Instalados, remova (`u`) nesta ordem: PuTTY, Reddit Wallpaper Changer, Microsoft ASP.NET Core Runtime 8.0, foobar2000.
- [ ] Na primeira remoção a barra mostra `Aguardando autorização para executar como administrador`; **aparece o único UAC do auxiliar: aceite** (anote se apareceu ou piscou). Depois, `permissão de administrador concedida`.
- [ ] As outras três remoções e os órfãos abaixo **não** abrem UAC novo. Cada remoção mostra o plano em passos no modal; anote os que abriram janela de desinstalador (esperado: nenhuma além do progresso do msiexec).
- [ ] Some do ARP (`& $W list` depois). Anote se algum desinstalador deixou chave.

Órfãos, na mesma TUI (aba Instalados, filtre por "wingetrs test"): em cada "wingetrs test orphan", "Remover registro órfão".
- [ ] HKCU removido no processo; HKLM pelo auxiliar, sem novo UAC; toast `Registro órfão de wingetrs test orphan ... removido (cópia em ...)` com o caminho do `.reg`.

Kindle, na mesma TUI: "Amazon Kindle", `u`, leia o modal e **cancele** (Esc).
- [ ] O modal diz qual instalação (2.9.1 do usuário / 2.8.3 da máquina) e mostra `Outra instalação de Amazon Kindle usa as mesmas pastas: copie antes de remover.`, `Pasta compartilhada: C:\Users\jonyd\AppData\Local\Amazon`, `Tamanho da cópia: ...`. Nada removido. (CLI: `& $W uninstall "Amazon Kindle" --dry-run` mostra o mesmo e `Comando: ...\Kindle\application\uninstall.exe /S`.)

Saia com `q`.
- [ ] Ao sair, nenhum `wingetrs.exe` elevado sobra (`Get-Process wingetrs -ErrorAction SilentlyContinue`).

> **Alternativa CLI para D3:** `& $W uninstall PuTTY.PuTTY` etc. — cada comando sobe o seu auxiliar e pede **um** UAC (o auxiliar vive só enquanto o comando vive). Órfãos: `& $W cleanup --orphans`, marque com Espaço só os dois "wingetrs test orphan", Enter, `s` — [UAC] para o HKLM.

**D4. Log, pastas de execução e staging**
```powershell
Get-Content $log | Select-Object -Skip $logStart | Select-String 'job\.(start|end)' | ForEach-Object { $_.Line.Substring(0, [Math]::Min(300, $_.Line.Length)) }
Get-ChildItem -Force "$env:LOCALAPPDATA\wingetrs\installers" -Filter 'run-*' -ErrorAction SilentlyContinue
Get-ChildItem -Force "C:\Program Files\wingetrs\staging" -ErrorAction SilentlyContinue
```
- [ ] Cada job tem `job.start` e `job.end`; os quatro instaladores de D2 e as remoções HKLM de D3 com `elevated=true`, os de usuário `elevated=false`; `agreements=` vazio para todos.
- [ ] Nenhuma pasta `run-*` sobrando; `staging` vazio ou inexistente.

**D5. Restaurar os órfãos com o `.reg` do wingetrs**
```powershell
$bk = Get-ChildItem "$env:LOCALAPPDATA\wingetrs\backups" -Recurse -Filter '*wingetrs-test-orphan*.reg' | Sort-Object LastWriteTime | Select-Object -Last 2
$bk.FullName
reg import ($bk | Where-Object Name -like 'hkcu-*').FullName
reg export "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" "$P\orf-hkcu-depois.reg" /y
Compare-Object (Get-Content "$P\orf-hkcu-antes.reg") (Get-Content "$P\orf-hkcu-depois.reg")
```
[ADM] (import em HKLM exige elevação):
```powershell
reg import "<caminho do hklm64-wingetrs-test-orphan.reg listado acima>"
reg export "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" "$P\orf-hklm-depois.reg" /reg:64 /y
Compare-Object (Get-Content "$P\orf-hklm-antes.reg") (Get-Content "$P\orf-hklm-depois.reg")
```
- [ ] Os dois `Compare-Object` vazios. Se o nome tiver sufixo (`-2.reg`, de C3), use o mais novo.

## E. Retratos "depois", comparação e limpeza

```powershell
& $W uninstall sharkdp.hyperfine --yes
reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /f
```
[ADM]:
```powershell
reg delete "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /f
```
Terminal normal:
```powershell
Snap depois
Compare-Object (Import-Csv "$P\scoop-antes.csv") (Import-Csv "$P\scoop-depois.csv") -Property FullName, Length, LastWriteTime
Compare-Object (Get-Content "$P\env-antes.reg") (Get-Content "$P\env-depois.reg")
Compare-Object (Import-Csv "$P\arp-antes.csv") (Import-Csv "$P\arp-depois.csv") -Property PSPath, DisplayVersion
Compare-Object (Import-Csv "$P\msix-antes.csv") (Import-Csv "$P\msix-depois.csv") -Property PackageFullName
Compare-Object (Import-Csv "$P\root-antes.csv") (Import-Csv "$P\root-depois.csv") -Property FullName
Compare-Object (Get-Content "$P\list-antes.txt") (Get-Content "$P\list-depois.txt")
Import-Csv "$P\run-depois.csv"; Import-Csv "$P\staging-depois.csv"
pwsh -File scripts/leftovers.ps1
```
- [ ] `hyperfine` removido (anote se pela ponte `winget uninstall` ou pelo ARP); as duas chaves de teste sumiram.
- [ ] `~/scoop`: nenhuma diferença. `HKCU\Environment`: no máximo `Path`/`PSModulePath` que já existiam; anote o que mudou.
- [ ] ARP, MSIX, raiz do wingetrs (`apps`, `persist` sem `yori`/`7zip`) e `list`: nenhuma diferença; qualquer outra = sobra, anote o pacote.
- [ ] `run-*` e `staging` vazios; `leftovers.ps1` sem processos novos.
- [ ] Mantenha `C:\Program Files\wingetrs\wingetrs.exe` se for usar o auxiliar depois; senão, [ADM] `Remove-Item -Recurse 'C:\Program Files\wingetrs'`. Feche o terminal de administrador.
- [ ] `Remove-Item -Recurse "$P"` só depois de colar as saídas nos relatórios. `C:\Windows\Installer\SourceHash{...}` do rust fica (permanente, aceito pela spec da 2b).
