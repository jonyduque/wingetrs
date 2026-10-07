# Fase 1 — validação ao vivo e benchmarks (2026-10-07)

Base: `1d6f6f6ba21fd5c2ca604bea4041d27715ec124c`. Máquina: Windows 11 Home 10.0.26220.
Binário: `cargo build --release` (`Finished release profile [optimized]`).
Catálogo: `%LOCALAPPDATA%\wingetrs\catalog.db` (23 818 240 bytes, 19 478 pacotes).
Todo número abaixo vem de uma saída colada nesta página. "não medido" = não medido.

## Resumo: meta × medido × winget

| Critério (spec §9.2) | Meta | wingetrs (mediana, 10 exec. após 2 de aquecimento) | winget | Status |
|---|---|---|---|---|
| `search powertoys` (cache quente) | < 100 ms | 192,2 ms (2ª rodada: 205,8 ms) | 640,2 ms (`winget search powertoys --source winget`) | **FALHA** |
| `search powertoys -n 3` | < 100 ms | 197,1 ms | — | **FALHA** |
| `list` | < 500 ms | 409,4 ms (2ª rodada: 404,0 ms) | 3 200,7 ms (`winget list`) | OK |
| Primeiro frame da TUI com cache | < 50 ms | 3–6 ms (9 de 10 aberturas); 1 outlier de 104 ms | não medido | OK (com ressalva) |
| `update` sem mudanças, caminho 304 | < 300 ms | **não medido** (ver abaixo) | não medido | — |
| `update` sem mudanças, caminho Fresh (sem rede) | — | 65,6 ms | — | informativo |
| Correlação com o `winget list` | ≥ 90% | **81,9%** (95/116 IDs) | — | **FALHA** |

Piso do processo (para referência): `wingetrs --version` = 44,1 ms / 47,0 ms de mediana — esse
custo (criação de processo + invocação via PowerShell no bench) está embutido em todas as medidas.

Comparação com o observado pelo orquestrador: `search powertoys -n 3` ≈ 0,26 s → medi 197,1 ms
(abaixo do observado, mas confirma a falha da meta de 100 ms); `list` 0,36–0,48 s → confirmado
(404–409 ms); `first_frame_ms` 2–4 sem TTY → confirmado com TTY real (3–6 ms, 1 outlier).

## Comandos e saídas

Script: `scripts/bench.ps1` (Stopwatch em volta de `& $Exe @argv`, saída descartada; `-Args` é uma
string única separada por espaços porque `pwsh -File` não repassa arrays; `-DelayMs` dorme entre execuções).

```
$ B="pwsh -NoProfile -File scripts/bench.ps1"
$ for a in 'search powertoys' 'search powertoys -n 3' 'list'; do $B -Exe target/release/wingetrs.exe -Args "$a"; done
target/release/wingetrs.exe search powertoys | runs=10 warmup=2 exit=0 | min=169,2 ms median=192,2 ms max=229,7 ms
target/release/wingetrs.exe search powertoys -n 3 | runs=10 warmup=2 exit=0 | min=170,9 ms median=197,1 ms max=208,7 ms
target/release/wingetrs.exe list | runs=10 warmup=2 exit=0 | min=333,0 ms median=409,4 ms max=429,3 ms

$ $B -Exe winget -Args 'search powertoys --source winget --disable-interactivity'; $B -Exe winget -Args 'list --disable-interactivity'
winget search powertoys --source winget --disable-interactivity | runs=10 warmup=2 exit=0 | min=605,7 ms median=640,2 ms max=696,4 ms
winget list --disable-interactivity | runs=10 warmup=2 exit=0 | min=3.124,2 ms median=3.200,7 ms max=3.327,7 ms

$ pwsh -NoProfile -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'update' -DelayMs 1000
target/release/wingetrs.exe update | runs=10 warmup=2 exit=0 | min=60,8 ms median=65,6 ms max=72,0 ms
$ ./target/release/wingetrs.exe update
winget: em dia
scoop:main: em dia
scoop:extras: em dia

# 2ª rodada (mesma sessão, ao final)
target/release/wingetrs.exe search powertoys | runs=10 warmup=2 exit=0 | min=176,7 ms median=205,8 ms max=259,7 ms
target/release/wingetrs.exe list | runs=10 warmup=2 exit=0 | min=363,7 ms median=404,0 ms max=437,5 ms
target/release/wingetrs.exe --version | runs=10 warmup=2 exit=0 | min=41,5 ms median=47,0 ms max=51,8 ms
# 1ª medida do piso
target/release/wingetrs.exe --version | runs=10 warmup=2 exit=0 | min=41,2 ms median=44,1 ms max=62,8 ms
```

