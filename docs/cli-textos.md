# Inventário dos textos do CLI

## Como usar

1. Cada linha das tabelas abaixo é um texto que o CLI imprime, pergunta ou mostra como progresso. O ID é estável: cite-o ao pedir mudança.
2. Preencha só a última coluna, "Formatação desejada". Texto livre ou exemplo, por exemplo `✓ verde negrito`, `recuo 2, cinza`, `trocar por: "Pronto."`, `remover`.
3. "Texto atual" é o literal do código com `{placeholders}`; texto montado em pedaços aparece já montado. `\|` dentro de célula é uma barra vertical literal.
4. "Estilo atual" usa as siglas da legenda abaixo; "sem estilo" = cor padrão do terminal. Todo glifo vem de `tui::theme::Glyphs` e muda com `--glyphs nerd|unicode|ascii` (fora de terminal é sempre unicode).
5. Linhas citam `arquivo:linha` do commit `31e4539` (HEAD). Arquivos com mudança não commitada (WIP das Tasks 189/208) foram lidos pelo HEAD; o que existe só no WIP está na última seção (WIP-xx). Em `install_cmd.rs` as linhas depois de ~775 deslocam +12 no working tree.

## Legenda

### Estilos (`src/cli/style.rs:12-27`, cores ANSI básicas que seguem o tema do terminal)

| Sigla | Token | Aparência |
|---|---|---|
| HDR | `HEADER` | magenta negrito (cabeçalhos de tabela, rótulos `Nome:`, verbos `Instalar`) |
| ID | `ID` | ciano (ids, `bucket/id`, sugestões) |
| NOME | `NAME` | negrito (nome de app, contagens, números dos passos) |
| VER | `VERSION` | sem estilo |
| NOVA | `NEW_VERSION` | verde negrito (versão nova / disponível) |
| FONTE | `SOURCE` | azul (`winget`, `scoop:main`) |
| OK | `OK` | verde |
| AVISO | `WARN` | amarelo |
| ERRO | `ERROR` | vermelho negrito |
| DIM | `DIM` | cinza (bright black) |
| CAMINHO | `PATH` | ciano itálico (pastas e arquivos) |
| TECLA | `KEY` | ciano negrito (comandos e flags na ajuda) |

### Glifos usados no CLI (`src/tui/theme.rs:57-148`)

| Papel | Campo | unicode (padrão) | nerd | ascii | Onde |
|---|---|---|---|---|---|
| sucesso | `toast_ok` | ✓ | f00c | + | `output::success`, `report_line`, `up_to_date`, `folders_copied` |
| aviso | `toast_warn` | ⚠ | f071 | ! | `output::warning`, `cancelled`, passos que pedem UAC, bloco de avisos |
| erro | `toast_err` | ✗ | f057 | x | `error_report`, `failed_item`, `report_line` (falha) |
| seta | `arrow` | → | f061 | -> | `output::action` (próximo passo), `PATH do usuário →`, `de → para`, tabela de upgrades |
| cursor do seletor | `selected` | ▶ | ▶ | > | seletor de setas |
| spinner | `spinner` | ⣾⣽⣻⢿⡿⣟⣯⣷ | idem | \| / - \ | linha de progresso |
| caixa marcada | literal | `[x]` / `[ ]` | idem | idem | seletor de vários |

Outros literais fixos: `·` separa partes da linha de progresso e do cabeçalho do plano; `—` separa licença e URL; `#`/`-` desenham a barra de download (20 posições); `…` termina frases de fase fora de terminal e cortes de coluna.

### Moldes recorrentes (viram texto nas tabelas)

- `sucesso(rótulo, resto)` = `✓ {rótulo} {resto}` (glifo e rótulo OK, resto livre) — `output.rs:52`.
- `aviso(texto)` = `⚠ aviso: {texto}` (AVISO) — `output.rs:62`. Sai em stderr quando chamado por `emit::err_with`.
- `item com falha` = `✗ Erro: {texto}` (ERRO) — `output.rs:73`.
- `ação(texto)` = `→ {texto}` (seta DIM, texto livre) — `output.rs:92`.
- `cancelado` = `⚠ Cancelado.` (AVISO) — `output.rs:84`.
- `erro de comando` (stderr) = `✗ {o que falhou}` (✗ ERRO) + `→ {o que fazer}` + causas — `output.rs:161`.
- `plano numerado` = título + linhas `  {n}. {texto}` com n em NOME alinhado à direita — `output.rs:846`.

### Saídas e canais

stdout: resultado do comando (tabelas, planos, `✓ Instalado:`). stderr: linha de progresso animada (`Phases`), avisos de `err_with`, erro final. Fora de terminal o progresso vira uma linha simples por fase (PRG-30). Ajuda do clap: ver AJU-01.

---

## 1. update

Código: `src/cli/mod.rs:711-728` (comando), `src/cli/output.rs:248` (`report_line`). O comando **não tem linha de progresso** (nada aparece até o fim; `--background` não imprime nada).

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| UPD-01 | output.rs:253 | fonte já estava em dia | `✓ {fonte}: em dia` | ✓ OK; fonte FONTE; `em dia` OK; sem recuo | |
| UPD-02 | output.rs:254 | índice igual ao anterior | `✓ {fonte}: sem mudanças` | ✓ OK; fonte FONTE; resto sem estilo | |
| UPD-03 | output.rs:255-262 | catálogo atualizado | `✓ {fonte}: {n} pacotes ({m} ignorados)` | ✓ OK; fonte FONTE; n e m NOME; `pacotes` OK; `(… ignorados)` sem estilo | |
| UPD-04 | output.rs:263-266 | fonte falhou (rede, HTTP, integridade) | `✗ {fonte}: falhou: {mensagem}` | ✗ ERRO; fonte FONTE; `falhou:` ERRO; mensagem sem estilo | |
| UPD-05 | mod.rs:718 | `--background` | (nenhuma saída) | — | |
| UPD-06 | update.rs:68 | outro `update` rodando (exit 1) | `outra atualização em andamento` | erro de comando (ERR-01) | |
| UPD-07 | update.rs:70 | falha interna | `falha interna: {0}` | idem | |
| UPD-08 | update.rs:383 | importação de bucket sem catálogo aberto | `erro interno: catálogo não aberto para importar` | vira `falhou:` em UPD-04 | |

`{fonte}` = `winget` ou `scoop:{bucket}`. A `{mensagem}` de UPD-04 é o texto de um erro de módulo (ver seção 16: HTTP, cache, índice, tarball).

## 2. search

Código: `src/cli/mod.rs:730-772`, `src/cli/output.rs:188`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| SRC-01 | output.rs:188-205 | sempre (cabeçalho) | `Nome  Id  Versão  Fonte` (colunas alinhadas, 2 espaços entre elas) | HDR; sem recuo | |
| SRC-02 | output.rs:188-205 | uma linha por resultado | `{nome}  {id}  {versão}  {fonte}` | nome NOME; id ID; versão VER; fonte FONTE | |
| SRC-03 | output.rs:276-324 | busca sem resultado | só o cabeçalho SRC-01, sem mensagem | HDR | |
| SRC-04 | output.rs:276-324 | terminal estreito | coluna Nome cortada com `…` (mín. 8 colunas) | — | |
| SRC-05 | mod.rs:762-768 | `--json` | JSON indentado de `PackageEntry` | sem cor | |
| SRC-06 | mod.rs:581 | sem catálogo e sem rede | erro: `sem catálogo local e sem rede` / `→ conecte-se à internet e rode `wingetrs update`` | ERR-01 | |
| SRC-07 | mod.rs:~560 | primeira busca com catálogo vazio | atualização bloqueante sem texto nem spinner | — | |

## 3. show

Código: `src/cli/mod.rs:774-830`, `src/cli/output.rs:207`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| SHW-01 | output.rs:214 | sempre | `Nome: {nome}` | rótulo HDR; valor NOME | |
| SHW-02 | output.rs:215 | sempre | `Id: {id}` | rótulo HDR; valor ID | |
| SHW-03 | output.rs:216 | sempre | `Versão: {versão}` | rótulo HDR; valor VER | |
| SHW-04 | output.rs:217 | sempre | `Fonte: {winget\|scoop}` | rótulo HDR; valor FONTE | |
| SHW-05 | output.rs:219 | se houver | `Publicador: {texto}` | rótulo HDR; valor sem estilo | |
| SHW-06 | output.rs:222 | se houver | `Descrição: {texto}` | idem | |
| SHW-07 | output.rs:225 | se houver | `Página: {url}` | idem | |
| SHW-08 | output.rs:228 | se houver | `Licença: {texto}` | idem | |
| SHW-09 | output.rs:231 | se houver | `Tags: {a, b, c}` | idem | |
| SHW-10 | output.rs:234 | se houver | `Versões: {v1, v2, …}` | idem | |
| SHW-11 | output.rs:237-246 | uma por instalador | `Instalador: {arq} {tipo} {escopo\|-} {url}` | rótulo HDR; resto sem estilo | |
| SHW-12 | mod.rs:789 | id não existe | erro: `pacote não encontrado: {id}` / `→ procure o id com `wingetrs search <termo>`` | ERR-01 | |
| SHW-13 | mod.rs:799 | id em mais de uma fonte | erro: `pacote ambíguo: {id} existe em {fonte1, fonte2}` / `→ use --source` | ERR-01 | |
| SHW-14 | mod.rs:810 | manifesto winget sem hash | `pacote winget sem hash de manifesto` | ERR-01 (causa) | |
| SHW-15 | mod.rs:823 | manifesto scoop ausente | `pacote scoop sem manifesto` | idem | |
| SHW-16 | mod.rs:826-830 | `--json` | JSON indentado de `PackageDetails` | sem cor | |
| SHW-17 | winget/mod.rs:24 | `--version` inexistente | `versão {0} não encontrada` | ERR-01 | |

