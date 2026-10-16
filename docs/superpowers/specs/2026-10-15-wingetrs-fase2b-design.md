# wingetrs — Fase 2b: downloads `.7z` e `.msi` sem script (design)

Data: 2026-10-15
Status: **aprovada (decisões do usuário em 2026-10-15); plano `docs/superpowers/plans/2026-10-15-wingetrs-fase2b.md` (Tasks 116–119; P2–P8 depois das sondas).**

**Decisões do usuário (2026-10-15):**
- **D1-1:** `sevenz-rust2`.
- **D2-3:** `msi` + `cab` reproduzindo o layout do `msiexec /a`, com o `msiexec /a` como reserva sob confirmação. O usuário autorizou rodar o `msiexec /a`. Consequência: dois caminhos para o mesmo resultado exigem teste de equivalência (ao vivo, `#[ignore]`), e o que o `msiexec /a` grava fora da raiz (medido na S3) entra como exceção explícita no `CLAUDE.md` antes do P4.
- **D3-1**, mais `.tar.xz` se a S1 mostrar leitor xz sem dependência C.
- **D4-1:** `catch_unwind` só em volta do decodificador.
- Todas as sondas (S1–S5) autorizadas, inclusive o 7-Zip do usuário e o `msiexec /a`.
Rascunho de um agente opus, só leitura. Base: HEAD `0c01402`. Referência do Scoop: `~/scoop/apps/scoop/current/lib/decompress.ps1`, commit `e6aa3b3`, só leitura.

## (a) Objetivo e escopo

A 2b instala os apps do Scoop cujo download é `.7z` ou `.msi` e que não têm script. Ela usa o mesmo plano puro, o mesmo executor com staging e rollback e o mesmo `safepath` da 2a.

**Entra:**
- `.7z`, com `extract_dir`, `extract_to` e `#/rename`;
- `.msi`, **sem executar o instalador**, com `extract_dir` relativo ao `SourceDir`, como o Scoop faz;
- opcionalmente (D3), `.nupkg` (que é um zip) e `.tar.xz`.

**Fica fora:**
- script e innosetup (vão para a 2c);
- `.rar`, `.zst` e `.7z` em vários volumes (`.001`);
- 7z com senha;
- MSI com gabinete externo, com arquivo não comprimido ou que precise de transform;
- qualquer escrita fora de `%LOCALAPPDATA%\wingetrs`.

**Cobertura.** Indicador aproximado, medido em Python sobre o `catalog.db` de 2026-10-14 aberto com `immutable=1`. **Não é o oráculo**; o número oficial vem da S5.

| Sub-caso (sem script, com hash) | Main (1.684) | Extras (2.425) |
|---|---|---|
| 2a simples | 1.288 (76,5%) | 1.092 (45,0%) |
| `.7z` sem `extract_dir` | 27 (1,6%) | 72 (3,0%) |
| `.7z` com `extract_dir` | 20 (1,2%) | 37 (1,5%) |
| `.msi` com `extract_dir` | 35 (2,1%) | 68 (2,8%) |
| `.msi` sem `extract_dir` | 2 (0,1%) | 14 (0,6%) |
| **Subtotal 7z + msi** | **84 (5,0%)** | **191 (7,9%)** |
| `.nupkg` (zip) | 8 (0,5%) | 30 (1,2%) |
| `.tar.xz` | 6 (0,4%) | 0 |
| Com script ou innosetup (2c) | 294 (17,5%) | 1.108 (45,7%) |

**O que o rascunho mediu nos manifestos:**
- Nenhum `.7z` sem script traz um `.tar` dentro.
- O `extract_dir` dos MSI usa o nome **de origem** da tabela Directory: `PFiles` em 42, `PFiles64` em 14, `Rust` em 3, `LocalApp` em 2.
- Entre os manifestos com script, 44 baixam `.msi` e 64 baixam `.7z`. Eles reaproveitam o extrator na 2c.

