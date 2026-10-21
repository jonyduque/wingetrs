# Prova ao vivo de 2026-10-21 (Tasks 131, 163 e 188)

Fonte: `%LOCALAPPDATA%\wingetrs-prova\run-20261019-094613\section-{0,A,B,C,C-2,D,E}.md` (logs consultados só por busca focada). A rodada começou em 2026-10-19 (seções 0, A, B e C em modo automático) e terminou em 2026-10-21 (C-2, D e E com o usuário). Os IDs abaixo são os passos do roteiro (ex.: D2a). "Auto" = verificação automática do roteiro; "usuário" = resposta s/n.

Limite da evidência: na seção E (retrato "depois", `compare.ps1`, `leftovers.ps1`) e nos passos C4b/C4c/C4e–C4h/C5* da C-2 o roteiro só marca "feito no modo automático", sem saída gravada. Nesses casos vale a evidência da rodada automática de 2026-10-19 (seção C), citada com a data.

## Task 131, item 2 (2b: extração própria + reserva `msiexec /a`)

| Item | Resultado | Passo e evidência |
|---|---|---|
| 2 apps 7z, instalar e remover | **não feito** (0 de 2 nesta prova) | B3b (`yori`, 5 partes `.cab` baixadas como `.7z`): código 1, "formato não suportado pelo extrator: conteúdo não reconhecido como 7z", desfeito. Não há segundo app 7z no roteiro. O yori instalou num terminal normal na prova da Task 204 (fora desta rodada). B6c: remoção do yori código 1 por consequência ("não está instalado"). |
| 2 apps MSI, um com `extract_dir` | feito (2 MSI) | B4b `7zip` (`7z2604-x64.msi`, "Extraindo … (MSI)", código 0) e B6d remoção código 0, `apps\7zip` removido. `rust` (A1: "só a subpasta Rust"; A3 código 0; A4a/A4d remoção código 0, `apps\rust` removido). Ressalva: o `rust` (com subpasta) passou pela reserva porque o extrator recusou o MSI (A3, usuário s), então "MSI com `extract_dir` pelo extrator próprio" não foi provado. |
| 1 `.tar.xz` | **não feito**: o roteiro não tem passo para `.tar.xz` | Nenhum app `.tar.xz` foi escolhido. |
| 1 `.nupkg` | feito | B1a (plano, 4 [OK]), B1b instalar código 0, B1c `Get-Module -ListAvailable Pester` lista 6.2.0 em `wingetrs\modules\Pester`, B6a remoção código 0, `apps\pester` removido. |
| 1 MSI pela reserva com `--allow-msiexec` | feito | A3: `install main/rust --allow-msiexec --yes` código 0, `apps\rust\current` existe; usuário: extrator recusou e passou para `msiexec /a` (s), Ctrl+C avisou uma vez que o passo não se interrompe (s), linha continuou viva (s). Duração no log: 635 s (18:09Z a 18:19Z). |
| Nada sobra na raiz | feito em parte | B6a/B6b/B6d/B6h e A4a/A4d: `apps\<app>` removido [OK]. B6g: `list` sem pester/xming/yori [OK]. A2 (Ctrl+C durante o download): [FALHOU] "apps\rust não existe" (a pasta ficou) e o usuário respondeu "Não sei" para a pasta sobrando: ver "Falhas não triadas". O `compare.ps1` antes x depois (E6) rodou sem saída gravada: **não conferido**. |
| Ponte e reserva não rodam juntas | feito | A4b: `install sharkdp.hyperfine --yes` (ponte winget) esperou em "Aguardando o Windows Installer (outro instalador em uso)" com relógio enquanto a janela 1 rodava o `rust` com `--allow-msiexec`, depois rodou o winget (usuário sss); [OK] saída sem "1618"; código 0. Ressalva: a janela 1 saiu com código 1 (ver "Falhas não triadas"). A4c: `hyperfine 1.20.0` num terminal novo. |

Item 1 (NOTICE) está no commit `docs: NOTICE with third-party attributions`.

## Task 163, item 5 (2c, prova ao vivo com autorização)