## 4. list

Código: `src/cli/mod.rs:834-905`, `src/cli/output.rs:351-439`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| LST-01 | output.rs:419-423 | sempre | `Nome  Id  Instalada  Disponível  Fonte  Origem` | HDR | |
| LST-02 | output.rs:351-405 | uma linha por app correlacionado | `{nome}  {id}  {instalada}  {disponível}  {fonte}  {origem}` | nome NOME; id ID; instalada VER; disponível NOVA; fonte FONTE; origem DIM | |
| LST-03 | output.rs:328 | app cujo desinstalador sumiu | nome vira `{nome} (sobra provável)` | NOME | |
| LST-04 | output.rs:337 e installed/correlate.rs | app com vários registros | versão vira `{versão} +N registros`; nos blocos LST-06/07 `, +N registros` | VER | |
| LST-05 | output.rs:357 | app do wingetrs quebrado | coluna Instalada mostra `quebrado` | VER | |
| LST-06 | output.rs:428-443 | há correspondência ambígua (e não é `--upgradable`) | linha em branco, `Ambíguos ({n}):`, depois `  {nome} ({versão}, {origem}): {cand1, cand2}` por item | título HDR, n NOME; itens recuo 2 sem estilo | |
| LST-07 | output.rs:428-443 | apps sem correspondência | `Não correlacionados ({n}):`, depois `  {nome} ({versão}, {origem})` | idem | |
| LST-08 | output.rs:444-451 | sempre sem `--upgradable` | linha em branco + `{n} instalados: {a} correlacionados, {b} ambíguos, {c} não correlacionados` | números NOME; resto sem estilo | |
| LST-09 | output.rs:351 | com `--upgradable` | só as linhas LST-02 que têm disponível; sem LST-06..08 | — | |
| LST-10 | mod.rs:893-899 | `--json` | JSON indentado das linhas | sem cor | |
| LST-11 | mod.rs:~842-900 | enquanto varre registro e catálogo | nenhum spinner (varredura + correlação silenciosas) | — | |

## 5. install — apps do Scoop (plano, escolha do alvo)

Código: `src/cli/install_cmd.rs` (HEAD) e `src/cli/output.rs`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| INS-01 | output.rs:447, install_cmd.rs:692 | alvo em mais de um bucket, sem terminal ou com `--yes` | erro: `'{alvo}' existe em mais de um lugar; use bucket/id:` + linhas `  {bucket/id}` | ✗ ERRO; ids ID; recuo 2 | |
| INS-02 | output.rs:457-465 | nada encontrado, sem sugestão | erro: `nenhum pacote encontrado para '{consulta}'` / `→ procure o nome com `wingetrs search <termo>`` | ERR-01 | |
| INS-03 | output.rs:457-465 | nada encontrado, com sugestões | erro: idem / `→ você quis dizer: {a}, {b}?` | sugestões ID | |
| INS-04 | install_cmd.rs:694-705 | alvo ambíguo em terminal | seletor (SEL-03) sem pergunta: cada linha `{bucket/id}  {versão}  ({nota})` | ver SEL-03 | |
| INS-05 | install_cmd.rs:769 | `--scope` num app do Scoop | erro: `--scope só vale para pacotes do winget` / `→ rode de novo sem --scope` | ERR-01 | |
| INS-06 | ops.rs:573-575 | candidato não instalável | `{nota do candidato}` ou `{id} não vem de um bucket do scoop` | ERR-01 | |
| INS-07 | ops.rs:577 | manifesto ausente no catálogo | `{id} sem manifesto no catálogo` | ERR-01 | |
| INS-08 | planner.rs:43 | versão já instalada | `{app} {versão} já está instalado` | ERR-01 | |
| INS-09 | output.rs:539-566 | 1ª linha do plano (curto e `--dry-run`), instalação | `Instalar {app} {versão}  {bucket} · {arch}` (arch `sem arquitetura` se não há) | `Instalar` HDR; app NOME; versão VER; resto DIM | |
| INS-10 | output.rs:539-566 | idem, atualização | `Atualizar {app} {antiga} → {nova}  {bucket} · {arch}` | `Atualizar` HDR; app NOME; antiga VER; seta sem estilo; nova NOVA; resto DIM | |
| INS-11 | output.rs:588-664 | plano curto: comandos | `  {n}. Comandos: {c1, c2}` | n NOME; recuo 2 | |
| INS-12 | output.rs:588-664 | plano curto: dados persistidos | `  {n}. Dados mantidos entre versões: {p1, p2}` | idem | |
| INS-13 | output.rs:588-664 | plano curto: atalho | `  {n}. Atalho no Menu Iniciar: {nome}` | idem | |
| INS-14 | output.rs:588-664 | plano curto: variável | `  {n}. Variável de usuário {nome} = {valor}` | idem | |
| INS-15 | output.rs:588-664 | plano curto: PATH (só se a pasta ainda não está no PATH) | `  {n}. PATH do usuário → {pasta}` | seta sem estilo; pasta CAMINHO | |
| INS-16 | output.rs:588-664 | plano curto, atualização sem `--keep` | `  {n}. Remove a versão {antiga}` | n NOME | |
| INS-17 | output.rs:666-679 | plano com MSI e `--allow-msiexec` | `  Se o extrator recusar este MSI, usa o Windows Installer (msiexec /a)` | DIM, recuo 2 | |
| INS-18 | output.rs:666-679 | plano com MSI sem a flag | `  Se o extrator não suportar este MSI, pede --allow-msiexec` | DIM, recuo 2 | |
| INS-19 | install_cmd.rs:725 | `--dry-run` com MSI | a mesma dica, mas sem recuo (`output::dim`) | DIM, sem recuo | |
| INS-20 | output.rs:568-586 | `--dry-run`: 1ª linha | igual a INS-09/INS-10 | idem | |
| INS-21 | output.rs:568-586 | `--dry-run`: um passo por linha | `  {n}. {passo}` (n alinhado à direita) — passos em PLN-01..PLN-26 | n NOME | |
| INS-22 | output.rs:568-586 | `--dry-run`: avisos do planejador | `⚠ aviso: {nota}` (ex.: PLN-24) | ⚠ e `aviso:` AVISO; sem recuo | |
| INS-23 | output.rs:89 | fim do `--dry-run` | `Simulação: nada foi alterado.` | sem estilo | |

## 6. Passos do plano (`--dry-run` do Scoop, `InstallPlan::step_lines`)

Código: `src/install/plan.rs:264-436` (texto) e `src/cli/output.rs:568` (numeração). Todo caminho vem de `Layout` e é impresso sem estilo.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| PLN-01 | plan.rs:278 | 1º uso do lançador | `Instalar o lançador de comandos em {shims}\.launcher\shim.exe` | sem estilo | |
| PLN-02 | plan.rs:282 | por download | `Baixar [{i}] {url} ({arquivo}, {algoritmo} {hash})` | sem estilo | |
| PLN-03 | plan.rs:290 | extração própria | `Extrair [{i}] {arquivo}[ ({motor})] para {staging}[ (só a subpasta {dir})]`; motor: `7z`, `MSI, sem executar o instalador`, `tar.xz`, `zip` | sem estilo | |
| PLN-04 | plan.rs:290 | arquivo simples | `Copiar [{i}] {arquivo} para {staging}` | idem | |
| PLN-05 | plan.rs:311 | instalador Inno | `Extrair [{i}] {arquivo} (Inno Setup, com o innounp) para {staging}[ (só {dir})]` | idem | |
| PLN-06 | plan.rs:332 | sempre | `Mover {staging} para {pasta da versão}` | idem | |
| PLN-07 | plan.rs:333 | `installer.file` | `Rodar o instalador {caminho} {args}` | idem | |
| PLN-08 | plan.rs:347 | instalador `.ps1` | `Rodar o instalador PowerShell {caminho} {args}` | idem | |
| PLN-09 | plan.rs:361 | hook do manifesto | `Rodar o script {pre_install\|post_install\|installer.script…} do manifesto (PowerShell)` | idem | |
| PLN-10 | plan.rs:364 | atualização | `Rodar o script {hook} da versão antiga (PowerShell)` | idem | |
| PLN-11 | plan.rs:368 | atualização | `Rodar o desinstalador da versão antiga {caminho} {args}` | idem | |
| PLN-12 | plan.rs:382 | dados persistentes | `Persistir {pasta\|arquivo} {origem} -> {destino} ({junção\|hardlink})` | idem | |
| PLN-13 | plan.rs:397 | sempre | `Apontar {current} -> {pasta da versão} (junção)` | idem | |
| PLN-14 | plan.rs:402 | por comando | `Criar comando {nome} em {shims} ({nome}.exe + {nome}.shim) -> {alvo}[ {args}]` (tipos de arquivo variam: `.exe + .shim`, `.cmd`, etc.) | idem | |
| PLN-15 | plan.rs:411 | por atalho | `Criar atalho {link} -> {alvo}[ {args}][ (ícone {ico})]` | idem | |
| PLN-16 | plan.rs:429 | módulo PowerShell | `Ligar módulo PowerShell {modulo} -> {current} e garantir {modules} no PSModulePath do usuário` | idem | |
| PLN-17 | plan.rs:434 | `env_set` | `Definir variável de usuário {nome} = {valor}` | idem | |
| PLN-18 | plan.rs:437 | `env_add_path` | `Adicionar ao PATH do usuário: {pasta}` | idem | |
| PLN-19 | plan.rs:440 | comandos criados | `Garantir {shims} no PATH do usuário` | idem | |
| PLN-20 | plan.rs:443 | sempre | `Gravar {pasta da versão}\manifest.json` | idem | |
| PLN-21 | plan.rs:448 | sempre | `Registrar {app} {versão} ({bucket}) em {state.db}` | idem | |
| PLN-22 | plan.rs:454 | atualização sem `--keep` | `Remover a versão antiga {pasta}` | idem | |
| PLN-23 | plan.rs:462 | `Display` do plano (testes, log; não vai ao CLI) | `Instalar {app} {versão} (bucket {b}, {arch})` / `Atualizar {app} de {a} para {n} (bucket …)`; linhas `{n:>3}. {passo}`; `Aviso: {nota}` | só em snapshots do planejador | |
| PLN-24 | planner.rs:400 | atualização sem manifesto antigo verificável | nota: `o desinstalador e os scripts de desinstalação da versão {v} não vão rodar: o manifesto gravado não existe ou não pôde ser verificado` | aparece como INS-22 | |
| PLN-25 | planner.rs:45 | dois executáveis com o mesmo comando | `dois executáveis com o mesmo nome de atalho de comando: {0}` | ERR-01 | |
| PLN-26 | planner.rs:47-54 | manifesto inseguro/não suportado | `caminho inseguro no manifesto: {0}`; `argumento inseguro no manifesto: {0}`; `não suportado: {0}`; `o script {hook} foi recusado: {motivos; …}` | ERR-01 | |

