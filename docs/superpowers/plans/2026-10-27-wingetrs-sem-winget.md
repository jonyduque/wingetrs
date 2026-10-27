# wingetrs — menos winget (Tasks 238–243) e emenda da 237

Decisões do usuário (2026-10-27), sobre o censo `examples/bridge_census.rs` (commit 38a17d3; relatório `.claude/tasks/census-report.md`; dados em `C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\census.tsv`, enquanto existir):

- **d1:** ordem aprovada (abaixo). `msstore` fica fora por ora.
- **d2:** Task 228 arquivada no BACKLOG (só servia à ponte).
- **d3:** `portable` de escopo máquina (6 pacotes) continua na ponte.
- **d4:** seletor do CLI ordena as opções pela letra, com Cancelar sempre por último.
- **d5:** na TUI, Esc/Enter tiram o foco da busca; sem foco, as letras são atalhos; `/` volta à busca.
- **d6:** Ctrl+Q sai sempre; `q` só sai com a busca vazia. (Ctrl+Esc não chega ao terminal no Windows: abre o menu Iniciar.)
- **d8:** Task 237 ganha "Substituir" (padrão) e "Adotar" (só quando a origem bate).

Censo (15 791 pacotes, versão mais nova): nativo 9 521 (60,3 %), portátil 3 219 (20,4 %), **ponte 2 998 (19,0 %)**, recusado 51, erro 2. Ponte por motivo: dependências 1 656 (1 608 só de pacote, 45 com recurso do Windows, 3 externas); parâmetros fora da lista de elevação 782 (440 = `/PreventRebootRequired=true` em NSIS da Mozilla; depois `/V/passive` 61, `""` 32, `unattended` 30, `"winget"` 15, `/q` 12, `--hidden` 10); zip com instalador dentro 554 (inno 168, exe 113, msi 95, nullsoft 92, wix 59, burn 17, msix 5, appx 5); portable máquina 6.

Correção do levantamento de 2026-10-26: "auxiliar indisponível cai na ponte" estava errado. O instalador nativo de máquina já usa o UAC próprio (`runas`, Task 196); o auxiliar só serve à ponte e às remoções HKLM. Não há tarefa para isso.

## Ordem

1. **238** (zip sob `catch_decoder`) ‖ **242** (seletor do CLI) ‖ **243** (foco da busca e Ctrl+Q na TUI): arquivos disjuntos, em paralelo.
2. **234** (KnownFolder para `reg.exe`/`msiexec.exe`, plano de 2026-10-22).
3. **239** (lista de elevação).
4. **237** com a emenda abaixo (plano de 2026-10-22).
5. **240** (dependências de pacote; precisa do `existing_install` da 237).
6. **230** (bytes processados nos extratores, plano de 2026-10-22).
7. **241** (zip com instalador dentro; precisa da 238).
8. **232** (barra de progresso na extração).
9. Depois: J, 226, 225, 231, 223, 224 (ordem do plano de 2026-10-22).

## Emenda da Task 237 (2026-10-27)

Caso real (prova da 210, 2026-10-27): o eprocia já estava instalado pelo `install.ps1` do próprio projeto (ARP `HKCU\…\Uninstall\eprocIA`, `AppData\Local\Programs\eprocIA`). `wingetrs install https://github.com/jonyduque/eprocia` rodou o NSIS sem aviso e criou uma segunda instalação (`HKLM\…\Uninstall\eprociaeprocia`, `C:\Program Files\eprocia\eprocia`). O `list` mostrou "+2 registros" e a remoção avisou da pasta compartilhada `AppData\Roaming\eprocia`.

Substitui o item 2 do contrato:

