# Task 118 - Sonda S2: corpus 7z contra o 7z.exe

Status: DONE_WITH_CONCERNS (ver Concerns: 40% do corpus "termina em .7z" nao e 7z puro)

Commits: base 45e5b26; commit unico deste relatorio (SHA na mensagem final ao orquestrador; `git log --format='%h %s' 45e5b26..HEAD`).

Gate: nao aplicavel (sonda; nada em `src/`, `Cargo.toml` ou `Cargo.lock` do projeto; `git status` desta tarefa mostra so este relatorio).

Mutation proofs: no mutation proof (sonda). Checks: (1) cada `7z.exe` foi lancado com `subprocess.Popen(...).communicate(timeout=600)` (kill + communicate no timeout; nenhum caso chegou la) ou em primeiro plano no bash; `scripts/leftovers.ps1` no fim: `[OK] nenhum processo de agente sobrando (> 2 min)`. (2) Tudo em `target/probe-2b/` (corpus `corpus-7z/`, extracoes em `work-7z/` apagadas a cada arquivo, sonda Rust `s2/`, resultados `s2-results.json`, `s2-run.log`, `corpus.log`, `corpus-7z.json`); scripts Python no scratchpad da sessao. `catalog.db` aberto `file:...?immutable=1` (uri=True).

Ambiente: 2026-10-15/16, sozinho na maquina (nenhum outro agente ativo), 12 CPUs logicas, 7-Zip 26.04 (x64) em `C:\Program Files\7-Zip\7z.exe`, sevenz-rust2 =0.24.0, rustc 1.99.0. Sonda `target/probe-2b/s2` depende do wingetrs por `path = "../../.."` e chama `wingetrs::install::safepath::safe_components` em cada nome; teto proprio de 4 GiB declarados (soma de `ArchiveEntry::size`) e erro se uma entrada render mais bytes que o declarado.

## Corpus
Catalogo: 4115 manifestos scoop; 156 sem script com URL (64bit, senao raiz) terminando em `.7z` (inclui o truque `#/dl.7z`). Ordem por SHA-256 de `source/id`; 40 baixados, **1.343.536.283 B** no total, hash do manifesto conferido em todos (sha256/sha1 conforme prefixo). Descartados no caminho: 6 por tamanho > 150 MB (qq-nt, octave, okular, webview2, kawaii-player, azure-functions-core-tools), 1 hash errado (objconv: `objconv.zip#/dl.7z`), 2 erros de rede (idris 404, chroot timeout).

## Tabela por arquivo
Tempos: mediana de 3, wall-clock do processo (inclui start e Defender), segundos. rs1 = `set_thread_count(1)`, rs12 = 12 threads (processo filho separado). Metodos/dicionario/contagens: listagem do sevenz-rust2 (`Archive::open`). Tipo: `7z l -slt`.

