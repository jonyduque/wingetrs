# Armadilhas medidas (wingetrs)

Cada item aconteceu de verdade neste projeto e tem evidência no ledger (`.claude/tasks/progress.md`) ou nos relatórios de tarefa. Cite o item pelo título ao escrever um brief que toque o assunto.

## Testes e provas

- **Teste que não consegue falhar.** Em 9 das 23 tarefas a mutação do revisor sobreviveu a testes que "cobriam" a regra (help raiz que nunca lista flags de subcomando; `starts_with` que não distingue ShortDescription de Description; trailer do `search.idx` só exercitado por tamanho; `assert!(path.exists())` quando o próprio código cria o arquivo). Toda regra precisa da sua mutação colada, e o revisor roda uma mutação **de outra forma** contra a suíte.
- **Cada checagem de validação precisa de uma fixture que falhe só nela.** O `FileHash` do BlockMap nunca era checado (procurava atributo, o XML tem elemento filho) e nenhum teste percebeu até existir uma fixture com blocos certos e FileHash errado.
- **Mutação equivalente existe.** Ramo inalcançável (`winners.next().is_none().then_some(first)`) sobrevive sem ser lacuna de teste: simplifique o código em vez de inventar teste.
- **Corpo literal de teste no plano também passa pelo clippy.** `nonminimal_bool`, `double_comparisons`, `manual_is_multiple_of`, `assertions_on_constants` dispararam em testes transcritos. Ao escrever o plano, compile os testes num rascunho ou já escreva na forma aceita.
- **wiremock 0.6 divide valores de header por vírgula.** `header("If-Modified-Since", "Wed, 07 Oct …")` nunca casa; use `headers(name, vec!["Wed", "07 Oct …"])`.

- **Instrumento que para de vigiar em silêncio.** `check_rules.ps1` com `StrictMode`: `Get-Content` de arquivo de **uma linha** devolve string, `.Count` falhava, o arquivo era pulado e o script saía 0. Achado pelo revisor da Task 27 (stubs `//!`). Correção: `@(Get-Content …)` + `$ErrorActionPreference = 'Stop'`; provado plantando violação num arquivo de uma linha. Todo instrumento novo: prove com violação plantada **no menor caso** (arquivo vazio, uma linha).

## Concorrência

- **Thread antecipada órfã** (Tasks 18 e 20): `spawn` antes de um `?` sem `join` em todos os caminhos. Use guard com `Drop` ou junte antes do `?`.
- **Pânico engolido** (`join().ok()`, `if let Ok(..) = h.join()`, `unwrap_or_default`) devolve resultado parcial em silêncio. `scripts/check_rules.ps1` (no gate) barra os padrões conhecidos.
- **Handle de `spawn_blocking` descartado** faz o tokio engolir o pânico (a TUI nunca carregava os instalados).

## Desempenho (Windows, esta máquina)

- **Piso de processo varia de 38 a 300 ms com a carga da máquina.** Compare só deltas: `bench.ps1 -Baseline target/release/examples/noop.exe` (execuções intercaladas). Quando o número for decisivo, meça A/B contra a base num worktree temporário.
- **Palpite de gargalo falha.** Reduzir chamadas `DisplayName` do MSIX não teve ganho medido (revertido); o span mostrou outro lugar. Spans primeiro, otimização depois.
- **Alocação domina leitura em massa.** Ler 19,5 mil linhas como `&str`: 27 ms; copiando para `String`: 78 ms; o heap padrão do Windows serializa threads (8 threads só levaram a 42 ms).
- **Abrir conexão SQLite + primeira consulta custa 11–15 ms** (medido também em Python) e 8 conexões paralelas competem no `prepare` (4 → 16–20 ms). Para leitura quente, prefira um arquivo próprio lido de uma vez (`search.idx`).
- **`COUNT(*)` percorre a tabela**; para "está vazio?" use `EXISTS`. `Catalog::open` roda DDL do schema; para leitura use `open_read_only`.
- **Processos órfãos de agentes distorcem medições.** Rode `scripts/leftovers.ps1` antes de medir e depois de cada tarefa.

## Dados do mundo real

- **Pacotes MSIX "sparse"** (VS Code, Zed, Edge: `SignatureKind=Developer`, versão sintética 1.0.0.0) casavam por nome+publisher e viravam falso "atualização disponível". O winget só correlaciona MSIX por PFN.
- **Frameworks MSIX** (VCLibs, UI.Xaml, WindowsAppRuntime) são listados pelo winget; vários pacotes por família (arquiteturas e versões lado a lado) — uma linha por família, maior versão.
- **ProductCode compartilhado entre canais** (Android Studio/Beta/Canary, WezTerm/nightly): o desempate decisivo é o catálogo de rastreamento do próprio winget (`installed.db`), lido só para leitura; faixa ARP só vale quando todos os candidatos têm faixa (a 1ª versão errou 2 pacotes).
- **Banco SQLite de terceiros em modo WAL**: abrir só-leitura pode criar `-shm`/`-wal` na pasta dele. Leia o byte 18 do cabeçalho (2 = WAL) e não abra.
- **Versões que parecem número no YAML** (`1.10`) ficam texto com `serde-saphyr` (verificado); não troque de crate sem repetir a sonda.
- **MSZIP**: cada bloco usa os últimos 32 KiB já descomprimidos como dicionário; sem isso o 2º bloco falha.

## Ambiente e repositório

- **`core.autocrlf=true`** converteria fixtures de texto e quebraria comparações de hash: `.gitattributes` marca `tests/fixtures/** -text`.
- **`find /` no Git Bash varre o disco inteiro** (inclusive `C:\Program Files\Git`); proibido no CLAUDE.md.
- **O modo automático bloqueia encerrar processos** sem pedido explícito do usuário: liste com `leftovers.ps1` e pergunte.
- **Revisor também erra.** Um achado de "fixture sem CRLF" era falso (180 `\r`, SHA conferido): recuse com a evidência, não implemente por segurança.
