#### Execution rules

- Leia `CLAUDE.md` (regras do projeto, comandos proibidos, economia de contexto). Não leia o plano nem a spec inteiros; este brief é autocontido.
- TDD: escreva o teste, rode e veja falhar pelo motivo certo, implemente, rode e veja passar.
- **Ciclo rápido:** durante o TDD e as mutações rode só o módulo de teste da tarefa (`cargo test --quiet --test it <modulo>::`; os testes de integração ficam em `tests/it/<modulo>.rs`, num binário só; exceção: `cargo test --quiet --test install_extract`), nunca a suíte toda. Medido na Task 133 depois de mexer em `src/`: compilar o `it` ~15 s; compilar todos os alvos ~18 s; passo `test` do gate ~34 s (inclui o link). O gate completo roda **uma vez**, antes do commit final. Arquivo novo em `tests/it/` só compila com `mod <modulo>;` no `tests/it/main.rs` (o `check_rules` recusa se faltar); a linha já vem criada pelo orquestrador: não edite o `main.rs`.
- Gate antes de cada commit: `pwsh -File scripts/verify.ps1` deve terminar com `[OK]   gate verde`. Se demorar, espere ativamente (um laço que confere o arquivo de saída até aparecer o exit) antes de encerrar o turno — não encerre esperando notificação.
- Commits pequenos, mensagem em inglês no formato `feat(<módulo>): ...`/`test(...)`/`fix(...)`, terminando com a linha `Co-Authored-By: Claude <noreply@anthropic.com>`. Nunca `git push`.
- Não adicione dependências fora das listadas no brief. Não edite arquivos fora da lista **Files** sem registrar o motivo no relatório.
- **Cada regra listada em "Checks beyond the steps" ou "The decision this task must get right" precisa da sua prova de mutação colada.** Se você pular uma, diga qual e por quê — o orquestrador vai rodá-la.
- **Desempenho:** spans antes de otimizar; benchmark sempre com `-Baseline target/release/examples/noop.exe`; número decisivo ⇒ compare A/B contra a base (worktree temporário **com o mesmo `target/`** via `CARGO_TARGET_DIR`, removido no fim). Otimização sem ganho medido é revertida.
- **Threads/pânico:** toda thread que você criar é juntada em todos os caminhos; `join()` nunca termina em `.ok()`/`if let Ok`/`unwrap_or_default` (o gate barra). Veja `docs/ARMADILHAS.md`.
- Nunca varra o disco inteiro (`find /`, `Get-ChildItem C:\ -Recurse`); procure só no repositório e em caminhos conhecidos. Encerre todo processo que você iniciou em segundo plano (inclusive laços `until … sleep`) antes de responder; confira com `pwsh -File scripts/leftovers.ps1`.
- Se algo do brief não existir ou contradizer o código real: pare, registre `BLOCKED: <motivo com evidência>` no relatório. Não invente uma alternativa silenciosa.

#### Autorrevisão antes de entregar

Os achados que mais voltaram da revisão no lote 2b (Tasks 116–128); confira cada um no seu diff antes de reportar:
- **Cada recusa tem uma fixture que falha só nela** (anti-item, limite de entradas, cada formato de dicionário, sobreposição, "terminou com"...). Apague a linha da checagem de cabeça: algum teste cai? Se não, falta fixture.
- **Todo número lido da entrada** (tamanho, offset, dicionário, contagem, tamanho de cabeçalho) tem teto **antes** de alocar ou decodificar, inclusive em caminhos secundários (candidatos SFX, cabeçalho compactado, saltos entre arquivos).
- **Código de terceiros que lê dado externo** (abrir **e** ler) fica dentro do `catch_decoder`/`catch_unwind` do D4-1.
- **"Nada gravado"** é provado olhando uma pasta que existiria se algo tivesse sido gravado, não uma que nunca existe.
- **Cada braço de `match`/`if`** tem um teste que o alcança, nas duas ordens quando a ordem importa.
- **Laço de nova tentativa** (Task 136): todo estado por tentativa (resultado, código, linhas, motivo da falha) é zerado no início de cada iteração, e há um teste em que a 2ª tentativa falha por **outro** motivo que a 1ª.
- **Versão nunca comparada como texto** (Task 134): use `crate::core::version`, com testes "maior", "menor" e "mesma com zeros a mais".
- **Teste de "nunca faz X" cuja regressão seria esperar** (Task 140): sem prazo, o mutante só fica lento (600 s de `wait_max`) e o teste passa. Envolva em `tokio::time::timeout`/prazo curto e exija o resultado; a mutação tem que falhar rápido.
- **Código nunca se molda à âncora do brief** (Task 143): `#[rustfmt::skip]` ou linha artificial só para o texto da mutação casar é proibido. Formate normalmente e use `-AnchorFile`/`-ReplacementFile` com o trecho real; se o brief citar texto que mudou, diga no relatório.
- **Recusa nova em extrator/parser** (Task 138): rode contra o corpus real da sonda do formato (ou peça a sonda) antes de entregar; fixture sintética não prova que arquivos reais passam.

#### Report contract

Grave `.claude/tasks/task-{N}-report.md` com, nesta ordem:

1. `Status: DONE | DONE_WITH_CONCERNS | BLOCKED`
2. `Commits:` só os **seus** SHAs, colados da saída real de `git log --format='%h %s' <base>..HEAD -- <arquivos da lista Files>` (com outros agentes no mesmo checkout, `<base>..HEAD` sozinho traz commits alheios). Nunca um comando ou texto no lugar dos SHAs.
3. `Gate:` as linhas finais reais de `scripts/verify.ps1` (copiadas, não resumidas).
4. `Mutation proofs:` para cada regra listada em "Checks beyond the steps", o comando `scripts/mutate.ps1` executado e a linha de saída real (`[OK] mutação PEGA...`). Uma regra sem saída colada conta como não verificada. Se a tarefa declarar "no mutation proof", escreva isso explicitamente.
5. `Decisions:` escolhas que o brief não fixou, com o motivo (máx. 5 linhas).
6. `Concerns:` o que ficou de fora ou parece errado (ou `none`).

Não conta como entrega: "tests pass" sem saída colada; prova de mutação sem a linha `[OK] mutação PEGA`; número que não foi medido (escreva "não medido").

A mensagem final ao orquestrador: no máximo 10 linhas — status, SHAs, caminho do relatório.
- **Disco:** nunca crie worktree ou `target/` separado para compilar ou rodar o gate (cada `target` de debug passa de 10 GB e o disco encheu em 2026-10-16). Se o checkout não compila por causa de arquivo de outro agente, espere ou rode só os seus testes e diga no relatório; o orquestrador roda o gate. Sonda que baixa corpus apaga os downloads e as árvores extraídas no fim, mantendo só os vetores que o relatório cita.
