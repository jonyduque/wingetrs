# Brainstorm de desempenho e precisão (2026-10-15)

Seis agentes trabalharam em dois pontos, três por ponto: opus (Plan), sonnet (general-purpose) e fable (Plan). Todos só leram o código. **Nada aqui foi medido agora**: os números citados vêm do ledger ou dos logs existentes, e o resto é hipótese, cada uma com o experimento que a decide.

## Correção da base

O relatório de pendências de 2026-10-15 citava números antigos:

| Ponto | Citado | Real (último medido) | Fonte |
|---|---|---|---|
| `search` | 98,6–101 ms | **69–81 ms** absolutos, 30,6–41,5 ms acima do `noop.exe` | ledger, linha `19-bench` (depois do `search.idx`) |
| Oráculo `list` | 95/116 = 81,9 % | **114/116 = 98,3 %** | Task 17 (110/116) → Task 21 (114/116) |

Nenhum dos dois números foi medido de novo depois das Tasks 102–112.

## Ponto 1: tempo de `wingetrs search`

### Consenso dos três agentes

1. **Medir antes, e isso decide o resto.**
   - `bench.ps1 -Args '--version' -Baseline noop` mostra o custo fixo do binário.
   - `search` menos `--version` mostra o custo de `run()`.
   - Faltam spans: `startup.ensure`, `startup.log` (com `prune_logs`), `startup.config`, `search.probe`, `search.join_wait` (em volta de `EarlySearch::join`, `src/cli/mod.rs:666`) e `search.print`. Também faltam spans `idx.read/parse/scan/sort` em `index_file::search_with`.
   - **`search.join_wait` é o número que decide**:
     - perto de 0 ⇒ o caminho crítico é o setup e a saída da thread principal;
     - acima de 0 ⇒ o caminho crítico é a varredura.
   - Uma amostra do log dá `stage="search" via="idx" ms=9.7` com a máquina ociosa. Isso sugere que a varredura é cerca de 10 ms, não 28–35 ms; aqueles 28–35 ms foram medidos com um `find` órfão a 100 % de CPU.
   - Antes de medir: rodar `wingetrs update` (para não disparar `update --background` no meio) e `leftovers.ps1`.
2. **Piso do binário.**
   - O exe tem 16 MB e importa user32, shell32, ole32, oleaut32, combase, crypt32, ws2_32 e bcrypt, todas sem carga atrasada (`objdump -p`). O `noop.exe` importa só kernel32, ntdll e o CRT.
   - Duas saídas:
     - `/DELAYLOAD` (via `build.rs` ou `.cargo/config.toml`) para as DLLs que a busca não usa;
     - trocar `directories::BaseDirs` (`src/cache/mod.rs:57`, que chama `SHGetKnownFolderPath` e com isso carrega shell32 e combase) por `%LOCALAPPDATA%`, com a API só como alternativa.
   - Experimento sem tocar o produto: um `examples/noop_imports.rs` que importa uma função de cada DLL, medido contra o `noop`. Se o delta for cerca de 0, a ideia morre.
   - Teto: 6–15 ms. As notas divergem sobre o "exe +15 ms".
3. **Caminho rápido do `search`, sem tokio e sem `Http`.**
   - Com o índice válido, responder antes de `Runtime::new` (`mod.rs:919`) e de `Http::new` (`:624`).
   - `refresh_if_stale` vai para depois de imprimir.
   - O resultado da thread antecipada é reaproveitado no lugar do segundo `open` do `search.idx`, e o segundo `exists()` sai.
   - Spans medidos: `runtime` 0,6–1,2 ms, `http` 0,1–0,6 ms e `runtime_drop` 1–1,8 ms. Este último fica depois da saída, portanto no caminho crítico com certeza.
   - Ganho esperado: poucos ms.

### Divergência

| Proposta | Quem | Ganho | Quando fazer |
|---|---|---|---|
| Máscara de caracteres `u64` por registro no `search.idx` (condição necessária para subsequência; um caractere não-ASCII liga todos os bits; desliga com `!neg`) | fable (1º), sonnet (3º) | varredura de 17 ms para menos de 5 ms (hipótese), e as 8 threads deixam de ser necessárias | só se `join_wait > 0` |
| Ler o `search.idx` antes do clap | opus | desconhecido | só se `Cli::try_parse_from` frio custar ≥ 1 ms |
| `score` em vez de `indices` na varredura; índices só para os `limit` primeiros; seleção top-k | opus, fable | só em consultas largas | só se consultas de 1–2 letras importarem |
| Layout quente/frio do `search.idx` (quente ≈ 1,98 MB, frio ≈ 0,86 MB) | os três, por último | ≤ 2–3 ms | junto com a troca de `MAGIC` da máscara, se ela vier |

