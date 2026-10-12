Status: DONE

Commits:
81d4af1 perf(buckets): keep search.idx when rm removed no rows
b4cbe21 perf(catalog): skip winget map scans for scoop sources
9ede683 perf(catalog): spans for delete_source_rows and bucket rm index write

Gate (final, com os 3 commits no working tree):
[OK]   fmt 1s
[OK]   clippy 9s
[OK]   test (632 passed, 16 ignored) 155s
[OK]   gate verde

Mutation proofs:
- winget ainda limpa mapas (`if kind == Scoop {` -> `if true {`, teste removing_a_winget_source_still_drops_its_maps):
  [OK] mutação PEGA: 'if kind == SourceKind::Scoop {...' (fez o teste falhar, exit 101)
- recusa MapsForScoop (`&& has_maps` -> `if false && has_maps`):
  [OK] mutação PEGA: 'if kind == SourceKind::Scoop && has_maps' -> 'if false && has_maps' fez o teste falhar (exit 101).
- índice só quando mudou:
  [OK] mutação PEGA: 'removed > 0 ||' -> 'true ||' fez o teste falhar (exit 101).
- Sem `meta` => winget: sem mutação aplicável (nenhum caminho público cria pacotes sem meta; `map_or(Winget, ..)` conferido no diff).

Medição (sozinho, leftovers limpo; base = commit 1 com os spans nos dois lados; 15 execuções, 2 de aquecimento descartadas; piso noop ~31 ms):
Mudança 1 (cenário i, bucket rm extras, 2421 pacotes, span catalog.delete_source_rows):
  span base med=32.9 max=39.1 | nova med=9.6 max=11.1 | delta pareado med=-23.5 ms (L=max(5,3.3)=5) => FICA
  wall base med=175.0 | nova 149.1 | delta -24.9 ms (não-regressão ok)
  brutos span base: 29.4 31.4 38.8 30.4 34.0 34.5 32.8 30.4 34.6 32.4 38.5 32.9 39.1 34.2 31.5
  brutos span nova: 11.0 10.2 9.5 9.8 9.7 9.4 9.3 10.4 9.5 9.2 11.1 10.3 7.8 9.6 8.9
Mudança 2 (cenário ii, bucket rm perfx nunca baixado, wall; L=max(5, 9.4)=9.4):
  Série 1: base med=125.7 max=172.4 | nova med=85.8 max=222.7 | delta -39.4 ms (max nova > max base: só (2) falhou, 1 outlier 222.7)
  brutos base: 117.1 157.7 161.7 159.9 125.2 122.0 125.7 129.7 172.4 131.2 120.6 119.0 126.7 123.7 118.2
  brutos nova: 126.2 78.2 122.3 99.9 99.8 85.8 82.7 222.7 80.0 81.4 86.2 82.3 85.2 83.4 95.7
  Série 2 (repetição): base med=122.0 max=139.6 | nova med=83.1 max=104.5 | delta pareado -36.8 ms; sobre o piso 92.1 -> 53.2 => FICA
  brutos base: 118.5 139.6 116.0 113.9 126.1 126.9 139.2 122.1 113.7 119.2 125.2 120.7 122.0 116.1 128.2
  brutos nova: 79.1 82.7 84.1 77.5 78.0 92.4 84.1 104.5 91.8 84.2 83.1 80.7 94.1 79.3 77.1
bucket.rm.index_write no cenário (i): base med=30.4 ms (max 45.5), nova med=31.5 (max 35.4) — ~30 ms por regravação do índice quando há linhas (número que reabriria a P8). (A série tinha as 7 primeiras execuções de wall em 1,4-3,1 s, ruído externo; o span não foi afetado.)

Decisions:
- Os 3 commits foram feitos com o gate verde sobre a árvore final; commit 2 contém src/catalog/mod.rs + tests/catalog.rs, commit 3 src/buckets.rs + tests/buckets.rs (o teste 5 só passa com o commit 3).
- delete_source_rows virou wrapper (span) + delete_source_rows_inner; o inner recebe `kind`.
- config.toml dos pristines montado à mão (scoop_buckets = ["main","extras"]; 75b com [[buckets]] perfx), pois a raiz real não tem config.toml e o `bucket rm` da 1ª execução tirava extras do config.

Concerns: none (as séries em worktrees/exes foram apagadas; `git worktree list` limpo).

## Adendo (revisão)

Testes novos (src/catalog/mod.rs, mod tests): `removing_a_source_without_meta_row_drops_its_maps_as_winget` e `removing_a_source_with_unknown_kind_drops_its_maps_as_winget`. Gate: `[OK]   test (634 passed, 16 ignored)` / `[OK]   gate verde`.
- `map_or(SourceKind::Winget,` -> `map_or(SourceKind::Scoop,`: `[OK] mutação PEGA: 'map_or(SourceKind::Winget,' -> 'map_or(SourceKind::Scoop,' fez o teste falhar (exit 101).` (teste: o de "without_meta_row"; o de "unknown_kind" passa, pois tem linha em meta)
- `s.parse().unwrap_or(SourceKind::Winget)` -> `...Scoop)`: `[OK] mutação PEGA: ... fez o teste falhar (exit 101).` Teste que falhou: `catalog::tests::removing_a_source_with_unknown_kind_drops_its_maps_as_winget`.

Comandos e raízes de medição. S = ...\scratchpad do agente (fora do repo). Todos os comandos que mediram rodaram com `. "$S\perf-env.ps1"` (LOCALAPPDATA = `$S\la`, raiz isolada `$S\la\wingetrs`, copiada de `%LOCALAPPDATA%\wingetrs` por `perf-root.ps1`; WINGETRS_TEST_ENV_KEY=Software\wingetrs-test\perf; WINGETRS_TEST_SHORTCUT_DIR=$S\sc):
- `pwsh -File $S\perf-root.ps1`; `perf75-new.exe update`; `bucket list`; `bucket add perfx ScoopInstaller/Versions --yes`
- `ab-cmd.ps1 -Base perf75-base.exe -New perf75-mid.exe -CmdArgs 'bucket rm extras' -Pristine $S\pristine75a -Span 'catalog\.delete_source_rows' -Runs 15`
- `ab-cmd.ps1 -Base perf75-mid.exe -New perf75-new.exe -CmdArgs 'bucket rm perfx' -Pristine $S\pristine75b -Runs 15` (2 séries)
- `ab-cmd.ps1 -Base perf75-mid.exe -New perf75-new.exe -CmdArgs 'bucket rm extras' -Pristine $S\pristine75a -Span 'bucket\.rm\.index_write' -Runs 15`
Nenhum comando de medição rodou contra a raiz real; a raiz real foi só lida (cópia do catálogo em modo ro). Fora das medições, os testes/mutações rodaram com CARGO_TARGET_DIR próprio e usam diretórios temporários. O catalog.db real escrito às 18:25 não veio de mim (não medido a causa; possivelmente refresh em 2º plano de outro processo/usuário).
A frase "o 1º `bucket rm extras` removia extras do config" refere-se à raiz isolada `$S\la\wingetrs` (config.toml gravado ali pelo `bucket add perfx`/`bucket rm`), não à real, que não tem config.toml.
