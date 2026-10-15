Status: DONE_WITH_CONCERNS

Commits:
fd0f938 fix(bridge): keep each reader's pipe open until its thread is joined, even after a panic
8595113 fix(bridge): stop reading two seconds after the tool exits when a grandchild holds its output

Gate:
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 15s
[OK]   test (847 passed, 22 ignored) 224s
[OK]   gate verde

`git diff --stat Cargo.lock`: vazio (sem saída).

Antes da correção (stub `run_with_grace` que ignora a carência), teste 1:
```
thread 'grandchild_holding_pipe_does_not_hang' (10596) panicked at tests\bridge.rs:388:28:
bridge still running after 5.0081013s
test result: FAILED. 0 passed; 1 failed; ... finished in 8.25s
```

DESVIO 1: `CancelSynchronousIo` não funciona aqui. Com a implementação literal do brief, o teste 1
continuou em timeout (`bridge still running after 5.004145s`), e instrumentando a chamada, 158 de 158
chamadas devolveram `HRESULT(0x80070490)` (ERROR_NOT_FOUND). Causa: o std lê `ChildStdout`/`ChildStderr`
com `ReadFileEx` sobreposto + `SleepEx(INFINITE, TRUE)`
(`library/std/src/sys/process/windows/child_pipe.rs:243-250, 383-405`), então nunca há E/S síncrona para
cancelar. Substituto: `CancelIoEx(handle do pipe, None)`, que está na mesma feature `Win32_System_IO`.
A leitura termina com 995 e a flag marcada. Para o handle não ser reaproveitado entre o `is_finished()`
e o cancelamento, a thread leitora devolve o pipe junto com o resultado, e ele só fecha no `join`.
O orquestrador foi avisado por mensagem durante a tarefa. O resto segue o brief: flag antes de cada
`read`, nova tentativa a cada 50 ms e erro ignorado.

DESVIO 2: o script literal do teste 2 (`start /b cmd /d /c \"ping ... & echo tarde\" & exit /b 0`)
quebra porque o Rust escapa o `"` interno como `\"` e o cmd não entende. Saída real:
`'\"ping -n 2 127.0.0.1 >nul & echo tarde\"' não é reconhecido como um comando interno`. Troquei por
circunflexos: `start /b cmd /d /c ping -n 2 127.0.0.1 ^>nul ^& echo tarde& exit /b 0`. As asserções
não mudaram.

Testes novos: `grandchild_holding_pipe_does_not_hang`, `grace_lets_late_output_arrive`,
`panic_in_on_line_with_grandchild_does_not_hang`, `chatty_grandchild_does_not_delay_the_cut`
(em `tests/bridge.rs`) e `stop_ignores_finished_readers` (unitário em `src/bridge/spawn_win.rs`).
`spawn_runs_fake_tool` não mudou e passa.

Mutation proofs:
1. Leitor sem conferir a flag + `stop()` com um cancelamento só (é medição: as duas mutações aplicadas
   juntas à mão, já que o `mutate.ps1` aceita uma âncora só; arquivo restaurado depois). Teste 1 rodado 10x:
   `passou 10/10`. A repetição não é necessária neste cenário: o `ping` fica quase sempre dentro de uma
   leitura pendente, então o primeiro `CancelIoEx` já pega. A repetição cobre só a janela estreita entre
   duas leituras, que o teste não exercita.
2. `try_wait` só no ramo de timeout. Mutante: `continue;` no ramo `Ok(line)`, que pula `try_wait` e o
   corte quando chega linha. Gerador: `for /l %i in (1,1,150000) do @echo x` no neto. Medido: 150000
   linhas em 11,39 s, maior intervalo 130 ms, um único intervalo de 100 ms ou mais (na partida). Com
   60000 linhas (4,68 s) o mutante sobrevivia porque o neto terminava antes do limite de 5 s.
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor "<tail.add(line);\n }\n Err(RecvTimeoutError::Timeout) => {}>" -Replacement "<... tail.add(line);\n continue;\n } ...>" -TestCommand 'cargo test --quiet --test bridge chatty'`
   `[OK] mutação PEGA: '                tail.add(line);` (teste falhou: `finished in 12.12s`)
