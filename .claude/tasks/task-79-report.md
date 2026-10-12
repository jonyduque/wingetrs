Status: DONE_WITH_CONCERNS

Commits:
```
d054fea fix(test): stop the index reader on every writer exit so a failed rewrite fails instead of hanging
(este relatório: commit seguinte)
```
Base: 84cc6eb. `CARGO_TARGET_DIR=target/t79`. Só `tests/index_file.rs` mudou; nenhum código de produção.

## Veredito

O travamento é do **próprio teste** (H4, com o mecanismo de H1): qualquer `Err` de `index_file::write` vira pânico no `unwrap()` do escritor **dentro de `std::thread::scope`**. O `scope` junta todas as threads antes de propagar o pânico, e o leitor só para quando `stop` é ligado, o que só acontecia no caminho de sucesso. Resultado: o leitor fica em laço para sempre, a thread principal espera no join do `scope`, e a mensagem do pânico fica presa na captura do libtest (por isso a Task 78 não viu nenhum os error 5/32).

Correção: o laço do escritor foi para `rewrite_while_reading`, e um guard `StopOnDrop` liga `stop` em todo caminho de saída (retorno, `?`, pânico). O erro de escrita agora volta como `Err` e o teste falha com a mensagem, sem travar. Novo teste determinístico `a_failed_rewrite_stops_the_reader_and_returns_the_error`: um handle sem compartilhamento em `search.idx.tmp` antes da 4ª reescrita (como um scanner) faz `write` falhar com os error 32. Ele usa um detector de travamento de 60 s, contra < 1 s de uma execução normal.

## Contagens

Harness: `<scratchpad>\t79\loop.ps1` (Start-Process, `--test-threads=1 --nocapture`, timeout por execução de 30 s para o teste isolado e 60 s para o binário inteiro, contra ~0,3 s e ~1,4 s/3,4 s normais, ou seja, ≥ 17× de margem; no timeout, registra o estado das threads e mata o processo).

| Binário | Modo | Execuções | Travamentos |
|---|---|---|---|
| base release, teste isolado | quieto (rel + dbg em paralelo) | 1000 | 0 |
| base debug, teste isolado | idem | 1000 | 0 |
| base release, binário inteiro | quieto, 1 laço | 600 | 0 |
| base release, teste isolado | carga, 4 laços em paralelo | 6000 | 0 |
| **base com falha injetada** (`tests/t79_diag.rs`, temporário, apagado) | quieto | 3 | **3** |
| corrigido release, binário inteiro | rel + dbg em paralelo | 600 | 0 |
| corrigido debug, binário inteiro | idem | 400 | 0 |

O travamento natural **não se reproduziu** aqui (0 em 8600 execuções da base). A Task 78 teve 2/120. O corrigido teve 0 em 1000 execuções do binário inteiro (≥ 3× as 120 rodadas que travaram na Task 78, e ≥ 8×). Com a correção, uma recorrência não trava mais: ela aparece como falha com a mensagem de erro do `write`.

## Evidência (falha injetada no corpo da base)

Estado das threads no timeout (o mesmo padrão da Task 78: uma thread rodando, as demais esperando):
```
HANG r1 pid=29268 threads: 37468:Wait:Unknown:cpu=16 3808:Wait:UserRequest:cpu=0 32508:Wait:Unknown:cpu=47 16080:Running::cpu=29156
HANG r2 pid=33900 threads: 36488:Wait:Unknown:cpu=16 28624:Wait:UserRequest:cpu=0 1868:Wait:Unknown:cpu=47 31936:Running::cpu=29391
HANG r3 pid=33796 threads: 24732:Wait:Unknown:cpu=16 29320:Wait:UserRequest:cpu=0 36188:Wait:Unknown:cpu=31 23640:Running::cpu=29328
```
Rastro em stderr (`--nocapture`). `[t79] reader saw stop` nunca aparece:
```
[t79] write 0
[t79] write 1
[t79] write 2
[t79] write 3
thread 'base_body_with_a_failed_third_write' (32508) panicked at tests\t79_diag.rs:55:54:
called `Result::unwrap()` on an `Err` value: Io("O arquivo já está sendo usado por outro processo. (os error 32)")
```
A thread 32508 (a do teste, que fazia as escritas) entrou em pânico e ficou em `Wait` no join do `scope`. A 16080 é o leitor, com 29 s de CPU. Sem `cdb`/`procdump` na máquina, não houve dump de pilha.

