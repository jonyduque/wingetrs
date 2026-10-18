# Validação da fase 2c (Task 163)

Data: 2026-10-18. Base: `995ffbb`. Nada de manifesto real foi executado; o catálogo foi aberto só para leitura.

## 1. Oráculo antes × depois

Comando: `cargo run --release --example oracle_2c -- %LOCALAPPDATA%\wingetrs\catalog.db` (o exemplo agora imprime também a seção "Instalável pela 2c"). Catálogo local: só os buckets `main` e `extras`, 4118 manifestos scoop, 1403 deles 2c (mesmo número do oráculo da Task 142, `docs/superpowers/reports/2026-10-18-oraculo-2c.md`).

**Antes:** a 2a não planejava nenhum manifesto 2c (psmodule, innosetup, `installer`/`uninstaller` e scripts eram `Unsupported`), por isso a coluna "instaláveis antes" é 0 por definição, não medida de novo com o código antigo.
**Depois:** `resolve(Arch::host())` + `plan_install` (instalação nova, sem launcher) sem erro, com o portão do planejador (`supported = PRELUDE_FUNCTIONS`). "Recusados pelo portão" = `PlanError::ScriptRefused`.

Resumo: **1318 de 1403 (93,9 %)** dos manifestos 2c planejam; 77 (5,5 %) são recusados pelo portão; 8 (0,6 %) falham por formato de download ou arquitetura (não são da 2c). Todo S-psm, S-inno, S-file e S-ps1 do catálogo planeja.

Saída real (colada sem edição):

### Instalável pela 2c (saída do `oracle_2c`)

Instalável = `resolve` (arquitetura do host) + `plan_install` sem erro, com o portão do planejador (`supported = PRELUDE_FUNCTIONS`, 44 funções). Nada foi executado. Antes (2a): nenhum manifesto 2c planejava (a 2a recusava psmodule, innosetup, installer e scripts), então a coluna "antes" é 0.

| bucket | 2c | instaláveis antes | instaláveis depois | recusados pelo portão | outro erro |
|---|---:|---:|---:|---:|---:|
| extras | 1109 | 0 | 1039 (93.7%) | 66 (6.0%) | 4 (0.4%) |
| main | 294 | 0 | 279 (94.9%) | 11 (3.7%) | 4 (1.4%) |
| todos | 1403 | 0 | 1318 (93.9%) | 77 (5.5%) | 8 (0.6%) |

| pior sub-caso (`worst`) | 2c | instaláveis antes | instaláveis depois | recusados pelo portão | outro erro |
|---|---:|---:|---:|---:|---:|
| S-psm | 10 | 0 | 10 (100.0%) | 0 (0.0%) | 0 (0.0%) |
| S-inno | 93 | 0 | 93 (100.0%) | 0 (0.0%) | 0 (0.0%) |
| S-file | 4 | 0 | 4 (100.0%) | 0 (0.0%) | 0 (0.0%) |
| S-ps1 | 3 | 0 | 3 (100.0%) | 0 (0.0%) | 0 (0.0%) |
| S-hook | 1293 | 0 | 1208 (93.4%) | 77 (6.0%) | 8 (0.6%) |

#### As 10 razões de recusa mais frequentes do portão

Cada razão conta uma vez por manifesto; um manifesto pode ter várias.

| # | razão | manifestos |
|---:|---|---:|
| 1 | mexe no registro da máquina (HKLM) | 47 |
| 2 | pede elevação (RunAs/sudo) | 27 |
| 3 | chama o comando scoop (instalaria no ~/scoop); não suportado | 4 |
| 4 | chama o msiexec; o wingetrs só usa msiexec /a pela fila própria | 3 |
| 5 | grava em Program Files/Windows | 1 |
| 6 | usa -Global (instalação para todos os usuários) | 1 |