### Qual caminho do `update` foi medido

`update` sem `--force` mediu o caminho **Fresh**: `src/update.rs:12` `MAX_AGE_SECS = 900` e
`needs_refresh` (`src/update.rs:70-76`) pulam a rede quando `checked_at` tem menos de 15 min
(`http.json` tinha `checked_at: 1791372491`, `date +%s` = `1791372586`). Saída "em dia" = `Outcome::Fresh`
(`src/cli/output.rs:84`). O caminho **304** (requisição condicional com ETag) **não foi medido**:
a execução de `wingetrs update --force` foi bloqueada pelo classificador de permissões do ambiente
do agente, e não contornei o bloqueio. Follow-up F4 abaixo.

### Primeiro frame (`first_frame_ms`)

TUI aberta em console novo (`Start-Process`, TTY real), fechada após 1,5 s, 2 lotes de 5, com
`HTTP(S)_PROXY=http://127.0.0.1:9` para impedir rede:

```
2026-10-07T11:30:09.268253Z  INFO wingetrs::tui::run: first frame first_frame_ms=6
2026-10-07T11:30:10.351212Z  INFO wingetrs::tui::run: first frame first_frame_ms=104
2026-10-07T11:30:12.393585Z  INFO wingetrs::tui::run: first frame first_frame_ms=5
2026-10-07T11:30:13.685722Z  INFO wingetrs::tui::run: first frame first_frame_ms=4
2026-10-07T11:30:15.192264Z  INFO wingetrs::tui::run: first frame first_frame_ms=4
2026-10-07T11:30:23.739469Z  INFO wingetrs::tui::run: first frame first_frame_ms=5
2026-10-07T11:30:25.299961Z  INFO wingetrs::tui::run: first frame first_frame_ms=4
2026-10-07T11:30:26.879886Z  INFO wingetrs::tui::run: first frame first_frame_ms=3
2026-10-07T11:30:28.569415Z  INFO wingetrs::tui::run: first frame first_frame_ms=4
2026-10-07T11:30:29.914247Z  INFO wingetrs::tui::run: first frame first_frame_ms=4
```

Ressalva: o span mede de `tui::run()` até `terminal.draw` (`src/tui/run.rs:44-68`), não desde a
criação do processo (o piso de processo é ~44 ms, ver acima). O outlier de 104 ms não foi
investigado; causa provável é a alocação do console novo/`ratatui::init` concorrendo com a
abertura da janela (hipótese, não medida).

### Testes `#[ignore]` ao vivo (Tasks 11/12)

```
$ cargo test --release --quiet --test installed --test correlate -- --ignored --nocapture
running 1 test
rows=250 matched=118 elapsed_ms=377
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.38s
running 1 test
apps=250 errors=[] elapsed_ms=258
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.26s
```

## Perfil da meta que falhou: `search`

Binário de perfil fora do repositório (scratch, crate `prof` dependendo de `wingetrs` por path),
cronometrando as mesmas chamadas que `Command::Search` faz (`src/cli/mod.rs:195-213`):