**Defeitos achados:**
- `extract_one` copia como arquivo solto qualquer extensão que não reconhece. Tirar `.7z` de `UNSUPPORTED_EXTS` sem criar o despacho copiaria o `.7z` sem erro.
- A lista de extensões está repetida em três lugares: `manifest.rs`, `extract.rs` e `plan.rs`.
- `manifest.rs:229` diz "innosetup: suportado na fase 2b", mas o innosetup é da 2c.

## Como o Scoop faz

- **7z.** `Invoke-Extraction` escolhe o extrator pela regex `Test-7zipRequirement`. O helper `7zip` vira dependência automática. A extração roda `x -o<dest> -xr!*.nsis -y`. Com `extract_dir`, acrescenta `-ir!<extract_dir>\*` e depois move a pasta (`movedir`, robocopy `/e /move`).
- **MSI.** O padrão é `msiexec.exe /a <msi> /qn TARGETDIR=<dest>\SourceDir`; com `use_lessmsi`, usa o lessmsi. Com `extract_dir`, extrai em `_tmp` e move `SourceDir\<extract_dir>`. O código de saída só serve para saber se falhou.

## Opções pesquisadas

- **A. Rust puro.**
  - `sevenz-rust2` 0.24: Apache-2.0, **MSRV 1.93** (hoje o projeto declara `rust-version = "1.89"`). Suporta LZMA, LZMA2, BZIP2, PPMD e Deflate por feature, mais BCJ, BCJ2, DELTA e AES. Deflate64 não aparece. A API é `ArchiveReader::for_each_entries`, e `ArchiveEntry` traz os atributos Windows.
  - `msi` 0.10 (MIT): lê tabelas e streams, e também escreve, o que permite gerar fixtures no teste.
  - `cab` 0.6 (MIT): LZX e MSZIP, sem release desde 2024. Não está confirmado se o MSZIP mantém o dicionário entre blocos, nem se há Quantum.
- **B. `7zr.exe` oficial,** baixado e fixado por SHA-256. Custa um processo externo, e o zip-slip só seria checado depois da extração.
- **C. `msiexec /a`.** Roda o motor do Windows Installer, inclusive ações customizadas da `AdminExecuteSequence`, que são código de terceiros e contradizem o "sem script". Tem o mutex 1618. O UAC e as escritas fora da raiz não foram medidos.
- **D. lessmsi.** Aplicativo .NET, processo externo. A fidelidade ao layout do `msiexec /a` não foi medida.

## (b) Decisões em aberto

| # | Pergunta | Opções | Recomendação |
|---|---|---|---|
| D1 | Motor do 7z | 1 `sevenz-rust2`: valida antes de gravar e conta o orçamento por byte, mas é crate nova e sobe o MSRV para 1.93. 2 `7zr.exe` fixado: fidelidade total, mas é processo externo. 3 7z do usuário com a 2 de reserva: depende da máquina | **1**, se a S2 mostrar saída idêntica ao 7z.exe em todo o corpus. Método ausente ⇒ `NãoSuportado("7z com método X")` |
| D2 | Motor do MSI | 1 `msi` + `cab`, reproduzindo o layout do `msiexec /a`: não executa nada de terceiros. 2 `msiexec /a /qn`: fidelidade, mas roda ações customizadas. 3 a 1 com a 2 de reserva sob confirmação: exige teste de equivalência. 4 lessmsi | **1**, se a S3 der árvore idêntica (caminhos + SHA-256) em ≥ 95% dos MSI do corpus. O resto vira `NãoSuportado` com o motivo |
| D3 | Extensões extras | 1 `.7z` + `.msi` + `.nupkg`. 2 a 1 + `.tar.xz` (se o `lzma-rust2` já ler xz). 3 a 2 + `.tar.bz2` e `.zst` (o `zstd-sys` exige toolchain C) | **1** |
| D4 | Pânico dentro do decodificador de terceiros | 1 `catch_unwind` só em volta da chamada, virando `ExtractError::Decoder`: exceção à regra, documentada no CLAUDE.md e no `check_rules.ps1`. 2 propagar o pânico. 3 processo filho `__extract` | **1** |