| tipo de recusa (`FlagKind`) | manifestos |
|---|---:|
| Hklm | 47 |
| Elevation | 27 |
| ScoopCommand | 4 |
| Msiexec | 3 |
| GlobalSwitch | 1 |
| SystemWrite | 1 |

#### Outros erros do planejador (não são do portão)

| erro | manifestos |
|---|---:|
| resolve: não suportado: download .zst: formato não suportado | 3 |
| resolve: sem variante para a arquitetura desta máquina | 3 |
| resolve: não suportado: download .rar: formato não suportado | 2 |

#### Recusas só dentro de `if ($global) { ... }`

Recusados pelo portão com algum bloco `if ($global)`: 41. Planejam com o corpo desses blocos apagado: 38 (`main/7zip`, `main/ghostscript`, `main/git-with-openssh`, `main/git`, `main/pwsh`, `main/python`, `main/vim`, `main/vulkan`, `extras/alacritty`, `extras/astrogrep`). Desses, todo flag de recusa é `-Global` (`GlobalSwitch`): 0 (). Atribuem `$global` em algum script (o bloco poderia rodar): 0 ().

Notas de leitura:
- Uma linha de recusa do portão conta uma vez por manifesto. A soma das linhas passa de 77 porque um manifesto pode ter várias razões.
- O portão do planejador (`PRELUDE_FUNCTIONS`, 44 funções) recusa 77 manifestos, o mesmo total da tabela da Task 150 com as 262 funções do Scoop: nenhum manifesto do catálogo é recusado hoje só por função do Scoop que o prelúdio não tem.
- "Recusas só dentro de `if ($global)`": o oráculo apaga o corpo (entre as chaves) de cada `if ($global) { ... }` de todos os scripts do manifesto, mantendo as quebras de linha, e planeja de novo. O casamento de chaves pula aspas e comentários `#`, mas não here-strings (aproximação). Detalhes na seção 3.

## 2. Custo do host de scripts

`examples/bench_script_host.rs` grava um `runner.ps1` sintético e o roda uma vez pelo `PwshHost` (o `run` de produção: recusa elevado, lê a GPO no modo `locate`, `PATH` fixo, `-NoProfile -NonInteractive -ExecutionPolicy Bypass -File`). Modos: `empty` (runner vazio, só o BOM) e `prelude` (texto real de `runner_ps1` sem variáveis, dot-source do prelúdio real e um hook vazio). `locate` = `PwshHost::locate()` (pwsh 7.6.7 nesta máquina); o 5.1 vai pelo caminho absoluto (`-Args` do `bench.ps1` é separado por espaço, e o caminho do pwsh 7 tem espaço).

Comando: `pwsh -File scripts/bench.ps1 -Exe target/release/examples/bench_script_host.exe -Args '<programa> <pasta> <modo>' -Runs 15 -Baseline target/release/examples/noop.exe` (pasta no scratchpad, apagada depois). Só este agente rodava na máquina.

| host | runner | mediana do delta (15 execuções) |
|---|---|---:|
| pwsh 7 (`locate`) | vazio | **583,1 ms** |
| pwsh 7 (`locate`) | prelúdio + hook vazio | **717,3 ms** |
| powershell 5.1 | vazio | **487,1 ms** |
| powershell 5.1 | prelúdio + hook vazio | **548,4 ms** |

Saída real:

```
target/release/examples/bench_script_host.exe locate C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\t163-bench empty | runs=15 warmup=2 exit=0 | min=599,0 ms median=618,2 ms max=655,0 ms | baseline median=35,2 ms | delta median=583,1 ms
target/release/examples/bench_script_host.exe locate C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\t163-bench prelude | runs=15 warmup=2 exit=0 | min=731,1 ms median=757,8 ms max=896,0 ms | baseline median=34,6 ms | delta median=717,3 ms
target/release/examples/bench_script_host.exe C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\t163-bench empty | runs=15 warmup=2 exit=0 | min=500,3 ms median=519,2 ms max=542,6 ms | baseline median=33,8 ms | delta median=487,1 ms
target/release/examples/bench_script_host.exe C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\t163-bench prelude | runs=15 warmup=2 exit=0 | min=564,1 ms median=584,2 ms max=622,7 ms | baseline median=34,1 ms | delta median=548,4 ms
```

