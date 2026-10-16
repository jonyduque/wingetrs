# Task 117 - Sonda S1: sevenz-rust2, msi, cab

Status: DONE_WITH_CONCERNS (ver Concerns)

Commits: base 450a3c3; commit unico do relatorio (SHA na mensagem final).

Gate: nao aplicavel (sonda; nenhum codigo de produto; Cargo.toml/Cargo.lock do projeto intocados; o `git status` desta tarefa mostra so este relatorio).

Mutation proofs: no mutation proof (sonda; "Checks beyond the steps" so pede git status e leftovers).

Ambiente: rustc/cargo 1.99.0, 2026-10-15. Sonda em `target/probe-2b/s1` (workspace vazio proprio), fixtures em `target/probe-2b/fx`. Todos os tempos de build: **sob carga** (2 a 7 `cargo.exe` de outros agentes rodando).

## Versoes (crates.io, `cargo search`, 2026-10-15)
sevenz-rust2 0.24.0, msi 0.10.0, cab 0.6.0 (lzma-rust2 0.22.0 vem junto). Fixadas com `=`.

## 1. MSRV e duplicatas
`rust-version` publicado (grep nos Cargo.toml do registry): sevenz-rust2 **1.93**; lzma-rust2 1.85; ppmd-rust 1.82; bzip2 0.6.1 1.82; cfb 1.74; time 1.88 (ja no lock). **msi e cab nao declaram rust-version** (edition 2021); piso efetivo = o das dependencias (time 1.88 / cfb 1.74). Nao medido compilando com 1.89 (so ha o toolchain 1.99 e um nightly; nao baixei outro). Projeto declara 1.89: so sevenz-rust2 obriga subir (para 1.93, como a spec preve).

`cargo tree -d` na sonda: "nothing to print" (zero duplicatas dentro da sonda). Contra o `Cargo.lock` do wingetrs (comparacao nome+versao, 45 pacotes na arvore normal):
- Ja identicos no lock: adler2, block-buffer 0.12.1, cfg-if, const-oid, core_detect, cpufeatures 0.3.1, crc32fast 1.5.2, crypto-common 0.2.2, digest 0.11.3, encoding_rs, flate2 1.1.10, fnv, getrandom 0.4.3, hybrid-array, miniz_oxide 0.9.1, multiversion_no_op, num-conv, powerfmt, scopeguard, simd-adler32, simdutf8, time 0.3.55, time-core, typenum, web-time.
- Versao diferente (a sonda resolveu mais novo; com o lock do projeto o cargo reutiliza o compativel): sha2 sonda 0.11.1 vs lock 0.11.0 (e 0.10.9); deranged 0.5.9 vs 0.5.8; uuid 1.28.0 vs 1.27.0.
- **Novos no lock** (16): aes, block-padding, byteorder, bzip2, cab, cbc, cfb, cipher, cpubits, inout, libbz2-rs-sys, lzma-rust2, lzxd, msi, ppmd-rust, sevenz-rust2.
- Colisao com `zip` 8.6.0 / flate2 / sha2 / crc32fast: nenhuma incompatibilidade; compartilham flate2 1.1.10 e crc32fast 1.5.2. Nota: `cab` usa flate2 com backend padrao (miniz_oxide), o projeto usa `zlib-rs`; o cargo unifica features (efeito nao medido).

## 2. Licencas (`cargo tree -e normal --format "{p} {l}"`)
sevenz-rust2 Apache-2.0; msi MIT; cab MIT; lzma-rust2 Apache-2.0; cfb MIT; lzxd MIT OR Apache-2.0; bzip2 MIT OR Apache-2.0; **libbz2-rs-sys: bzip2-1.0.6**; **ppmd-rust: CC0-1.0 OR MIT-0**; **encoding_rs: (Apache-2.0 OR MIT) AND BSD-3-Clause** (exige aviso de atribuicao); byteorder Unlicense OR MIT; fnv Apache-2.0/MIT; resto MIT OR Apache-2.0. Nenhuma GPL/LGPL/copyleft. Nenhuma dependencia C (libbz2-rs-sys e ppmd-rust sao Rust puro).