## 7. Confirmação, consentimento e código de terceiros

Código: `src/cli/install_cmd.rs:507-551`, `src/cli/output.rs:1010-1308`, `src/cli/picker.rs:430-460`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| CNF-01 | install_cmd.rs:550 | depois do plano, sem `--yes`, em terminal | pergunta `Continuar?` com seletor Sim/Não (SEL-01) | ver SEL-01 | |
| CNF-02 | install_cmd.rs:547 | sem terminal e sem `--yes` | erro: `a confirmação exige um terminal interativo` / `→ use --yes` | ERR-01 | |
| CNF-03 | install_cmd.rs:849 etc. | resposta Não, Esc ou `q` | `⚠ Cancelado.` | AVISO | |
| CNF-04 | output.rs:1250-1257 | script de manifesto, com `--dry-run` ou `--allow-scripts` | título `Script de terceiro ({hook}), roda com os seus privilégios e pode alterar qualquer arquivo seu` | HDR | |
| CNF-05 | output.rs:1207 | idem | `  Análise: pede confirmação` / `pede atenção redobrada; revise antes de permitir` / `recusado` | sem estilo, recuo 2 | |
| CNF-06 | output.rs:1207-1214 | por alerta do portão estático | `  ! {detalhe} (linha {n})` (detalhes: CNF-20..CNF-39) | AVISO, recuo 2 | |
| CNF-07 | output.rs:1259-1263 | corpo do script | `  {n} \| {linha}` por linha, n alinhado à direita | n DIM; linha sem estilo | |
| CNF-08 | output.rs:1267-1277 | `installer.file` | título `Instalador de terceiro: {arquivo} {args}` + linha `  roda com os seus privilégios e pode alterar qualquer arquivo seu` | título HDR; linha recuo 2 | |
| CNF-09 | output.rs:1267-1277 | `uninstaller.file` | `Desinstalador de terceiro: {arquivo} {args}` + idem | idem | |
| CNF-10 | output.rs:1278-1291 | instalador `.ps1` | como CNF-08/09, mais linhas CNF-05/06 | idem | |
| CNF-11 | output.rs:1292-1296 | manifesto de desinstalação ilegível | `Código de terceiro: {o quê}` + linha de privilégios | HDR | |
| CNF-12 | output.rs:1262 | vários apps | linha `{app}:` antes de cada bloco | NOME | |
| CNF-13 | output.rs:1070 | arquivo chamado por um script | rótulo do hook `arquivo chamado por {hook}: {alvo}` | dentro do título CNF-04 | |
| CNF-14 | output.rs:1146-1195 | hooks por fase | nomes `pre_install`, `post_install`, `pre_uninstall`, `uninstaller.script`, `post_uninstall`, sufixos `{hook} da versão instalada` e `, na remoção` | dentro do título CNF-04 | |
| CNF-15 | output.rs:1295-1306 | sem `--allow-scripts` (mesmo com `--yes`) | erro: `{apps} roda código de terceiros ({script post_install, instalador setup.exe, …})` / `→ revise com --dry-run e rode de novo com --allow-scripts` | ERR-01 | |
| CNF-16 | output.rs:1213 | rótulo curto de CNF-15 | `script {hook}`, `instalador {arquivo}`, `desinstalador {arquivo}` | idem | |
| CNF-17 | install/executor.rs:243-252 | MSI recusado pelo extrator, sem flag | `{app}: este MSI precisa do Windows Installer ({motivo}); rode de novo com --allow-msiexec. O msiexec /a não pode ser cancelado depois de começar e grava arquivos em C:\Windows\Installer.` | ERR-01 | |
| CNF-18 | executor.rs:252 | consentimento de script ausente (defesa) | `{app}: precisa da sua confirmação para rodar {o quê}` | ERR-01 | |
| CNF-19 | ops.rs:226 | manifesto mudou depois da revisão | `{app}: o manifesto mudou desde que você revisou os scripts; nada foi removido, peça a remoção de novo` | ERR-01 | |
| CNF-20 | script_scan.rs:431 | portão estático | `pede elevação (RunAs/sudo)` | detalhe de CNF-06 | |
| CNF-21 | script_scan.rs:432 | idem | `mexe no registro da máquina (HKLM)` | idem | |
| CNF-22 | script_scan.rs:433 | idem | `usa -Global (instalação para todos os usuários)` | idem | |
| CNF-23 | script_scan.rs:434 | idem | `grava em Program Files/Windows` | idem | |
| CNF-24 | script_scan.rs:435 | idem | `chama o comando scoop (instalaria no ~/scoop); não suportado` | idem | |
| CNF-25 | script_scan.rs:436 | idem | `chama o msiexec; o wingetrs só usa msiexec /a pela fila própria` | idem | |
| CNF-26 | script_scan.rs:437 | idem | `executa código montado na hora (iex, scriptblock, & $var)` | idem | |
| CNF-27 | script_scan.rs:438 | idem | `baixa arquivos da internet por conta própria` | idem | |
| CNF-28 | script_scan.rs:439 | idem | `mexe no registro do usuário (HKCU ou reg)` | idem | |
| CNF-29 | script_scan.rs:440 | idem | `cria ou mexe em tarefa agendada ou serviço do Windows` | idem | |
| CNF-30 | script_scan.rs:441 | idem | `caminho absoluto fora da pasta do app` | idem | |
| CNF-31 | script_scan.rs:442 | idem | `mexe nos internos do prelúdio do wingetrs (__wr)` | idem | |
| CNF-32 | script_scan.rs:443-445 | idem | variável de ambiente persistente / `muda variável de ambiente por [Environment]::SetEnvironmentVariable` | idem | |
| CNF-33 | script_scan.rs:446 | idem | `possível código escondido em comentário ou here-string` (prefixa outro detalhe: `{…}: {detalhe}`) | idem | |
| CNF-34 | script_scan.rs:451-455 | idem | `roda um arquivo que não é mostrado`; `redefine comando por apelido`; `roda um programa do pacote: {nome}`; `roda um arquivo ou comando calculado na hora`; `roda um arquivo que não é mostrado por outro programa ({lançador}): {arquivo}` | idem | |
| CNF-35 | script_scan.rs:458 | idem | `só roda em instalação global; não roda no wingetrs ({detalhe})` | idem | |
| CNF-36 | script_scan.rs:550 | idem | `função do Scoop ainda não suportada: {nome}` | idem | |
| CNF-37 | script_scan.rs:~785 | idem | `redefine a função do prelúdio do wingetrs: {nome}` | idem | |
| CNF-38 | script_scan.rs:893 | idem | `instalador .ps1: conteúdo não analisado` | idem | |
| CNF-39 | script_scan.rs:992 | idem | `{alvo}, linha {n}: {detalhe}` (alerta de arquivo chamado) | idem | |

## 8. Progresso (linha animada no stderr)