**Descartados:**
- trigramas (o átomo padrão do nucleo é subsequência, e para ela o trigrama não é condição necessária);
- mmap (cerca de 1 ms, e conflita com o `rename` do escritor);
- `panic="abort"` (quebra o `resume_unwind`);
- mais threads ou conexões.

### Ordem recomendada

1. Medição (passo 0).
2. Piso do binário: `noop_imports`, depois `/DELAYLOAD` e `%LOCALAPPDATA%`.
3. Caminho rápido.
4. Máscara, só com evidência do passo 0.

**Meta:** expressar como delta sobre o `noop` (por exemplo ≤ 30 ms), não em absoluto, porque o piso variou de 38 a 300 ms com a carga da máquina.

## Ponto 2: precisão da correlação (`list` × `winget list`)

### Classificação das 21 falhas originais (Task 15)

| Classe | Falhas | Estado |
|---|---|---|
| Frameworks MSIX descartados na coleta | 15 | resolvido (Task 17) |
| Faltava desempate | AndroidStudio, FFmpeg, UniGetUI, WezTerm | resolvido (Task 21, `installed.db`) |
| Dado que falta no catálogo (sem ProductCode; nome `msys2installer` ≠ `msys2`) | MSYS2.MSYS2 | **aberto** |
| Oráculo errado / divergência proposital (o winget diz 4.0, mas a versão instalada é 3.4.9-1) | RubyWithDevKit | **aberto, por decisão** (`tests/correlate.rs:352`) |
| Faltava normalização | nenhuma | não inventar uma regra "installer" (risco de falso positivo) |

### Propostas (os três agentes convergem)

1. **Oráculo reproduzível e versionado.**
   - O `oracle.py` nunca foi para o git.
   - A proposta é um `scripts/oracle_list.ps1`, só leitura, que use o `Get-WinGetPackage` (o mesmo motor COM do `winget list`) ou o `winget list`. Ele compara linha a linha e reporta quatro números:
     - recall;
     - concordância por linha (pega um id trocado);
     - precisão;
     - faltas por classe.
   - Inclui uma lista de divergências aceitas, com o motivo (Ruby). A taxa bruta e a taxa líquida aparecem separadas.
   - Depois disso, um teste `#[ignore = "live"]` com a lista de faltas conhecidas: um acerto novo faz o teste falhar, então a lista só encolhe.
   - Para provar o instrumento: plantar dois ids trocados no JSON. A concordância tem de cair e o recall não.
2. **`installed.db` como recurso quando o catálogo não acha nada** (é o `else` de `CompositeSource::SearchInstalled`, ref. :240-258).
   - Lugar: o ramo `None` de `correlate` (`src/installed/correlate.rs:439`).
   - Só com referência forte (ProductCode, UpgradeCode ou PFN), e só se o id ainda existir no catálogo.
   - Esperado: MSYS2, 115/116.
3. **`OtherRecord` guarda a `correlation`.** Desde a Task 102, um registro dobrado perde o id no JSON. Se o principal for um MSIX `1.0.0.0` sem correspondência, um ARP com correspondência some. O campo é só aditivo.
4. **`TrackingWriteTime`** para escolher entre vários ids rastreados, como o winget faz (ref. :206-215). Precisa de uma sonda do esquema de `manifest_metadata` numa **cópia** do `installed.db` antes.
5. **Ruby:** manter Ambiguous ("nunca se escolhe em silêncio", spec §5.4). A alternativa é paridade total com o winget, que mostraria uma atualização falsa para 4.0.

Rejeitado: sugestão só por nome quando falta o publisher. Não pode contar como Matched, então não move o oráculo.

## Rascunhos completos

Os rascunhos completos estão no scratchpad da sessão, em `brainstorm/*.md`. Os agentes Plan são só leitura, e os textos deles foram consolidados aqui.
