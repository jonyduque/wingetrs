# Task 99 — Sonda de ícones para apps do Scoop

Status: DONE_WITH_CONCERNS

Commits:
```
9529f8f chore(tasks): scoop icon probe report for task 99
```
(+ o commit seguinte, que só preenche esta linha de SHA)

Gate: não se aplica — tarefa de sonda, nenhum arquivo de código mudou (só este relatório). `verify.ps1` não foi rodado.

Mutation proofs: no mutation proof — a tarefa não altera código; as duas checagens de "Checks beyond the steps" são o `git status --short` e os números com saída colada (abaixo).

## Método

- Binário: `target/release/wingetrs.exe` estava desatualizado (13/10 20:34, com 10+ commits em `src` depois). Compilado em `CARGO_TARGET_DIR=target/t99` no HEAD `4a1471e`, copiado para o scratchpad e `target/t99` apagado.
- Raiz temporária `<S>\t99-root\wingetrs` com o catálogo copiado pelo `python -I … s.backup(d)` da Task 77 + `config.toml`. Todos os `show` com `LOCALAPPDATA=<S>\t99-root`. O primeiro `show` de fumaça disparou o refresh automático em segundo plano (o catálogo copiado não tinha `http.json`), que atualizou a cópia; a amostra foi congelada em `sample.json` **depois** disso. Nenhum `show` medido teve refresh: `bg_update=False` nos 96 (contador abaixo).
- Os manifestos do Scoop não estão em pastas de bucket: ficam na coluna `raw` de `packages` (`source = scoop:main|scoop:extras`) do `catalog.db`. A amostra sai dali.
- Scripts: `<S>\scoop-icon-probe\{common,path_w,path_h,analyze,w_icons}.py`; resultados brutos `w.json`, `h.json`, `analysis.txt`, `w_icons.txt` no mesmo diretório.
- W, rede: soma de `time.busy + time.idle` dos spans `show.version_data` e `show.manifest` (`-vv`, stderr). Spans async ficam *idle* esperando a rede, por isso a soma. Cada `show` é um processo novo, com um handshake TLS novo.
- H: `urllib` com timeout de 10 s **total** (leitura em blocos com prazo), limite de 1 MiB, UA `wingetrs-probe`, ≥ 200 ms entre requisições ao mesmo host, Pillow 10.4.0 para as dimensões.

Saída da verificação de sanidade do W (os 96 `show`):
```
96 Counter({(0, False, 2, 2, False): 63, (0, True, 2, 2, False): 33})
```
(chave = rc, tem Icons, blobs novos, spans de rede, refresh em segundo plano)

## Amostra (random.seed(99), 75 + 75)

main: ls-lint, langsmith-cli, fake, rathole, dust, gel-arm64, git-quick-stats, curlie, vfox, cc65, git-xargs, trufflehog, lazyssh, pastel, stdiscosrv, tcc, pget, cedar, rumdl, ntfs2btrfs, fabric-ai, markdown-oxide, tor, resticprofile, quarkus-cli, force, x265, wixtoolset, kubent, lemminx, starpls, flow, difftastic, msixmgr, slang, promtail, jdtls, cangjie-lts, ticker, pandoc, srecord, waypoint, ccat, naiveproxy, hub, loreserver, awsqueue, go-size-analyzer, powerping, agentsview, maven, fff, kiota, pdfbox, cake, msmpi, oracle-instant-client-sdk, eza, recycle-bin, msys, bbdown, kalk, csview, fontreg, tgrep, docker, upbound-cli, wuzz, mimo-code, railway, dum, offlineinsiderenroll, duckdb, avrdude, cmkr