Código: `src/cli/output.rs:1360-1690` (`Phases`), `src/install/progress.rs` (títulos), `src/cli/install_cmd.rs:260-475` (desenho). Forma geral em terminal: **uma linha** redesenhada a cada 120 ms: `{spinner} {texto da fase} · {mm:ss}[ · {extra}][ · não pode ser cancelado]` e, à direita, um aviso amarelo. Largura máxima = colunas do terminal − 1 (o aviso é cortado primeiro). Ao terminar a linha é apagada (`end_progress`); resultados saem depois em stdout. Todo o texto da linha é sem estilo, exceto o aviso (AVISO). Spinner usa o conjunto de glifos da config (não `--glyphs`).

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| PRG-01 | output.rs:1650-1662 | baixando, total conhecido | `⣾ Baixando [#####---------------] 25%  1.2 / 4.8 MiB` (20 posições, `#` feito, `-` falta) | sem estilo | |
| PRG-02 | output.rs:1660 | baixando, total desconhecido | `⣾ Baixando 1.2 MiB` | sem estilo | |
| PRG-03 | output.rs:1663-1666 | após 0,5 s de dados | acrescenta ` · 2.3 MiB/s` e, com total, ` · ~6 s` ou ` · ~3 min` | sem estilo | |
| PRG-04 | output.rs:1672 | 10 s sem bytes novos | aviso ` · sem dados há {n} s` | AVISO | |
| PRG-05 | output.rs:1645 | todos os downloads completos | `⣾ Instalando…` | sem estilo | |
| PRG-06 | progress.rs:104 | fase `Preparing` | `Preparando · 00:03` | sem estilo | |
| PRG-07 | progress.rs:105 | `WaitingInstaller` | `Aguardando o Windows Installer (outro instalador em uso) · 00:12` | sem estilo | |
| PRG-08 | progress.rs:108 | `Extracting` | `Extraindo {arquivo} ({ZIP\|tar.gz\|7z\|MSI\|nupkg\|tar.xz\|CAB\|cópia\|ext}) · 00:04` | sem estilo | |
| PRG-09 | progress.rs:109 | `Installer` (msiexec /a) | `Windows Installer (msiexec /a) · 00:20 · não pode ser cancelado` (até o log aparecer) | sem estilo | |
| PRG-10 | output.rs:1585-1604 | `Installer` com log lido | `… · [~]{N.NNN} arquivos · {último arquivo cortado pela esquerda com …}` | sem estilo | |
| PRG-11 | output.rs:1593 | `Installer` sem atividade 60 s | aviso ` · sem atividade há {n} min; o Windows Installer pode estar esperando outro instalador` | AVISO | |
| PRG-12 | output.rs:1351 | `Installer` passa de 30 s | linha fixa acima da animação, uma vez: `O Windows Installer pode levar vários minutos em pacotes grandes; não feche o terminal.` | sem estilo | |
| PRG-13 | progress.rs:110 | `Placing` | `Colocando os arquivos · 00:02` | sem estilo | |
| PRG-14 | progress.rs:111 | `Linking` | `Criando comandos · 00:01` | sem estilo | |
| PRG-15 | progress.rs:112 | `Recording` | `Registrando · 00:01` | sem estilo | |
| PRG-16 | progress.rs:113 | `Removing` | `Removendo {app} · 00:03` (para bucket: `Removendo bucket {nome}`) | sem estilo | |
| PRG-17 | progress.rs:114 | `Scanning` | `Procurando resíduos · 00:01` | sem estilo | |
| PRG-18 | progress.rs:115-117 | `WaitingElevation` (UAC aberto) | `Aguardando autorização para executar como administrador (se o UAC não aparecer, clique no escudo piscando na barra de tarefas) · 00:05 · não pode ser cancelado` | sem estilo | |
| PRG-19 | progress.rs:118-120 | `RunningInstaller` | `Instalador de {nome} rodando (pode abrir janelas) · 01:10 · não pode ser cancelado` | sem estilo | |
| PRG-20 | progress.rs:121 | `Verifying` | `Conferindo a instalação · 00:02` | sem estilo | |
| PRG-21 | progress.rs:122 | `CopyingFolders` | `Copiando as pastas de dados para backups · 00:08` | sem estilo | |
| PRG-22 | progress.rs:123-125 | `Uninstalling` | `Desinstalador de {nome} rodando (pode abrir janelas) · 00:30 · não pode ser cancelado` | sem estilo | |
| PRG-23 | progress.rs:126-128 | `CheckingRemoval` | `Conferindo a remoção de {nome} (até 2 min) · 00:10 · não pode ser cancelado` | sem estilo | |
| PRG-24 | progress.rs:129-131, 170 | `SelfElevating` (instalador que pede o próprio UAC) | `Aguardando autorização para executar como administrador · instalador de {nome} rodando (pode abrir janelas) · 00:20 · não pode ser cancelado`; após 120 s: `{nome}: rodando o instalador (pode pedir autorização de administrador) · …` | sem estilo | |
| PRG-25 | progress.rs:132 | `Bridge` | `{winget\|scoop} rodando (pode abrir janelas) · 00:40 · não pode ser cancelado` | sem estilo | |
| PRG-26 | progress.rs:133 | `RunningScript` | `Rodando o script {hook} do pacote · 00:05 · não pode ser cancelado` | sem estilo | |
| PRG-27 | progress.rs:134-136 | `RunningPackageInstaller` | `Rodando o instalador do pacote ({arquivo}; pode abrir janelas) · 00:30 · não pode ser cancelado` | sem estilo | |
| PRG-28 | output.rs:1626 | instalador nativo/ponte/script 5 min | aviso ` · rodando há {n} min; pode haver uma janela esperando resposta` | AVISO | |
| PRG-29 | install_cmd.rs:57 | Ctrl+C num passo que não cancela | linha acima da animação: `um passo já começou e não pode ser interrompido ({passo}); esperando terminar` (passo padrão `instalador`) | sem estilo | |
| PRG-30 | output.rs:1733-1744 | stderr **não** é terminal | uma linha por mudança de fase, sem relógio: `Baixando…`, `Preparando…`, `Extraindo x.msi (MSI)…` etc. (título da fase + `…`) | sem estilo, sem `\r` | |
| PRG-31 | progress.rs:12 | antes de todo UAC | `Aguardando autorização para executar como administrador` (vira a fase PRG-18) | sem estilo | |
| PRG-32 | progress.rs:15 | UAC recusado | `Permissão de administrador recusada.` | ver ERR-01 | |
| PRG-33 | progress.rs:18 | auxiliar elevado já ativo | `permissão de administrador concedida` (linha interna: só troca a fase, não é mostrada) | — | |
| PRG-34 | winget_cmd.rs:68 | Ctrl+C antes de lançar | `cancelando: nada foi iniciado; esperando a etapa atual parar` | sem estilo | |
| PRG-35 | mod.rs:984-987 | `bucket add` | fase `Extraindo bucket {nome} (GitHub) · 00:02` | sem estilo | |
| PRG-36 | output.rs:1697-1722 | ponte winget/scoop | saída própria da ferramenta, cada linha `  {linha}` (recuo 2) em stdout acima da animação | sem estilo (Task 215 vai formatar) | |
| PRG-37 | tui/jobs.rs:856-858 | ponte espera `msi.lock` | linhas da ponte `aguardando outro instalador`, `iniciando o winget`, `atualizando a fonte do winget` | como PRG-36 | |

## 9. install — pacotes do winget (motor nativo)