| arquivo | bytes | tipo (7z l) | solido / blocos | metodos (sevenz-rust2) | dict max | arq/dirs | descomp. | 7z | rs1 | rs12 | resultado |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 00-pia-desktop | 24.939.400 | PE (`.exe#/dl.7z`) | - | - | - | - | 87.286.277 | 2,88 | - | - | nao suportado: `BadSignature(MZ)` |
| 01-zulip | 122.051.345 | 7z | nao / 74 | LZMA2+BCJ_X86 | 1 MiB | 74/2 | 397.342.292 | 2,48 | 10,52 | 3,61 | identico |
| 02-vcxsrv | 42.993.397 | Nsis | - | - | - | - | 91.526.720 | 11,85 | - | - | nao suportado (MZ) |
| 03-wexond | 55.939.420 | 7z | nao / 74 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 1 MiB | 74/3 | 180.873.562 | 3,53 | 4,86 | 5,24 | identico |
| 04-discord | 100.485.712 | 7z | sim / 2 | LZMA2; LZMA2+BCJ_X86 | 32 MiB | 95/12 | 351.395.178 | 6,34 | 9,16 | 11,78 | identico |
| 05-duckdns | 25.701 | 7z | nao / 1 | LZMA+LZMA+LZMA2+BCJ2 | 96 KiB | 1/0 | 93.184 | 0,05 | 0,03 | 0,03 | identico |
| 06-gnupg1 | 2.433.357 | Nsis | - | - | - | - | 6.805.791 | 0,27 | - | - | nao suportado (MZ) |
| 07-gitahead | 31.182.656 | PE | - | - | - | - | 71.458.051 | 2,04 | - | - | nao suportado (MZ) |
| 08-7ztm | 9.557.019 | 7z | sim / 2 | LZMA; LZMA+BCJ_X86 | 16 MiB | 2013/294 | 39.771.518 | 4,61 | 3,26 | 3,20 | identico |
| 09-git-annex | 27.286.765 | Nsis | - | - | - | - | 116.165.027 | 0,78 | - | - | nao suportado (MZ) |
| 10-awareness | 1.555.920 | PE | - | - | - | - | 2.009.443 | 0,08 | - | - | nao suportado (MZ) |
| 11-licecap | 256.960 | PE | - | - | - | - | 507.690 | 0,06 | - | - | nao suportado (MZ) |
| 12-dupeguru | 32.392.211 | Nsis | - | - | - | - | 118.718.848 | 2,99 | - | - | nao suportado (MZ) |
| 13-kdenlive | 134.973.971 | 7z SFX (`.exe#/dl.7z`) | - | - | - | - | 660.503.701 | 33,28 | - | - | nao suportado (MZ: SFX, assinatura 7z nao esta no offset 0) |
| 14-httping | 23.364 | 7z | nao / 1 | LZMA2+BCJ_X86 | 64 KiB | 1/0 | 49.664 | 0,05 | 0,03 | 0,03 | identico |
| 15-free-manga-downloader | 5.204.917 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 24 MiB | 76/6 | 21.424.525 | 0,57 | 0,70 | 0,69 | identico |
| 16-x265 | 3.065.581 | 7z | sim / 1 | LZMA+LZMA+LZMA2+BCJ2 | 64 MiB | 8/0 | 61.349.376 | 0,44 | 0,45 | 0,52 | identico |
| 17-nmkoder | 106.339.999 | 7z | sim / 2 | LZMA2; LZMA2+BCJ_X86 | 128 MiB | 120/17 | 286.304.176 | 4,83 | 6,99 | 6,77 | identico |
| 18-fastfetch | 795.825 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 4 MiB | 41/2 | 3.999.159 | 0,16 | 0,16 | 0,15 | identico |
| 19-inkscape | 110.359.970 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 512 MiB | 12152/803 | 660.170.071 | 49,48 | 28,71 | 32,33 | identico |
| 20-skylark | 12.739.499 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 48 MiB | 133/12 | 42.741.419 | 1,17 | 1,49 | 1,44 | identico |
| 21-pandoc-crossref | 13.893.977 | 7z | nao / 1 | LZMA2+BCJ_X86 | 32 MiB | 1/0 | 107.537.920 | 1,17 | 1,54 | 1,76 | identico |
| 22-dfu-programmer | 280.209 | 7z | sim / 2 | LZMA2; LZMA2+BCJ_X86 | 1,5 MiB | 29/16 | 1.048.836 | 0,16 | 0,13 | 0,13 | identico |
| 23-xl-converter | 59.608.130 | 7z | sim / 3 | LZMA2; LZMA2+DELTA; LZMA2+BCJ_X86 | 32 MiB | 699/146 | 251.050.622 | 5,89 | 7,32 | 5,30 | identico |
| 24-imagemagick-lean | 12.019.095 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 256 MiB | 23/0 | 258.742.701 | 1,50 | 2,42 | 2,82 | identico |
| 25-yubikey-personalization-gui | 15.068.200 | PE | - | - | - | - | 54.226.492 | 1,21 | - | - | nao suportado (MZ) |
| 26-srvman | 206.542 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 1 MiB | 33/5 | 850.263 | 0,19 | 0,10 | 0,10 | identico |
| 27-subconverter | 2.477.763 | 7z | sim / 2 | LZMA2; LZMA2+BCJ_X86 | 12 MiB | 441/34 | 10.509.385 | 1,57 | 0,96 | 0,91 | identico |
| 28-crow-translate | 36.318.795 | 7z | sim / 2 | LZMA2; LZMA2+BCJ_X86 | 16 MiB | 529/105 | 163.504.979 | 3,27 | 5,24 | 3,62 | identico |
| 29-deepgit | 23.596.707 | 7z | sim / 2 | LZMA2; LZMA2+BCJ_X86 | 32 MiB | 218/37 | 76.397.689 | 2,11 | 2,38 | 2,42 | identico |
| 30-winmerge2011 | 13.283.955 | 7z SFX (`.cpl#/dl.7z`) | - | - | - | - | 57.176.531 | 1,52 | - | - | nao suportado (MZ: SFX) |
| 31-elm | 5.626.081 | Nsis | - | - | - | - | 34.820.295 | 0,57 | - | - | nao suportado (MZ) |
| 32-unshield | 65.799 | 7z | sim / 1 | LZMA+LZMA+LZMA2+BCJ2 | 192 KiB | 3/0 | 152.576 | 0,06 | 0,04 | 0,05 | identico |
| 33-clementine | 108.901.781 | Nsis | - | - | - | - | 368.919.539 | 7,82 | - | - | nao suportado (MZ) |
| 34-git-tower | 90.122.352 | PE | - | - | - | - | 116.468.870 | 0,79 | - | - | nao suportado (MZ) |
| 35-linqpadless | 4.267.854 | zip (`.nupkg#/dl.7z`) | - | - | - | - | 10.588.052 | 0,24 | - | - | nao suportado (`BadSignature(PK)`) |
| 36-exercism | 4.226.173 | zip (`.zip#/dl.7z`) | - | - | - | - | 10.028.088 | 0,17 | - | - | nao suportado (PK) |
| 37-yuescript | 535.091 | 7z | nao / 1 | LZMA2+BCJ_X86 | 2 MiB | 1/0 | 2.045.952 | 0,12 | 0,11 | 0,12 | identico |
| 38-megui | 117.264.854 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 64 MiB | 184/10 | 150.310.962 | 1,55 | 1,71 | 1,82 | identico |
| 39-dn-famitracker | 11.169.936 | 7z | sim / 2 | LZMA2; LZMA+LZMA+LZMA2+BCJ2 | 64 MiB | 14/1 | 63.240.179 | 0,87 | 1,09 | 1,18 | identico |

