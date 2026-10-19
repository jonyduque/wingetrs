# Prova ao vivo — scripts guiados

Automatizam o que dá para automatizar no roteiro `docs/superpowers/reports/2026-10-18-prova-ao-vivo-roteiro.md` (seções 0, A–E). **Você roda; nenhum agente roda as seções** (elas instalam e removem programas de verdade).

PowerShell 7, **não elevado**, no Windows Terminal, na raiz do repositório.

## Como rodar

```powershell
pwsh -File scripts/live/run-all.ps1                 # tudo, numa pasta nova
pwsh -File scripts/live/run-all.ps1 -From C         # retoma na seção C (última execução)
pwsh -File scripts/live/run-all.ps1 -From D -RunDir "$env:LOCALAPPDATA\wingetrs-prova\run-20261019-101500"
pwsh -File scripts/live/section-b.ps1 -StartAt B4a  # uma seção, a partir de um passo
pwsh -File scripts/live/section-c.ps1 -DryRun       # só mostra os passos (nada roda, nada é gravado)
```

Cada execução tem uma pasta `%LOCALAPPDATA%\wingetrs-prova\run-<data-hora>` (o `$P` do roteiro): retratos `antes`/`depois`, `hkcu-7zip-antes.reg`, `orf-*.reg`, `putty.msi`, `state.json` (início do log do wingetrs, resultado do C1), e por seção `section-X.log` (saída completa) e `section-X.md` (resultado). Sem `-RunDir`, as seções A–E usam a execução mais recente; a seção 0 sempre cria uma nova.

Parâmetros comuns: `-W` (padrão `C:\Program Files\wingetrs\wingetrs.exe`, a cópia do passo 0.2, usada em todas as seções como no roteiro), `-Wdev` (padrão `target\release\wingetrs.exe`, usado só nos retratos e no C3), `-OutDir`, `-StartAt`, `-DryRun`. Parâmetros são valores únicos: nenhum precisa de lista (ver `docs/ARMADILHAS.md` sobre `pwsh -File` e arrays).

## Em cada passo

1. Título e o comando exato.
2. `Enter` executa · `p` pula · `q` encerra a seção (o `.md` é gravado mesmo assim).
3. Passos com **[UAC]** mostram um aviso em vermelho antes (aceitar, recusar ou "não deve abrir").
4. Passos que removem ou alteram algo (desinstalar, `reg import`, `reg delete`) pedem um `s` a mais.
5. Depois de rodar: verificações automáticas (código de saída, textos esperados do roteiro, pasta/arquivo existe ou sumiu, chave de registro, atalho, `wingetrs list`, pastas `installers\run-*` durante a execução) e as perguntas visuais (`s`/`n`) e anotações livres.

Modos de execução:

| Modo | Quando | Saída |
|---|---|---|
| captura | `--dry-run`, `list`, `reg` | stdout e stderr no log, textos conferidos |
| stdout | `--yes` | stdout no log; a linha de progresso (stderr) fica viva na tela |
| anexado | seletor `Continuar?`, janela de instalador, UAC | direto no terminal, não capturada: os textos esperados viram perguntas |
| janela nova | A2, A3 (Ctrl+C) e o terminal 1 do A4 | numa janela própria, para o Ctrl+C não parar o roteiro; feche-a (`exit`) para voltar |
| [ADM] | 0.2, C2 (HKLM), D5 (HKLM), E3, E8 | o roteiro imprime o bloco para o seu terminal de administrador; `Enter` = já rodei, `s` = abre um PowerShell elevado só para o bloco (um UAC a mais) |
| manual | TUI (0.3, D3) | instruções; você abre a TUI em outro terminal e responde |

Nada roda elevado sozinho.

## Scripts

| Script | O que faz |
|---|---|
| `snapshot.ps1 -Name antes\|depois [-Out <pasta>] [-Exe <wingetrs>]` | Retrato só de leitura: `~/scoop` (caminho, tamanho, data), `HKCU\Environment` (`reg export`), PATH do usuário e da máquina, ARP (HKCU e HKLM, 64 e 32 bits, por `Get-ItemProperty`), MSIX, atalhos `wingetrs Apps`, `wingetrs list`, raiz do wingetrs (`apps`, `shims`, `persist`, `modules`), `installers\run-*`, `Program Files\wingetrs\staging`, `HKCU\Software\7-Zip` e `leftovers.ps1`. Grava só na pasta de saída (o próprio `wingetrs list` escreve no log dele). |
| `compare.ps1 -Before <pasta> -After <pasta>` | Diferença por área (`+` novo, `-` sumiu, `~` mudou) e resumo. Saída 2 se `~/scoop` mudou, 1 se falta pasta, 0 caso contrário. |
| `section-0.ps1` | `cargo build --release`, retrato "antes", backup do 7-Zip, início do log, `leftovers`, cópia para Program Files [ADM] com conferência de dono/ACL/hash, dry-runs do `installer_mode` e o ajuste na TUI. |
| `section-a.ps1` | Task 131: plano do rust, seletor + Ctrl+C no download, Ctrl+C no `msiexec /a`, `rustc`, fila do `msi.lock` com o hyperfine em paralelo. |
| `section-b.ps1` | 2c: pester, xming, yori, 7zip (com `--allow-scripts`), recusas (python, vulkan, pwsh, lazarus), remoções e restauração do `HKCU\Software\7-Zip`. |
| `section-c.ps1` | 2d sem UAC: MSI de máquina sem elevação, chaves órfãs de teste, recusa do auxiliar em `target\release`, TreeSize/NanaZip/Sumatra/Kumospace, upgrades do Sumatra e do clippy. |
| `section-d.ps1` | Bloco de UAC: PuTTY recusado, quatro instaladores de máquina (vigia `run-*`), TUI com o auxiliar, log `job.start`/`job.end`, restauração dos órfãos. |
| `section-e.ps1` | Limpeza (hyperfine, chaves de teste, 7-Zip se ainda diferente), retrato "depois", `compare.ps1`, `leftovers`, remoção opcional de `C:\Program Files\wingetrs`. |
| `run-all.ps1 [-From 0\|A\|B\|C\|D\|E] [-RunDir <pasta>]` | Roda as seções em ordem numa pasta só e retoma de onde parou. |
| `_common.ps1` | Motor dos passos (usado pelas seções). |

## O que continua manual

- Tudo que é visual: UAC (aparece, piscou, nome do editor), janelas de instalador, linha de progresso viva, seletor na linha, modais e toasts da TUI — o roteiro pergunta e grava a resposta.
- A TUI (0.3, D3): você a abre em outro terminal e segue as instruções.
- Apertar Ctrl+C na hora certa (A2, A3) e escolher no seletor (`s`, Espaço, Enter).
- Os blocos [ADM] no terminal de administrador (ou `s` para o roteiro abrir um elevado).
- Colar os `section-*.md` nos relatórios das Tasks 131, 163 e 188 e apagar a pasta da execução no fim.