extras: archwsl, game-editor, sts, rider, compactgui, rune, coretemp, famistudio, msklc, adventuregamestudio, prime95, notepadnext, esearch, updatehub, rayburst, helm-ls, bulk-rename-utility, i2pdbrowser, sidebar-diagnostics, laragon, wealthfolio, raspberry-pi-imager, eclipse-modeling, otvdm, sunshine, buttercup, electron-fiddle, firefox, rebased, eclipse-committers, dbvis, duality, fastflix, mirthsync, geany, gimp, redis-tui, feem, hterm, winmute, neohtop, thunderbird-esr, joplin, drive-letter-changer, pdfcraft, goland, clash-party, tinode-mysql, mediainfo-gui, altools, z, bleachbit, ocauxiliarytools, imgburn, poedit, quicklook, obs-studio-small, quicksfv, proxybridge, reqable, pine64-updater, vieb, czkawka-gui, gitify, bambu-studio, pixelorama, quickcpu, signal, jenv, mujoco, iwck, mtuner, didder, museeks, injector

## Caminho W (winget)

```
== W ==
index build (catalog read, excluded): 169 ms
candidatos por app (6 = >5): {0: 73, 1: 66, 2: 8, 4: 1, 6: 2}
apps com >=1 candidato: 77
shows executados: 96 falhas: 0
passam no domínio (0/1/2+): Counter({0: 95, 1: 51, 2: 4})
par único: 51 | com Icons: 15
pares via github.com: 20 | com Icons: 3
pares github.com com mesmo dono no caminho: 19 | com Icons: 3
domínios dos pares com >1 app: {'github.com': 20, 'jetbrains.com': 2}
manifestos winget vistos com Icons: 33 de 96
rede W por app (só apps com show): n=77 mediana=1108 ms p90=2270 ms max=5722 ms soma=106169 ms
parede W por app (processo inteiro, só apps com show): n=77 mediana=1188 ms p90=2428 ms max=6118 ms soma=114131 ms
requisições W: 192 | por app com show: 2.49
```
Extra:
```
W pares vindos de apps com >2 candidatos: 0 | >1: 2
W com ícone e >1 candidato: 1
W rede por show: 1103 ms mediana
W ícone em apps github.com: 3
```
- Candidatos: 73/150 sem nenhum, 66 com 1, 8 com 2, 1 com 4, 2 com > 5.
- Par único: 51/150. Com `Icons`: 15/150 (10 %).
- Rede por app com candidato: mediana 1108 ms, p90 2270 ms, máximo 5722 ms. Apps sem candidato custam 0 de rede.
- O par github.com→github.com (20 de 51) é fraco pelo critério de domínio. 1 dos 20 tem dono diferente no caminho (compactgui: `ImminentFate/CompactGUI` vs `IridiumIO/CompactGUI`, aparentemente o mesmo projeto transferido), e nenhum dos 20 é par errado.

## Caminho H (homepage)

```
== H ==
pulados: {'github.com': 53}
homepages buscadas: 97
ícone válido: 38 por fonte: {'apple-touch-icon': 29, 'icon': 7, 'og:image': 2} formatos: {'PNG': 37, 'ICO': 1}
fonte escolhida (todas as tentativas): {'og:image': 16, 'apple-touch-icon': 29, 'icon': 17}
tipos presentes no <head>: {'og:image': 47, 'apple-touch-icon': 29, 'icon': 47}
apps com algum SVG candidato: 20 | só SVG: 4
falhas: {'ícone fora de tamanho/proporção': 24, 'só SVG': 4, 'sem <head> útil': 27, 'http 429': 1, 'timeout': 1, 'http 403': 1, 'http 503': 1}
tempo H por app (buscadas): n=97 mediana=866 ms p90=2448 ms max=10510 ms soma=114132 ms
requisições H: 159
bytes H: 10385930 (9.9 MiB) | por app buscada: 105 KiB
bytes nas apps com ícone (página+ícone): n=38 mediana=127537 B p90=418085 B max=528163 B soma=6040723 B
```
Extra:
```
H ok com tempo >5s: 0 | >3s: 3 | max ok ms: 3366
H buscadas >5s (todas): 1
dims fora: [[1200, 630], [32, 32], [37, 37], [16, 16], [600, 330], [2400, 1260], [16, 16], [16, 16], [32, 32], [3200, 1800], [1200, 630], [1200, 630], [1200, 630], [16, 16], [20, 20], [332, 673], [1200, 630], [1200, 630], [32, 32], [1280, 672], [25, 25], [1200, 630], [1280, 695], [32, 32]]
```
- `github.com`: 53/150 (35 %) pulados. Nenhuma app sem homepage.
- Cobertura: 38/150 (25 %), ou 38/97 das homepages buscadas. Por fonte, os 38 se dividem em `apple-touch-icon` 29, `icon` 7 e `og:image` 2.
- `og:image` foi escolhido 16 vezes e passou só 2. Quase sempre é um banner 1200×630, que cai na proporção.
- SVG: 20 apps têm algum candidato SVG e 4 só têm SVG (git-quick-stats, pandoc, rayburst, vieb). O SVG foi excluído da ordem de download.
- Falhas:

  | Tipo | Apps |
  |---|---|
  | sem `<head>` útil | 27 |
  | ícone fora de tamanho/proporção | 24 |
  | só SVG | 4 |
  | timeout | 1 (cangjie-lang.cn) |
  | 403 | 1 (raspberrypi.org) |
  | 429 | 1 (x265.org) |
  | 503 | 1 (buttercup.pw) |

