# Fase 2a — validação (Task 40)

Data: 2026-10-10. Base: `f12a223`. Máquina: a do desenvolvimento (Windows 11, x64).
Binários: `cargo build --release` e `cargo build --release --examples` antes de qualquer medição.

## 1. Prova ao vivo (LOCALAPPDATA real)

Executada pelo orquestrador em 2026-10-10 com autorização do usuário, binário release de `ab2dbff`,
`LOCALAPPDATA` e `HKCU\Environment` reais. Três pacotes escolhidos para cobrir recursos diferentes:

| Pacote | Bucket | O que exercita |
|---|---|---|
| `ripgrep` 15.2.0 | main | `extract_dir`, lançador de shims (1ª instalação), PATH |
| `delta` 0.20.1 | main | `extract_dir`, shim sem lançador novo |
| `cpu-z` 3.01 | extras | `persist` de arquivo (hardlink), atalho no Menu Iniciar, `bin` com alvo renomeado |

Antes: `reg query HKCU\Environment` e `reg export` salvos; 50 `.lnk` sob `Start Menu\Programs`;
pasta `wingetrs Apps` inexistente. O `Path` do usuário é `REG_EXPAND_SZ` e contém `%USERPROFILE%\go\bin`.

Instalação (`install <p> --yes`, tempo de parede incluindo download): ripgrep 2034 ms, delta 1260 ms,
cpu-z 4501 ms; todos exit 0.

Conferências com tudo instalado:
- `Path` do usuário bruto (`reg query`) = valor anterior + `;C:\Users\jonyd\AppData\Local\wingetrs\shims`;
  tipo continua `ExpandString`, `%USERPROFILE%` preservado sem expandir.
- Com o PATH lido do registro (máquina + usuário, como num terminal novo), `where.exe` acha
  `wingetrs\shims\delta.exe` e `cpuz.exe`; `rg` resolve primeiro para o do Chocolatey (PATH da máquina
  vem antes) e o de `wingetrs\shims` aparece em 3º.
- `shims\rg.exe --version` → `ripgrep 15.2.0 (rev e89fff89ac)`; `delta --version` (pelo PATH) → `delta 0.20.1`.
  `cpuz` é GUI e não foi aberto.
- `CPU-Z.lnk` em `wingetrs Apps` aponta para `apps\cpu-z\current\cpuz_x64.exe`, que existe.
- `persist\cpu-z\cpuz.ini` tem 2 links (hardlink com a versão instalada).
- `wingetrs list` mostra os três com fonte `scoop:main`/`scoop:extras` e origem `wingetrs`.

Desinstalação (`uninstall <p> --yes`): ripgrep 1757 ms, delta 869 ms, cpu-z 1360 ms; todos exit 0.
O cpu-z informou "Manter cpuz.ini (dados persistentes; use --purge)".

Depois:
- `reg query HKCU\Environment` idêntico ao de antes (todas as linhas); `Path` continua `ExpandString`.
- Lista de `.lnk` idêntica (50); pasta `wingetrs Apps` removida.
- `apps\` vazio; `shims\` só com `.launcher\shim.exe`; `persist\cpu-z\cpuz.ini` mantido (esperado sem `--purge`).

Contagem de referência da entrada `shims` no PATH (o plano de cada `uninstall` lista "Desfazer entrada
no PATH" mesmo quando outro app ainda a usa): instalados ripgrep e delta, removido ripgrep → `shims`
continua no PATH; removido delta → sai. `HKCU\Environment` final idêntico ao inicial. O efeito está
certo; o texto do plano engana (pendência abaixo).

## 2. Ciclo completo em raiz temporária (substituto isolado, não é a prova ao vivo)

Ambiente: `LOCALAPPDATA=<scratchpad>\la` (catálogo copiado do real com `sqlite3 ".backup"`, aberto
`mode=ro` na origem; `search.idx` copiado), `WINGETRS_TEST_ENV_KEY=Software\wingetrs-test\bench-10543`,
`WINGETRS_TEST_SHORTCUT_DIR=<scratchpad>\t40-sc`. Nada em `HKCU\Environment` nem no Menu Iniciar reais.
Download real do ripgrep 15.2.0 e do lançador (rede).

Primeira instalação (download incluso):

```
Instalar ripgrep 15.2.0 (bucket main, 64bit)
  1. Instalar o lançador de comandos em …\la\wingetrs\shims\.launcher\shim.exe
  2. Baixar [1] https://github.com/BurntSushi/ripgrep/releases/download/15.2.0/ripgrep-15.2.0-x86_64-pc-windows-msvc.zip (ripgrep-15.2.0-x86_64-pc-windows-msvc.zip, sha256 71b2fef860abe467217a538ff31de02f5258807c0129f771846f87bd029aafc5)
  3. Extrair [1] … para …\apps\ripgrep\.staging-15.2.0 (só a subpasta ripgrep-15.2.0-x86_64-pc-windows-msvc)
  4. Mover …\apps\ripgrep\.staging-15.2.0 para …\apps\ripgrep\15.2.0
  5. Apontar …\apps\ripgrep\current -> …\apps\ripgrep\15.2.0 (junção)
  6. Criar comando rg em …\shims (rg.exe + rg.shim) -> …\apps\ripgrep\current\rg.exe
  7. Garantir …\la\wingetrs\shims no PATH do usuário
  8. Registrar ripgrep 15.2.0 (main) em …\state.db

