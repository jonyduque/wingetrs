Status: DONE_WITH_CONCERNS

Commits:
```
5ee41fc fix(catalog): test the index read retry without a wall clock
32ca45a fix(install): retry renames and removals blocked by a transient external handle
17fe9ee fix(install): extraction io errors name the operation and path
```

Gate (HEAD 5ee41fc, 3 runs seguidas depois do último commit, `CARGO_TARGET_DIR` separado; os 3 terminaram assim):
```
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (619 passed, 16 ignored) 34s
[OK]   gate verde
```
(antes do commit 3, com o mesmo conteúdo: `test (619 passed, 16 ignored) 153s` / `gate verde`; commit 1: `test (613 passed, 16 ignored) 192s` / `gate verde`)

## Evidência

### Passo 2: sonda (`<S>\probe78`, só std, handle aberto em `d\f.txt`)
```
share=0      rename(dir)=Err(Some(5)) rename(file)=Err(Some(32)) remove_file=Err(Some(32)) create_after_remove=Err(Some(32)) rename_onto_removed=Err(Some(5)) cleanup=Ok
share=1|2    rename(dir)=Err(Some(5)) rename(file)=Err(Some(32)) remove_file=Err(Some(32)) create_after_remove=Ok         rename_onto_removed=Err(Some(5)) cleanup=Ok
share=1|2|4  rename(dir)=Err(Some(5)) rename(file)=Ok         remove_file=Ok         create_after_remove=Ok         rename_onto_removed=Ok         cleanup=Ok
```
Leitura: **renomear uma pasta com qualquer filho aberto dá 5 em todo modo de compartilhamento**, inclusive o 1|2|4 que os scanners usam. H1 plausível. H1b (pendente de exclusão) não se reproduz nesta máquina: com 1|2|4, `remove_file` + `create` no mesmo nome dá `Ok` (exclusão com semântica POSIX).

### Passo 3: reprodução (binários de 304ff4e+commit 1, `<S>\t78`)
Defender antes: `RealTimeProtectionEnabled : True`, `AMRunningMode : Normal`; depois (carga e cabeça): idem. Nada foi alterado.
- Carregado `-Rounds 40 -Parallel 4`: `execuções=480 falhas=0` ⇒ subi para 120/6, uma vez.
- Carregado `-Rounds 120 -Parallel 6`: `execuções=2160 falhas=5`
```
r119-install_executor-…-3.out: ---- failure_at_any_step_restores_the_initial_state_exactly stdout ----
r119-install_executor-…-3.out: falha no passo 1 (extrair [1]): E/S: renomear C:\Users\jonyd\AppData\Local\Temp\.tmpPS7ba4\root\apps\app\.wingetrs-extract-30200-64: Acesso negado. (os error 5); desfeito: ["extrair [1]"]
r18-install_executor-…-2.out: ---- failure_during_upgrade_keeps_v1_current_and_user_data stdout ----
r18-install_executor-…-2.out: falha no passo 1 (extrair [1]): E/S: renomear C:\Users\jonyd\AppData\Local\Temp\.tmpgX4gB4\root\apps\app\.wingetrs-extract-32996-62: Acesso negado. (os error 5); desfeito: ["extrair [1]"]
r80-install_executor-…-5.out: ---- failure_at_any_step_restores_the_initial_state_exactly stdout ----
r80-install_executor-…-5.out: falha no passo 1 (extrair [1]): E/S: renomear C:\Users\jonyd\AppData\Local\Temp\.tmplm1SAE\root\apps\app\.wingetrs-extract-31928-63: Acesso negado. (os error 5); desfeito: ["extrair [1]"]
r83-install_executor-…-1.out: ---- failure_message_lists_undone_steps stdout ----        (assert index: left 1, right 10 ⇒ falhou na extração)
r87-install_executor-…-4.out: ---- failure_after_replacing_a_kept_version_restores_it stdout ----  ("failed elsewhere": left 1, right 10 ⇒ extração)
```
As 5 falhas são na extração (passo 1); as 3 com mensagem são o `rename` da pasta `.wingetrs-extract-*` inteira para o destino (`merge_into`, destino ausente) — exatamente o caso que a sonda mostra dar 5 com qualquer handle num filho.
- Controle quieto `-Rounds 120 -Parallel 1 -Threads 1`: `execuções=360 falhas=2`, **nenhum os error 5/32**. As 2 falhas são travamentos (ver Concerns): `index_file` parado em `rewriting_while_a_reader_loops_never_fails_or_tears` (rodadas 67 e 116), encerrados por mim (processo meu).