- Tempo por homepage buscada: mediana 866 ms, p90 2448 ms, máximo 10510 ms (o timeout). Nos acertos o máximo foi 3366 ms, e nenhum passou de 5 s.
- Bytes: 9,9 MiB no total, 105 KiB por homepage buscada.

## Comparação

```
W sozinho: 15/150 | H sozinho: 38/150 | W∪H: 45/150 | ambos: 8
W: tempo total 106.2 s, média 708 ms/app, req 192 (1.28/app)
H: tempo total 114.1 s, média 761 ms/app, req 159 (1.06/app)
W→H: tempo total 204.2 s, média 1361 ms/app, req 329 (2.19/app)
H→W: tempo total 188.8 s, média 1259 ms/app, req 293 (1.95/app)
```
| Ordem | Cobertura | Tempo médio/app (sequencial) | Requisições/app |
|---|---|---|---|
| W sozinho | 15/150 (10 %) | 708 ms | 1,28 |
| H sozinho | 38/150 (25 %) | 761 ms | 1,06 |
| W então H | 45/150 (30 %) | 1361 ms | 2,19 |
| H então W | 45/150 (30 %) | 1259 ms | 1,95 |

A cobertura combinada é a mesma nas duas ordens. Muda só qual ícone vence nas 8 apps em que os dois acertam, além do custo.

### Conferência manual

**W, os 15 pares únicos com `Icons`.** Ícones baixados do CDN com dimensões e bytes (`w_icons.txt`):
```
W docker | https://docs.docker.com/engine -> Docker.DockerDesktop | https://www.docker.com/products/docker-desktop | ico:256x256/113242B
W updatehub | https://github.com/NexovaDev/UpdateHub -> Nexova.UpdateHub | https://github.com/NexovaDev/UpdateHub | ico:256x256/175054B
W laragon | https://laragon.org/ -> LeNgocKhoa.Laragon | https://laragon.org/ | ico:256x256/113907B
W rebased | https://github.com/DetachHead/rebased -> detachhead.rebased | https://github.com/DetachHead/rebased | ico:256x256/410598B
W dbvis | https://www.dbvis.com/ -> DBVis.DBVisualizer | https://www.dbvis.com/download/ | ico:256x256/121348B
W winmute | https://github.com/lx-s/WinMute -> LX-Systems.WinMute | https://github.com/lx-s/WinMute | ico:256x256/112011B
W joplin | https://joplinapp.org -> Joplin.Joplin | https://joplinapp.org/ | ico:256x256/37145B
W goland | https://www.jetbrains.com/goland/ -> JetBrains.GoLand | https://www.jetbrains.com/go/ | ico:256x256/304886B
W mediainfo-gui | https://mediaarea.net/en/MediaInfo -> MediaArea.MediaInfo.GUI | https://mediaarea.net/MediaInfo | ico:256x256/35025B
W bleachbit | https://www.bleachbit.org/ -> BleachBit.BleachBit | https://www.bleachbit.org/ | ico:256x256/60468B
W imgburn | http://www.imgburn.com/ -> LIGHTNINGUK.ImgBurn | https://www.imgburn.com/index.php?act=download | ico:256x256/102852B
W vieb | https://vieb.dev -> Jelmerro.Vieb | https://vieb.dev | ico:256x256/50982B
W gitify | https://www.gitify.io -> Gitify.Gitify | https://www.gitify.io/ | ico:256x256/15189B
W pixelorama | https://orama-interactive.itch.io/pixelorama -> OramaInteractive.Pixelorama | https://orama-interactive.itch.io/pixelorama | ico:256x256/16986B
W signal | https://signal.org -> OpenWhisperSystems.Signal | https://www.signal.org/ | ico:256x256/192369B
```
Resultado: 0 errados e 1 duvidoso. O `docker` do Scoop é o CLI/engine e foi pareado com o Docker Desktop: a marca é a mesma, o produto não. Os 15 ícones são ICO 256×256 de 15 a 411 KB.