## (c) Design (com D1-1, D2-1, D3-1, D4-1)

```
src/install/archive.rs          ArchiveKind::of(nome) -> Result<ArchiveKind, &'static str>  (Zip | TarGz | SevenZip | Msi | Plain; Nupkg ⇒ Zip)
src/install/extract/mod.rs      (era extract.rs) Budget, Mapper, write_entry, TempGuard; despacho por ArchiveKind
src/install/extract/sevenz.rs   extract_7z
src/install/extract/msi.rs      extract_msi: tabelas + gabinetes
src/install/extract/msi_layout.rs  (puro) Directory/Component/File → caminho relativo ao SourceDir
manifest.rs / plan.rs           a lista local sai; o suporte e o verbo vêm de ArchiveKind
examples/oracle_2a.rs           cobertura por ArchiveKind e sub-caso
```

**Erros novos em `ExtractError`:** `Encrypted`, `UnsupportedMethod`, `MsiExternalCabinet`, `MsiUncompressed`, `MsiLayout`, `Decoder`, `DictionaryTooLarge`. As mensagens são em português.

**7z:**
1. **Validação antes de gravar qualquer coisa.** Recusa:
   - nome que falha no `safepath`;
   - anti-item;
   - atributo de reparse `0x400`;
   - modo unix de symlink (`0x8000` com `0o120000`);
   - tamanho declarado acima do orçamento;
   - dicionário LZMA acima do limite (se a crate expuser esse tamanho).
2. **Extração.** Entradas fora do `extract_dir` são consumidas e cobradas no orçamento, porque um arquivo sólido decodifica tudo. Arquivo vazio é criado. `*.nsis` é excluído, como faz o Scoop.

**MSI:**
- Recusa MSI sem compressão. Um `Cabinet` com `#` é um stream interno; sem `#`, vira `MsiExternalCabinet`.
- `msi_layout` monta o caminho de cada arquivo:
  - o nome de origem é o que vem depois do `:` em `DefaultDir`;
  - em `short|long`, vale o longo;
  - `.` é a pasta do pai;
  - `TARGETDIR` vira `SourceDir` e é descartado;
  - cada componente passa pelo `safepath`;
  - ciclo, pai ausente e profundidade acima de 64 são recusados.
- Cada arquivo do gabinete é localizado pela chave `File` e passa pelo `mapper` (o `extract_dir` é relativo ao SourceDir) e pelo `write_entry` com orçamento.

**Plano e executor.**
- `Step::Extract` não muda.
- O `Display` passa a mostrar o motor: `(7z)` ou `(MSI, sem executar o instalador)`.
- `spawn_blocking` só entra se a S2 mostrar o runtime bloqueado; nesse caso o handle é sempre aguardado.

## (d) Sondas

| # | Sonda | Modelo | Autorização |
|---|---|---|---|
| S1 | Crate de sondagem com `sevenz-rust2`, `msi` e `cab`. Medir: MSRV, `cargo tree -d`, licenças, delta de tamanho e de tempo de build, se o MSZIP mantém o dicionário entre blocos, Quantum, se `StreamReader: Seek`, se há leitor de xz, tamanho do dicionário, 7z truncado ou corrompido (100 mutações) e dicionário de 1,5 GiB (pânico, alocação, pico de memória), pânico de thread interna | sonnet | não |
| S2 | Corpus 7z: as primeiras 40 URLs sem script, ordenadas pelo SHA-256 de `bucket/id`, até 150 MB cada e 3 GB no total. Registrar métodos, sólido, dicionário, razões, atributos e nomes recusados. Comparar `sevenz-rust2` (1 e N threads) contra o `7z.exe` do usuário: caminhos, SHA-256 e tempos | opus | sim, para usar o `7z.exe` do usuário |
| S3 | Corpus MSI: os 119 sem script, até 300 MB cada. Registrar SummaryInfo, Media, compressão, formas de `DefaultDir`, ações customizadas na `AdminExecuteSequence` e se o `extract_dir` existe no layout. Com autorização: `msiexec /a` um por vez (código, tempo, UAC, escrita em `%TEMP%` e em `C:\Windows\Installer`), comparado com `msi_layout` + `cab`. **A porcentagem idêntica decide a D2** | opus | **sim** |
| S4 | Limites: refazer `DEFAULT_LIMITS` com o corpus e propor o limite de dicionário LZMA | sonnet | não |
| S5 | Cobertura oficial com `oracle_2a` por sub-caso (depois de P1 e P7) | sonnet | não |