"descomp." = bytes extraidos pelo 7z.exe (soma dos arquivos). Comparacao = conjunto de caminhos relativos (arquivos e diretorios) + SHA-256 + tamanho, 7z vs rs1 e rs1 vs rs12.

## Totais
- **Identicos: 24/24 dos 7z de verdade** (24/40 do corpus), tanto com 1 thread quanto com 12 threads. **Diferentes: 0.**
- **Nao suportados: 16/40**, todos por formato, nenhum por metodo: 12 instaladores PE/NSIS e 2 zip renomeados por `#/dl.7z` (o Scoop usa o 7-Zip como extrator universal), e 2 **7z SFX** (kdenlive, winmerge2011: arquivo 7z valido apos um stub PE; `ArchiveReader::open` so procura a assinatura no offset 0). Entre os 7z de verdade, nenhum metodo recusado: LZMA, LZMA2, BCJ_X86, BCJ2, DELTA.
- **Recusados (nome/atributo): 0.** Nenhum nome recusado pelo `safe_components`, nenhum anti-item, nenhum atributo reparse `0x400` nem symlink (bit `0x8000` + modo unix `S_IFLNK`) nos 24 7z. Nos 16 nao suportados nao ha listagem do sevenz-rust2 (nao medido).
- **Razao descompactado/compactado maxima: 21,5** (24-imagemagick-lean: 258.742.701 / 12.019.095); depois 20,0 (x265), 7,7 (pandoc-crossref); nos nao suportados a maior e 6,2 (elm, NSIS). Maior dicionario declarado: 512 MiB (inkscape, LZMA2:29); 3 arquivos com dict >= 256 MiB.
- Soma dos tempos (24 suportados, medianas): 7z 92,2 s, rs1 89,4 s, rs12 86,0 s (dominada pelo inkscape, ruidoso).