Leitura: cada hook custa ~0,5–0,6 s só para subir o PowerShell, e o prelúdio soma ~130 ms no pwsh 7 e ~60 ms no 5.1. Um manifesto com `pre_install` + `installer.script` + `post_install` paga isso três vezes (~2,2 s no pwsh 7). A spec (f) previa "um runner por fase, se valer a pena": não implementado e não medido o ganho; o custo absoluto é pequeno perto de download e extração, então não recomendo mudar agora. Não medido: o custo com variáveis de contrato e `effects.jsonl` cheios, e o tempo de hooks reais.

## 3. `-Global` e `if ($global)` (pedido do orquestrador)

Números (seção 1, saída do oráculo):
- Manifestos recusados **só** porque usam `-Global` dentro de `if ($global)`: **0**. Só um manifesto do catálogo tem flag `GlobalSwitch` (`main/python`), e ele também é recusado por `HKEY_LOCAL_MACHINE` dentro do mesmo tipo de bloco.
- Manifestos recusados só por construções dentro de `if ($global) { ... }` (qualquer regra: `-Global`, HKLM, ...): **38 dos 77 recusados** (41 recusados têm algum bloco desses; 3 seguem recusados sem ele). Os 10 primeiros: `main/7zip`, `main/ghostscript`, `main/git-with-openssh`, `main/git`, `main/pwsh`, `main/python`, `main/vim`, `main/vulkan`, `extras/alacritty`, `extras/astrogrep`. Padrão típico (`7zip`, `python`, `git`): `if ($global) { $content = $content -replace 'HKEY_CURRENT_USER', 'HKEY_LOCAL_MACHINE' }` ao preparar um `.reg`.
- Desses 38, nenhum atribui `$global` (`$global =`, `$script:global =`, `$local:global =`, `Set-Variable global`).
- Com os 38, a 2c planejaria 1356 de 1403 (96,6 %) em vez de 1318 (93,9 %).

O runner define `$global = $false` (o wingetrs só instala por usuário), então o corpo desses blocos nunca roda numa execução normal.

Opções (decisão do usuário; o portão não foi alterado):

| opção | prós | contras |
|---|---|---|
| A. Manter a recusa | nada muda; zero código novo | 38 apps populares (git, python, 7zip, pwsh, vim) continuam recusados por código que nunca roda; a mensagem ("mexe no registro da máquina") engana o usuário |
| B. O portão ignora o corpo de `if ($global) { ... }` quando nenhum trecho do manifesto reatribui `global` e não há código dinâmico | +38 manifestos; o script roda como o Scoop rodaria | a garantia depende de `$global` continuar `$false` em tempo de execução: atribuição escondida (`iex`, `New-Variable`, `Set-Variable -Name ("glo"+"bal")`, `${global}`, parâmetro de função, escopo pai) escapa de uma busca textual; o casamento de chaves precisa tratar here-strings, `elseif`, `if ($global -and ...)`; o `else` continua analisado |
| C. Como B, mas o wingetrs **apaga** o corpo desses blocos do texto que roda (mantendo as linhas), e a confirmação mostra o script original e marca o bloco como "não roda (só para instalação global)" | o que roda é exatamente o que foi analisado: mesmo que `$global` vire `$true` de algum jeito, o código recusado não existe mais | o wingetrs passa a alterar código de terceiros (o hash do que roda difere do manifesto); um erro no casamento de chaves pode apagar código errado (o pior caso é o hook quebrar, não fazer algo novo); mais um caminho a testar |
| D. Rebaixar para `ConfirmStrong` com aviso, sem apagar nada | o usuário decide caso a caso | a regra D8 = 1 (recusar HKLM/`-Global`) deixa de valer para esses blocos; a garantia passa a ser só a confirmação |