## 3. Tamanho e tempo (release, strip padrao, mesmo main.rs com features; target dir limpo por variante)
| variante | exe (bytes) | delta vs base | build limpo (sob carga) |
|---|---|---|---|
| base | 139.264 | 0 | 4,43 s |
| s7 (sevenz-rust2 + lzma-rust2 xz) | 772.608 | +633.344 | 37,71 s |
| msi | 806.400 | +667.136 | 14,06 s |
| cab | 243.712 | +104.448 | 8,24 s |
| todas | 1.498.624 | +1.359.360 | 58,64 s |

Os deltas incluem o codigo de teste da sonda que usa cada crate. msi pesa por encoding_rs (tabelas de codepages). Ressalva: liguei a feature `xz` de lzma-rust2 por conta propria (sevenz-rust2 usa lzma-rust2 sem `xz`).

## 4. 7z
**Listar antes de decodificar** (sem decodificar nada; `ArchiveReader::open(path, Password::empty())?.archive()`):
```
pub struct Archive { pub blocks: Vec<Block>, pub files: Vec<ArchiveEntry>, pub stream_map: StreamMap, pub is_solid: bool, .. }
pub struct ArchiveEntry { pub name: String, pub has_stream: bool, pub is_directory: bool, pub is_anti_item: bool,
  pub has_windows_attributes: bool, pub windows_attributes: u32, pub has_crc: bool, pub crc: u64, pub size: u64, pub compressed_size: u64, .. }
pub struct Block { pub coders: Vec<Coder>, pub has_crc: bool, pub crc: u64, .. }   // Block::get_unpack_size() -> u64
impl Coder { pub fn encoder_method_id(&self) -> &[u8]; pub fn properties(&self) -> &[u8] }
Archive::open(path) -> Result<Archive, Error>;  Archive::read(&mut R, &Password)
ArchiveReader::{open, new, from_archive, archive(), set_thread_count(u32),
  for_each_entries(FnMut(&ArchiveEntry, &mut dyn Read) -> Result<bool, Error>), read_file(name),
  file_compression_methods(name, &mut Vec<EncoderMethod>)}
```
Saida real (`s1 list7z t.7z`): `block0 props=[03] coder=Some("LZMA2") id=[21] unpack=10000`; `"sub/b.bin" size=4000 dir=false anti=false has_stream=true attr=0x2020(has=true) crc=0x9ee1b9b9`; diretorios e arquivo vazio com `has_stream=false`. **O dicionario nao vem pronto**: e preciso decodificar `properties()` (LZMA2: 1 byte; LZMA: byte lc/lp/pb + 4 bytes LE) para aplicar teto antes de decodificar.

**xz**: `lzma_rust2::XzReader::new(reader, allow_multiple_streams)` / `new_mem_limit(.., mem_limit_kb)` com a feature `xz` (puxa `sha2`, ja no lock); Rust puro. `tar -cJf` (GNU tar 1.35) e `tar.exe -cJf` (bsdtar do Windows) decodificados: SHA-256 do fluxo == `xz -dc` (c58dc1cc.. e 1095da59..), `tar -tf` lista `./a.txt ./empty.txt ./emptydir/ ./sub/ ./sub/b.bin`. Existe tambem `XzReaderMt` (usa `work_pool` com catch_unwind + join; nao usei). **D3-1 (+.tar.xz) e viavel sem dependencia nova**, ligando a feature `xz` de lzma-rust2.

**Mutacoes** (`s1 fuzz7z`, semente xorshift fixa 0x9E3779B97F4A7C15, cada caso num processo filho, limite de 10 s):
- 90 flips (1 a 3 bytes) + 10 truncamentos (1/11..10/11) de `t_hc.7z` (cabecalho sem compressao) e de `t.7z` (cabecalho comprimido): 100 casos cada -> **0 panico, 0 abort, 0 travamento, 100% erro tipado** (pior caso 359 ms).
- Dirigido ao cabecalho (500 flips nos ultimos 250 B + 10 truncamentos) em `t_hc.7z`, `t_lzma.7z` (LZMA em vez de LZMA2) e (120 B) `t.7z`: 510 casos cada -> 1 a 3 "ok" (flip inofensivo), resto erro tipado; **0 panico/abort/hang**. Repetido com `set_thread_count(4)` em `t_hc.7z`: 510 casos, 0/0/0. `big.7z` (300 MiB de zeros, 300 flips, 4 threads): 310 casos, 0/0/0, pior 2,37 s.
- Ressalva: fixtures de 4 KB, um unico emissor (7-Zip); nao e fuzzer guiado por cobertura. O filho roda com `catch_unwind` e conta saida 101 como panico; nenhum caso chegou la.