## Tempo do maior arquivo
- Maior do corpus: **13-kdenlive, 135 MB, nao suportado** (SFX); 7z.exe: 33,28 s (mediana).
- Maior suportado: **01-zulip, 122 MB** (nao solido, 74 blocos LZMA2+BCJ, 397 MB descompactados). Repetido em separado, 3 rodadas cada: 7z (padrao, MT) 2,57 / 2,82 / 2,66 s; `7z -mmt1` 7,19 / 7,33 / 7,21 s; **rs1 10,85 / 11,00 / 10,25 s** (interno 10,07-10,83 s); rs12 4,04 / 3,92 / 3,85 s. Com 1 thread o sevenz-rust2 fica ~1,45x atras do 7z com 1 thread.
- Maior descompactado: 19-inkscape (660 MB, 12.955 entradas): ruido alto, dominado por criacao de arquivos (Defender nao medido): 7z 47,9 / 53,9 / 61,1 s; rs1 72,6 / 51,3 / 24,8 s; rs12 28,3 / 25,0 / 26,8 s.
- 17-nmkoder (106 MB, solido): 7z 4,8 s; `-mmt1` 5,0-5,5 s; rs1 6,5-7,2 s; rs12 6,6-6,9 s (solido em 2 blocos: N threads nao ajuda).
- Conclusao para `spawn_blocking`: extracao com 1 thread leva de segundos a mais de 1 minuto no corpus; precisa sair do runtime async.

## Decisions
- Uma URL por manifesto (`architecture.64bit` se tiver `.7z`, senao raiz), como a ordem do brief; `#/nome.7z` conta como "termina em .7z" (e o que o Scoop ve).
- Tempos por wall-clock do processo filho (Python `perf_counter`), igual para 7z e sevenz-rust2; o tempo interno do s2 foi impresso tambem (diferenca ~0,17 s de start).
- Extracao do s2 pula entradas recusadas pelo `safe_components` (contando) e anti-itens; nao aplica atributos nem datas (a comparacao e so caminho + SHA-256 + tamanho).
- Repeti zulip/nmkoder/inkscape com as 3 rodadas visiveis e com `7z -mmt1` para separar ganho de multithread de velocidade do decodificador.

## Concerns
- **D1-1 nao cobre o corpus "termina em .7z" inteiro**: 40% (16/40) sao instaladores/zip/SFX que o Scoop extrai com 7-Zip. Para os 7z de verdade, 100% identicos. Um wingetrs so com sevenz-rust2 precisa recusar (ou rotear para outro caminho) PE/NSIS e localizar a assinatura em SFX (procurar `7z\xBC\xAF\x27\x1C` depois do stub; nao testado).
- Com `set_thread_count(1)` (modo exigido pela 117) o maior arquivo suportado leva ~10,5 s contra 2,6 s do 7z padrao; o MT do crate e o que tem threads sem join (Task 117).
- Meu parser do `7z l -slt` contou entradas errado (1 por arquivo, bug do script); por isso a tabela usa as contagens do sevenz-rust2, e a comparacao de arvore cobre a igualdade. Atributos reparse/symlink vieram so da listagem do sevenz-rust2.
- Tempos do inkscape (12 mil arquivos) variam 3x entre rodadas; efeito do Defender nao medido.