## Hipóteses

- **H1 (laço/parada do teste): confirmada em parte.** `stop` só era ligado no caminho de sucesso. A ordenação `Relaxed` do `AtomicBool` não é problema: a visibilidade final é garantida e o laço só precisa ver o valor em algum momento.
- **H2 (retry sem limite em `persist` ou espera por algo do leitor): descartada.** `persist` tenta 5× e espera 4×20 ms; `read_retrying_with` tenta 5×10 ms; `fs_retry::retry_with` itera sobre `DELAYS`, que é finito. Nenhum laço é ilimitado. O leitor não segura nada entre leituras (`std::fs::read` abre e fecha).
- **H3 (rename do Windows bloqueia em vez de falhar): sem evidência.** Na injeção, o `write` falha na hora (os error 32), não bloqueia. Nas 8600 execuções nada travou dentro do `write`. Falta a pilha da ocorrência original, então não dá para descartar de forma absoluta, mas o padrão observado na Task 78 é idêntico ao da injeção.
- **H4 (deadlock do harness: pânico numa thread com a outra esperando sem timeout): confirmada** pela injeção (3/3), com a impressão digital da Task 78.

## Mutações (código final, `CARGO_TARGET_DIR=target/t79`)
```
[OK] mutação PEGA: 'self.0.store(true, std::sync::atomic::Ordering::Relaxed);' -> 'let _ = self.0;' fez o teste falhar (exit 101).
  test a_failed_rewrite_stops_the_reader_and_returns_the_error has been running for over 60 seconds
  a_failed_rewrite_stops_the_reader_and_returns_the_error --- FAILED
  thread 'a_failed_rewrite_stops_the_reader_and_returns_the_error' (24044) panicked at tests\index_file.rs:264:9:
  a failed rewrite left the reader looping (hang)
[OK] mutação PEGA: '            write()' -> '            let r = write(); if r.is_ok() { stop.store(true, Ordering::Relaxed); } std::mem::forget(_stop); r' fez o teste falhar (exit 101).
  .test a_failed_rewrite_stops_the_reader_and_returns_the_error has been running for over 60 seconds
  a_failed_rewrite_stops_the_reader_and_returns_the_error --- FAILED
  thread 'a_failed_rewrite_stops_the_reader_and_returns_the_error' (35156) panicked at tests\index_file.rs:264:9:
  a failed rewrite left the reader looping (hang)
[OK] mutação PEGA: 'if n == 3 {' -> 'if n == 99 {' fez o teste falhar (exit 101).
  a_failed_rewrite_stops_the_reader_and_returns_the_error --- FAILED
  thread '<unnamed>' (20256) panicked at tests\index_file.rs:259:9:
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.13s
```
- 1ª: o guard nunca liga `stop`.
- 2ª: reproduz a base, com `stop` ligado só no sucesso. O `.` no início mostra que `rewriting_while_a_reader_loops_never_fails_or_tears` passou com o mutante e só o teste novo pegou.
- 3ª: a injeção desligada cai no `assert!(held.is_some(), "the failure was never injected")` (:259).

Gate: `[OK]   test (622 passed, 16 ignored)` / `[OK]   gate verde`. `leftovers.ps1`: `[OK] nenhum processo de agente sobrando`.

## Concerns
- **Causa do `Err` original não identificada.** O travamento natural não se reproduziu em 8600 execuções da base nesta sessão. A explicação mais provável é um `write` que falhou de forma transitória: por exemplo, um scanner segurando `search.idx.tmp` ou `search.idx` por mais de 80 ms, além do retry de `persist`, ou um erro do SQLite em `encode_catalog`. Isso vem da injeção, não foi observado. Não ampliei o retry de produção (o brief proíbe timeouts especulativos). Se acontecer de novo, o teste agora falha mostrando o erro. Esse é o dado que falta para decidir se `persist` deve passar a usar `fs_retry::retry` (escada de 1,27 s) em vez de 5×20 ms.
- O detector de 60 s no teste novo deixa, no caso de travamento, uma thread presa até o processo de teste terminar. Isso é aceitável para um detector: só acontece quando o teste já falhou.
