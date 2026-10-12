# Armadilhas medidas (wingetrs)

Cada item aconteceu de verdade neste projeto e tem evidência no ledger (`.claude/tasks/progress.md`) ou nos relatórios de tarefa. Cite o item pelo título ao escrever um brief que toque o assunto.

## Testes e provas

- **Teste que não consegue falhar.** Em 9 das 23 tarefas a mutação do revisor sobreviveu a testes que "cobriam" a regra (help raiz que nunca lista flags de subcomando; `starts_with` que não distingue ShortDescription de Description; trailer do `search.idx` só exercitado por tamanho; `assert!(path.exists())` quando o próprio código cria o arquivo). Toda regra precisa da sua mutação colada, e o revisor roda uma mutação **de outra forma** contra a suíte.
- **Cada checagem de validação precisa de uma fixture que falhe só nela.** O `FileHash` do BlockMap nunca era checado (procurava atributo, o XML tem elemento filho) e nenhum teste percebeu até existir uma fixture com blocos certos e FileHash errado.
- **Mutação equivalente existe.** Ramo inalcançável (`winners.next().is_none().then_some(first)`) sobrevive sem ser lacuna de teste: simplifique o código em vez de inventar teste.
- **Corpo literal de teste no plano também passa pelo clippy.** `nonminimal_bool`, `double_comparisons`, `manual_is_multiple_of`, `assertions_on_constants` dispararam em testes transcritos. Ao escrever o plano, compile os testes num rascunho ou já escreva na forma aceita.
- **Caso degenerado esconde a regra.** Task 41: alvos = "último plano de cada grupo", testado só com grupos de 1 plano — `first()` e `last()` coincidem e a mutação sobreviveu. Ao escrever teste de regra sobre posição/ordem/conjunto, use entradas onde as alternativas erradas dão resultado diferente (grupo com dependência, lista com ≥ 2 itens fora de ordem).
- **Span criado não é span medido.** Task 45: o teste contava spans fechados; `let _x = debug_span!(..)` sem entrar, ou `.instrument(Span::none())`, fecha o span com duração ~0 e passava. Teste de instrumentação conta só spans entrados (`on_enter`) e confere o pai (etapa dentro da operação).
- **Teste com limiar de relógio quebra o gate sob carga.** Quatro gates vermelhos num dia com agentes compilando em paralelo (`elapsed() < 5 s`, `< 400 ms`…). Prove ordem, concorrência ou contagem (barreira, chegadas, quem soltou o lock primeiro); limiar só como detector de travamento, com margem ≥ 10×.
- **Mutação "PEGA" por falha de build.** Com o `wingetrs.exe` de debug travado (TUI do usuário aberta), o `cargo test` do `mutate.ps1` sai 101 por "failed to remove file" e conta como mutação pega (Task 68). Rode com `CARGO_TARGET_DIR` separado e confira na saída o nome do teste que falhou.
- **Gate em paralelo com a mutação do revisor.** O `mutate.ps1` edita o arquivo na árvore compartilhada; um `verify.ps1` rodando ao mesmo tempo compila a mutação e fica vermelho (Task 71: 3 testes em `tests/catalog.rs:522`; antes, um fmt). O gate do orquestrador roda antes ou depois do revisor, nunca junto.
- **wiremock 0.6 divide valores de header por vírgula.** `header("If-Modified-Since", "Wed, 07 Oct …")` nunca casa; use `headers(name, vec!["Wed", "07 Oct …"])`.

- **Instrumento que para de vigiar em silêncio.** `check_rules.ps1` com `StrictMode`: `Get-Content` de arquivo de **uma linha** devolve string, `.Count` falhava, o arquivo era pulado e o script saía 0. Achado pelo revisor da Task 27 (stubs `//!`). Correção: `@(Get-Content …)` + `$ErrorActionPreference = 'Stop'`; provado plantando violação num arquivo de uma linha. Todo instrumento novo: prove com violação plantada **no menor caso** (arquivo vazio, uma linha).

## Concorrência

- **Poll com waker falso perde o despertar.** Task 53: `now_or_never()` num `EventStream` do crossterm registra um waker noop, e o crossterm só guarda um waker até ser acordado ⇒ a próxima tecla acorda ninguém e a TUI congela. Teste com stream falso não pega: o falso precisa imitar o comportamento da biblioteca real. Drene sempre com o `cx` da tarefa (`poll_fn`).
- **Thread antecipada órfã** (Tasks 18 e 20): `spawn` antes de um `?` sem `join` em todos os caminhos. Use guard com `Drop` ou junte antes do `?`.
- **Pânico engolido** (`join().ok()`, `if let Ok(..) = h.join()`, `unwrap_or_default`) devolve resultado parcial em silêncio. `scripts/check_rules.ps1` (no gate) barra os padrões conhecidos.
- **Teste concorrente que trava em vez de falhar.** Task 79: o escritor entrava em pânico (`unwrap()` de um `Err`) dentro de `thread::scope`; o scope esperava o leitor, que só parava com uma flag ligada no sucesso ⇒ travou 2/120 rodadas e o erro real ficou escondido. Sinal de parada vai num guard com `Drop` (liga em Ok, Err e pânico), e teste com laço concorrente tem detector de travamento.
- **Handle de `spawn_blocking` descartado** faz o tokio engolir o pânico (a TUI nunca carregava os instalados).

## Desempenho (Windows, esta máquina)