Recomendação: **C**, se o usuário quiser os 38 apps; senão **A**. B fica abaixo de C porque depende de provar a ausência de reatribuição por análise textual, que o próprio portão admite não ser garantia (spec f, "análise estática passa falsa segurança").

## 4. Emendas à spec (decisões R1–R9 do plano)

| # | Emenda | Decisão |
|---|---|---|
| R1 | `$ErrorActionPreference='Stop'` no runner (spec c) | não muda: o hook roda com `Continue` dentro de `try/catch` que sai com 1 em erro terminante (o Scoop não define `Stop`) |
| R2 | `Expand-MsiArchive` no prelúdio | usa o lessmsi (ramo `USE_LESSMSI` do Scoop), nunca `msiexec` |
| R3 | `installer.file` terminando em `.msi` | recusado com motivo (nem o Scoop executa) |
| R4 | auxiliares marcados `helper=1` | sem coluna nova; auxiliar = dependência oculta protegida pelo `dependents` |
| R5 | `spawn_win` "passa para `src/proc/`" | fica em `src/bridge/spawn_win.rs` e ganha `RunSpec`/`run_spec` |
| R6 | `scoop` como comando externo dentro de hook | recusado pelo portão com motivo claro (4 manifestos no catálogo; confirmado na seção 1) |
| R7 | lugar da recusa por "ainda não suportado" | sai de `manifest::resolve` e vai para `planner::plan_install` (`PlanError::Unsupported`) |
| R8 | consentimento | `Runners` no `ExecCtx` com corredores de terceiros `Option`; `None` ⇒ o executor recusa antes de qualquer passo |
| R9 | codificação | `prelude.ps1` só ASCII; `runner.ps1` gravado com BOM UTF-8 |

Também valem as emendas já registradas na spec: D1 = B (prelúdio próprio) e a emenda pós-sondas.

## 5. Riscos que ficaram

- **Análise estática não é garantia** (spec f): o portão recusa 77 de 1293 S-hook; os outros rodam com confirmação. A UI nunca diz "seguro".
- **`if ($global)`**: 38 recusas vêm de código que nunca roda (seção 3); decisão pendente.
- **Risco obsoleto na spec (f):** "substituições desatualizadas quando a lib muda" e "a lib escapa e grava no `~/scoop`" eram da opção A; com D1 = B o risco é "manifesto usa função que o prelúdio não tem", que hoje recusa 0 manifestos do catálogo (seção 1). Ver Concerns no relatório da tarefa.
- **Custo do host:** ~0,6–0,7 s por hook no pwsh 7 (seção 2); "um runner por fase" não implementado.
- **Catálogo incompleto:** só `main` e `extras`; nerd-fonts (367 manifestos que chamam `scoop`) e outros buckets não medidos.
- **Prova ao vivo pendente** (seção 7): nada da 2c rodou ainda num manifesto real pela CLI/TUI; GPO que bloqueia scripts não reproduzida (spec, emenda pós-sondas).
- **innounp GPL-3.0 e hash por branch:** inalterado (vem pelo manifesto; hash diferente ⇒ `HashDivergente`).

## 6. Proposta de texto D2-1 para o `CLAUDE.md` (não aplicada)

Para o orquestrador aplicar **só com confirmação do usuário**, como mais um subitem de "Escrever fora de `%LOCALAPPDATA%\wingetrs`" (ao lado das exceções da 2b e da 2d). Ajustado ao D1 = B (a spec ainda fala da lib do Scoop) e ao que foi implementado:

> - **Exceção da fase 2c (código de terceiros):** os scripts de manifesto (`pre_install`, `installer.script`, `post_install`, `pre_uninstall`, `uninstaller.script`, `post_uninstall`) e os instaladores de `installer.file`/`uninstaller.file` são código de terceiros que roda com os privilégios do usuário e pode gravar onde o usuário pode; o wingetrs aceita o que eles gravam. O **código do wingetrs** continua limitado aos lugares da 2a: as funções do Scoop que mexem em PATH, variáveis, shims, atalhos e `PSModulePath` vêm do prelúdio próprio (`install/prelude.ps1`, nada da lib do Scoop é baixado nem carregado), viram efeitos em `effects.jsonl` e são aplicadas e registradas pelo wingetrs. Script só roda depois do portão estático (`script_scan`) e da confirmação que mostra o script e a análise (`--allow-scripts` na CLI, nunca implícito em `--yes`), nunca com o processo elevado, nunca com política de grupo que bloqueia scripts, com PowerShell de caminho absoluto (KnownFolder) e `PATH` fixo. **Testes nunca** rodam script de manifesto real nem `PwshHost::locate` fora de `tests/it/script_host_live.rs` (`#[ignore = "live: ..."]`, verificado pelo `check_rules`): usam pwsh falso (`.cmd`) ou script sintético numa raiz temporária. Agentes nunca executam scripts de manifesto reais nem o innounp sobre instaladores reais sem pedido do usuário. Motivo: spec da 2c (D1 = B, D2 = 1, D8 = 1), decidida em 2026-10-18.

## 7. Prova ao vivo

**Não executada** (sem autorização do usuário nesta tarefa). Prova ao vivo pendente, para o usuário rodar (`wingetrs` = `target\release\wingetrs.exe`, PowerShell 7, sessão **não** elevada):

```powershell
# 0. Antes (uma vez)
cargo build --release
Get-ChildItem -Recurse -Force "$env:USERPROFILE\scoop" -ErrorAction SilentlyContinue | Select-Object FullName, Length, LastWriteTime | Export-Csv "$env:TEMP\wr163-scoop-antes.csv" -NoTypeInformation
reg export HKCU\Environment "$env:TEMP\wr163-env-antes.reg" /y
pwsh -File scripts/leftovers.ps1

# 1. S-psm (psmodule puro, sem scripts)
target\release\wingetrs.exe install main/pester --dry-run
target\release\wingetrs.exe install main/pester --yes

# 2. S-inno: xming (o menor da sonda D4; usa innounp sobre o instalador real)
target\release\wingetrs.exe install main/xming --dry-run
target\release\wingetrs.exe install main/xming --yes

# 3. S-hook com veredito Confirm. Candidato: main/yori (post_install = um Remove-Item em $dir).
#    Confira no --dry-run que o veredito é "Confirm" antes de instalar; se não for, troque o app.
target\release\wingetrs.exe install main/yori --dry-run
target\release\wingetrs.exe install main/yori --allow-scripts

# 4. Depois (e de novo após remover os três com `uninstall ... --allow-scripts`)
Get-ChildItem -Recurse -Force "$env:USERPROFILE\scoop" -ErrorAction SilentlyContinue | Select-Object FullName, Length, LastWriteTime | Export-Csv "$env:TEMP\wr163-scoop-depois.csv" -NoTypeInformation
Compare-Object (Import-Csv "$env:TEMP\wr163-scoop-antes.csv") (Import-Csv "$env:TEMP\wr163-scoop-depois.csv") -Property FullName, Length, LastWriteTime
reg export HKCU\Environment "$env:TEMP\wr163-env-depois.reg" /y
Compare-Object (Get-Content "$env:TEMP\wr163-env-antes.reg") (Get-Content "$env:TEMP\wr163-env-depois.reg")
pwsh -File scripts/leftovers.ps1
```

Esperado: nenhuma diferença no `~/scoop`; em `HKCU\Environment` só as entradas de PATH/variáveis que o plano mostrou (e nenhuma depois da remoção); `leftovers.ps1` sem processos novos. O veredito Confirm do `yori` **não foi medido** (escolhido pelo tamanho do script).