Autorização: dada pelo usuário para a rodada. Itens pedidos: um S-psm, o `xming` (S-inno) e um S-hook de veredito `Confirm`; antes/depois de `scoop`, `HKCU\Environment` e `leftovers.ps1`.

| Item | Resultado | Passo e evidência |
|---|---|---|
| S-psm | feito | `pester` (módulo PowerShell): B1a–B1c, B6a (ver acima). |
| `xming` (S-inno) | feito | B2a plano (4 [OK], "Inno Setup, com o innounp", md5 e580debb), B2b código 0 e `shims\XLaunch.exe` existe, B2c `innounp` e `xming` no `list`, B6b remoção código 0, B6h remoção do `innounp` código 0. |
| S-hook com veredito `Confirm` | feito | `7zip` (`post_install` com `if ($global)`, `uninstaller.script` que importa `.reg`): B4a plano ("Análise: pede confirmação"; "pede atenção redobrada" para o desinstalador), B4b código 0 (`install-context.reg` e `uninstall-context.reg` em `apps\7zip\current`, sem `{{7zip_dir}}`, e `HKCU\Software\Classes\*\shellex\ContextMenuHandlers\7-Zip` continua ausente), B6d remoção com `--allow-scripts --purge` código 0. B6e: `HKCU\Software\7-Zip` apagado pelo script (esperado, código 1 do `reg query`), B6f restaurado do `.reg` com `Compare-Object` vazio. O `yori` (S-hook `post_install`): B3a plano [OK] ×8, B3b código 1 pela falha do extrator 7z (acima), não pelo script; em terminal admin recusa no passo 16 (ver "Falhas triadas"). |
| Recusas do portão | feito (extra) | B5b `python` (usa `-Global`), B5c `vulkan`, B5d `pwsh`, B5e `lazarus`: código 1 e o motivo esperado [OK]. B5a `git` plano [OK]. |
| `list` do scoop, diff de `HKCU\Environment` e `leftovers.ps1` antes e depois | **não conferido** | Retratos "antes" (0.1b, 13 linhas [ok], `leftovers` listou 24 processos antigos de bash/chrome/node de 18/10, nenhum do wingetrs, código 1) existem. O retrato "depois", `compare.ps1` e `leftovers.ps1` finais (E5–E7) rodaram em modo automático sem saída gravada. |

## Task 188, itens 1 a 4 (2d)