```
http_new_ms=0.4
scan_all_ms=261.5 apps=250
scan_all_ms=149.7 apps=250
scan_all_ms=148.6 apps=250
entries=19478 hits=3 open_ms=4.3 load_entries_ms=90.0 nucleo_search_ms=17.0 drop_ms=11.6
entries=19478 hits=3 open_ms=2.8 load_entries_ms=87.4 nucleo_search_ms=18.4 drop_ms=12.0
entries=19478 hits=3 open_ms=4.1 load_entries_ms=104.8 nucleo_search_ms=16.3 drop_ms=11.7
```

```
$ sqlite3 catalog.db "select count(*), sum(length(raw)), sum(length(description)), sum(length(tags)) from packages;"
19478|5607526|276586|781764
```

Causa provável: `Catalog::load_entries` (`src/catalog/mod.rs:291`) lê **todas** as 19 478 linhas com
`PKG_COLS`, incluindo `raw` (5,6 MB), e materializa um `PackageEntry` por linha (~90–105 ms); o
nucleo varre todas (~17–22 ms) e o `drop` do vetor custa ~10–13 ms. Somado ao piso do processo
(~44 ms), explica os ~190–205 ms. Fazer `search` caber em 100 ms exige não materializar o catálogo
inteiro por chamada.

## Perfil de `list` (meta atingida, margem pequena)

Spans existentes no log (`list scan_ms=… correlate_ms=…`):

```
2026-10-07T11:28:27.490554Z  INFO wingetrs::cli: list scan_ms=169 correlate_ms=96
2026-10-07T11:28:27.920931Z  INFO wingetrs::cli: list scan_ms=266 correlate_ms=90
2026-10-07T11:28:28.335483Z  INFO wingetrs::cli: list scan_ms=265 correlate_ms=83
2026-10-07T11:28:28.670611Z  INFO wingetrs::cli: list scan_ms=180 correlate_ms=85
2026-10-07T11:28:29.081835Z  INFO wingetrs::cli: list scan_ms=261 correlate_ms=79
```

A coleta de instalados domina (150–266 ms). Hipótese não medida por coletor: leitura de propriedades
MSIX (`src/installed/appx_win.rs:47`, comentário do próprio código: "Per-package property reads are slow").

## Oráculo: `winget list --source winget` × `wingetrs list --json`

```
$ winget list --source winget --disable-interactivity > winget_list.txt   # exit=0, 154 linhas
$ target/release/wingetrs.exe list --json > wrs_list.json                  # exit=0
$ python -I oracle.py winget_list.txt wrs_list.json
winget rows=152 distinct_ids=116 truncated_ids=0
wingetrs rows=250 matched_rows=118 matched_distinct_ids=115
oracle hits=95/116 rate=81.9%
wingetrs_matched_not_in_winget=20 (winget-source=0)
```

Parser: colunas pelo cabeçalho (`Nome`, `ID`, `Versão`); IDs terminados em `…` seriam comparados por
prefixo (nenhum truncado nesta saída). 152 linhas → 116 IDs distintos (MSIX multi-arquitetura repete ID).
Comparação case-insensitive com `correlation.Matched.package_id`.

**Taxa: 95/116 = 81,9% (meta ≥ 90%: FALHA).** Excluindo os 15 pacotes *framework* MSIX
(classe excluída de propósito, ver abaixo): 95/101 = 94,1% (cálculo sobre os números acima).
Nenhum falso positivo do lado winget: os 20 `Matched` do wingetrs ausentes do `winget list` são todos
de origem `scoop:*` (regra `ScoopManifest`), fora do escopo do oráculo.

### Divergências (21, todas listadas)

Estado do wingetrs vindo de `wrs_list.json`; presença no catálogo via `sqlite3 catalog.db`.