## (e) Tarefas (a numerar depois da 115)

| P | Tarefa | Modelo | Testes e mutação principais |
|---|---|---|---|
| P1 | `ArchiveKind` central, sem mudar o comportamento (7z e msi continuam `NãoSuportado`) e com a mensagem do innosetup corrigida para 2c | sonnet | tabela com ≥ 15 nomes; **equivalência** manifesto × extract × plano para cada nome; extensão desconhecida nunca é copiada como solta. Mutação: tirar uma extensão de `of()` |
| P2 | Dependências + extrator 7z (depende de S1, S2 e D1) | opus | 7z gerado no teste; `../x` na 2ª entrada ⇒ **nada** gravado; reparse; symlink unix; anti-item; orçamento cobrando as entradas puladas de um arquivo sólido; senha; truncado sem pânico. Mutação: validar só durante a extração |
| P3 | `msi_layout` puro (depende de S3) | opus | origem ≠ destino com ≥ 2 níveis; `short\|long`; `.`; `SourceDir` descartado; ciclo; profundidade; vetores de 3 MSI reais. Mutação: inverter origem e destino |
| P4 | Extrator MSI (depende de P3, S1 e D2) | opus | MSI e gabinete gerados no teste; dois gabinetes; gabinete externo e MSI sem compressão ⇒ `NãoSuportado`; nome inseguro ⇒ nada gravado. Mutação: aceitar `Cabinet` sem `#` |
| P5 | Habilitar no manifesto e no plano | sonnet | `cdrtools` 7z resolve; msi resolve; `.rar` não; snapshot `insta` do dry-run. Mutação: devolver `.7z` à lista |
| P6 | Ciclo completo no executor | opus | instalar e remover sem deixar nada; 7z corrompido volta ao estado idêntico; o dry-run bate byte a byte; pânico do decodificador vira erro com rollback. Mutação: tirar o `catch_unwind` (o teste não pode travar) |
| P7 | Oráculo por sub-caso | sonnet | `unsup-7z` passa a dar "ok" |
| P8 | Prova ao vivo (com autorização), spec, CLAUDE.md, ARMADILHAS, NOTICE (Apache-2.0) | opus | — |

Ordem: P1 → (P2 em paralelo com P3; não há arquivo em comum) → P4 → P5 → P6 → P7 → P8.

## (f) Riscos

| Risco | Mitigação |
|---|---|
| Layout do MSI diferente do `msiexec /a` | `MissingExtractDir` antes de promover; a S3 decide a D2; vetores reais no P3 |
| Bomba de memória ou de disco | limite de dicionário (S4), orçamento por byte produzido (incluindo as entradas puladas), limite de entradas |
| Pânico ou abort da crate | D4-1; a S1 mede o abort (estouro de pilha não é capturável) |
| Symlink ou reparse | recusado pelo atributo antes de gravar |
| Extração longa bloqueia o runtime | medida na S2; se preciso, `spawn_blocking` com o handle aguardado |
| `cab` sem manutenção; MSRV 1.93 | versões travadas no `Cargo.lock`; `rust-version` sobe com registro na spec |
| Apache-2.0 num projeto MIT | compatível; NOTICE na distribuição (P8) |
| Três listas de extensões divergem | `ArchiveKind` único + equivalência (P1) |
| Deflate64 ou outro método | a S2 lista os métodos; o ausente vira `NãoSuportado`, nunca extração parcial |