**H, os 15 primeiros ícones válidos** (de `analysis.txt`):
```
H fake | https://fake.build -> apple-touch-icon apple-touch-icon.png [180, 180]
H gel-arm64 | https://www.geldata.com -> apple-touch-icon apple-touch-icon.png [180, 180]
H vfox | https://vfox.dev/ -> icon logo.png [1024, 1024]
H git-xargs | https://blog.gruntwork.io/introducing-git-xargs-an-open-source-tool-to-update-multiple-github-repos-753f9f3675ec -> apple-touch-icon social-icon-glacier-on-midnight-2x.png [256, 256]
H stdiscosrv | https://syncthing.net/ -> apple-touch-icon apple-touch-icon-152x152.png [152, 152]
H fabric-ai | https://danielmiessler.com/p/fabric-origin-story -> icon favicon.png [200, 200]
H resticprofile | https://creativeprojects.github.io/resticprofile/ -> icon logo.png?1776264364 [540, 540]
H slang | https://shader-slang.com/ -> apple-touch-icon apple-touch-icon.png [180, 180]
H jdtls | https://projects.eclipse.org/projects/eclipse.jdt.ls -> og:image eclipse-foundation-400x400.png [400, 400]
H loreserver | https://lore.org -> apple-touch-icon apple-touch-icon.png [180, 180]
H fff | https://fff.dmtrkovalenko.dev -> apple-touch-icon apple-touch-icon.png [180, 180]
H oracle-instant-client-sdk | https://www.oracle.com/database/technologies/instant-client.html -> apple-touch-icon favicon-180.png [180, 180]
H eza | https://eza.rocks -> apple-touch-icon apple-touch-icon.png [180, 180]
H msys | https://sourceforge.net/projects/mingw/ -> apple-touch-icon logo-180x180.png [180, 180]
H docker | https://docs.docker.com/engine -> apple-touch-icon apple-touch-icon.png [180, 180]
```
Resultado: 3 errados e 2 genéricos.
- Errados: git-xargs recebeu o ícone do blog da Gruntwork, fabric-ai o favicon do blog do autor e msys o logo do SourceForge.
- Genéricos: jdtls recebeu o logo da Eclipse Foundation e oracle-instant-client o logo da Oracle. São da organização certa, mas não do produto.
- A causa é a homepage apontar para um blog ou para uma plataforma de hospedagem (`sourceforge.net`), e não para o site do produto.

## Recomendação

**Ordem: W então H.** A cobertura é a mesma (45/150 = 30 %). W→H custa cerca de 100 ms/app a mais na média (1361 contra 1259 ms) e 0,24 requisição/app a mais. Em troca, o ícone do W vence nas 8 apps em que os dois acertam:
- vem do CDN com `IconSha256` verificado;
- é ICO 256×256 em 15 de 15;
- teve 0 de 15 errados, contra 3 de 15 errados no H.