Instalado: ripgrep 15.2.0
exit 0  2002 ms
```

Depois da instalação:

```
& "$env:LOCALAPPDATA\wingetrs\shims\rg.exe" --version  ->  ripgrep 15.2.0 (rev e89fff89ac)
rg.shim: path = "…\la\wingetrs\apps\ripgrep\current\rg.exe"
HKCU\Software\wingetrs-test\bench-10543  Path : …\la\wingetrs\shims
wingetrs list -> linha "ripgrep … 15.2.0 … scoop:main … wingetrs" (resumida: a saída original tem códigos ANSI)
(reg query HKCU\Environment /v Path) contendo "scratchpad": 0 linhas
```

Desinstalação:

```
Desinstalar ripgrep 15.2.0
  1. Apagar …\la\wingetrs\apps\ripgrep
  2. Desfazer atalho de linha de comando: rg
  3. Desfazer entrada no PATH: …\la\wingetrs\shims
Desinstalado: ripgrep 15.2.0
exit 0
```

Conferência após a última desinstalação: `Test-Path apps\ripgrep` = `False`; `shims\rg.*` = 0
arquivos; pasta de atalhos de teste = 0 itens; a chave de teste ficou **vazia** (sem o valor
`Path`), mas a própria chave `HKCU\Software\wingetrs-test\bench-10543` continua existindo (ver
Pendências). `shims\.launcher\shim.exe` permanece (lançador compartilhado, esperado). A pasta
temporária foi apagada ao fim.

Observação: a 1ª execução na raiz nova criou `http.json` e disparou a atualização em segundo plano
do catálogo (`refresh_if_stale`); as medições abaixo vêm depois, com 2 execuções de aquecimento.

## 3. Benchmarks

`scripts/leftovers.ps1` antes de medir: processos de **outro projeto** vivos desde 11:19
(`bash.exe` 23052/3968/10564 com `bun run check:q` de `dinoRig.ts`, `conhost.exe` 23764,
`python.exe` 26260). Não são desta tarefa; não foram encerrados. Medido mesmo assim, só deltas
contra `noop.exe` intercalado.

`install --yes` / `uninstall --yes` (download em cache) — `bench.ps1` não serve (a 2ª execução de
`install` falharia com "já instalado"), então um laço intercalado: a cada rodada `noop` → `install`
→ `noop` → `uninstall`, 2 aquecimentos + 15 medidas, delta = alvo − noop imediatamente anterior:

```
install ripgrep --yes | runs=15 warmup=2 | min=461,2 ms median=522,6 ms max=660,6 ms | delta median=496,6 ms
uninstall ripgrep --yes | runs=15 warmup=2 | min=72,5 ms median=80,8 ms max=113,9 ms | delta median=53,2 ms
baseline (noop) median=28,1 ms | execuções com exit != 0: 0
```

Meta `install --yes` com download em cache < 1 s: **atingida** (mediana 522,6 ms absoluta, delta 496,6 ms).

`install --dry-run` (reprodutível com `bench.ps1 -Baseline`):

```
target/release/wingetrs.exe install ripgrep --dry-run | runs=15 warmup=2 exit=0 | min=106,8 ms median=114,1 ms max=116,7 ms | baseline median=27,3 ms | delta median=86,2 ms
```

Tempo do planejador: **não há span do planejador no binário** (nenhum `info_span` em
`src/install` fora de `registry_win.rs`; `install -v` não registra etapas). Medido no exemplo
`oracle_2a` (função pura `resolve` + `plan_install`, fixture do ripgrep, 2000 iterações × 5 lotes):

```
planejador (resolve + plan_install, ripgrep): 2000 iterações x 5 lotes, µs/chamada min=12.5 mediana=13.3 max=14.0
```

Divisão interna dos ~410 ms entre `--dry-run` e `install --yes` (extração, junção, shim, registro,
aviso de ambiente): **não medido** (sem spans).

## 4. Oráculo de compatibilidade

`cargo run --release --example oracle_2a -- tests/fixtures/scoop/manifests %LOCALAPPDATA%\wingetrs\catalog.db 50`
(catálogo real aberto `SQLITE_OPEN_READ_ONLY`; só planeja, nada instala; arquitetura do host).
"Simples" = sem `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`, `installer`,
`uninstaller`, `psmodule`, `innosetup` na raiz nem nas variantes de arquitetura. Amostra = os 50
primeiros simples ordenados por SHA-256 de `fonte/id` (determinística e independente das checagens
do `resolve`).

```
host arch: X64
fixture bat-bin__main__alass.json: ok
fixture bin-alias__main__alist.json: ok
fixture depends__main__apimtemplate.json: ok
fixture env__main__android-cli.json: ok
fixture exe-direct__main__agent-browser.json: ok
fixture extract-dir__main__acli.json: ok
fixture md5__main__alist.json: ok
fixture persist-file-and-dir__main__i2p-zero.json: ok
fixture ps1-bin__main__concfg.json: ok
fixture ripgrep__main__ripgrep.json: ok
fixture sha1__main__astyle.json: ok
fixture sha512__main__antigravity-cli.json: ok
fixture shortcuts-persist__main__danser-go.json: ok
fixture tar-gz__main__air.json: ok
fixture unsup-7z__main__cdrtools.json: resolve: não suportado: download .7z: suportado na fase 2b
fixture unsup-innosetup__main__boost.json: resolve: não suportado: usa pre_install (PowerShell/instalador próprio): suportado na fase 2c
fixture unsup-msi__main__7zip19.00-helper.json: resolve: não suportado: download .msi: suportado na fase 2b
fixture unsup-script__main__7zip.json: resolve: não suportado: usa post_install (PowerShell/instalador próprio): suportado na fase 2c
fixture url-rename__main__agent-browser.json: ok
fixtures: 19 total, 15 ok, 4 unsup-* recusados como esperado, 0 divergentes
catálogo: 4090 manifestos scoop, 2694 simples
  falha scoop:extras/pia-desktop: resolve: não suportado: download .7z: suportado na fase 2b
  falha scoop:extras/zulip: resolve: não suportado: download .7z: suportado na fase 2b