2. **Seletor**: "{nome} {versão} já está instalado ({quem})", nesta ordem de recomendação (o cursor começa na primeira que existir):
   - **Substituir pela do wingetrs** (padrão): roda o desinstalador registrado da instalação existente pelo caminho de remoção da 2d (com a cópia de pastas compartilhadas e o "Aguardando autorização…" quando pede UAC). Só depois de a chave sumir, instala pelo caminho do wingetrs. Se a remoção falhar, não instala e mostra o erro.
   - **Adotar**: só grava o registro do wingetrs apontando para a instalação existente (release: `install.json` com `mode = installer` e a chave ARP; winget: a tabela `owned`; Scoop do usuário: não oferecido). Não roda nada. Só aparece quando a origem bate: release ⇒ a chave existente tem a mesma colmeia (HKCU × HKLM) e o mesmo gerador de instalador que o wingetrs rodaria (NSIS ⇒ `UninstallString` termina em `uninstall.exe`/`Uninst*.exe` de NSIS; Inno ⇒ `unins000.exe`; MSI ⇒ `MsiExec.exe /X{…}` com o mesmo `ProductCode`); winget ⇒ a chave casa com o manifesto pela correlação do `list`. Em qualquer dúvida, a opção some.
   - **Atualizar a instalação existente** (quando há versão mais nova e o wingetrs sabe atualizar aquela origem).
   - **Instalar mesmo assim** (com "ficarão duas cópias" quando a origem é outra).
   - **Cancelar** (sempre a última).
   Com `--yes` ou sem terminal, o item 3 vale como está. `--adopt` e `--replace` (novos) são as formas explícitas de Adotar e Substituir.

Testes extras:
- `release_existing_arp_other_origin_offers_replace_not_adopt`: o caso do eprocia (HKCU por script, release NSIS de máquina) ⇒ o seletor tem Substituir, Instalar mesmo assim, Cancelar; nenhum Adotar.
- `release_existing_same_origin_adopt_writes_record_only`: Launcher falso não recebe nenhuma chamada; o `install.json` aponta para a chave existente.
- `replace_runs_uninstaller_then_installs`, `replace_uninstall_failure_installs_nothing`.

Mutações: Adotar oferecido com colmeia diferente ⇒ o primeiro teste PEGA; instalar depois de remoção falha ⇒ `replace_uninstall_failure_installs_nothing` PEGA.

## Bloco 238

### Task 238: Extração de zip sob `catch_decoder`

**Modelo:** sonnet (revisor sonnet). Origem: BACKLOG K (revisão da 235, 2026-10-23).
- `src/install/extract.rs`: `extract_zip` (perto da linha 511) roda fora de `catch_decoder` (linha 32). O tar (linha 575), o 7z, o CAB e o MSI já rodam dentro. Um pânico do decodificador `deflate`/`deflate64` derruba o processo em vez de virar erro de arquivo inválido. A Task 241 vai extrair muito mais zips.

**Files:** `src/install/extract.rs`, o teste de extração de zip existente (`grep -rn "extract_zip\|deflate64" tests/`).

**Contrato:**
- `extract_zip` roda dentro de `catch_decoder`, como o tar: pânico vira o mesmo erro tipado que o tar devolve no pânico (`grep -n "catch_decoder" -A8 src/install/extract.rs` mostra a forma). Nada de `unwrap`.
- O desfazer da extração parcial continua igual ao de um erro comum.
- Sem mudança para zip válido (os testes atuais passam sem edição).

#### Verificação (cole cada `[OK] mutação PEGA`)
- Teste: um zip cujo leitor entra em pânico. Use o ponto de injeção que o tar usa no teste dele, se houver. Se não houver, crie um `#[cfg(test)]` mínimo no caminho do zip, como o tar. Resultado: erro de arquivo inválido, processo vivo e pasta de destino limpa.
- Mutação: tirar o `catch_decoder` do `extract_zip` faz o teste falhar.

**Relatório:** `.claude/tasks/task-238-report.md`.

## Bloco 239

### Task 239: Lista de elevação: parâmetros medidos no censo

**Modelo:** opus (revisor opus: código que decide o que roda como administrador). Origem: censo de 2026-10-27, 782 pacotes na ponte por "parâmetros do instalador fora da lista permitida para elevação".
- A lista está em `src/bridge/worker.rs`: `exe_params_allowed` (perto da linha 397) e `exe_switch`. O planejador a aplica antes do UAC próprio do instalador (`src/winget_install/plan.rs`, `helper_accepts`, perto da linha 201; o primeiro token recusado sai na função perto da linha 259).
- NSIS hoje aceita só `/S`, `/norestart` e `/D=<pasta em Program Files>` no fim.

**Files:** `src/bridge/worker.rs`, os testes da lista (`grep -rn "exe_params_allowed" tests src`), `examples/bridge_census.rs` só para rodar de novo.