Além disso, W é o único caminho que cobre apps com homepage no github.com: 3 ícones aqui, e H pula essas apps (35 % da amostra).

**Limites:**
- **W:** até **2** candidatos por app. Nenhum par único veio de app com mais de 2 candidatos (`>2: 0`). Isso corta os casos de 4 e 6 candidatos, que só custaram rede. Manter o par único por domínio registrável. Para `github.com`, exigir também o mesmo dono no caminho: 19 de 20 dos pares passam e o 20º é uma transferência legítima.
- **H:** timeout total de **5 s**. Nenhum acerto passou de 3366 ms e só 1 tentativa passou de 5 s, que foi o timeout. Limite de **1 MiB** para a página; o maior par página+ícone foi 528 163 B.
- **H, ordem dos candidatos:** `apple-touch-icon` primeiro, depois `link rel=icon` com `sizes` ≥ 64. Tirar `og:image` da ordem (2/16 de aproveitamento; banner 1200×630). Pular `github.com` e também `sourceforge.net`, `blog.*` e `*.medium.com`, que são as fontes dos 3 erros vistos.
- **H, SVG:** só 4/97 dependeriam de SVG; ignorar.
- **Cache:** guardar o resultado por app, inclusive "sem ícone", para não repetir a rede.

**Custo grosseiro de uma página de 15 itens do Scoop** (W→H, cache frio, médias da amostra):
- Requisições: 15 × 2,19 ≈ **33**.
- Tempo sequencial: 15 × 1,36 s ≈ **20 s**.
- Tempo em paralelo: com 4 a 6 em paralelo, a página fica perto do p90 de uma app, cerca de **2,5 a 5 s**. Esse número é extrapolação, não medido.
- Download: o W baixa 112 011 B de ICO por acerto (mediana dos 15). O H, 105 KiB por homepage buscada.
- Cache quente: 0 requisições.
- O tempo W medido inclui um handshake TLS por `show` (processo novo). Dentro da TUI, com a conexão reaproveitada, deve ser menor (não medido).

## Checks

`git status --short` (antes do commit):
```
?? .claude/tasks/task-99-base.txt
?? .claude/tasks/task-99-report.md
```
(`task-99-base.txt` é do orquestrador e já estava lá no início.)

Limpeza: `t99-root` (com o catálogo copiado), as pastas de download, o binário copiado e `target/t99` foram apagados. Saída do leftovers:
```
Carregado em 252ms
[OK] nenhum processo de agente sobrando (> 2 min)
```

## Decisions

- O IPv6 desta máquina não responde: `urllib` levou 40 s em syncthing.net, contra 0,7 s no `curl`. A primeira rodada do H foi morta e refeita com os endereços IPv4 primeiro, para aproximar o happy eyeballs do hyper. O timeout de 10 s passou a valer para a requisição inteira, não por operação de socket.
- SVG ficou fora da ordem de download, porque o `image` do Rust não o decodifica, e é contado à parte. Ordem dentro do H: `apple-touch-icon` (maior `sizes` primeiro), depois `icon`, depois `og:image`.
- Domínio registrável: últimos 2 rótulos, ou 3 quando o sufixo está numa lista curta de sufixos multi-rótulo e de plataformas (`github.io`, `pages.dev`, `itch.io`…). Não usei a PSL completa, que não está disponível.
- No W, as dimensões vêm de um download à parte dos `IconUrl` dos 15 pares (`w_icons.py`), fora do tempo medido.

## Concerns

- O tempo W mede processos `show` isolados (um TLS por processo), então superestima o custo dentro da TUI. O quanto é não medido.
- O H tenta só o primeiro candidato. 24 apps falharam em tamanho/proporção, e várias tinham um segundo candidato que talvez passasse; o ganho de tentar o próximo é não medido.
- A amostra é pequena: 15 acertos no W e 38 no H. As taxas de erro (0/15 e 3/15) têm intervalo largo.