### Passo 4: discriminador (worktree `<S>\wt-78-diag`, `diag()` em todo rename/remove dos arquivos citados, 5 ms por até 5 s)
Laço `-Rounds 120 -Parallel 6` com esses binários: `execuções=2160 falhas=0`. Linhas em `%TEMP%\wingetrs-diag-*.txt`:
```
contagem por operação:    700 idx-read;32      1 rename-extract-dir;5
rename-extract-dir;C:\Users\jonyd\AppData\Local\Temp\.tmpqgnhYb\root\apps\app\.wingetrs-extract-32432-64;5;16
idx-read (t até liberar): n=700 min=7 med=31 p90=55 max=228 (ms); >50 ms: 95
TIMEOUT: 0
```
- `rename-extract-dir`: liberou sozinho em 16 ms com a nossa thread parada no laço ⇒ o dono do handle é **outro** (H1). Nenhum TIMEOUT ⇒ H2 sem evidência.
- `idx-read`: todos vêm do `search_retries_while_the_file_is_briefly_locked` (o lock exclusivo é do próprio teste; a thread dona solta "depois de 5 ms" e levou até 228 ms) ⇒ mecanismo de H4.
- H3: o caminho que falhou fica dentro do tempdir do próprio teste (`.tmpqgnhYb\root\…`, `.tmpPS7ba4\root\…` etc.) e o nome tem pid+seq. Construções de nome temporário (`grep -n`):
```
src/install/download.rs:79:    let tmp = p.with_extension(format!(      ("json.{pid}-{seq}.tmp")
src/install/download.rs:125:    let tmp = tmp_dir.join(format!(           ("dl-{pid}-{seq}.part")
src/install/extract.rs:193:    let tmp = parent.join(format!(           (".wingetrs-extract-{pid}-{seq}")
src/install/executor.rs:490 free_aside                              (".replaced-{v}-{pid}-{seq}")
src/install/layout.rs:26:        self.app_dir(app).join(format!(".staging-{v}"))
src/install/shim.rs:116:    let zip_path = dest.with_extension("zip.part");
src/install/shim.rs:117:    let stage = dest.with_extension("stage");
src/cache/mod.rs:142:        tmp.push(".tmp");                     (HttpState)
src/cache/mod.rs:186:        let tmp = self.dir.join(format!("{actual}.tmp"));
src/install/state.rs:238  version_dir.join("install.json.tmp")
```
Os sem pid+seq ficam dentro da pasta da app/versão/cache de uma raiz (`install.lock` serializa a escrita) e, nos testes, dentro do tempdir de cada teste; nenhuma falha caiu neles ⇒ H3 sem evidência, sem correção.