| # | ID (winget) | Estado no wingetrs | Causa provável |
|---|---|---|---|
| 1 | Google.AndroidStudio | Ambiguous [Google.AndroidStudio, .Beta, .Canary] (ProductCode) | ProductCode compartilhado entre canais; sem desempate |
| 2 | Gyan.FFmpeg | Ambiguous [Gyan.FFmpeg, .Essentials, .Shared] (NameAndPublisher) | nome/publisher idênticos entre variantes; sem desempate |
| 3 | RubyInstallerTeam.RubyWithDevKit.4.0 | Ambiguous [RubyInstallerTeam.Ruby.3.4, RubyWithDevKit.3.4] (ProductCode) | instalado é 3.4.9-1; winget provavelmente usa seu registro de instalação próprio (pacote instalado via winget como 4.0). wingetrs parece mais fiel à versão instalada |
| 4 | Devolutions.UniGetUI | Ambiguous [Devolutions.UniGetUI, MartiCliment.UniGetUI.Pre-Release] (ProductCode) | ProductCode compartilhado com pré-release |
| 5 | wez.wezterm | Ambiguous [wez.wezterm, wez.wezterm.nightly] (ProductCode) | ProductCode compartilhado com nightly |
| 6 | MSYS2.MSYS2 | Unmatched | catálogo não tem ProductCode para MSYS2 (`map_productcode` vazio p/ o pacote e p/ `{31267c81-…}`); nome normalizado no catálogo é `msys2installer` vs ARP `MSYS2`. winget provavelmente casa pelo seu registro de instalação |
| 7 | Microsoft.DotNet.Native.Runtime | ausente (framework) | `appx_win.rs:10` descarta `IsFramework()` |
| 8 | Microsoft.VCLibs.Desktop.14 | ausente (framework) | idem |
| 9 | Microsoft.VCLibs.14 | ausente (framework) | idem |
| 10 | Microsoft.UI.Xaml.2.6 | ausente (framework) | idem |
| 11 | Microsoft.UI.Xaml.2.7 | ausente (framework) | idem |
| 12 | Microsoft.UI.Xaml.2.8 | ausente (framework) | idem |
| 13–21 | Microsoft.WindowsAppRuntime.1.1 … 1.8 e .2 (9 IDs) | ausente (framework); só aparecem "Windows App Runtime DDLM … Unmatched" | idem |

Contagem da classe framework: `grep ^MISS | grep -c -E "VCLibs|UI.Xaml|DotNet.Native|WindowsAppRuntime"` → `15`.

## Follow-ups propostos

- **F1 (search < 100 ms):** não materializar o catálogo a cada `search` — consulta SQL só com colunas
  de busca (`id, name, moniker, tags`, sem `raw`/`description`) e carga do `PackageEntry` completo só
  para os N resultados; ou índice persistido (ex.: FTS/trigram em SQLite) com fuzzy apenas nos candidatos;
  evitar o `drop` do vetor no fim do processo (`std::process::exit`/`mem::forget` no CLI). Teto atual: ~190–205 ms.
- **F2 (correlação ≥ 90%):** decidir sobre pacotes framework MSIX — incluí-los na coleta (como o winget)
  ou excluí-los explicitamente do critério na spec; sozinho leva a taxa de 81,9% para 94,1%.
- **F3 (Ambiguous):** desempate entre candidatos do mesmo ProductCode/nome (ex.: preferir o ID que é
  prefixo dos demais / sem sufixo de canal `.Beta/.Canary/.nightly/.Pre-Release`, ou o candidato cuja
  versão/ARP range contém a versão instalada). Potencialmente resolve 4 das 6 divergências não-framework (AndroidStudio, FFmpeg, UniGetUI, WezTerm; hipótese não testada).
- **F4 (update 304):** medir `update` no caminho 304 (requer `update --force` ou esperar 900 s) numa
  sessão com permissão de rede; meta < 300 ms ainda sem evidência.
- **F5 (list, margem):** medir tempo por coletor (ARP/MSIX/Scoop) com span por coletor; hipótese: MSIX.

## Mutação

**No mutation proof**: esta tarefa não entrega código de produto (só `scripts/bench.ps1` e este relatório).