**Dicionario de 1,5 GiB declarado** (`dict15g.7z`: `t_hc.7z` com o byte de propriedade LZMA2 trocado por 37 = 3<<29 e CRCs do cabecalho refeitos): listagem aceita; extracao OK (10000 bytes) com **pico de working set 6 MiB** (1 e 4 threads). O decodificador nao pre-aloca o dicionario: `LzDecoder` cresce o buffer conforme a saida. Com saida grande o pico acompanha a saida ate o teto do dicionario: `big.7z` (300 MiB de zeros, dict 64 MiB) pico WS 71 MiB (1 thread) / 429 MiB (4 threads); o mesmo arquivo com dict 1,5 GiB declarado: WS 263 MiB (1 thread) / 621 MiB (4 threads), commit 387 / 773 MiB (polling de `Process.PeakWorkingSet64`/`PeakPagedMemorySize64` a cada 2 ms em `target/probe-2b/mem.ps1`; valores sao piso). **Nao ha alocacao imediata de 1,5 GiB, mas tambem nao ha limite**: `reader.rs:18` fixa `MAX_MEM_LIMIT_KB = usize::MAX/1024`, e o caminho LZMA (nao LZMA2) nem checa. Uma bomba (saida enorme, dict ate 4 GiB) cresce ate o teto. O wingetrs precisa recusar antes de decodificar: somar `ArchiveEntry::size`, checar `Block::get_unpack_size` e decodificar `properties()` para limitar o dicionario.

**Threads/panico**: o crate cria threads proprias **por padrao**: `ArchiveReader::new` usa `available_parallelism()` e, para LZMA2 com `threads >= 2`, `decoder.rs:119` usa `Lzma2ReaderMt`, que faz `thread::spawn` (`lzma2_reader_mt.rs:116`) **sem `catch_unwind`**; seu `Drop` (l.454-461) so sinaliza shutdown e **nao junta** os handles (comentario "JoinHandles will be dropped"). Um panico no worker mata a thread sem gravar erro (so o ramo `Err(io)` chama `set_error`); o panico nao chega ao chamador. Nao consegui provocar panico no worker (nenhuma mutacao panicou): isto e **leitura de codigo, nao medido ao vivo**. Mitigacao: `reader.set_thread_count(1)` seleciona `Lzma2Reader` sem threads; ai a regra de juntar threads nao se aplica e o `catch_unwind` (D4-1) cobre o resto. O `work_pool` (xz MT e escrita) tem `catch_unwind` + join, mas nao esta no caminho de leitura de 7z.

## 5. MSI / CAB
**MSI minimo** (`s1 msigen`): `Package::create(PackageType::Installer, Cursor)`; `create_table` Directory/Component/File/Media; `insert_rows(Insert::into(..).row(vec![Value::from(..), Value::Int(..), Value::Null]))`; `write_stream("cab1")` com o `mszip.cab` do makecab (230.496 B). Releitura pela mesma crate: tabelas `[Component, Directory, File, Media, _Columns, _Tables, _Validation]`, linhas voltam (`File: ["F1","C1","hello.txt",5,1]`, `Media: [1,1,"#cab1"]`), `streams=["cab1"]`, **stream do cab identico byte a byte** (`roundtrip equal=true len=230496`).

**Ressalva**: o **Windows Installer (`WindowsInstaller.Installer.OpenDatabase`, leitura) recusa o MSI gerado**, mesmo com `set_arch`, `set_uuid`, `set_page_count(200)`, `set_word_count`. Controle: um MSI do sistema (`C:\Windows\Installer\107cd.msi`, tambem uma copia na pasta da sonda) abre. Causa nao investigada (minha tabela minima pode estar incompleta, ou a escrita da crate). Nao afeta o uso do wingetrs (so le MSI), mas **o MSI gerado aqui nao serve de fixture para `msiexec`**.