**Contrato:**
1. **NSIS:** aceitar `/PreventRebootRequired=true` (sem diferenciar maiúsculas, valor só `true` ou `false`). É uma opção do instalador da Mozilla, documentada em `https://firefox-source-docs.mozilla.org/browser/installer/windows/installer/FullConfig.html`; cite o link num comentário de uma linha. Cobre 440 pacotes.
2. **Cada outro token do censo** (`/V/passive`, `""`, `unattended`, `"winget"`, `/q`, `--hidden` e o resto da cauda que o censo mostrar com ≥ 3 pacotes): para cada um, o relatório traz uma linha "token | tipo de instalador | pacotes | o que faz (fonte) | aceito? por quê". Aceite só um token que (a) não aponta caminho, (b) não carrega arquivo de configuração/script/transform, (c) não muda a pasta de instalação para fora de `Program Files` e (d) tem significado conhecido e documentado. Tokens posicionais (`unattended`, `"winget"`, `install`) e strings vazias continuam recusados, salvo prova em contrário no relatório. Não aceite nada por "parece inofensivo".
3. Mesmas regras para a TUI e o CLI (é uma função só). Nenhuma mudança no protocolo do pipe.
4. Rode o censo de novo (`cargo run --release --example bridge_census -- %LOCALAPPDATA%\wingetrs\catalog.db <pasta nova no scratchpad>`) e cole no relatório o antes e o depois de "parâmetros … fora da lista".

#### Verificação (cole cada `[OK] mutação PEGA`)
- Testes de tabela: `/PreventRebootRequired=true` aceito para nullsoft e recusado para os outros tipos; `/PreventRebootRequired=C:\x` recusado; cada token novo aceito com um caso positivo e um caso vizinho recusado (ex.: o mesmo token com `=<caminho>`).
- Mutação: aceitar qualquer valor em `/PreventRebootRequired=` faz o teste do caminho falhar.
- Mutação: aceitar o token para todos os tipos faz o teste "recusado para inno" falhar.

**Relatório:** `.claude/tasks/task-239-report.md`.

## Bloco 240

### Task 240: Dependências de pacote pelo motor nativo

**Modelo:** opus (revisor opus). Origem: censo de 2026-10-27: 1 656 pacotes na ponte por "tem dependências", dos quais 1 608 só com `PackageDependencies`. Depende da 237 (`existing_install`).
- Hoje: `src/winget_install/select.rs` (`verdict`, perto da linha 362) manda para a ponte qualquer instalador com `Dependencies` não vazio.

**Files:** `src/winget_install/select.rs`, `src/winget_install/plan.rs` (um tipo de plano com a lista de dependências), `src/winget_install/deps.rs` (novo: resolução), `src/cli/winget_cmd.rs` e `src/cli/run_win.rs` (instalar em ordem), `src/tui/jobs.rs`/`app.rs` (mesma ordem e confirmação), `src/cli/output.rs` (passos), testes `tests/it/winget_deps.rs` (novo).

**Contrato:**
1. **Só `PackageDependencies` vira nativo.** Com `WindowsFeatures`, `WindowsLibraries` ou `ExternalDependencies` não vazios, o pacote continua na ponte (motivo de hoje). Exceção: `ExternalDependencies` não bloqueia; vira uma linha "Requer (externo): <texto>" no plano.
2. **Resolução** (`deps::resolve(catalog, manifest, host) -> Result<Vec<DepStep>, Refusal>`): para cada `{PackageIdentifier, MinimumVersion}`, procura no catálogo do winget; consulta `existing_install` (237); se já houver instalação com versão ≥ `MinimumVersion` (comparação de versão do winget já existente, `grep -rn "fn compare_versions\|Version::parse" src/sources/winget`), pula; senão, planeja a dependência pelo mesmo `plan_with_mode`. Recursivo, com conjunto de visitados (ciclo ⇒ recusa "dependências em ciclo: a → b → a") e profundidade máxima 8. A ordem é pós-ordem (folhas primeiro), sem repetição.
3. **Tudo ou ponte:** se alguma dependência cair na ponte ou for recusada, o pacote principal vai para a ponte com o motivo "dependência <id>: <motivo>". Nenhuma instalação parcial é planejada.
4. **Confirmação:** o plano lista as dependências antes do pacote ("Instala antes: Microsoft.VCRedist.2015+.x64 14.44 (exe, máquina)"). Uma confirmação para o conjunto. Cada dependência que pede UAC aparece com o "Aguardando autorização…" na sua vez. `--dry-run` mostra a árvore.
5. **Execução:** uma por vez, na ordem. Falha numa dependência ⇒ para, não instala o principal e diz o que ficou instalado. O JobLog registra cada passo. As dependências instaladas entram no registro de propriedade (`owned`) da 237.
6. **Upgrade:** `upgrade` de um pacote com dependências resolve de novo e instala só as que faltam ou estão abaixo do mínimo.