| Item | Resultado | Passo e evidência |
|---|---|---|
| 1. UAC aparece ou só pisca | feito | D3: "o UAC do auxiliar apareceu" (anotação do usuário). Windows Terminal x conhost: **não distinguido** (D1: anotação em branco). |
| 1. Recusar dá `Permissão de administrador recusada.` | feito | D1 `install PuTTY.PuTTY`: código 1; usuário viu "Aguardando autorização…" (s), "Pede permissão de administrador para o Windows Installer" (s) e "Permissão de administrador recusada." (s); log: `job.end … outcome=falhou … detail=Permissão de administrador recusada.`; D1 "nenhuma pasta installers\run-* sobrando" [OK]. A checagem "list sem PuTTY" falhou por falso positivo (ver triadas). |
| 1. Nenhum novo UAC na sessão após recusar | **não feito**: só houve uma tentativa recusada | D1 foi um único pedido. |
| 1. `is_elevated` falso na TUI e verdadeiro no auxiliar (log) | **não confirmado** | O log (D4) mostra `elevated=false` em todos os `job.end`, inclusive nas remoções HKLM da D3. Não há linha com `elevated=true`. Ver "Falhas não triadas". |
| 1. MSI de máquina sem elevação | feito | C1b (`msiexec /i putty.msi /quiet`, sem admin): código **1603** (rodada de 2026-10-19). Confirma a hipótese "MSI de máquina exige elevação". |
| 2. Um de cada tipo pelo motor nativo (instalar e remover) | feito em parte | msi/wix: PuTTY D2a (código 0, UAC "windows installer", `/passive` s) e remoção D3. msi: Reddit Wallpaper Changer D2b. burn: ASP.NET Core 8 D2c (UAC "Microsoft ASP.NET"). inno: TreeSize C4a (janela do Inno s) e remoção C4f. nullsoft: clippy C5e/C5g/C5j. msix: NanaZip C4b/C4g (rodada de 2026-10-19: instalar e remover código 0, `Get-AppxPackage` confere). exe com silencioso: SumatraPDF C4c/C4h (2026-10-19, código 0). exe interativo: Kumospace C4d (janela concluída pelo usuário) e C4i. foobar2000 (D2d): **não feito**, download falhou (ver triadas). |
| 2. Um pela ponte (zip ou portable) | feito a instalação; remoção **não feita** | hyperfine (zip) via winget: A4b código 0 e A4c `hyperfine 1.20.0`. Remoção em E1 recusada (0x8A15007D, ver triadas). |
| 2. Um `upgrade` `install` | **não feito** | Sumatra 3.6 não instalou pelo winget (C5b, `0x8000ffff Catastrophic failure`, 2026-10-19), então C5c/C5d/C5i deram "não está instalado" (código 1). Na C-2 esses passos rodaram em modo automático sem saída gravada. |
| 2. Um `uninstallPrevious` | feito | clippy: C5e instalou 1.7.8; C5f (dry-run) mostrou "Remove a versão instalada antes"; C5g `upgrade 0-don.clippy --yes` código 0, "Atualizado: Clippy 1.7.9"; C5h uma linha só do clippy [OK]; C5j remoção código 0. |
| 2. Um UAC por sessão | feito | D3: primeira remoção mostrou "Aguardando autorização…" e depois "permissão concedida" (s); as outras três remoções e os órfãos não abriram UAC novo (s). |
| 2. `staging` vazio no fim | feito | D4: `C:\Program Files\wingetrs\staging` vazio [OK]; nenhuma `installers\run-*` sobrando [OK]. D2a–D2c: `run-*` só com a cópia do instalador, sem `.log`, e removida no fim [OK]. |
| 2. `job.start`/`job.end` com `elevated=` e `agreements=` | feito em parte | D4: linhas `job.start`/`job.end` presentes com `agreements=-` nos instaladores; `elevated=false` em todas (ver acima). Resposta do usuário à pergunta composta: "-" (não respondida). |
| 3. Órfãos HKCU e HKLM, remoção e `reg import` idêntico | feito | C2a/C2b criaram as chaves (HKLM com `/reg:64`); C2c dry-run listou os dois (e Cities: Skylines, CS:GO, USB-Driver-Installer); D3 removeu o HKCU no processo e o HKLM pelo auxiliar com toast "Registro órfão … removido (cópia em …)" (s); D5a import HKCU, `Compare-Object` com 0 diferenças; D5b import HKLM no terminal admin, "Compare-Object orf-hklm-antes/depois vazio" [OK]. E2/E3: chaves de teste apagadas. |
| 3. Recusa do auxiliar rodando de `target\release` (C3) | **falhou nas respostas, não triado** | C3: usuário respondeu **n** a "Apareceu 'Para usar funções de administrador, o wingetrs precisa estar numa pasta que só o administrador grava'" e **n** a "Nenhum UAC abriu"; a anotação "Aguardando autorização…" apareceu antes da recusa: n. Chave HKLM continuou [OK] e o `.reg` do HKCU foi para `backups\orphans-…`. Ver "Falhas não triadas". |
| 4. Kindle: modal (e dry-run) | feito em parte | D3: resposta **n** à pergunta composta (modal mostra "Outra instalação de Amazon Kindle usa as mesmas pastas", "Pasta compartilhada", "Tamanho da cópia" e nada removido); o usuário removeu a 2.9.1 (`job.end kind=remove target=Amazon Kindle … removido (pastas copiadas para …\backups\…)`, 14:21:47Z a 14:22:04Z). Dry-run do Kindle não tinha passo no roteiro. |
| 5. Ledger e `docs/ARMADILHAS.md` | fora desta entrega | É do orquestrador. |

## Falhas triadas

