# Task 133 — relatório

Status: DONE_WITH_CONCERNS

Commits:
```
5ad93bf test: merge integration tests into a single binary
```
(`git log --format='%h %s' 35cddb6..HEAD -- tests .claude/tasks/_common.md`; o HEAD de partida era 0700f38, um commit só de doc do orquestrador em cima de 35cddb6)

Gate (depois, árvore final):
```
[OK]   rules 4s
[OK]   rules-selftest 11s
[OK]   fmt 2s
[OK]   clippy 18s
[OK]   test (982 passed, 24 ignored) 34s
[OK]   gate verde
```
Gate antes (base, Medição A): `[OK]   test (982 passed, 24 ignored) 123s` / `[OK]   gate verde`. As mesmas contagens.

## Medições (CARGO_BUILD_JOBS=4, máquina sem outro build; `touch src/install/extract.rs` antes de cada uma)

| Medida | A (50 binários) | B (`it` + `install_extract`) | Delta |
|---|---|---|---|
| (a) `cargo test --no-run --quiet --all-targets` | 80,96 / 77,20 / 86,34 s, mediana **80,96 s** | 17,47 / 18,63 / 17,53 s, mediana **17,53 s** | **-78 %** |
| (b) um binário (A: `--test install_extract`; B: `--test it`) | 11,05 / 10,40 / 10,46 s, mediana **10,46 s** | 15,61 / 15,35 / 15,49 s, mediana **15,49 s** | **+48 %** |
| (c) passo `test` do `verify.ps1` (logo depois de (b): inclui relinkar o que ficou velho) | **123 s** | **34 s** | -72 % |
| (c') `cargo test --quiet --all-targets` quente (sem compilar) | não medido limpo (a única rodada quente da base parou no flake do `install_7z`, ver Concerns); soma dos `finished in` no log do gate: 44,6 s em 55 binários | 30,9 s de parede; soma dos `finished in`: 24,1 s em 7 binários | — |

O critério (a) caiu 78 % (≥ 20 %): mudança mantida. O ciclo de um binário ficou mais lento (o `it` é maior que o `install_extract`): 10,5 s → 15,5 s.

## Equivalência

- Lista A: `cargo test --all-targets -- --list` na base, 1006 testes (`grep ': test$' | sort`).
- Lista B: `--test it -- --list` (923) com o prefixo `<modulo>::` tirado + `--lib --bins --examples --benches --test install_extract -- --list` (83) = 1006. Normalizar a lista `--all-targets` inteira com um `sed` cego tiraria também prefixos de testes unitários (`bridge::`, `catalog::`, `update::`…), por isso o `it` é listado à parte. Todo teste do `it` tem prefixo de módulo (0 sem).
- `diff listA.txt listB_norm.txt` → saída vazia, `diff exit 0`.
- `INSTA_UPDATE=no cargo test --quiet --test it`: 6 rodadas seguidas, todas `ok. 899 passed; 0 failed; 24 ignored`; `find tests -name '*.snap.new' -o -name '*.pending-snap'` vazio. Os 36 `.snap` foram movidos com `git mv` para `tests/it/snapshots/it__<nome antigo>.snap` (o caminho que o insta pediu na 1ª rodada: `Snapshot file: tests\it\snapshots\it__tui_view__offline_snapshot.snap`); `git show -M --numstat HEAD` dá 0/0 linhas para os 36 e para `spans/mod.rs` (conteúdo idêntico).
- `cargo test --quiet --release --test it -- --ignored --list`: 24 testes, os mesmos 24 `#[ignore]` de `tests/it/*.rs` (cli_install, correlate, http, install_msi_live, install_msi_queue, install_win_effects ×7, install_win_scan ×6, installed ×2, scoop_winget_match, tui_state ×2, tui_view).

## Estado global (Review Focus 1)

- **Assinante global do `tracing` — ISOLADO: `tests/install_extract.rs` fica binário próprio.** `release_on_retry::install` e `spans::install` fazem `set_global_default` dentro de um `Once`; num processo só o primeiro vence. Provado: com os 50 juntos, a 1ª rodada deu 15 falhas, 4 delas de spans (`update::update_emits_a_fetch_span_per_source_and_parse_only_for_downloaded_ones` em `update.rs:305` `left: 0 right: 1` com lista `[]`; `install_ops::uninstall_emits_its_stage_spans`, `install_emits_one_span_per_stage_and_per_app`, `planning_spans_wrap_resolve_and_plan`) — o `Releaser` ganhou a corrida e o `Router` nunca foi instalado (as outras 11 eram os snapshots ainda no lugar antigo). Com `install_extract` fora: 6/6 verdes. `spans` agora é `mod spans;` no `main.rs` (um `Once` só para `update` e `install_ops`, que já o compartilhavam por cópia), usado como `use crate::spans;`.
- **`WINGETRS_LOCK_TIMEOUT_MS` (`update.rs`) — sem interferência.** Lido só por `LockWait::from_env` (`src/update.rs:169`), chamado em `src/cli/mod.rs:507,647` e `src/cli/run_win.rs:22`; nenhum teste fora de `update.rs` chama esses caminhos em processo. Filhos herdam o ambiente, mas o único teste que segura o lock e lança o exe (`cli_offline::spawn_update_with_lock_held`) passa o próprio `.env("WINGETRS_LOCK_TIMEOUT_MS", …)`; os demais que seguram lock passam `LockWait::Block/NoWait` explícito (`install_ops.rs:211,414,435`) ou chamam `UpdateLock` direto (`cache.rs`). Sem lock segurado o timeout não importa.
- **`WINGETRS_TEST_ENV_KEY` (`install_win_effects.rs`) — sem interferência.** O `set_var` está em `test_key()`, chamado só por `expand_sz_is_preserved_verbatim` (`#[ignore = "live"]`): no gate nunca roda. Leitores em processo: `RegEnvStore::new`, `RegScanner::from_env`, `prod_win` — nenhum teste fora de `install_win_effects` os chama (grep). Filhos: só `cli_color`, `cli_install`, `cli_offline` lançam o exe; o único `live` entre eles (`cli_install::install_then_uninstall_restores_the_root`) passa `.env("WINGETRS_TEST_ENV_KEY", …)` próprio.
- **`static`/`thread_local` dos testes** (`install_executor` TICKS/CANCEL_*, `tui_jobs` ORDER, `install_extract` CURRENT/GLOBAL) — privados do módulo, só o próprio arquivo os toca, como antes. Statics de `src/` agora compartilhados entre módulos: contadores de nome temporário (`*_TMP_SEQ`, `COUNTER`, `ASIDE_SEQ`), locks de arquivo (`PINS_LOCK`, `OVERRIDES_LOCK`, `MAP_LOCK`) — só serializam/únicos, sem efeito no resultado; `cli::style::ENABLED` só muda em `style::init`, chamado só por `cli::run` (processo filho), os testes usam `with_enabled` (thread-local); `spawn_win` e `fs_retry` são `#[cfg(test)]` do lib.
- **Hook de pânico** (`install_executor::decoder_panic_rolls_back`): já documentado no teste como seguro com outros testes do binário — o hook conta só pânicos com `PANIC_MARK` e chama o anterior; um pânico dentro de decodificador de outro módulo (`install_7z`, `install_archive`) vira `tracing::error!` em vez de mensagem no stderr enquanto o hook está trocado, e nenhum teste confere o stderr do próprio processo. O hook padrão volta antes de qualquer asserção.

## Mutation proofs

1. Teste quebrado ainda falha no binário novo (assert de `cli_parse`):
   `pwsh -File scripts/mutate.ps1 -Path tests/it/cli_parse.rs -Anchor '    let cli = Cli::try_parse_from(["wingetrs", "-v"]).unwrap();<LF>    assert!(!wingetrs::cli::stderr_logging(&cli));' -Replacement '…<LF>    assert!(wingetrs::cli::stderr_logging(&cli));' -TestCommand 'cargo test --quiet --test it cli_parse::stderr_logging_is_off_without_subcommand'`
   → `[OK] mutação PEGA: '    let cli = Cli::try_parse_from(["wingetrs", "-v"]).unwrap();` (teste que caiu: `thread 'cli_parse::stderr_logging_is_off_without_subcommand' (15232) panicked at tests\it\cli_parse.rs:89:5`)
   Snapshot movido:
   `pwsh -File scripts/mutate.ps1 -Path tests/it/snapshots/it__cli_parse__help_root.snap -Anchor 'Gerenciador de pacotes rápido sobre winget e scoop' -Replacement 'Gerenciador de pacotes ZZZ sobre winget e scoop' -TestCommand '$env:INSTA_UPDATE="no"; cargo test --quiet --test it cli_parse::help_snapshots'`
   → `[OK] mutação PEGA: 'Gerenciador de pacotes rápido sobre winget e scoop' -> 'Gerenciador de pacotes ZZZ sobre winget e scoop' fez o teste falhar (exit 101).` (teste que caiu: `thread 'cli_parse::help_snapshots' (29532) panicked at …insta-1.49.0\src\runtime.rs:734:13`) — prova que o insta lê o `.snap` do lugar novo.
2. `--ignored --list` em release: lista os 24 `live` (acima). Sem mutação (é listagem).
3. Regra de relógio varre `tests/it/`:
   `pwsh -File scripts/mutate.ps1 -Path tests/it/cli_parse.rs -Anchor '    let cli = Cli::try_parse_from(["wingetrs", "-v"]).unwrap();' -Replacement '    let _ = std::time::Instant::now().elapsed() < std::time::Duration::from_secs(1);<LF>    let cli = …' -TestCommand 'pwsh -File scripts/check_rules.ps1'`
   → `[OK] mutação PEGA: '    let cli = Cli::try_parse_from(["wingetrs", "-v"]).unwrap();' -> '    let _ = std::time::Instant::now().elapsed() < std::time::Duration::from_secs(1);` … `fez o teste falhar (exit 1).`
4. Gate: acima.

## Decisions

- Isolado só o `install_extract` (um arquivo) em vez de `update` + `install_ops` (os dois usuários de `spans`): menos binários fora do `it`.
- `include_bytes!/include_str!` relativos (29 linhas em 15 arquivos) ganharam um `../`; as fixtures ficam em `tests/fixtures/`. O brief dizia que todas usavam `CARGO_MANIFEST_DIR`; essas não, o compilador recusou.
- `#[allow(clippy::unwrap_used, clippy::expect_used)] mod spans;` no `main.rs`: antes `spans` herdava o `allow` de `update.rs`/`install_ops.rs`. Os `#![allow]`/`#![cfg(windows)]` de topo de cada arquivo continuam como atributos internos do módulo.
- Comentário de `install_executor.rs:1161` passou a citar `tests/it/install_msi.rs`. `scripts/oracle_list.allow.json` cita `tests/correlate.rs:352` num texto de motivo: fora da lista Files, não mexi.

## Concerns

- **Flake anterior à tarefa em `install_7z::encoded_header_with_huge_unpack_size_refused`** (log vazio na asserção de `install_7z.rs:541`, o único teste com `tracing::subscriber::with_default`): na base, `cargo test --quiet --test install_7z` falhou 3/10; derrubou também a minha 1ª medição quente de (c'). Depois: `--test it install_7z::` falhou 2/10; a suíte `it` inteira passou 6/6 + o gate. É a mesma corrida do cache de interesse de callsite que o comentário de `spans::install` descreve; corrigir muda o teste (fora do escopo). Recomendo tarefa própria.
- O ciclo de um binário ficou mais lento (+48 %, 10,5 → 15,5 s); o `_common.md` foi atualizado com os números de B.
- `leftovers.ps1` mostrou shells de outro projeto (vitest, desde 15:15/18:15), não iniciados por mim; nenhum cargo/rustc/wingetrs sobrando.

---

# Task 133b — flake do `install_7z` (log vazio no `with_default`)

Status: DONE

Commits:
```
ae0648d test(install): capture the 7z refusal log through the global test subscriber
```
(`442c274` no mesmo intervalo é do orquestrador.)

Gate:
```
[OK]   rules 5s
[OK]   rules-selftest 13s
[OK]   fmt 2s
[OK]   clippy 12s
[OK]   test (982 passed, 24 ignored) 43s
[OK]   gate verde
```

Correção (opção 1): `crate::spans` ganhou `Log`, que grava eventos formatados (DEBUG para cima) da thread onde foi instalado. A gravação passa pelo mesmo subscriber global do binário, instalado uma vez: `registry().with(Router).with(fmt::layer().with_ansi(false).with_writer(ThreadLog).with_filter(filter_fn(nível <= DEBUG && thread com Log)))`. Em `install_7z::encoded_header_with_huge_unpack_size_refused`, o `with_default(fmt()...)` virou `let _recording = log.install(); run(...)`. A asserção é a mesma linha exata (`"candidato 7z recusado offset=4098 error=arquivo inválido: cabeçalho 7z grande demais\n"`). O `LogBuf`, que ficou sem uso, saiu. A edição em `tests/it/spans/mod.rs` sai do arquivo único autorizado, mas a opção 1 exige estender o `spans`.

Causa: o callsite `tracing::debug!` de `src/install/sniff.rs:122` é compartilhado com os outros testes do `install_7z`, que rodam em paralelo sem subscriber. O interesse do callsite fica em cache global. Com um subscriber escopado, outra thread que registrava o callsite podia gravar `never` e a linha sumia. Com um único subscriber global e filtro dinâmico, o interesse é sempre `sometimes`.

Provas:
- `cargo test --quiet --test it install_7z::` 20x: `20 test result: ok. 20 passed; 0 failed` (antes, na base: 3/10 falhas; no `it`: 2/10).
- Suíte `it` inteira 3x: `3 test result: ok. 899 passed; 0 failed; 24 ignored`.
- `pwsh -File scripts/mutate.ps1 -Path tests/it/spans/mod.rs -Anchor '*m.level() <= tracing::Level::DEBUG' -Replacement '*m.level() <= tracing::Level::INFO' -TestCommand 'cargo test --quiet --test it install_7z::encoded_header_with_huge_unpack_size_refused'`
  → `[OK] mutação PEGA: '*m.level() <= tracing::Level::DEBUG' -> '*m.level() <= tracing::Level::INFO' fez o teste falhar (exit 101).` (panicked at `tests\it\install_7z.rs:540:5`)
- `pwsh -File scripts/mutate.ps1 -Path src/install/sniff.rs -Anchor '"candidato 7z recusado"' -Replacement '"candidato 7z ZZZ"' -TestCommand 'cargo test --quiet --test it install_7z::encoded_header_with_huge_unpack_size_refused'`
  → `[OK] mutação PEGA: '"candidato 7z recusado"' -> '"candidato 7z ZZZ"' fez o teste falhar (exit 101).` (panicked at `tests\it\install_7z.rs:540:5`)

Outros `with_default`/`set_default`: em `tests/` não sobrou nenhum (grep). Em `src/` existe só `src/cli/mod.rs:1031` (`file_layer_logs_the_duration_of_a_closed_wingetrs_span`, no `#[cfg(test)]` do lib). Ele não corre o mesmo risco e ficou como está. Os dois spans que ele confere (`install.prefetch`, `noise`) são callsites declarados no próprio corpo do teste e só são alcançados dentro do `with_default`, então nenhuma outra thread registra esses callsites. As ocorrências de `.with_default(Level::WARN)` em `src/cli/mod.rs:339,377` são de `Targets`, não de subscriber.

Decisions: o filtro por thread (`LOG.with(is_some)`) evita formatar eventos nos testes que não pediram log. Nenhum teste depende disso: é custo, não comportamento.

Concerns: none.