### Passo 5: correção (H1 e H4; H1b/H2/H3 sem evidência ⇒ nada)
- `src/fs_retry.rs`: `is_transient`, `retry_with`, `retry` (debug `"transient fs error, retrying"` antes de cada espera, warn ao desistir). `DELAYS` = 10, 20, 40, 80, 160, 320, 640 ms (total 1,27 s): ≥ 2× o maior `t` medido (228 ms ⇒ 456 ms; o do scanner foi 16 ms) e ≤ 2 s; o próximo degrau (1280) passaria de 2 s.
- Aplicado a: `merge_into` (3 renames + remove), `TempGuard` e limpeza do temporário, `Step::Promote` (2 renames), `RestoreAside`, `remove_any`, `persist` do executor, `HttpState::save` e `BlobStore::put_verified`, os 2 renames de `download.rs`, `state.rs` (`install.json`). `read_retrying` passou a usar `fs_retry::is_transient`; os laços de `index_file` ficaram.
- H4 (commit 3): a cabeça do passo 6 (1ª tentativa) falhou em `search_retries_while_the_file_is_briefly_locked` com `Io("O arquivo já está sendo usado por outro processo. (os error 32)")` (1 em 612 execuções) — junto com o `t` de até 228 ms do passo 4, é a evidência. `read_retrying_with(path, sleep)` + unitário `read_retrying_survives_a_lock_released_by_the_retry`; o teste de integração por relógio saiu.

### Passo 6: prova
- Base (passo 3), carregado 120/6: `execuções=2160 falhas=5` (todas os error 5 na extração).
- Cabeça 32ca45a (sem H4), carregado 120/6, interrompida por mim na rodada 34 para corrigir H4: 612 execuções, 1 falha (a de H4 acima), 0 na extração.
- Cabeça 5ee41fc, carregado 120/6: `execuções=2160 falhas=0`.
Critério (base ≥ 5, cabeça 0 no mesmo N) atingido, mas no limite: base = 5 exatamente.

## Mutation proofs
(todas com `CARGO_TARGET_DIR=<S>\t78`)

1. Instrumento (`io_at("apagar")` → `?` puro). Commit 1:
```
pwsh -File scripts/mutate.ps1 -Path src/install/extract.rs -Anchor 'fs::remove_file(&to).map_err(io_at("apagar", &to))?' -Replacement 'fs::remove_file(&to)?' -TestCommand 'cargo test --quiet --test install_extract extract_io_error_names_operation_and_path'
[OK] mutação PEGA: 'fs::remove_file(&to).map_err(io_at("apagar", &to))?' -> 'fs::remove_file(&to)?' fez o teste falhar (exit 101).
  extract_io_error_names_operation_and_path --- FAILED
```
   Na cabeça (âncora de duas linhas `\n                    .map_err(io_at("apagar", &to))?;` → `?;`):
```
[OK] mutação PEGA: '
                    .map_err(io_at("apagar", &to))?;' -> '?;' fez o teste falhar (exit 101).
  extract_io_error_names_operation_and_path --- FAILED
```
2. Retry ligado na produção:
```
pwsh -File scripts/mutate.ps1 -Path src/install/extract.rs -Anchor 'fs_retry::retry("apagar", &to, || fs::remove_file(&to))' -Replacement 'fs::remove_file(&to)' -TestCommand 'cargo test --quiet --test install_extract extraction_retries_a_held_file_until_it_is_released'
[OK] mutação PEGA: 'fs_retry::retry("apagar", &to, || fs::remove_file(&to))' -> 'fs::remove_file(&to)' fez o teste falhar (exit 101).
  extraction_retries_a_held_file_until_it_is_released --- FAILED
```
3. Escada inteira:
```
pwsh -File scripts/mutate.ps1 -Path src/fs_retry.rs -Anchor 'for d in delays {' -Replacement 'for d in delays.iter().take(1) {' -TestCommand 'cargo test --quiet --lib retry_with_gives_up_after_the_last_delay'
[OK] mutação PEGA: 'for d in delays {' -> 'for d in delays.iter().take(1) {' fez o teste falhar (exit 101).
  fs_retry::tests::retry_with_gives_up_after_the_last_delay --- FAILED
```
4. Só transitórios:
```
pwsh -File scripts/mutate.ps1 -Path src/fs_retry.rs -Anchor 'e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(SHARING_VIOLATION)' -Replacement 'let _ = e; true' -TestCommand 'cargo test --quiet --lib retry_with_does_not_retry_other_errors'
[OK] mutação PEGA: 'e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(SHARING_VIOLATION)' -> 'let _ = e; true' fez o teste falhar (exit 101).
  fs_retry::tests::retry_with_does_not_retry_other_errors --- FAILED
```
5. Prova estatística: colada acima (base 5/2160, cabeça 0/2160).
6. Todas as "PEGA" acima mostram o nome do teste que falhou (target separado).
Extra (H4, teste 7):
```
pwsh -File scripts/mutate.ps1 -Path src/catalog/index_file.rs -Anchor '                sleep(std::time::Duration::from_millis(10));' -Replacement '                std::thread::sleep(std::time::Duration::from_millis(10));' -TestCommand 'cargo test --quiet --lib read_retrying_survives_a_lock_released_by_the_retry'
[OK] mutação PEGA: … fez o teste falhar (exit 101).
  catalog::index_file::tests::read_retrying_survives_a_lock_released_by_the_retry --- FAILED
```

