Status: DONE

Commits:
d1633f3 perf(update): open the catalog and save http state only when something changed
4e787f1 perf(update): span for catalog open
(base 3ac1d17)

Gate (final, commit 2):
[OK]   rules-selftest 10s
[OK]   fmt 1s
[OK]   clippy 10s
[OK]   test (637 passed, 16 ignored) 98s
[OK]   gate verde
(commit 1 also gated: test 634 passed, gate verde)

Mutation proofs (scripts/mutate.ps1, src/update.rs):
- needs_catalog sempre true (anchor '!index_exists || results' -> 'true || results'):
  teste 1 (`cargo test --quiet needs_catalog`): `[OK] mutação PEGA` (assertion failed: !needs_catalog(&idle(), true))
  teste 2 (`--test update fresh_update_touches`): `[OK] mutação PEGA` (assertion failed: !data.catalog_path().exists())
- `|| !index_exists` removido (`!index_exists ||` -> `false ||`), `cargo test --quiet needs_catalog`: `[OK] mutação PEGA` (assertion failed: needs_catalog(&idle(), false))
- state.save incondicional (`if state_changed {` -> `if true {`), `--test update fresh_update_touches`: `[OK] mutação PEGA: 'let saved = if state_changed {' -> 'let saved = if true {' fez o teste falhar (exit 101).`
- gravação só com Updated (`Outcome::Unchanged | Outcome::Updated { .. }` -> `Outcome::Updated { .. }`), `--test update not_modified_update`: `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
- Ramo `no_catalog()` (import sem catálogo) é inalcançável por construção (needs_catalog é true quando há import): mutação equivalente, sem teste.

Measurement (L = max(5 ms, 10% do sobre-o-piso da base)). Binários: base = 4e787f1 (worktree temporário, removido), nova = d1633f3, ambos release. Raízes isoladas em <scratchpad> = C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad, via perf-root.ps1 / perf-env.ps1 / ab-cmd.ps1 do protocolo (LOCALAPPDATA=<S>\la, WINGETRS_TEST_ENV_KEY, WINGETRS_TEST_SHORTCUT_DIR=<S>\sc).
Série 1 (tudo Fresh, raiz <S>\la logo após `perf76-new.exe update`; `ab-cmd.ps1 -CmdArgs update -Runs 15`, sem -Span):
 wall base med=82.4 max=146.6 | nova med=73.1 max=113.5 | delta pareado med=-14.2 ms
 piso noop med=38.7 | sobre o piso: base 43.7, nova 34.3 ms
 brutos base: 131.1 132.8 140.8 96.4 82.4 146.6 84.2 76.6 74.5 79.4 146.2 78.2 79.0 82.4 65.2
 brutos nova: 113.3 95.0 87.0 72.3 74.4 69.1 70.0 73.5 72.8 73.1 113.5 81.3 69.3 65.6 69.3
 L=5; delta -14.2 <= -L e max(nova) <= max(base): PASSA. (Série ruidosa nas primeiras rodadas; mediana e máximo ainda favorecem a nova.)
Série 2 (tudo 304, http.server local porta 8766 em <S>\srv304, raiz vazia <S>\la304, `update --force`; confirmado com -v avulso: winget/scoop:main/scoop:extras todos "sem mudanças"):
 wall base med=92.9 max=258.4 | nova med=94.2 max=250.6 | delta pareado med=+0.5 ms
 piso noop med=41.2 | sobre o piso: base 51.7, nova 53.0 ms
 brutos base: 102.3 89.7 258.4 89.5 92.9 76.6 89.7 88.3 86.6 86.1 149.6 100.2 106.2 238.0 173.6
 brutos nova: 162.4 73.8 85.4 168.6 78.4 70.9 250.6 91.3 149.1 87.2 74.9 157.1 106.8 94.2 155.5
 L=5; delta +0.5 <= +L: aceitável (sem ganho esperado: nesta série o catálogo ainda é aberto? Não: o índice existe e tudo é Unchanged, então a nova não abre o catálogo; o ganho ficou abaixo do ruído da série, que é grande).
Decisão: série 1 passa, série 2 <= +L: o commit 2 FICA.

Limpeza: <S>\la, la304, srv304, tgt-base-76, sc, perf76-*.exe removidos (caminhos fixos, não vazios); git worktree list sem wt-base-76; reg query HKCU\Software\wingetrs-test\perf => chave inexistente; python do http.server (PID 10596) encerrado no finally; leftovers.ps1 limpo. Processos python.exe de 12/10 21:11-21:16 que aparecem no Get-Process não são meus (anotados, não encerrados).

Decisions:
- Teste 3 (`not_modified_update_still_refreshes_checked_at`) não existia; criado. Helpers `all_urls`, `endpoints_all_answering`, `seed_state_and_index` adicionados em tests/update.rs.
- `mod tests` colocado no fim de src/update.rs (clippy items_after_test_module).
- Em `Fetched::Winget/Scoop` sem catálogo: `no_catalog()` devolve Outcome::Failed "erro interno..." (inalcançável).
- TDD estrito (ver falhar antes) não feito para os testes de integração; provas por mutação cobrem.

Concerns: none (nota da Task 73: catálogo antigo só ganha ix_pkg_id_lower num update que importe algo, conforme o brief).

Review fixes:
- Added `not_modified_update_rebuilds_a_missing_search_index` (tests/update.rs): populated catalog, 304 for all, search.idx deleted; asserts all Unchanged, catalog.db exists, search.idx rebuilt with 28 packages.
- Mutation (`!index_exists ||` -> `false ||`, `cargo test --quiet --test update rebuilds_a_missing`): `[OK] mutação PEGA`; the test failed by name (not_modified_update_rebuilds_a_missing_search_index --- FAILED).
- Gate: [OK]   test (638 passed, 16 ignored) / [OK]   gate verde