- **Piso de processo varia de 38 a 300 ms com a carga da máquina.** Compare só deltas: `bench.ps1 -Baseline target/release/examples/noop.exe` (execuções intercaladas). Quando o número for decisivo, meça A/B contra a base num worktree temporário.
- **Palpite de gargalo falha.** Reduzir chamadas `DisplayName` do MSIX não teve ganho medido (revertido); o span mostrou outro lugar. Spans primeiro, otimização depois.
- **Alocação domina leitura em massa.** Ler 19,5 mil linhas como `&str`: 27 ms; copiando para `String`: 78 ms; o heap padrão do Windows serializa threads (8 threads só levaram a 42 ms).
- **Abrir conexão SQLite + primeira consulta custa 11–15 ms** (medido também em Python) e 8 conexões paralelas competem no `prepare` (4 → 16–20 ms). Para leitura quente, prefira um arquivo próprio lido de uma vez (`search.idx`).
- **`COUNT(*)` percorre a tabela**; para "está vazio?" use `EXISTS`. `Catalog::open` roda DDL do schema; para leitura use `open_read_only`.
- **`WM_SETTINGCHANGE` custa meio segundo.** `SendMessageTimeout(HWND_BROADCAST, …, "Environment")` mediu 438–568 ms (2,5 s com uma janela lenta) e é metade do `install` real com cache (~1 s); benchmarks com `WINGETRS_TEST_ENV_KEY` não fazem broadcast e mostram ~0,5 s. Faça no máximo um por operação em lote.
- **Leitura paralela travando dentro do `update`.** Task 54: 8 conexões só-leitura no mesmo processo que acabou de parsear e gravar o catálogo travaram juntas por 0,4–1 s (sem busy handler, 1ª linha em < 0,5 ms; mmap, WAL e espera de 2 s não explicam); uma conexão só: 55–65 ms, nunca travou. Paralelismo que ganha no harness precisa ser medido no processo real.
- **Processos órfãos de agentes distorcem medições.** Rode `scripts/leftovers.ps1` antes de medir e depois de cada tarefa.

- **Caminho do manifesto é dado externo.** `extract_to`/`extract_dir` (Task 26) e `persist`/`env_add_path`/`bin`/`shortcuts` (Task 28) aceitavam `..` e absolutos; o scoop recusa `env_add_path` fora do app ("outside of app directory") e ignora entradas vazias. Regra: tudo passa por `install::safepath`.
- **Regra listada no brief sem teste nomeado vira regra sem teste.** Tasks 24, 28, 29 e 36: o implementador testou só as fixtures e as regras de "What binds" ficaram descobertas (mutações sobreviveram). Ao escrever plano: cada regra de "What binds"/"Decision" tem o seu teste (literal ou descrito com entrada e saída).

## Dados do mundo real

- **Pacotes MSIX "sparse"** (VS Code, Zed, Edge: `SignatureKind=Developer`, versão sintética 1.0.0.0) casavam por nome+publisher e viravam falso "atualização disponível". O winget só correlaciona MSIX por PFN.
- **Frameworks MSIX** (VCLibs, UI.Xaml, WindowsAppRuntime) são listados pelo winget; vários pacotes por família (arquiteturas e versões lado a lado) — uma linha por família, maior versão.
- **ProductCode compartilhado entre canais** (Android Studio/Beta/Canary, WezTerm/nightly): o desempate decisivo é o catálogo de rastreamento do próprio winget (`installed.db`), lido só para leitura; faixa ARP só vale quando todos os candidatos têm faixa (a 1ª versão errou 2 pacotes).
- **Banco SQLite de terceiros em modo WAL**: abrir só-leitura pode criar `-shm`/`-wal` na pasta dele. Leia o byte 18 do cabeçalho (2 = WAL) e não abra.
- **Versões que parecem número no YAML** (`1.10`) ficam texto com `serde-saphyr` (verificado); não troque de crate sem repetir a sonda.
- **MSZIP**: cada bloco usa os últimos 32 KiB já descomprimidos como dicionário; sem isso o 2º bloco falha.

## Ambiente e repositório

- **Apagar por caminho interpolado.** Task 72: script gerado com `Remove-Item "$root\$Name\*"` e `$Name` vazio apagou o scratchpad inteiro da sessão. Antes de apagar, confira que cada parte do caminho é não vazia (`if (-not $Name) { throw }`) e que o alvo fica dentro de uma pasta criada pelo próprio script.
- **`core.autocrlf=true`** converteria fixtures de texto e quebraria comparações de hash: `.gitattributes` marca `tests/fixtures/** -text`.
- **`find /` no Git Bash varre o disco inteiro** (inclusive `C:\Program Files\Git`); proibido no CLAUDE.md.
- **O modo automático bloqueia encerrar processos** sem pedido explícito do usuário: liste com `leftovers.ps1` e pergunte.
- **SHA relatado que existe mas é de outro agente.** Task 58: com agentes em paralelo, o `git commit` falhou em silêncio (erro suprimido, arquivo untracked na lista) e o implementador relatou o SHA do commit de outro agente; `git cat-file -t` dizia `commit`. Audite com `git show --stat <sha>` (os arquivos são os da tarefa?) e `git status --short` (nada da tarefa sobrando na árvore).
- **Revisor também erra.** Um achado de "fixture sem CRLF" era falso (180 `\r`, SHA conferido): recuse com a evidência, não implemente por segurança.