Código: `src/cli/winget_cmd.rs` (HEAD), `src/winget_install/plan.rs`, `src/cli/output.rs:846-905`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| WIN-01 | plan.rs:300, output.rs:881 | 1ª linha do plano nativo | `{nome} {versão} ({tipo}, {usuário\|máquina\|escopo do instalador})` ex.: `7-Zip 26.04 (exe, máquina)` | nome+versão NOME; `(…)` DIM; sem recuo | |
| WIN-02 | plan.rs:18 | instalador de máquina | `  {n}. ⚠ Pede permissão de administrador para este instalador` | n NOME; ⚠ AVISO; texto sem estilo | |
| WIN-03 | plan.rs:23 | MSI de máquina | `  {n}. ⚠ Pede permissão de administrador para o Windows Installer` | idem | |
| WIN-04 | plan.rs:20 | instalador de máquina | `  {n}. Roda numa pasta temporária do wingetrs` | n NOME | |
| WIN-05 | plan.rs:26 | instalador que se auto-eleva | `  {n}. ⚠ O instalador pode pedir permissão de administrador` | idem WIN-02 | |
| WIN-06 | plan.rs:324 | instalador interativo | `  {n}. Abre a janela do instalador (sem modo silencioso)` | n NOME | |
| WIN-07 | plan.rs:327 | modo silencioso com progresso | `  {n}. Mostra o progresso do instalador` | idem | |
| WIN-08 | plan.rs:328 | modo silencioso | `  {n}. Sem janela` | idem | |
| WIN-09 | plan.rs:28-30 | pacote MSIX | `  {n}. Instala pelo Windows (pacote MSIX), sem janela` e `  {n}. Só para este usuário` | idem | |
| WIN-10 | plan.rs:32 | atualização que remove a versão antiga antes | `  {n}. Remove a versão instalada antes (o desinstalador pode abrir janela ou pedir permissão de administrador)` | idem | |
| WIN-11 | plan.rs:348-358 | por contrato de licença | `  {n}. Licença: {rótulo} — {url\|texto}` | `Licença` HDR (`labeled`); resto sem estilo | |
| WIN-12 | plan.rs:365 (`--dry-run`) | detalhes depois dos passos | `  Instalador: {url}` | rótulo HDR; recuo 2 | |
| WIN-13 | plan.rs:371 (`--dry-run`) | idem | `  Linha de comando: msiexec.exe /i "<instalador>" {args}` ou `<instalador> {args}` | rótulo HDR; recuo 2 | |
| WIN-14 | plan.rs:377 (`--dry-run`) | idem | `  SHA-256: {HEX MAIÚSCULO}` | rótulo HDR; recuo 2 | |
| WIN-15 | winget_cmd.rs:378 | depois do plano | DRY_RUN_DONE (INS-23), seguido de CNF-01..03 sem `--dry-run` | — | |
| WIN-16 | winget_cmd.rs:524 | `--yes` | linhas `Licença aceita: {rótulo} — {url}` (uma por contrato, antes de rodar) | rótulo HDR | |
| WIN-17 | winget_cmd.rs:550, 478-489 | sucesso | `✓ Instalado: {nome} {versão}` | ✓ e `Instalado:` OK; nome NOME; versão VER | |
| WIN-18 | winget_cmd.rs:479 | instalador terminou mas o app não apareceu | `⚠ aviso: {texto do aviso}` | AVISO | |
| WIN-19 | output.rs:941 | instalador pediu reinício | `Reinicie o computador para concluir.` ou `O instalador está reiniciando o computador.` | AVISO sem glifo | |
| WIN-20 | winget_cmd.rs:513 | motor/planejador escolheu a ponte | passo único do plano: `  {n}. Instala pelo winget[ ({motivo})]` com cabeçalho `{nome versão} via winget` | nome NOME; `via winget` DIM | |
| WIN-21 | output.rs:922-930 | upgrade pela ponte, com motivo | `Atualizando pelo winget: {motivo}` (ou `Instalando pelo winget: …`) | DIM | |
| WIN-22 | winget_cmd.rs:249-253 | `--dry-run` da ponte | `  Comando: winget install …` | rótulo HDR; recuo 2 | |
| WIN-23 | winget_cmd.rs:272-274 | sucesso da ponte | `✓ Instalado: {resumo}` (resumo vem de `tui/jobs.rs`: `{nome}: instalado via winget`, `…; reinicie o computador para concluir`) | OK + resto | |
| WIN-24 | tui/jobs.rs:1153-1156 | falhas da ponte | `{nome}: instalado via winget…` / `O winget instalou {x}, não {y}: o catálogo local do winget está mais velho que o do wingetrs. Tente de novo mais tarde.` / `{nome}: falha ao iniciar {programa}: {e}` / `{nome}: {tool} não encontrado` / `{nome}: identificador inválido: {id}` | ERR-01 | |
| WIN-25 | bridge/mod.rs:166 | escopo máquina sem instalador | `sem instalador para máquina; instalando para o usuário` | linha da ponte (PRG-36) | |
| WIN-26 | bridge/elevated.rs:19 | UAC recusado na ponte | `Permissão de administrador recusada. Atualizando sem elevar.` | linha da ponte | |
| WIN-27 | helper_folder.rs:9-13, winget_cmd.rs:127 | exe fora de pasta só-admin | `Para usar funções de administrador, o wingetrs precisa estar numa pasta que só o administrador grava (ex.: C:\Program Files\wingetrs). Ele está em {pasta}.` | AVISO sem glifo, uma vez | |
| WIN-28 | winget_cmd.rs:508, select.rs | motor `native` recusa | erro com o texto da recusa (WIN-29) | ERR-01 | |
| WIN-29 | select.rs:286-349 | seleção de instalador | `o instalador declara esta arquitetura ({a}) como não suportada`; `nenhum instalador para esta arquitetura ({a})`; `exige Windows {v} ou mais novo`; `versão mínima do Windows inválida no manifesto`; `nenhum instalador aplicável a este Windows`; `nenhum instalador para escopo {s}` | ERR-01 | |
| WIN-30 | select.rs:365-383 | motivos da ponte (viram `Instala pelo winget ({motivo})`) | `tipo desconhecido`; `tipo {zip\|portable\|font\|pwa\|msstore}`; `tem dependências` (todos com sufixo `: instalado pelo winget`, removido ao exibir) | dentro de WIN-20 | |
| WIN-31 | plan.rs:175-200 | allowlist do auxiliar elevado | `tipo {t} sem auxiliar elevado: instalado pelo winget`; `log nos parâmetros do instalador, e o instalador elevado não grava log: instalado pelo winget`; `{motivo} para elevação: {token}` / `{motivo} (elevação)` | dentro de WIN-20 | |
| WIN-32 | fetch.rs:22-33 | download do instalador | `o instalador não usa https: {0}`; `o manifesto não traz um SHA-256 válido para o instalador`; `identificador ou versão inválidos: {0}`; `E/S: {0}`; `o instalador mudou depois de conferido` | ERR-01 | |
| WIN-33 | winget_install/ops.rs:72-96 | execução do instalador | `outra instalação do wingetrs está em andamento`; `trava de instalação: {0}`; `instalação cancelada`; `Permissão de administrador recusada.`; `não foi possível iniciar o instalador: {0}`; `{message}` (categoria do código de saída, WIN-34) | ERR-01 | |
| WIN-34 | codes.rs:214-237 | código de saída do instalador | `o pacote está em uso; feche-o e tente de novo`; `outro instalador está rodando; tente de novo em instantes`; `um arquivo necessário está em uso`; `falta uma dependência do pacote`; `não há espaço em disco suficiente`; `memória insuficiente`; `o instalador recusou os parâmetros recebidos`; `o instalador precisa de rede e não a encontrou`; `o instalador pede para contatar o suporte do pacote`; `reinicie o computador para concluir`; `reinicie o computador e rode de novo para instalar`; `o instalador iniciou a reinicialização do computador`; `a instalação foi cancelada pelo usuário`; `o pacote já está instalado`; `já há uma versão mais nova instalada`; `uma política do sistema bloqueou a instalação`; `o sistema não é compatível com este pacote`; `o instalador terminou com um código esperado pelo manifesto`; `o instalador terminou com um código desconhecido` | ERR-01 | |
| WIN-35 | winget_install/switches.rs:33-36 | linha de comando | `este tipo de instalador não usa linha de comando`; `argumentos inválidos: {0}` | ERR-01 | |

## 10. uninstall (Scoop e Windows)

Código: `src/cli/install_cmd.rs:809-931` (HEAD), `src/cli/winget_cmd.rs:562-806`, `src/cli/output.rs:479-537`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| UNI-01 | output.rs:479-490 | app do wingetrs, 1ª linha | `Desinstalar {app} {versão}` | `Desinstalar` HDR; app NOME; versão VER | |
| UNI-02 | output.rs:491 | sempre | `  1. Apaga a pasta do app` | n NOME | |
| UNI-03 | output.rs:493-497 | se houver comandos | `  {n}. Comandos: {c1, c2}` | idem | |
| UNI-04 | output.rs:498-503 | por atalho | `  {n}. Atalho no Menu Iniciar: {nome}` | idem | |
| UNI-05 | output.rs:505-520 | PATH ainda usado por outro app | `  {n}. PATH do usuário mantém {pasta} (ainda usado por: {apps})` | pasta CAMINHO | |
| UNI-06 | output.rs:505-520 | PATH a remover | `  {n}. PATH do usuário → remove {pasta}` | seta sem estilo; pasta CAMINHO | |
| UNI-07 | output.rs:521-526 | variável definida pelo app | `  {n}. Variável de usuário {nome} volta ao valor anterior` | n NOME | |
| UNI-08 | output.rs:527-533 | dados persistentes | `  {n}. Dados mantidos: {pasta} (use --purge para apagar)` ou, com `--purge`, `  {n}. Apaga os dados: {pasta}` | n NOME | |
| UNI-09 | install_cmd.rs:871-874 | sucesso | `✓ Desinstalado: {app} {versão}` | ✓ e rótulo OK; app NOME; versão VER | |
| UNI-10 | install_cmd.rs:875-877 | notas do desinstalador (o que o app queria e não aconteceu) | `⚠ aviso: {nota}` em **stderr** | AVISO | |
| UNI-11 | install_cmd.rs:822 | app não é do wingetrs | cai no fluxo de apps do Windows (UNI-20..) sem aviso | — | |
| UNI-12 | ops.rs:214 | id não instalado | `{0} não está instalado pelo wingetrs` | ERR-01 | |
| UNI-13 | ops.rs:212 | outros apps dependem | `outros apps dependem deste: {a, b}` | ERR-01 | |
| UNI-14 | ops.rs:216, executor.rs:237 | arquivo aberto | `arquivo em uso: {0} — feche o programa e tente de novo` | ERR-01 | |
| UNI-15 | ops.rs:218-224 | trava | `outra instalação do wingetrs está em andamento; tente de novo em instantes`; `não foi possível obter o install.lock: {0}`; `o estado de {0} mudou desde o planejamento; tente de novo` | ERR-01 | |
| UNI-16 | install_cmd.rs:919 | nenhum resíduo escolhido | `Nenhum resíduo removido.` | sem estilo | |
| UNI-17 | install_cmd.rs:926 | por resíduo removido | `✓ Removido: {rótulo do resíduo}` | ✓ e `Removido:` OK | |
| UNI-18 | install_cmd.rs:927 | falha ao remover resíduo | `⚠ aviso: {erro}` (stderr) | AVISO | |
| UNI-19 | output.rs:798-806 | resíduos sem terminal e sem `--yes` | `⚠ aviso: resíduos encontrados (nada foi removido)` + linhas `  {rótulo}` + `→ use --yes para remover os de confiança alta` | AVISO; seta DIM | |
| UNI-20 | output.rs:723-736 | rótulo de resíduo (lista, seletor, UNI-17) | `Registro: HKCU\{chave} [{valor}]`, `Registro: HKCU\{chave}`, `Pasta: {caminho}`, `Atalho: {caminho}` | sem estilo | |
| UNI-21 | winget_cmd.rs:579 | app do Windows inexistente | `'{alvo}' não está instalado (nem pelo wingetrs, nem entre os apps do Windows)` | ERR-01 | |
| UNI-22 | winget_cmd.rs:585-591 | alvo ambíguo (sem terminal ou `--yes`) | `'{alvo}' corresponde a mais de um app instalado; rode sem --yes num terminal para escolher:` + linhas `  {nome} ({local})` | ERR-01 | |
| UNI-23 | winget_cmd.rs:593-602 | alvo ambíguo em terminal | seletor (SEL-03): `{nome}  {versão}  ({local})` | ver SEL-03 | |
| UNI-24 | winget_cmd.rs:625 | `--purge` em app de fora | erro: `--purge só vale para apps do wingetrs ({nome} foi instalado por outro programa)` / `→ rode de novo sem --purge` | ERR-01 | |
| UNI-25 | winget_cmd.rs:636 | registro sem desinstalação | `{nome}: registro sem desinstalação conhecida ({chave})` | ERR-01 | |
| UNI-26 | uninstall.rs:123 | 1ª linha | `Desinstalar {nome} {versão}` (nome+versão NOME) | `Desinstalar` HDR | |
| UNI-27 | uninstall.rs:116 | remoção elevada | `  {n}. ⚠ Pede permissão de administrador (UAC)` | ⚠ AVISO | |
| UNI-28 | uninstall.rs:119 | registro de máquina removido pelo winget | `  {n}. ⚠ O winget pode pedir permissão de administrador` | idem | |
| UNI-29 | uninstall.rs:142 | MSIX | `  {n}. Remove o pacote MSIX {família} (só deste usuário)` | n NOME | |
| UNI-30 | uninstall.rs:146 | portable instalado pelo winget | `  {n}. Remove pelo winget (pode mostrar o progresso dele)` | idem | |
| UNI-31 | uninstall.rs:149 | desinstalador interativo | `  {n}. Abre a janela do desinstalador` | idem | |
| UNI-32 | uninstall.rs:152 | desinstalador silencioso | `  {n}. Roda o desinstalador registrado do app` | idem | |
| UNI-33 | uninstall.rs:155 | sem comando | `  {n}. Sem comando de desinstalação registrado` | idem | |
| UNI-34 | uninstall.rs:176 | pastas compartilhadas | `  ⚠ Outra instalação de {nome} usa as mesmas pastas: copie antes de remover.` | ⚠ e texto AVISO; recuo 2, sem número | |
| UNI-35 | uninstall.rs:181-195 | idem | `  ⚠ Pasta compartilhada: {pasta}`; `  ⚠ Tamanho da cópia: {x MB} em {n} itens`; `  ⚠ Também é possível remover só o registro desta instalação` ou `  ⚠ Remover só o registro não está disponível: {motivo}` | idem | |
| UNI-36 | uninstall.rs:203 (`--dry-run`) | detalhe | `  Comando: {programa} {args}` | rótulo HDR | |
| UNI-37 | winget_cmd.rs:647 | `--dry-run` | DRY_RUN_DONE (INS-23) | — | |
| UNI-38 | winget_cmd.rs:677-680 | pastas compartilhadas com `--yes` / sem terminal | erro: `este app compartilha dados com outros` / `→ rode sem --yes para escolher` ou `→ rode num terminal para escolher` | ERR-01 | |
| UNI-39 | removal.rs:63-66 | escolha de remoção | seletor sem pergunta: `Backup das pastas e remover  {tamanho}`, `Remover sem copiar`, `Só o registro`, `Cancelar` | ver SEL-04 | |
| UNI-40 | winget_cmd.rs:793 | sucesso | `✓ Desinstalado: {nome} {versão}` | OK + NOME | |
| UNI-41 | uninstall.rs:31, winget_cmd.rs:798 | desinstalador saiu mas o registro ficou | `⚠ aviso: o desinstalador terminou, mas o registro continua` (ou o aviso do relatório) | AVISO | |
| UNI-42 | output.rs:932-938 | pastas copiadas | `✓ Pastas copiadas para {destino}` | ✓ OK; destino CAMINHO | |
| UNI-43 | output.rs:961-971 | só o registro removido | `✓ Removido: registro de {nome} (cópia em {arquivo .reg})` | OK; nome NOME; arquivo CAMINHO | |
| UNI-44 | output.rs:941 | reinício | WIN-19 | — | |
| UNI-45 | uninstall.rs:341-363 | recusas do auxiliar | `o auxiliar de administrador só apaga um registro HKLM …`; `nome de chave ARP inválido: {…}`; `sem desinstalador registrado; use "Remover registro órfão" se for sobra` | ERR-01 | |
| UNI-46 | uninstall.rs:495-910 | falhas da remoção | `variável desconhecida no desinstalador: {p}`; `desinstalador sem caminho absoluto: {p}`; `falha ao iniciar o winget: {e}`; `falha ao iniciar o desinstalador: {e}`; `fila do msiexec: {e}`; `não foi possível ler o espaço livre em {p}`; `nome de pasta inválido: {p}`; `não é uma pasta: {p}`; `pasta ilegível {p}: {e}`; `não foi possível criar/ler {p}: {e}`; `falha ao copiar {p}: {e}`; `falha ao exportar/apagar {chave}: {e}` | ERR-01 | |
| UNI-47 | bridge/worker.rs:160-823 | recusas do auxiliar elevado | `operação removida: o auxiliar elevado não roda instaladores (cada um pede o próprio UAC)`; `aguardando o desinstalador terminar`; `nome de chave ARP inválido`; `a chave não existe mais`; `a chave mudou desde a confirmação`; `desinstalador fora de pasta de administrador`; `desinstalador por interpretador recusado`; `linha do rundll32 recusada`; `parâmetros do msiexec recusados`; `argumento do desinstalador fora de pasta de administrador` | ERR-01 | |
| UNI-48 | removal.rs:144-157 | falhas da remoção | `{nome}: {erro}`; falha de cópia das pastas | ERR-01 | |