3. `Drop` sem `stop()`:
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor '        self.stop();' -Replacement '' -TestCommand 'cargo test --quiet --test bridge panic_in_on_line'`
   `[OK] mutação PEGA: '        self.stop();' -> '' fez o teste falhar (exit 101).`
4. Código vindo da carência (`-1`):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor 'Some((_, code)) => code,' -Replacement 'Some((_, _)) => -1,' -TestCommand 'cargo test --quiet --test bridge grandchild_holding'`
   `[OK] mutação PEGA: 'Some((_, code)) => code,' -> 'Some((_, _)) => -1,' fez o teste falhar (exit 101).`
   Extra (carência ignorada, corte imediato):
   `pwsh -File scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor '&& at.elapsed() >= grace' -Replacement '&& at.elapsed() >= Duration::ZERO' -TestCommand 'cargo test --quiet --test bridge grace_lets'`
   `[OK] mutação PEGA: '&& at.elapsed() >= grace' -> '&& at.elapsed() >= Duration::ZERO' fez o teste falhar (exit 101).`
5. Sobras:
   `pwsh -File scripts/leftovers.ps1` -> `[OK] nenhum processo de agente sobrando (> 2 min)`.
   Além disso, contei os `cmd.exe`/`PING.EXE` com `for /l` ou `127.0.0.1` na linha de comando, de 2 em 2 s
   depois do teste `chatty`: `nossos=0` de t=0 até t=10s. O neto morre logo depois do corte (escrita em
   pipe fechado). Também `nossos=0` no fim.
6. Nada ao vivo: nem o winget nem o scoop foram executados.

Decisions:
- `CancelIoEx` no handle do pipe em vez de `CancelSynchronousIo` no handle da thread (ver DESVIO 1).
- O leitor devolve o pipe (`Box<dyn Send>`) junto com o resultado, para que o handle passado ao
  `CancelIoEx` esteja sempre aberto.
- Quando o filho já foi colhido por `try_wait`, o código vem dele, sem chamar `child.wait()` de novo.
- `stop_ignores_finished_readers` usa `File` de arquivo temporário como "pipe" (`Read + AsRawHandle`).
- O laço só faz o corte uma vez (`cut`): `warn!` uma vez e depois drena até `Disconnected`.

Concerns:
- O mecanismo do brief (`CancelSynchronousIo`) foi trocado; o revisor deve conferir o `CancelIoEx` e o
  argumento de vida do handle.
- Erro 995 sem a flag continua sendo erro, mas nenhum teste cobre isso (não há como provocar um 995 sem
  `stop()` num teste unitário sem rede).
- A repetição a cada 50 ms não é exercitada por nenhum teste (medição 1: 10/10 passa sem ela).

Correção da revisão (fd0f938): se o leitor entrasse em pânico, o unwind fechava o pipe antes de
`is_finished()` virar true, e o `CancelIoEx` podia pegar um handle já fechado ou reaproveitado.
Agora o pipe fica num `Arc<Mutex<_>>`. A thread trava esse mutex (é a única que trava), e o slot
`Reader` guarda um clone (`_pipe`). O handle só fecha quando o slot cai, sempre depois do `join`.
O comentário SAFETY foi corrigido para essa invariante. Gate depois da correção:
[OK]   clippy 12s
[OK]   test (847 passed, 22 ignored) 382s
[OK]   gate verde
Mutações refeitas depois da correção:
[OK] mutação PEGA: '        self.stop();' -> '' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'Some((_, code)) => code,' -> 'Some((_, _)) => -1,' fez o teste falhar (exit 101).
Sobras: nenhum `cmd`/`ping` nosso vivo (`nossos=0`).