- **D1 e D3 "list sem PuTTY…"**: falso positivo. O `-match 'PuTTY'` sem caixa casava "InputTyper" (app do usuário). Corrigido em 28a40ec.
- **D2d foobar2000**: download falhou "sem conexão" em foobar2000.org (externo), código 1 (`job.end … outcome=falhou … detail=sem conexão: error sending request for url`).
- **D4 `job.start` ≠ `job.end`**: o único início sem fim é `main/rust` de 2026-10-19T15:08:57Z (processo morto pelo usuário). Corrigido em 28a40ec para contar só a seção.
- **E1 hyperfine não removido no terminal admin**: o winget recusa pacote de escopo usuário elevado (0x8A15007D). Vira Task 218 (recusa no plano). Mesmo erro na remoção do kondo-ui.
- **yori com `--allow-scripts` num terminal admin**: recusa só no passo 16 (`post_install`), depois de extrair; desfeito. Num terminal normal funcionou (prova da Task 204). Vira Task 218.
- **Kindle**: D3 removeu a 2.9.1 (usuário). A 2.8.3 de máquina já existia e ficou (o usuário a removeu depois).
- **`install.ps1`**: testado pelo usuário no terminal normal: ok.

## Falhas não triadas (observadas, sem diagnóstico)

- **A2**: após Ctrl+C no download do `rust`, a pasta `apps\rust` continuou existindo ([FALHOU] "apps\rust não existe"). O log mostra `job.end … outcome=cancelado … detail=Ctrl+C` três vezes. Sobrou pasta sem versão? "Não sei".
- **A3b**: `rustc --version` num terminal novo respondeu 1.99.0 de `C:\Users\jonyd\.cargo\bin` (o rustup do usuário vem antes do shim do wingetrs no PATH); o roteiro esperava 1.99.1. Provavelmente ordem do PATH, não defeito do wingetrs (não verificado).
- **A4b**: a janela 1 (`install main/rust --allow-msiexec --yes`) saiu com código 1. No log do dia há um `job.end … outcome=falhou … detail=outra instalação do wingetrs está em andamento; tente de novo em instantes` (18:10:53Z, 30 s) e uma instalação do `rust` concluída com êxito (18:09:06Z a 18:19:41Z). Não está confirmado qual processo foi a janela 1.
- **B3b / B4a**: o `yori` falhou no formato do 7z (acima). Em B4a a checagem "saída contém: uninstaller.script (na remoção)" falhou, mas a saída tem "Script de terceiro (uninstaller.script, na remoção)": parece texto da checagem diferente do da saída (não verificado no roteiro).
- **C3**: respostas n para o aviso da pasta e para "nenhum UAC abriu" (ver tabela da Task 188).
- **D4**: `elevated=false` em todos os `job.end`, mesmo nos trabalhos que rodaram com o auxiliar ou com UAC do instalador. Se o campo deveria valer true nesses casos, é defeito de log; se mede só o processo principal, a pergunta do roteiro estava mal formulada.
- **C5b**: `winget install SumatraPDF --version 3.6` falhou com `0x8000ffff` (erro do winget, 2026-10-19).
- **0.1e**: `leftovers.ps1` retornou 1 com processos de agentes antigos (bash, conhost, node, chrome de 18/10), nenhum do wingetrs.

## Fechamento

| Task | Status | O que falta |
|---|---|---|
| 131 (2b) | DONE_WITH_CONCERNS | Segundo app 7z (o yori falhou no extrator; ver a prova da 204) e `.tar.xz` sem prova. MSI com `extract_dir` pelo extrator próprio não provado (o `rust` foi pela reserva). Conferir `compare.ps1` (E6) para "nada sobra na raiz" e explicar a pasta `apps\rust` após Ctrl+C no download (A2). |
| 163 (2c) | DONE_WITH_CONCERNS | S-psm (pester), `xming` e S-hook (7zip) provados. O yori precisa de nova rodada num terminal normal registrada neste formato (ou citar a prova da 204). Diff antes/depois de scoop, `HKCU\Environment` e `leftovers` sem saída gravada (E5–E7). |
| 188 (2d) | DONE_WITH_CONCERNS | Falta: upgrade `install` (Sumatra não instalou), remoção pela ponte (Task 218), segunda tentativa de recusa de UAC, `elevated=true` no log, recusa do `wingetrs.exe` fora de pasta de administrador (C3 com respostas n) e foobar2000 (rede). Windows Terminal x conhost não distinguido. |
