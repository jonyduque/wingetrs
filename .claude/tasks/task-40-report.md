Status: DONE_WITH_CONCERNS

Commits:
```
7c45f7c docs(report): phase 2a validation
```

Gate:
```
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 1s
[OK]   test (407 passed, 16 ignored) 18s
[OK]   gate verde
```

Mutation proofs: tarefa de relatório — **no mutation proof** (declarado no brief). A verificação é o
orquestrador reexecutar `pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'install ripgrep --dry-run' -Runs 15 -Baseline target/release/examples/noop.exe`
(com o mesmo isolamento de ambiente) e fazer a prova ao vivo.

Prova ao vivo: **pendente de autorização** (não executada, conforme o orquestrador).

Resultados (detalhe e saídas coladas em `docs/superpowers/reports/2026-10-10-fase2a-validacao.md`):
- `install ripgrep --yes` com download em cache: mediana 522,6 ms, delta vs noop 496,6 ms (meta < 1 s atingida). `uninstall --yes`: mediana 80,8 ms, delta 53,2 ms. 15 medidas + 2 aquecimentos, 0 exits != 0.
- `install --dry-run` (`bench.ps1 -Baseline`): mediana 114,1 ms, delta 86,2 ms.
- Planejador (`resolve` + `plan_install`, puro): mediana 13,3 µs/chamada.
- Oráculo: fixtures 19 = 15 ok + 4 `unsup-*` recusados como esperado, 0 divergentes. Amostra 50 simples: 48 ok, 2 `.7z`. Todos os 2694 simples: 2365 ok; 326 extensões fora do nível; 3 outras (zigup, mqtt-spy, llvm-arm64).

Decisions:
- Arquivo extra fora de **Files**: `examples/oracle_2a.rs` (autorizado pelo orquestrador como exemplo de suporte; commitado). Só lê o catálogo com `SQLITE_OPEN_READ_ONLY`.
- "Simples" = sem chaves de script/instalador/psmodule/innosetup (raiz e variantes), independente das checagens do `resolve`; amostra = 50 primeiros por SHA-256 de `fonte/id` (determinística).
- install/uninstall medidos com laço intercalado próprio (`noop` antes de cada medida), porque `bench.ps1` repete o mesmo comando e o 2º `install` falharia; `--dry-run` usa `bench.ps1 -Baseline`.
- Tempo do planejador medido no exemplo (não há span de planejador no binário; adicionar um mexeria em `src/`, fora do escopo).
- Raiz temporária = catálogo copiado com `sqlite3 ".backup"` do real aberto `mode=ro`; pasta apagada no fim.

Concerns:
- Sobrou a chave **vazia** `HKCU\Software\wingetrs-test\bench-10543` (o `uninstall` remove o valor `Path`, não a chave). Não apagada à mão.
- `resolve` recusa `bin` com 4 elementos (`scoop:main/zigup`); o scoop provavelmente aceita (args extras) — não conferido no código do scoop.
- Sem spans por etapa em install/uninstall: a divisão dos ~410 ms de execução não foi medida.
- A 1ª execução na raiz nova disparou o refresh do catálogo em segundo plano (rede); as medidas vieram depois, com aquecimento.
- `leftovers.ps1` mostrou processos de outro projeto (bash 23052/3968/10564 `bun run check:q` dinoRig.ts, conhost 23764, python 26260, desde 11:19) durante as medições; não encerrados.
- Durante a 1ª instalação usei por engano `<scratchpad>\sc` (pasta pré-existente de 07/10 com `Main-master/`) como `WINGETRS_TEST_SHORTCUT_DIR`; ripgrep não tem atalhos, nada foi escrito lá (conteúdo inalterado) e as medidas usaram `t40-sc`, já apagada.