## Decisions
- `io_at` também em `read_dir`/`file_type` ("listar", "ler tipo"), `write_all` ("gravar") e no `create_dir_all`/`fs::copy` de `extract_one` ("criar pasta", "copiar"): mesma motivação (mensagem com operação e caminho); `rename` nomeia a origem.
- Teste 5 usa share 1|2|4 e espera `raw_os_error == Some(5)` (passo 2: renomear a pasta dá 5 em todo modo). Acrescentei `delays_double_from_10_ms_and_stay_under_2_s`.
- H4 virou um 3º commit (`fix(catalog): …`), porque a evidência só apareceu na prova da cabeça, depois do commit 2; a 1ª prova da cabeça foi interrompida e refeita inteira em 5ee41fc.
- `retry` só registra o `warn` quando a escada inteira se esgota com erro transitório; o laço próprio do `persist` de `index_file` (que tenta de novo em qualquer erro) ficou como estava, como o brief manda.

## Concerns
- **Travamento novo, fora do escopo:** `tests/index_file.rs::rewriting_while_a_reader_loops_never_fails_or_tears` travou 2× em 120 rodadas no modo quieto (`--test-threads=1`, binários da base): uma thread rodando (578 s e 51 s de CPU), as demais em espera; leituras de `search.idx` subindo (~340 ops/s, ~39 KB cada) e escritas paradas (o `search.idx` do tempdir sem mudar desde 3 s após o início) ⇒ o leitor fica em laço e o escritor (`index_file::write`) fica bloqueado em algum lugar. Sem depurador na máquina, a causa não foi achada. Não apareceu nos laços carregados (base, diag, cabeça). Encerrei os dois processos (meus). Recomendo tarefa própria (ex.: limite de tempo no laço do escritor como detector + dump do stack).
- Margem estatística mínima: a base teve exatamente 5 falhas em 2160 (taxa ~0,23 %); 0/2160 na cabeça é compatível com a correção, mas a confiança é modesta. O laço 40/4 não reproduziu nada.
- `t` do scanner medido uma única vez (16 ms); a escada (1,27 s) foi dimensionada pelo maior `t` medido no total (228 ms, que é de um lock do próprio teste). Erro de permissão genuíno agora custa ~1,27 s antes de falhar com a mesma mensagem (aceito pelo brief).
- `scripts/leftovers.ps1` aponta PID 7900 (`bash.exe` de 11/10, `bun run check:q` de outro projeto): não é meu, não encerrei.
- Limpeza feita: `<S>\t78`, `<S>\t78d`, `<S>\probe78`, `<S>\eacces-*`, `%TEMP%\wingetrs-diag-*.txt`, worktree `wt-78-diag` removido (`git worktree list` só mostra o principal); nenhum `install_executor-*`/`install_ops-*`/`index_file-*` vivo (contagem 0).