## 11. upgrade

Código: `src/cli/install_cmd.rs:933-1041` (HEAD), `src/cli/winget_cmd.rs:810-930`, `src/cli/output.rs:739-790`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| UPG-01 | output.rs:739-748 | `upgrade` sem alvo nem `--all` | linhas `{app}  {instalada}  →  {nova}` (tabela sem cabeçalho, colunas alinhadas) | app NOME; antiga VER; seta DIM; nova NOVA | |
| UPG-02 | output.rs:757-760 | apps fixados | `{app} (fixado)` | app NOME; `(fixado)` DIM | |
| UPG-03 | output.rs:765 | nada a atualizar | `Nada para atualizar.` | sem estilo | |
| UPG-04 | output.rs:763 | há o que atualizar | `→ use `wingetrs upgrade --all` ou `wingetrs upgrade <app>`` | seta DIM; texto sem estilo | |
| UPG-05 | install_cmd.rs:980 | app com versão nova que o wingetrs não atualiza | `⚠ aviso: {app}: {motivo}` (stderr, uma por app) | AVISO | |
| UPG-06 | output.rs:774 | `--all` pula fixado | `fixado, pulado: {app}` | DIM | |
| UPG-07 | output.rs:779 | alvo já na última versão | `✓ {app} já está na versão mais recente.` | ✓ OK; app NOME | |
| UPG-08 | install_cmd.rs:954 | alvo fixado | erro: `{app} está com a versão fixada` / `→ desafixe na TUI (menu de ações)` | ERR-01 | |
| UPG-09 | install_cmd.rs:954, winget_cmd.rs:954 | skipped do alvo | `{app}: {motivo}` | ERR-01 | |
| UPG-10 | install_cmd.rs:1000-1004 | estado inconsistente | `{app} não está instalado pelo wingetrs`; `{app} não foi encontrado no bucket {bucket}` | ERR-01 | |
| UPG-11 | output.rs:539-566, install_cmd.rs:1009 | plano de cada app | INS-10 (cabeçalho `Atualizar …`) + passos INS-11..16 | — | |
| UPG-12 | install_cmd.rs:1034-1037 | sucesso | `✓ Atualizado: {app} {versão}` por app | OK; app NOME | |
| UPG-13 | install_cmd.rs:1038 | caminho novo no PATH | RES-02 e RES-03 | — | |
| UPG-14 | winget_cmd.rs:825 | app do Windows sem pacote winget | `{nome} não foi associado a um pacote do winget: não há de onde atualizar` | ERR-01 | |
| UPG-15 | winget_cmd.rs:831 | fixado | `{nome} está com a versão fixada` / `→ desafixe na TUI (menu de ações)` | ERR-01 | |
| UPG-16 | winget_cmd.rs:902-907 | upgrade pela ponte, plano | `{nome} {instalada} → {nova} via winget` + linha com a linha de comando do winget | sem estilo (ao contrário de WIN-20) | |
| UPG-17 | winget_cmd.rs:924 | sucesso da ponte | `✓ Atualizado: {resumo}` | OK + resto | |
| UPG-18 | winget_cmd.rs:884 | sucesso nativo | `✓ Atualizado: {nome} {versão}` | OK + NOME | |
| UPG-19 | engine.rs:262-277 | atualização nativa recusada | `o pacote não permite atualizar por cima; remova e instale de novo`; `sem a versão instalada: a atualização precisa do plano com a versão anterior`; `a versão {x} do catálogo não é maior que a instalada ({y})`; `o pacote pede para remover a versão anterior, mas não há plano de remoção`; `remoção da versão anterior: {0}`; `a versão anterior não foi removida ({0}); a instalação nova não rodou`; `a versão anterior foi removida e a nova não instalou: {0}` | ERR-01 | |
| UPG-20 | ops.rs:230 | atualização parcial | `{erro} (já instalados: {a, b})` | ERR-01 | |

## 12. Resultados, dicas e Ctrl+C

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| RES-01 | install_cmd.rs:800 | install Scoop ok | `✓ Instalado: {app} {versão}` | ✓ e rótulo OK; app NOME; versão VER | |
| RES-02 | output.rs:681-721 | depois de RES-01 se o PATH novo não está neste processo (com comandos) | `Abra um terminal novo para usar: {c1, c2}` | DIM (`Abra… usar:`); comandos sem estilo | |
| RES-03 | output.rs:681-721 | idem, sem comandos | `Abra um terminal novo para o PATH novo valer.` | DIM | |
| RES-04 | install_cmd.rs:63 | Ctrl+C aceito | `⚠ Cancelado.` e código de saída 130 (o erro `cancelado pelo usuário` não é impresso) | AVISO | |
| RES-05 | output.rs:84 | recusa/cancelamento em qualquer comando | `⚠ Cancelado.` | AVISO | |
| RES-06 | mod.rs:995-996 | `bucket add` | ver BKT-05 | — | |
| RES-07 | job_log (arquivo) | sempre | `job.start`/`job.end` no log de jobs, só arquivo (`instalado`, `atualizado`, `removido`, `cancelado`) | não vai ao terminal | |

## 13. bucket