#### Verificação (cole cada `[OK] mutação PEGA`)
- `deps_resolve_skips_installed_at_min_version`, `deps_resolve_orders_leaves_first`, `deps_cycle_refused`, `deps_depth_limit`, `deps_bridge_child_sends_parent_to_bridge`, `deps_feature_stays_bridge`, `deps_external_is_note_only` (catálogo e índice falsos).
- `deps_failure_stops_before_main` (Launcher falso: código 1 na dependência ⇒ o principal nunca é lançado).
- Mutações: tirar o "pula se já instalado" ⇒ o primeiro PEGA; inverter a ordem ⇒ o segundo PEGA; seguir depois da falha ⇒ `deps_failure_stops_before_main` PEGA.
- Censo de novo: cole antes e depois de "tem dependências".

**Relatório:** `.claude/tasks/task-240-report.md`.

## Bloco 241

### Task 241: Zip com instalador dentro (`NestedInstallerType` não portátil)

**Modelo:** opus (revisor opus). Origem: censo de 2026-10-27: 554 pacotes. Por tipo de dentro: inno 168, exe 113, msi 95, nullsoft 92, wix 59, burn 17, msix 5, appx 5. Depende da 238.
- Hoje: `verdict` (`src/winget_install/select.rs`) manda `Zip` não portátil para a ponte.

**Files:** `src/winget_install/select.rs`, `src/winget_install/plan.rs`, `src/winget_install/fetch.rs` (o zip baixado e conferido), `src/winget_install/nested.rs` (novo: extrair e achar o instalador), o executor nativo existente (`grep -rn "fn run_native\|fn launch_installer" src/winget_install src/cli`), testes `tests/it/winget_nested.rs` (novo).

**Contrato:**
1. **Entra no motor nativo:** `Zip` com `NestedInstallerType` em {inno, exe, msi, wix, nullsoft, burn} e exatamente um `NestedInstallerFiles` com `RelativeFilePath`. `msix`/`appx` aninhados continuam na ponte. Os outros vereditos (dependências, lista de elevação) valem para o tipo de dentro: o plano é o de um instalador `NestedInstallerType` com os switches do manifesto.
2. **Caminho:** `RelativeFilePath` passa por `install::safepath` antes de virar `PathBuf`. Recusado ⇒ ponte com o motivo.
3. **Extração:** o zip é baixado e conferido pelo SHA-256 do manifesto (como hoje), extraído pelo extrator próprio (sob `catch_decoder`, 238) numa pasta nova dentro do cache do wingetrs, e o arquivo de dentro tem o SHA-256 calculado logo depois da extração. Antes de lançar, o SHA é calculado de novo e precisa bater (mesma proteção que o caminho nativo usa entre conferir e lançar; procure como ele faz e reaproveite). O lançamento usa o mesmo executor do instalador não aninhado (UAC próprio quando o escopo é máquina, "Aguardando autorização…" antes).
4. **Limpeza:** a pasta extraída é apagada depois do instalador terminar, em todos os caminhos (guard com `Drop`).
5. **Progresso:** fase "Extraindo <arquivo>" com spinner, fase e relógio (a barra vem na 232).

#### Verificação (cole cada `[OK] mutação PEGA`)
- `nested_inno_plans_native_with_inner_switches`, `nested_msix_stays_bridge`, `nested_path_escape_refused` (`..\x.exe`, absoluto, ADS), `nested_two_files_stays_bridge`.
- `nested_hash_changed_before_launch_refuses` (troca o arquivo extraído entre a extração e o lançamento ⇒ o Launcher falso não é chamado).
- `nested_dir_removed_after_failure`.
- Mutações: pular a segunda conferência do SHA ⇒ `nested_hash_changed_before_launch_refuses` PEGA; tirar o `safepath` ⇒ `nested_path_escape_refused` PEGA.
- Censo de novo: cole antes e depois de "tipo zip".