amostra: 50 manifestos, 48 ok, 2 falhas
      2  resolve: não suportado: download .7z: suportado na fase 2b
  falha scoop:main/zigup: resolve: manifesto inválido: bin: item com 4 elementos
  falha scoop:extras/mqtt-spy: plan: caminho inseguro no manifesto: ..\..\..\shims\mqtt-spy.cmd
  falha scoop:main/llvm-arm64: resolve: sem variante para a arquitetura desta máquina
todos os simples: 2694 manifestos, 2365 ok, 329 falhas
      1  plan: caminho inseguro no manifesto
      1  resolve: manifesto inválido
    156  resolve: não suportado: download .7z: suportado na fase 2b
    120  resolve: não suportado: download .msi: suportado na fase 2b
     37  resolve: não suportado: download .nupkg: suportado na fase 2b
      5  resolve: não suportado: download .rar: suportado na fase 2b
      6  resolve: não suportado: download .tar.xz: suportado na fase 2b
      2  resolve: não suportado: download .zst: suportado na fase 2b
      1  resolve: sem variante para a arquitetura desta máquina
planejador (resolve + plan_install, ripgrep): 2000 iterações x 5 lotes, µs/chamada min=12.5 mediana=13.3 max=14.0
```

Amostra (50): openhardwaremonitor, flow, kimi-code, coursier, sc1, dhall, lagrange, ugene, regedix,
gitignore, shfmt, vncviewer, mold, nosleep, quickjs-ng, dovi-tool, tinynvidiaupdatechecker, ln, odo,
anyburn, pia-desktop, git-absorb, vial, sqlite, zonvie, aurora, mfgtools, weylus, sirikali, zulip,
torrent-file-editor, kubevpn, minify, less, hunt-and-peck, hello-http, z.lua, gbt, awesomebump,
tproxy, ddosify, nvcleanstall, 86box, arduino-cli, gosuki, keyferret, pci-z, offlineinsiderenroll,
auto-dark-mode, pixie.

Leitura:

- Fixtures: 15 suportadas planejam; as 4 `unsup-*` são recusadas com o motivo esperado; 0 divergentes.
- Amostra de 50: 48 planejam; as 2 falhas são downloads `.7z` (fora do nível 2a por decisão, `NãoSuportado`).
- Todos os 2694 simples: 2365 (87,8 %) planejam. 326 das 329 falhas são extensões fora do nível 2a.
  As 3 restantes:
  - `scoop:main/zigup` — `bin` com 4 elementos (`["zigup.exe","zigup","--path-link","\"$persist_dir\\zig\\zig.exe\""]`).
    O scoop provavelmente aceita (atribuição múltipla do PowerShell põe o resto em `$arg`); não
    conferido no código do scoop. Lacuna de compatibilidade do `resolve` (1 manifesto).
  - `scoop:extras/mqtt-spy` — atalho `..\..\..\shims\mqtt-spy.cmd`: recusa correta de `safepath`.
  - `scoop:main/llvm-arm64` — só tem variante `arm64`; recusa correta em x64.

## 5. Pendências

- Chave `HKCU\Software\wingetrs-test\bench-10543` ficou vazia após o `uninstall` (o valor `Path`
  saiu, a chave não). Apagada pelo orquestrador com autorização do usuário (`reg delete`, conferida vazia antes).
- O plano do `uninstall` mostra "Desfazer entrada no PATH: …\shims" mesmo quando outro app ainda usa a
  entrada (que corretamente fica). Deveria dizer que a entrada é mantida.
- `--purge` não foi exercitado ao vivo (`persist\cpu-z\cpuz.ini` ficou no disco do usuário).
- Sem spans por etapa no `install`/`uninstall`; a divisão do tempo de execução não foi medida.
- `bin` com mais de 3 elementos é recusado (zigup).