Código: `src/cli/mod.rs:925-1025`, `src/buckets.rs`, `src/cli/output.rs:107`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| BKT-01 | output.rs:107-119 | `bucket list` | linhas sem cabeçalho: `{nome}  {dono/repo}  {n}  {AAAA-MM-DD}` | nome NOME; repo ID; n NOME; data ID | |
| BKT-02 | output.rs:111 | bucket ainda não baixado | `{nome}  {dono/repo}  não baixado` | idem | |
| BKT-03 | mod.rs:951 | `bucket add` de repositório desconhecido, em terminal | pergunta `O bucket {nome} ({repo}) não é conhecido. Adicionar?` (Sim/Não) | ver SEL-01 | |
| BKT-04 | mod.rs:943 | nome sem repositório e não conhecido | erro: `o bucket {nome} não é conhecido` / `→ informe o repositório: wingetrs bucket add {nome} <owner/repo>` | ERR-01 | |
| BKT-05 | mod.rs:995-996 | adicionado | `✓ Bucket adicionado: {nome}` e `→ {n} apps no catálogo` | ✓ e rótulo OK; nome NOME; seta DIM | |
| BKT-06 | mod.rs:1004 | `bucket rm` de main/extras em terminal | pergunta `{nome} é um bucket padrão. Remover mesmo assim?` | ver SEL-01 | |
| BKT-07 | mod.rs:1020 | removido | `✓ Bucket removido: {nome}` | OK; nome NOME | |
| BKT-08 | mod.rs:1013 | durante `rm` | fase PRG-16 com `Removendo bucket {nome}` | — | |
| BKT-09 | buckets.rs:22-53 | erros de validação | `só buckets do GitHub nesta versão: {0}`; `repositório inválido (use owner/repo ou https://github.com/owner/repo): {0}`; `nome de bucket inválido: {0}`; `o bucket {nome} ({repo}) não é conhecido; confirme num terminal ou rode de novo com --yes`; `o bucket {0} já existe com outro repositório`; `bucket não configurado: {0}`; `{0} é um bucket padrão; confirme num terminal ou rode de novo com --yes` | ERR-01 | |
| BKT-10 | buckets.rs:36-50 | falhas | `o bucket {b} tem apps instalados: {a, b}`; `falha ao remover do catálogo: {0}`; `não foi possível ler o state.db: {0}`; `falha ao gravar o estado: {0}`; `uma atualização está em andamento; tente de novo`; `repositório não encontrado: {0}`; `não parece um bucket do Scoop (nenhum manifesto em bucket/)`; `falha ao baixar o bucket: {0}` | ERR-01 | |
| BKT-11 | github/mod.rs:13-27 | consultas ao GitHub | `sem conexão com o GitHub`; `repositório ou release não encontrado`; `limite de requisições do GitHub atingido (renova em {reset})`; `o repositório não tem release publicada`; `resposta do GitHub ilegível`; `GitHub respondeu HTTP {0}`; `falha ao falar com o GitHub: {0}`; `nome de repositório inválido: {0}` | ERR-01 (módulo novo, WIP) | |
| BKT-12 | sources/scoop/mod.rs:8-12 | download do bucket | `bucket desconhecido: {0}`; `tarball inválido: {0}`; `manifesto scoop inválido: {0}` | ERR-01 | |

## 14. cleanup e repair

Código: `src/cli/winget_cmd.rs:951-1023`, `src/cli/output.rs:950-1013`, `src/cli/output.rs:1310`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| CLN-01 | winget_cmd.rs:958 | `cleanup` sem `--orphans` | erro: `diga o que limpar` / `→ use --orphans` | ERR-01 | |
| CLN-02 | output.rs:986 | nenhum órfão | `Nenhum registro órfão.` | sem estilo | |
| CLN-03 | output.rs:950-958 | por órfão | `Órfão: {nome} — {HKLM 64 bits\|HKLM 32 bits\|HKCU} — falta {caminho}` | `Órfão:` AVISO; nome NOME; hive DIM; caminho CAMINHO | |
| CLN-04 | output.rs:989-994 | depois da lista | `O app pode ter mudado de lugar (ex.: Steam em outra pasta).` | DIM | |
| CLN-05 | output.rs:997-1002 | depois da lista | `Antes de apagar, cada registro é exportado para um .reg em {pasta}` | pasta CAMINHO | |
| CLN-06 | winget_cmd.rs:978-986 | seletor de vários (nenhum marcado) | `{nome}  {hive}  (falta {caminho})` | ver SEL-05 | |
| CLN-07 | output.rs:1005-1011 | sem terminal e sem `--yes` | `Nada foi removido: sem terminal.` + `→ use --yes para remover todos os listados` | seta DIM | |
| CLN-08 | output.rs:1013 | seletor sem escolha | `Nenhum registro removido.` | sem estilo | |
| CLN-09 | output.rs:973-979 | por órfão removido | `✓ Removido: {nome} (backup {arquivo .reg})` | OK; nome NOME; arquivo CAMINHO | |
| CLN-10 | output.rs:981-984 | falha | `✗ Erro: {nome}: {erro}` | ERRO; nome NOME | |
| CLN-11 | winget_cmd.rs:1020 | ao final se houve falha | erro (exit 1): `{n} registro(s) órfão(s) não foram removidos` | ERR-01 | |
| CLN-12 | winget_cmd.rs:1003 | enquanto remove | fase do progresso por registro (UAC antes do HKLM: PRG-18) | — | |
| REP-01 | output.rs:1310-1332 | `repair` | 5 linhas fixas `{rótulo}: {lista\|nenhum}`: `Reconstruídos`, `Mantidos`, `Registros removidos (app sumiu do disco)`, `Pulados (sem install.json legível)`, `Pastas apagadas (só sobras de instalação interrompida)` | rótulos HDR; lista sem estilo | |
| REP-02 | install/state.rs:9-17 | falhas do estado | `erro no banco de estado: {0}`; `tipo de artefato desconhecido no banco de estado: {0}`; `erro de E/S no estado: {0}`; `o manifesto gravado de {0} é grande demais`; `o manifesto gravado de {0} não confere com o registro` | ERR-01 | |

## 15. Seletores (setas e Enter)

Código: `src/cli/picker.rs` (a única interação do CLI; tudo é seletor, nada é digitado). Desenho inline (sem tela alternativa), cursor oculto; no fim as linhas somem e fica **uma** linha-resumo. Cores do tema TUI (`Theme`): cursor = `accent`, desabilitado e dica = `muted`.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| SEL-01 | picker.rs:436-466 | toda pergunta Sim/Não (`Continuar?`, BKT-03, BKT-06) | linha da pergunta; depois `▶ s) Sim` e `  n) Não` (Não vem selecionado); dica `setas movem · letra ou Enter escolhe · Esc cancela` | pergunta sem estilo; linha atual `accent`; outras sem estilo; dica `muted` | |
| SEL-02 | picker.rs:190-205 | depois de escolher | resumo em uma linha: `{pergunta} {rótulo escolhido}` (ex.: `Continuar? Sim`); sem pergunta: `▶ {rótulo}`; vários: rótulos separados por `, ` ou `nenhum` | sem estilo | |
| SEL-03 | picker.rs:130-178 | escolha de um entre N (alvos ambíguos) | linhas `{▶\|espaço} [{letra}) ]{rótulo}  {detalhe}  ({nota})`; letras só movem o cursor; dica `setas ou letra movem · Enter escolhe · Esc cancela` | linha atual `accent`; itens não selecionáveis `muted` | |
| SEL-04 | picker.rs:130-178 | escolha de um entre poucos (remoção com pastas compartilhadas) | mesmo molde; letra escolhe na hora; dica SEL-01 | idem | |
| SEL-05 | picker.rs:130-178 | vários itens (resíduos, órfãos) | linhas `{▶\|espaço} [x] {rótulo}  {detalhe}  ({nota})` ou `[ ]`; dica `setas movem · Espaço marca · Enter confirma · Esc cancela` | idem | |
| SEL-06 | picker.rs:19 | seletor sem terminal | erro: `seletor exige terminal interativo` | ERR-01 | |
| SEL-07 | picker.rs:190 | resposta `q` ou Esc | cancela; vira `⚠ Cancelado.` (RES-05) ou resultado vazio | — | |
| SEL-08 | install_cmd.rs:894-905 | seletor de resíduos | rótulo UNI-20; detalhe `confiança alta` ou `confiança baixa`; nota `pode ser de outra instalação` (se houver); marcados os `preselected` | ver SEL-05 | |
| SEL-09 | install_cmd.rs:225-257 | falha interna | `o terminal ficou indisponível` | ERR-01 | |

## 16. Erros gerais (stderr) e módulos