**Leitura de MSI real** (`s1 msiread` no MSI do sistema, somente leitura): `type=Installer`, 32 tabelas, `Directory rows=37`, `Component rows=48`, `File rows=131`, `Media: [1,131,"1","#SPPREDIST.CAB","OFFICE16",NULL]`, `streams=["SPPREDIST.CAB","Binary.OCFXCA",..]`: as tabelas Media/File e o cab embutido sao alcancaveis. Nao extrai o cab embutido desse MSI com a crate `cab` (nao medido).

**cab** (`s1 cabread`; cabs do `makecab` do Windows em `fx/cab`: big.txt 434.965 B, rand.bin 150.000 B, hello.txt 5 B):
- MSZIP, 18 blocos: SHA-256 de big.txt, rand.bin e hello.txt **iguais aos originais** (efe99da9.., 51cb3219.., 2cf24dba..). big.txt tem muito mais de 32 KiB e texto repetitivo: o dicionario e mantido entre blocos (`mszip.rs` usa `reset(false)` entre blocos; confirmado pelo SHA).
- LZX (memoria 15 -> `Lzx(KB32)`, 18 blocos): SHA-256 identicos tambem (crate `lzxd`).
- **Quantum: sem leitor**: `ctype.rs:108` `invalid_data!("Quantum decompression is not yet supported.")` (so parseia o cabecalho). `makecab` nao gera Quantum (`Invalid Compression Type: QUANTUM`) nem `CompressionType=NONE`, entao Quantum e sem compressao nao foram exercitados ao vivo.
- `cab::FileReader` implementa `Read + Seek` (seek no meio de big.txt e rand.bin: bytes conferem). O brief pergunta de `StreamReader`: esse tipo e da crate `msi` (`stream.rs:27`, `impl<F: Read + Seek> Seek for StreamReader<F>`), confirmado por leitura de codigo, nao exercitado. Um panico no meu teste de seek em hello.txt (5 B) foi **erro da sonda** (ler 4 bytes a partir da posicao 2), nao da crate.

## 6. Recomendacao
As tres servem; nada bloqueia D1-1, D2-3 ou D3-1.
- **D1-1 sevenz-rust2 0.24**: usar com `set_thread_count(1)` (evita threads sem join e panico engolido) e com teto antes de decodificar (soma de `size`, `properties()` para o dicionario), pois o crate nao impoe limite. Exige `rust-version = "1.93"`. Sem panico/abort/hang em ~3.000 casos de mutacao; manter `catch_unwind` (D4-1) por cautela.
- **D3-1 .tar.xz**: viavel via `lzma_rust2::XzReader` (feature `xz`), validado contra `tar -cJf` GNU e bsdtar; falta um leitor de tar (nao medido aqui).
- **D2-3 msi + cab**: `msi` le tabelas e streams de MSI real; `cab` le MSZIP e LZX corretamente (SHA conferido) e tem Seek, **sem Quantum**. Manter `msiexec /a` de reserva (Quantum, cab externo/multi-volume nao testados). Licenca: `encoding_rs` traz BSD-3-Clause (aviso de atribuicao).

## Decisions
- Variantes de tamanho por features do mesmo `main.rs` (um target dir por variante): mesma base de codigo nas comparacoes.
- Habilitei `xz` em lzma-rust2 na sonda para responder a pergunta do brief.
- Fuzz em processo filho por caso (abort/hang exigem isolamento), semente fixa.
- Nao baixei toolchain 1.89: MSRV de msi/cab ficou "nao medido".

## Concerns
- MSI gerado pela crate e recusado pelo Windows Installer (causa nao investigada); afeta so fixtures, nao leitura.
- Panico em worker de `Lzma2ReaderMt` nao reproduzido (leitura de codigo).
- Tempos "sob carga"; MSRV de msi/cab, Quantum e CAB sem compressao, e extracao do cab embutido de um MSI real, nao medidos.