**Relatório:** `.claude/tasks/task-241-report.md`.

## Bloco 242

### Task 242: Seletor do CLI em ordem alfabética das letras, Cancelar por último

**Modelo:** sonnet (revisor sonnet). Pedido do usuário (2026-10-27, d4). Exemplo de hoje: `b) Backup das pastas e remover`, `r) Remover sem copiar`, `c) Cancelar`.
- O seletor é `src/cli/picker.rs` (`select_one`, perto da linha 401, com `Letters` e `default`).

**Files:** `src/cli/picker.rs`, testes do picker (`grep -rn "select_one" tests src`).

**Contrato:**
- Com letras, `select_one` mostra as opções ordenadas pela letra (sem diferenciar maiúsculas), com a opção de cancelar sempre por último. A opção de cancelar é a que o chamador já marca como tal; se não houver marca, crie uma (`PickerItem::cancel` ou um campo), nunca por comparação de texto.
- O cursor começa na opção que o chamador passou em `default` (a recomendada), onde quer que ela fique depois de ordenar. O índice devolvido continua sendo o da lista do chamador: nenhum chamador muda.
- Uma função só, dentro do picker; os chamadores não ordenam nada.
- Sem letras (`Letters::None` ou equivalente), a ordem fica como está.

#### Verificação (cole cada `[OK] mutação PEGA`)
- Teste: itens `b, r, c(cancelar)` com `default = 0` ⇒ ordem na tela `b, r, c`, com o cursor em `b`. Itens `r, a, c(cancelar)` ⇒ `a, r, c`, cursor no `r` se `default = 0`, e Enter devolve 0.
- Teste: a tecla da letra devolve o índice original.
- Mutação: devolver o índice ordenado em vez do original faz o segundo teste falhar.

**Relatório:** `.claude/tasks/task-242-report.md`.

## Bloco 243

### Task 243: TUI: foco da busca e Ctrl+Q

**Modelo:** sonnet (revisor sonnet). Pedido do usuário (2026-10-27, d5 e d6): com texto na busca, os atalhos de letra não funcionam (`src/tui/app.rs`, perto da linha 2406: `let empty = app.query.is_empty()`, e as letras só valem `if empty`; a linha 2469 manda qualquer `Char` para a busca).

**Files:** `src/tui/app.rs`, `src/tui/view.rs` (indicar o foco), `src/tui/help.rs` (textos), `tests/it/tui_*.rs` (testes de estado e snapshots).

**Contrato:**
1. Campo novo `search_focus: bool` no `App`. Digitar um caractere que não é atalho com a busca vazia começa a busca e liga o foco (como hoje).
2. **Com foco:** todo caractere vai para a busca. **Enter** tira o foco e mantém o filtro (não instala nem abre nada). **Esc** tira o foco e mantém o filtro. Setas e PageUp/PageDown continuam movendo a seleção.
3. **Sem foco:** as letras são atalhos mesmo com texto na busca. `/` liga o foco (com o cursor no fim do texto). Esc sem foco e com texto limpa a busca (o comportamento de Esc de hoje).
4. **Sair:** Ctrl+Q sai em qualquer estado (com o pedido de confirmação que o `request_quit` já faz). `q` sai só com a busca vazia e sem foco. Ctrl+C mantém o comportamento de hoje.
5. A linha da busca mostra o foco (cursor visível só com foco; cor só por token de `tui/theme.rs`). A ajuda (`?`) e a barra de atalhos citam `/`, Esc/Enter e Ctrl+Q.

#### Verificação (cole cada `[OK] mutação PEGA`)
- Testes de estado: digitar "fire" ⇒ foco ligado; Enter ⇒ foco desligado, filtro "fire" mantido; `i` ⇒ ação de instalar no item selecionado (não acrescenta "i" à busca); `/` ⇒ foco de volta; Esc com foco ⇒ foco desligado e texto mantido; Esc sem foco ⇒ texto limpo.
- Ctrl+Q com foco e texto ⇒ pedido de saída; `q` com texto ⇒ não sai.
- Snapshot 80×24 da linha de busca com e sem foco.
- Mutação: ignorar o foco (letras sempre na busca com texto) faz o teste do `i` falhar.

**Relatório:** `.claude/tasks/task-243-report.md`.