`error_report` (`output.rs:161-186`, chamada em `main.rs:43` → `emit.rs:77`) imprime em stderr, uma vez, no fim:

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| ERR-01 | output.rs:161-186 | todo comando que falha (exit 1) | `✗ {o que falhou}` e, se houver, `→ {o que fazer}` | ✗ ERRO; seta DIM; texto sem estilo | |
| ERR-02 | output.rs:176-183 | com `-v` | uma linha `  causa: {causa}` por causa da cadeia | DIM, recuo 2 | |
| ERR-03 | output.rs:181 | sem `-v` e com causas | `→ rode de novo com -v para ver a causa` | seta DIM | |
| ERR-04 | output.rs:143 | `Advice` em log/job | forma de uma linha `{o que}; {o que fazer}` | só arquivo | |
| ERR-05 | main.rs / install_cmd.rs:63 | Ctrl+C | nenhum texto de erro; exit 130 | — | |
| ERR-06 | clap | argumento inválido | mensagem do clap (`error:` vermelho `ERROR`; `valid` verde; `invalid` amarelo) em inglês/padrão do clap | CLAP_STYLES | |
| ERR-07 | mod.rs:38 | `--scope x` | `escopo inválido '{x}': use user ou machine` | via clap | |
| ERR-08 | mod.rs:708 | comando ausente num terminal sem TUI | `TUI ainda não implementada` (caminho legado) | ERR-01 | |
| ERR-09 | mod.rs:917 | install/uninstall/upgrade/cleanup/repair fora do Windows | `instalar e remover apps só funciona no Windows` | ERR-01 | |
| ERR-10 | mod.rs:922, 1042 | uso interno | `bridge-worker fora do lugar`; `auxiliar elevado: {e}` | ERR-01 | |
| ERR-11 | winget_cmd.rs:154 | ponte fora do Windows | `a ponte do winget só funciona no Windows` | ERR-01 | |
| ERR-12 | winget_cmd.rs:179 | catálogo | `pacote não encontrado no catálogo` | ERR-01 | |
| ERR-13 | run_win.rs:316 | atualização | `{nome}: versão instalada desconhecida` | ERR-01 | |
| ERR-14 | http.rs:17-25 | rede | `sem conexão: {0}`; `tempo esgotado`; `HTTP {0}`; `resposta maior que o limite de {0} bytes`; `redirecionamento de https para http recusado: {0} (permita em Ajustes)` | ERR-01 | |
| ERR-15 | cache/mod.rs:12-22 | cache de dados | `diretório de dados indisponível`; `E/S em {path}: {source}`; `hash não confere: esperado {a}, obtido {b}`; `estado HTTP inválido: {0}` | ERR-01 | |
| ERR-16 | config.rs:173-178 | `config.toml` | `não foi possível serializar a configuração: {0}`; `não foi possível ler {path}: {msg}`; `configuração inválida em {path}: {msg}` | ERR-01 | |
| ERR-17 | catalog/mod.rs:137-141, index_file.rs:19-21 | catálogo local | `erro no catálogo local: {0}`; `mapeamento aponta para pacote inexistente: {0}`; `fonte scoop não aceita mapeamentos: {0}`; `índice de busca inválido: {0}`; `E/S no índice de busca: {0}` | ERR-01 | |
| ERR-18 | sources/winget/index.rs:26-41 | índice do winget | `pacote MSIX inválido: {0}`; `arquivo ausente no MSIX: {0}`; `AppxBlockMap inválido: {0}`; `integridade: bloco {n} do índice não confere`; `integridade: hash do arquivo do índice não confere`; `formato de índice v{0} não suportado`; `índice ilegível: {0}`; `E/S: {0}` | ERR-01 | |
| ERR-19 | sources/winget/{mod,manifest,mszip,version_data}.rs | manifestos winget | `manifesto com hash divergente`; `manifesto não é UTF-8`; `manifesto inválido: {0}`; `assinatura MSZIP inválida`; `dados MSZIP truncados`; `bloco MSZIP sem marcador CK`; `deflate inválido: {0}`; `tamanho descomprimido diverge: esperado {a}, obtido {b}`; `versionData não é UTF-8`; `versionData inválido: {0}` | ERR-01 | |
| ERR-20 | install/download.rs:82-90 | download | `hash não confere para {url}`; `E/S: {0}`; `download cancelado` | ERR-01 | |
| ERR-21 | install/executor.rs:230-240 | execução do plano | `falha no passo {i} ({passo}): {mensagem}; desfeito: {lista}`; `{hook}` | ERR-01 | |
| ERR-22 | install/extract.rs:46-68 | extração | `caminho inseguro no arquivo: {0}`; `pasta {0} não existe no arquivo`; `arquivo inválido: {0}`; `formato não suportado pelo extrator: {0}`; `E/S: {0}`; `arquivo descompactado passa de {limit} bytes (possível bomba de descompressão)`; `arquivo com mais de {limit} entradas (possível bomba de descompressão)`; `arquivo 7z protegido por senha`; `método de compressão não suportado: {0}`; `dicionário de {d} bytes passa do limite de {l}`; `falha no decodificador: {0}`; `layout do MSI inválido: {0}` | ERR-01 | |
| ERR-23 | install/msi_layout.rs:39-47 | MSI | `nome inseguro no MSI: {0}`; `diretório pai ausente no MSI: {0}`; `ciclo na tabela Directory do MSI: {0}`; `diretório aninhado demais no MSI: {0}`; `componente sem diretório no MSI: {0}` | ERR-01 | |
| ERR-24 | install/msi_queue.rs:49-53 | fila do msiexec | `o Windows Installer está ocupado por outro programa`; `o msiexec terminou com código {0}` | ERR-01 | |
| ERR-25 | install/ops.rs:199-245 | operações do Scoop | `nenhum pacote encontrado para '{q}'`; `mais de um pacote corresponde: {a, b}`; `não suportado: {0}`; `dependência cíclica: {a -> b}`; `a dependência {app} não pode ser instalada: {motivo}`; `falha ao baixar {app}: {erro}`; `erro no ambiente do usuário: {0}` | ERR-01 | |
| ERR-26 | install/{manifest,shim,runner,script_effects}.rs | manifesto e ambiente | `manifesto inválido: {0}`; `não suportado: {motivo}`; `sem variante para a arquitetura desta máquina`; `lançador de shims indisponível: {0}`; `nome de variável inválido para o PowerShell: {0:?}`; `effects.jsonl: linha {n} inválida`; `effects.jsonl: operação desconhecida na linha {n}`; `effects.jsonl: mais de {N} linhas`; `effects.jsonl: linha {n} passa de {N} bytes` | ERR-01 | |
| ERR-27 | installed/winget_tracking.rs:14-19 | lista de instalados | `catálogo de rastreamento do winget ilegível: {0}`; `catálogo de rastreamento do winget em modo WAL; ignorado para não criar arquivos na pasta do winget` | ERR-01 / log | |
| ERR-28 | winget_install/msix.rs:10 | MSIX | `{texto} (0x{hresult})` | ERR-01 | |
| ERR-29 | icons.rs:14-20 | ícones (só TUI) | `falha ao baixar o ícone: {0}`; `ícone maior que o limite de … bytes`; `o ícone não confere com o hash do manifesto`; `falha no cache do ícone: {0}` | não chega ao CLI | |
| ERR-30 | install/traits.rs:12, executor.rs:239 | efeitos | `{0}` (mensagem repassada) | ERR-01 | |

## 17. Ajuda (clap)

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| AJU-01 | tests/it/snapshots/it__cli_parse__help_*.snap, mod.rs:44-340 | `wingetrs -h`, `wingetrs <cmd> -h` | Ajuda em português por subcomando: `root`, `update`, `search`, `show`, `list`, `install`, `uninstall`, `upgrade`, `bucket`, `repair`, `cleanup` (+ `bucket add\|list\|rm`; `bridge-worker` oculto). Estrutura: sobre, `Uso:`, `Comandos`/`Argumentos`/`Opções`, `Exemplos:` | cabeçalhos HDR; flags e comandos TECLA; placeholders DIM (`style.rs:29-35`) | |
| AJU-02 | style.rs:193-221 | rodapé da ajuda raiz | `Exemplos:` + 7 comandos com notas (`abre a TUI`, `atualiza o catálogo`, `busca pacotes`, `lista pacotes com atualização`, `instala um app do Scoop`, `atualiza tudo que o wingetrs instalou`) | comandos TECLA | |

---

## 18. Só no WIP (não commitado, Tasks 189/208)

Textos novos que existem apenas no working tree; não estão nos números de linha acima.

| ID | Onde | Quando aparece | Texto atual | Estilo atual | Formatação desejada |
|---|---|---|---|---|---|
| WIP-01 | cli/winget_cmd.rs (WIP) | fluxo de teste sem suporte a portable | `este fluxo não instala pacotes portable` / `este fluxo não atualiza pacotes portable` | ERR-01 | |
| WIP-02 | cli/winget_cmd.rs (WIP) | install portable do winget | exibe `Licença: …` (WIN-11 formato `labeled`, sem `Aceita`) e entra no fluxo de Scoop (INS-09…INS-23, com bucket `winget`) | como INS-xx | |
| WIP-03 | install/ops.rs `plan_manifest` (WIP) | portable de bucket diferente | `{app} já está instalado pelo wingetrs a partir do bucket {bucket}` | ERR-01 | |
| WIP-04 | winget_install/engine.rs (WIP) | atualização de portable instalado fora do wingetrs | motivo da ponte `tipo portable instalado fora do wingetrs: atualizado pelo winget` → WIN-21 | DIM | |
| WIP-05 | tui/jobs.rs (WIP) | build sem Windows | `pacotes do winget só no Windows` | ERR-01 | |
| WIP-06 | winget_install/portable.rs:76-225 (arquivo novo) | recusas ao planejar portable | `apelido de comando inválido no manifesto: {a}`; `identificador inválido para uma pasta: {id}`; `versão inválida para uma pasta: {v}`; `URL do instalador recusada: {url}`; `InstallerSha256 ausente ou inválido`; `zip portable sem NestedInstallerFiles no manifesto`; `zip portable com {n} arquivos aninhados (máximo {max})`; `caminho fora do pacote no manifesto: {p}`; `{id} não está no catálogo do winget`; `{id}: pacote winget sem hash de manifesto`; `{id} {versão} não é mais um pacote portable: remova e instale de novo` | ERR-01 | |
| WIP-07 | github/ (arquivo novo) | `bucket add` e consultas | ver BKT-11 | ERR-01 | |
| WIP-08 | tests/it/wi_cli.rs etc. (WIP) | testes | sem texto novo de usuário além do acima | — | |

Não foi possível classificar: a saída literal do `winget`/`scoop` repassada pela ponte (PRG-36), as mensagens dos erros de `clap` (ERR-06, geradas em tempo de execução) e os detalhes de alertas de `script_scan` montados por `line_rules` (CNF-20..39 cobrem as constantes; os restantes são montados em `script_scan.rs:673-980`).
