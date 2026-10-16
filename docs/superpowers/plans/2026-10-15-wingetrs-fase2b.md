# wingetrs Fase 2b (`.7z`, `.msi`, `.nupkg`) Implementation Plan

> **For agentic workers:** execução pelo laço `run-task` (`.claude/skills/run-task`). Briefs: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-15-wingetrs-fase2b.md`.

**Goal:** instalar apps do Scoop sem script cujo download é `.7z`, `.msi` ou `.nupkg` (e `.tar.xz` se a S1 mostrar xz sem custo), com o mesmo plano puro, executor com staging e rollback e `safepath` da 2a.

**Architecture:** um `ArchiveKind` central decide formato, suporte e verbo para manifesto, plano e extrator. 7z por `sevenz-rust2`; MSI por `msi` + `cab` reproduzindo o layout do `msiexec /a`, com o `msiexec /a` como reserva sob confirmação do usuário (D2-3).

**Spec:** `docs/superpowers/specs/2026-10-15-wingetrs-fase2b-design.md` (decisões do usuário em 2026-10-15: D1-1, D2-3, D3-1 + `.tar.xz` se a S1 mostrar leitor xz, D4-1; todas as sondas autorizadas).

Este plano cobre o P1 e as sondas S1–S3. As tarefas P2–P8 são escritas depois das sondas, porque dependem dos números delas.

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes de cada tarefa.
- Raiz: `%LOCALAPPDATA%\wingetrs` (`DataDir::from_env()`). Fora da raiz o wingetrs escreve **somente** o que a spec da 2a autoriza (`HKCU\Environment`, `Start Menu\Programs\wingetrs Apps`, Lixeira, chaves `HKCU` de resíduo confirmadas). Nunca escreva em arquivos do winget ou do `~/scoop`.
- Sondas trabalham em `target/probe-2b/` (ignorado pelo git) e só registram resultados em `.claude/tasks/task-N-report.md`. Nada de sonda entra em `src/` ou `Cargo.toml` do projeto.
- O catálogo local é `%LOCALAPPDATA%\wingetrs\catalog.db`, tabela `packages(source, id, raw, kind, …)`; manifestos do Scoop são `kind = 'scoop'`, JSON em `raw`. Abra **sempre** só leitura: `file:<caminho>?immutable=1` com `uri=True` (Python) ou `SQLITE_OPEN_READ_ONLY` (Rust). O arquivo está em WAL; não crie `-shm`/`-wal` novos.
- "Sem script" = nenhuma das chaves `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`, `installer`, `uninstaller`, `psmodule`, `innosetup` na raiz nem em `architecture.*` (mesma regra de `examples/oracle_2a.rs::is_simple`).
- Toolchain instalado: `rustc 1.99.0`; o projeto declara `rust-version = "1.89"`. Só a tarefa que adicionar `sevenz-rust2` sobe o `rust-version` (para 1.93), com registro na spec.
- Dependência nova só quando o brief manda. `cargo update` é proibido.
- Mensagens ao usuário em português; código e comentários em inglês. Regras ⚙ do `CLAUDE.md` valem (gate barra).
- Tarefas que medem tempo rodam sozinhas: nenhum `cargo test` do projeto em paralelo a uma medição.

## Review Focus

1. Arquivo com extensão de formato conhecido mas sem extrator (ex.: `.7z` antes do P2) nunca é copiado como arquivo solto: vira erro.
2. Nome com caixa mista ou fragmento `#/x.7z` decide o formato pelo nome final, igual nos três lugares.
3. `.tar.gz` não é confundido com `.gz` solto nem `.tar.xz` com `.xz`.
4. Processo externo lançado por uma sonda (7z.exe, msiexec) nunca fica vivo depois do relatório.
5. Sonda nunca grava fora de `target/probe-2b/` além do que o próprio `msiexec /a` grava, e isso é medido e listado.

---

### Task 116: `ArchiveKind` central (P1)

**Modelo:** sonnet.

#### Where this fits

- Hoje a lista de formatos existe em três lugares que podem divergir:
  - `src/install/manifest.rs:213` `UNSUPPORTED_EXTS` (`.7z`, `.msi`, `.rar`, `.nupkg`, `.tar.xz`, `.tar.bz2`, `.txz`, `.tbz2`, `.zst`) e o uso em `build_sources` (`:266`, mensagem `"download {ext}: suportado na fase 2b"`);
  - `src/install/extract.rs:212` `extract_one`: `.zip` → `extract_zip`, `.tar.gz`/`.tgz` → `extract_tar_gz`, **qualquer outra coisa é copiada como arquivo solto**;
  - `src/install/plan.rs:222`: verbo `"Extrair"` para `.zip`/`.tar.gz`/`.tgz`, `"Copiar"` para o resto.
- `src/install/manifest.rs:229` diz `"usa innosetup: suportado na fase 2b"`; o innosetup é da 2c.
- Snapshots `tests/snapshots/install_planner__*.snap` têm linhas `Copiar [1] x.exe para …` que **não podem mudar**.

#### What binds this task

Criar `src/install/archive.rs` (e `pub mod archive;` em `src/install/mod.rs`), sem mudar o comportamento de nenhum download que hoje funciona:

```rust
// src/install/archive.rs
//! The single source of truth for download formats (manifest, plan and extractor).

/// Download format, decided by the final file name (after `#/rename`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
    SevenZip,
    Msi,
    Nupkg,
    TarXz,
    /// Not an archive: copied as is (`.exe`, `.ps1`, `.jar`, ...).
    Plain,
}

impl ArchiveKind {
    /// `Err(ext)` for formats that no phase plans to read (`.rar`, `.tar.bz2`, `.tbz2`, `.zst`).
    /// Case-insensitive; longest suffix wins (`.tar.gz` before `.gz`).
    pub fn of(file_name: &str) -> Result<ArchiveKind, &'static str>;
    /// Whether the extractor can read this kind today. P1: Zip, TarGz, Plain.
    pub fn supported(self) -> bool;
    /// Suffix shown in messages (`".7z"`, `".msi"`, ...); `""` for Plain.
    pub fn ext(self) -> &'static str;
    /// Verb for the plan display: `"Extrair"` for archives, `"Copiar"` for Plain.
    pub fn verb(self) -> &'static str;
}
```

Sufixos de `of`: `.zip` → Zip; `.tar.gz`, `.tgz` → TarGz; `.7z` → SevenZip; `.msi` → Msi; `.nupkg` → Nupkg; `.tar.xz`, `.txz` → TarXz; `.rar`, `.tar.bz2`, `.tbz2`, `.zst` → `Err` com o sufixo; o resto → Plain.

Os três lugares passam a usar `ArchiveKind`:
1. **`manifest.rs`:** `UNSUPPORTED_EXTS` sai. `Err(ext)` ⇒ `unsupported(format!("download {ext}: formato não suportado"))`. `Ok(k)` com `!k.supported()` ⇒ `unsupported(format!("download {}: suportado na fase 2b", k.ext()))`. A mensagem do innosetup passa a `"usa innosetup: suportado na fase 2c"`.
2. **`extract.rs`:** `extract_one` faz `match` exaustivo em `ArchiveKind::of(s.file_name)`; Zip e TarGz como hoje; Plain copia como hoje; SevenZip, Msi, Nupkg, TarXz e `Err` ⇒ novo `ExtractError::Unsupported(String)` (`#[error("formato não suportado pelo extrator: {0}")]`) **antes de gravar qualquer coisa**. Sem braço `_ =>`.
3. **`plan.rs`:** `verb = ArchiveKind::of(&src.file_name).map_or("Copiar", ArchiveKind::verb)`.

**Files:**
- Create: `src/install/archive.rs`, `tests/install_archive.rs`
- Modify: `src/install/mod.rs`, `src/install/manifest.rs`, `src/install/extract.rs`, `src/install/plan.rs`
- Test: `tests/install_archive.rs`, `tests/install_extract.rs`, `tests/install_manifest.rs`

Testes (literais; ajuste só os nomes de import ao que existir):

```rust
// tests/install_archive.rs
use wingetrs::install::archive::ArchiveKind::{self, *};

const TABLE: &[(&str, Result<ArchiveKind, &str>)] = &[
    ("a.zip", Ok(Zip)),
    ("A.ZIP", Ok(Zip)),
    ("a.tar.gz", Ok(TarGz)),
    ("a.tgz", Ok(TarGz)),
    ("a.7z", Ok(SevenZip)),
    ("a.msi", Ok(Msi)),
    ("a.nupkg", Ok(Nupkg)),
    ("a.tar.xz", Ok(TarXz)),
    ("a.txz", Ok(TarXz)),
    ("a.rar", Err(".rar")),
    ("a.tar.bz2", Err(".tar.bz2")),
    ("a.tbz2", Err(".tbz2")),
    ("a.zst", Err(".zst")),
    ("a.exe", Ok(Plain)),
    ("a.gz", Ok(Plain)),
    ("a.xz", Ok(Plain)),
    ("setup.7z.exe", Ok(Plain)),
    ("a.zip.ps1", Ok(Plain)),
];

#[test]
fn archive_kind_table() {
    for (name, want) in TABLE {
        assert_eq!(ArchiveKind::of(name), *want, "{name}");
    }
}

#[test]
fn only_2a_kinds_supported_for_now() {
    for k in [Zip, TarGz, Plain] {
        assert!(k.supported(), "{k:?}");
    }
    for k in [SevenZip, Msi, Nupkg, TarXz] {
        assert!(!k.supported(), "{k:?}");
    }
}

#[test]
fn verb_matches_kind() {
    assert_eq!(Plain.verb(), "Copiar");
    for k in [Zip, TarGz, SevenZip, Msi, Nupkg, TarXz] {
        assert_eq!(k.verb(), "Extrair", "{k:?}");
    }
}
```

Teste de **equivalência** manifesto × extrator (em `tests/install_archive.rs`; use o helper de manifesto que `tests/install_manifest.rs` já usa para montar um JSON mínimo com `url` + `hash` e chamar `resolve`; para o extrator, grave um arquivo de 4 bytes com o nome da tabela num `tempfile::tempdir()` e chame `wingetrs::install::extract::extract(file, name, None, None, dest)`):

```rust
#[test]
fn manifest_and_extractor_agree() {
    for (name, kind) in TABLE {
        let manifest_ok = resolve_single_url(&format!("https://x.test/{name}")).is_ok();
        let accepted = matches!(kind, Ok(k) if k.supported());
        assert_eq!(manifest_ok, accepted, "manifesto: {name}");
        if !accepted {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join(name);
            std::fs::write(&file, b"junk").unwrap();
            let dest = dir.path().join("out");
            let err = wingetrs::install::extract::extract(&file, name, None, None, &dest)
                .expect_err(name);
            assert!(err.to_string().contains("não suportado"), "{name}: {err}");
            assert!(!dest.join(name).exists(), "{name} copiado como solto");
        }
    }
}
```

(`resolve_single_url` é um helper local do teste que monta `{"version":"1","url":<u>,"hash":"<64 zeros>"}` e chama o `resolve` público; se `resolve` exigir outros campos, copie o mínimo de `tests/install_manifest.rs`.)

Também em `tests/install_manifest.rs`: um manifesto com `"innosetup": true` dá erro cuja mensagem contém `"fase 2c"`.

#### Checks beyond the steps

1. Mutação: em `archive.rs`, trocar `".7z" => ` por algo que caia em Plain (ex.: anchor da linha do `.7z`, replacement com o sufixo `".7zz"`) ⇒ `manifest_and_extractor_agree` ou `archive_kind_table` **PEGA**. Cole a saída `[OK] mutação PEGA`.
2. Mutação: em `extract_one`, mandar `SevenZip` para o braço de Plain ⇒ `manifest_and_extractor_agree` **PEGA**.
3. `cargo test --quiet install_planner` passa sem atualizar nenhum snapshot (`git status tests/snapshots` limpo).
4. `grep -n "UNSUPPORTED_EXTS\|\.tgz\"" src/install/manifest.rs src/install/plan.rs` não acha nada.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `refactor(install): one ArchiveKind for manifest, plan and extractor`

---

### Task 117: Sonda S1 — crates `sevenz-rust2`, `msi`, `cab`

**Modelo:** sonnet. Sonda; nenhum código de produto.

#### Where this fits

- Spec 2b, §"Opções pesquisadas" e §(d) S1. Decisões já tomadas: D1-1 (`sevenz-rust2`), D2-3 (`msi` + `cab`, com `msiexec /a` de reserva), D3-1 (+ `.tar.xz` se houver leitor xz sem custo), D4-1 (`catch_unwind` em volta do decodificador).
- Versões alvo: `sevenz-rust2` 0.24.x, `msi` 0.10.x, `cab` 0.6.x (confirme a última de cada no crates.io e use essa, citando a data).

#### What binds this task

Crate de sondagem **fora do projeto**: `cargo new --bin target/probe-2b/s1` (não é membro do workspace; se o cargo reclamar, crie com `[workspace]` vazio no `Cargo.toml` dela). Nada muda no `Cargo.toml`/`Cargo.lock` do wingetrs.

Medir e colar no relatório, com o comando e a saída resumida de cada item:
1. **MSRV real** de cada crate (`rust-version` do `Cargo.toml` publicado) e `cargo tree -d` (duplicatas) contra as versões que o `Cargo.lock` do wingetrs já trava (`zip`, `flate2`, `sha2`, `crc32fast` etc.: liste as que colidem).
2. **Licenças** (`cargo tree -e normal --format "{p} {l}"`), incluindo as transitivas.
3. **Delta de tamanho** do exe release da sonda com e sem cada crate (`--release`, `strip` padrão). **Tempo de build** limpo com cada crate (marcado "sob carga" se houver outro `cargo` rodando).
4. **7z:**
   - API para listar entradas **antes** de decodificar (nome, tamanho, atributos, anti-item, `has_stream`, método, dicionário). Cole a assinatura.
   - Leitor de **xz** disponível (`lzma-rust2` ou feature) sem dependência C? Ele lê um `.tar.xz` gerado com `tar -cJf`?
   - 100 mutações (bytes aleatórios com semente fixa, truncamento em 10 pontos) de um `.7z` pequeno: algum pânico? algum abort? algum laço sem fim (limite de 10 s por caso)?
   - Um `.7z` com dicionário declarado de 1,5 GiB: aloca? quanto de memória de pico (`GetProcessMemoryInfo` ou o `Get-Process` do processo filho)?
   - A crate cria threads próprias? Pânico numa delas chega ao chamador?
5. **MSI/CAB:**
   - Gerar um MSI mínimo com a crate `msi` (tabelas `Directory`, `Component`, `File`, `Media`, stream `#cab`) e lê-lo de volta.
   - `cab`: o MSZIP mantém o dicionário entre blocos (arquivo > 32 KiB, comparar SHA-256 com o original)? Há Quantum? `StreamReader` implementa `Seek`?
   - `makecab` do Windows gera o `.cab` de teste (é só leitura do sistema; grave em `target/probe-2b/`).
6. **Recomendação** curta: as três crates servem? alguma bloqueia D1-1/D2-3/D3?

**Files:**
- Create: `.claude/tasks/task-117-report.md` (e o que quiser em `target/probe-2b/s1/`, que não vai para o git)

Testes: nenhum (sonda).

#### Checks beyond the steps

1. `git status --short` no fim mostra só o relatório.
2. Nenhum processo deixado vivo (`pwsh -File scripts/leftovers.ps1`).

**Commit:** `docs(probe): 7z, msi and cab crates for phase 2b`

---

### Task 118: Sonda S2 — corpus 7z contra o `7z.exe`

**Modelo:** opus. **Roda sozinha** (mede tempo). Autorizada pelo usuário em 2026-10-15 a usar o 7-Zip instalado.

#### Where this fits

- Depende da Task 117 (relatório em `.claude/tasks/task-117-report.md`: versões e API das crates).
- Spec 2b §(d) S2: "A porcentagem idêntica decide se a D1-1 vale para todo o corpus".

#### What binds this task

1. **Corpus.** Do `catalog.db` (só leitura, ver Global Constraints), os manifestos Scoop sem script cujo download (raiz ou `architecture.64bit`) termina em `.7z`. Ordene por SHA-256 de `source + "/" + id` e pegue as primeiras 40 URLs com tamanho ≤ 150 MB (HEAD antes; sem `Content-Length`, baixe e descarte se passar), total ≤ 3 GB. Baixe para `target/probe-2b/corpus-7z/` e confira o hash do manifesto; hash errado ⇒ registre e descarte.
2. **Ache o 7-Zip** só em caminhos conhecidos: `C:\Program Files\7-Zip\7z.exe`, `C:\Program Files (x86)\7-Zip\7z.exe`, `~/scoop/apps/7zip/current/7z.exe` (só executar, nunca escrever no `~/scoop`). Não varra o disco.
3. **Para cada arquivo:**
   - `7z l -slt` ⇒ métodos, sólido, dicionário, número de entradas, atributos (reparse `0x400`, symlink), anti-itens, nomes que o `wingetrs::install::safepath` recusaria (chame-o a partir de um example ou de um binário da sonda que dependa do wingetrs por `path = "../../.."`).
   - Extração com `7z x -o<dir> -y` e com o `sevenz-rust2` (1 thread e N threads, se a API permitir) em pastas separadas; compare a árvore (caminhos relativos + SHA-256 + tamanho) e os tempos (mediana de 3).
4. **Relatório:** tabela por arquivo e totais: idênticos, diferentes (com o primeiro caminho divergente), não suportados (método), recusados (nome/atributo). Razão descompactado/compactado máxima (alimenta a S4). Tempo do maior arquivo (decide `spawn_blocking`).

**Files:**
- Create: `.claude/tasks/task-118-report.md`

Testes: nenhum (sonda).

#### Checks beyond the steps

1. Cada `7z.exe` lançado é aguardado; no fim, `scripts/leftovers.ps1` limpo.
2. Nada gravado fora de `target/probe-2b/`.

**Commit:** `docs(probe): 7z corpus against 7-Zip for phase 2b`

---

### Task 119: Sonda S3 — corpus MSI, layout e `msiexec /a`

**Modelo:** opus. **Roda sozinha** (mede tempo; o Windows Installer é global). Autorizada pelo usuário em 2026-10-15 a rodar `msiexec /a`.

#### Where this fits

- Depende da Task 117 (crates `msi`/`cab`).
- Spec 2b §(c) MSI e §(d) S3. D2-3: o caminho principal é `msi` + `cab` reproduzindo o layout; o `msiexec /a` é a reserva, sob confirmação. **A porcentagem idêntica e o que o `msiexec /a` grava fora da pasta-alvo decidem se a reserva entra e qual exceção vai para o `CLAUDE.md`.**

#### What binds this task

1. **Corpus.** Do `catalog.db` (só leitura), os manifestos Scoop sem script com download `.msi` (cerca de 119). Mesma ordem determinística (SHA-256 de `source/id`), ≤ 300 MB cada, total ≤ 4 GB. Baixe em `target/probe-2b/corpus-msi/` com conferência de hash.
2. **Estático (todos):** SummaryInfo (template/plataforma, palavra de compressão), tabela `Media` (gabinetes internos `#` × externos), arquivos não comprimidos, formas de `DefaultDir` (`a|b`, `x:y`, `.`), ações customizadas na `AdminExecuteSequence` e na `AdminUISequence` (tipo e alvo), e se o `extract_dir` do manifesto existe no layout montado.
3. **`msiexec /a` (um por vez):**
   - Antes do primeiro: liste `%TEMP%` (só nomes de topo) e `C:\Windows\Installer` (só nomes de topo) e os `HKCU`/`HKLM\...\Installer` só de leitura (`reg query` de contagem de subchaves).
   - Comando: `msiexec.exe /a "<msi>" /qn TARGETDIR="<abs>\target\probe-2b\msi-admin\<id>\SourceDir" /l*v "<abs>\target\probe-2b\msi-admin\<id>.log"`. Timeout de 180 s: se estourar (UAC esperando clique, 1618), encerre **só esse** `msiexec` que você lançou, registre e siga.
   - Registre código de saída, tempo, se apareceu UAC (timeout sem progresso no log), e o que mudou em `%TEMP%`, `C:\Windows\Installer` e nas contagens do registro.
4. **Layout próprio:** monte com `msi` + `cab` a árvore que o `msiexec /a` gera (regras da spec §(c) MSI: nome de origem depois do `:` em `DefaultDir`, longo em `short|long`, `.` = pasta do pai, `TARGETDIR` = `SourceDir`). Compare com a do `msiexec /a`: caminhos relativos + SHA-256. Ignore só o próprio `.msi` administrativo que o `msiexec` copia para a raiz (registre).
5. **Relatório:** % idêntico (meta da spec: ≥ 95%), lista dos diferentes com a causa, lista dos não suportados (gabinete externo, sem compressão, transform), o que o `msiexec /a` gravou fora da pasta-alvo, e 3 MSI reais pequenos recomendados como vetores do P3 (id, URL, hash, tamanho).

**Files:**
- Create: `.claude/tasks/task-119-report.md`

Testes: nenhum (sonda).

#### Checks beyond the steps

1. Nunca `msiexec /i`, `/x`, `/f` ou `/p`: só `/a`.
2. No fim, nenhum `msiexec.exe` lançado por você vivo (`Get-Process msiexec` mostra só o serviço do sistema, se houver; não encerre o que você não lançou).
3. As pastas `target/probe-2b/msi-admin/*` podem ficar (ignoradas pelo git).

**Commit:** `docs(probe): msi corpus, layout and msiexec /a for phase 2b`

---

## Implementação (Tasks 123–131), escrita depois das sondas S1–S3 (2026-10-16)

Fatos medidos que valem para todas estas tarefas (relatórios `.claude/tasks/task-117-report.md`, `-118-`, `-119-`; spec §(c2)). Cada tarefa repete os que usa.

- Crates: `sevenz-rust2 = "=0.24.0"` (Apache-2.0, `rust-version` 1.93), `lzma-rust2 = "=0.22.0"` (feature `xz`), `msi = "=0.10.0"` (MIT), `cab` 0.6.0 **vendorizado** em `vendor/cab-0.6.0` com o patch sequencial (Task 125). Nenhuma traz código C nem licença copyleft; `encoding_rs` (via `msi`) exige aviso BSD-3-Clause.
- Testes nunca rodam `7z.exe` nem `msiexec`: fixtures pequenas versionadas ou geradas no próprio teste. Testes ao vivo levam `#[ignore = "live: ..."]`.

### Task 123: Farejar a assinatura e extrair 7z (P2)

**Modelo:** opus.

#### Where this fits

- `src/install/archive.rs` (Task 116): `ArchiveKind::of(nome)`; `SevenZip.supported()` hoje é `false` e **continua `false`** nesta tarefa (a Task 128 liga).
- `src/install/extract.rs`: `extract_all_with(sources, dest, limits)`, `extract_one` (`match` exaustivo em `ArchiveKind`; `SevenZip` ⇒ `ExtractError::Unsupported`), `Budget::for_file`/`charge`, `check_entries`, `Mapper::map`, `write_entry`, `safe_components`, `TempGuard`, `DEFAULT_LIMITS` (`ratio 100`, `floor 1 GiB`, `ceiling 32 GiB`, `max_entries 200_000`).
- Medido na S1 (Task 117): `ArchiveReader::open(path, Password::empty())?.archive()` lista `files: Vec<ArchiveEntry>` (`name`, `has_stream`, `is_directory`, `is_anti_item`, `has_windows_attributes`, `windows_attributes`, `size`) e `blocks: Vec<Block>` (`get_unpack_size()`, `coders[].properties()`, `encoder_method_id()`) **sem decodificar**. `for_each_entries(|entry, reader| -> Result<bool, Error>)` decodifica em ordem. **Sem limite de memória próprio** (`MAX_MEM_LIMIT_KB = usize::MAX/1024`). O modo multithread cria threads sem `join` nem `catch_unwind`: use sempre `set_thread_count(1)`. Dicionário: LZMA2 = 1 byte de propriedade `p`, dicionário = `(2 | (p & 1)) << (p / 2 + 11)` (`p = 40` ⇒ 4 GiB − 1); LZMA = 5 bytes (`lc/lp/pb` + 4 bytes LE do dicionário). ~3.000 mutações sem pânico.
- Medido na S2 (Task 118): 24/24 7z reais idênticos ao `7z.exe`; **16 de 40 ".7z" não são 7z** (12 instaladores PE/NSIS, 2 zip renomeados com `#/dl.7z`, 2 7z SFX). Razão máxima 21,5; maior dicionário 512 MiB; o maior 7z (122 MB) leva ~10,5 s com 1 thread.

#### What binds this task

1. **Dependências:** `sevenz-rust2 = { version = "=0.24.0", default-features = false, features = [...] }` com as features que leem LZMA, LZMA2, BCJ, BCJ2, Delta, PPMD, BZIP2 e Deflate, **sem** AES nem `compress` (confira os nomes no `Cargo.toml` publicado e cole-os no relatório). Em `[dev-dependencies]`, a mesma crate com `compress` para gerar 7z nos testes. `rust-version = "1.93"` no `Cargo.toml` (toolchain instalado: 1.99).
2. **Farejar** em `src/install/sniff.rs` (puro):

```rust
pub enum Sniffed { SevenZip, Zip, Pe, Other }
/// `head` = first bytes of the file (up to 64 KiB).
pub fn sniff(head: &[u8]) -> Sniffed;
/// Scans `r` from `start` for the 7z signature (SFX overlay); `None` if absent.
pub fn find_7z_in_pe<R: Read + Seek>(r: &mut R, start: u64, max_scan: u64) -> io::Result<Option<u64>>;
```

   - Assinaturas, em hex: 7z = `37 7A BC AF 27 1C` no começo; zip = `50 4B 03 04` ou `50 4B 05 06`; PE = `4D 5A`. A assinatura completa procurada no PE é `37 7A BC AF 27 1C 00 04`.
   - Para PE, procure a partir do fim das seções (overlay) até `max_scan = 32 MiB`; valide cada candidato abrindo-o como 7z (o `sevenz-rust2` confere o CRC do cabeçalho). Uma assinatura solta, sem 7z válido depois, não vira arquivo aceito.
3. **Despacho** em `extract_one`, para `ArchiveKind::SevenZip`: `SevenZip` ⇒ `extract_7z`; `Zip` ⇒ o `extract_zip` existente (com `tracing::info!` do nome e do formato real); `Pe` com 7z achado ⇒ `extract_7z` sobre um `OffsetReader<R>` (`Read + Seek` que soma o deslocamento); `Pe` sem 7z ⇒ `ExtractError::Unsupported("instalador .exe, não um arquivo 7z")`; `Other` ⇒ `Unsupported("conteúdo não reconhecido como 7z")`.
4. **`extract_7z`** em `src/install/extract_7z.rs` (torne `pub(super)` o que precisar de `extract.rs`):
   - Abrir com `set_thread_count(1)`. Erro de senha ⇒ `ExtractError::Encrypted`; método sem leitor ⇒ `ExtractError::UnsupportedMethod(String)`.
   - **Antes de decodificar qualquer bloco**, recusar: nome que `safe_components` recusa; `is_anti_item`; reparse (`has_windows_attributes && windows_attributes & 0x400 != 0`); symlink unix (`windows_attributes & 0x8000 != 0 && (windows_attributes >> 16) & 0o170000 == 0o120000`); `check_entries(files.len())`; soma de `size` acima do orçamento (`Budget::for_file`) ⇒ `TooLarge`; algum coder com dicionário acima de `MAX_7Z_DICT = 1 GiB` ⇒ `ExtractError::DictionaryTooLarge { dict, limit }`.
   - **Extração:** `for_each_entries`. Entradas que o `Mapper` não mapeia (fora do `extract_dir`) **são lidas até o fim e cobradas no orçamento**, porque um arquivo sólido decodifica tudo. `*.nsis` é pulado, como o Scoop faz (`-xr!*.nsis`), e também lido e cobrado. Diretório e arquivo vazio são criados. O resto vai por `write_entry`.
   - **D4-1:** a decodificação fica dentro de `std::panic::catch_unwind(AssertUnwindSafe(..))`; pânico ⇒ `ExtractError::Decoder(<mensagem, se o payload for &str ou String>)`. Comentário na linha: `// D4-1: third-party decoder panic becomes an error (spec 2b)`. Se `check_rules.ps1` acusar, acrescente nele a exceção só para esse comentário e prove com `check_rules_selftest.ps1`.
5. **Erros novos** em `ExtractError`: `Encrypted` ("arquivo 7z protegido por senha"), `UnsupportedMethod(String)` ("método de compressão não suportado: {0}"), `DictionaryTooLarge { dict: u64, limit: u64 }` ("dicionário de {dict} bytes passa do limite de {limit}"), `Decoder(String)` ("falha no decodificador: {0}").
6. **Não** mude `ArchiveKind::supported()` nem o executor.

**Files:**
- Create: `src/install/sniff.rs`, `src/install/extract_7z.rs`, `tests/install_7z.rs`
- Modify: `Cargo.toml`, `Cargo.lock` (só as linhas novas), `src/install/mod.rs`, `src/install/extract.rs`, e `scripts/check_rules.ps1` se preciso
- Test: `tests/install_7z.rs`, testes de módulo em `sniff.rs`

Testes (todo 7z é gerado no próprio teste com o `sevenz-rust2` de `dev-dependencies`; nada de `7z.exe`):
1. `sniff_table`: 7z, zip (as duas assinaturas), PE, vazio e 3 bytes ⇒ formatos esperados.
2. `extracts_7z_tree`: `a.txt`, `sub/b.bin` (100 KiB pseudoaleatório), pasta vazia, arquivo vazio ⇒ árvore e SHA-256 iguais.
3. `extract_dir_on_solid_7z_charges_skipped`: sólido com `keep/x` (10 bytes) e `skip/y` (2 MiB); `extract_dir = "keep"` e limites com `floor` = 1 MiB e `ratio` = 1 ⇒ `TooLarge`.
4. `unsafe_second_entry_writes_nothing`: `ok.txt` e depois `../x` ⇒ `UnsafePath`, e **nem o destino nem a pasta pai ganham nada** (liste a pasta pai antes e depois).
5. `reparse_and_unix_symlink_refused`: atributo `0x400`; atributo `0x8000 | (0o120777 << 16)` ⇒ erro, nada gravado. Se a API de escrita não permitir pôr atributos, edite os bytes do cabeçalho gerado e refaça o CRC; descreva no relatório.
6. `huge_dictionary_refused_before_decoding`: byte de propriedade LZMA2 trocado por 40 e CRC refeito ⇒ `DictionaryTooLarge`, em menos de 1 s.
7. `zip_named_7z_goes_to_zip`.
8. `pe_without_7z_is_unsupported`: `MZ` + 4 KiB de zeros ⇒ `Unsupported` com "instalador".
9. `sfx_7z_extracts`: `MZ` + 4 KiB + um 7z válido ⇒ extraído; `MZ` + a assinatura solta sem 7z válido ⇒ `Unsupported`.
10. `truncated_7z_is_error_not_panic`: o 7z do teste 2 cortado em 5 pontos ⇒ erro tipado em todos.
11. `nsis_entries_skipped`.

#### Checks beyond the steps

1. Mutação: mover a checagem de `safe_components` para dentro do `for_each_entries` ⇒ teste 4 **PEGA**.
2. Mutação: não cobrar entradas puladas ⇒ teste 3 **PEGA**.
3. Mutação: tirar o `catch_unwind` ⇒ algum teste **PEGA**. Se nenhum teste provoca pânico do decodificador real, prove com um teste de módulo que injeta um leitor que entra em pânico dentro do `catch_unwind`.
4. Mutação: `MAX_7Z_DICT = u64::MAX` ⇒ teste 6 **PEGA**.
5. `cargo tree -d` sem duplicata nova; licenças coladas no relatório.
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): 7z extraction with content sniffing, pre-decode validation and limits`

---

### Task 124: Layout de pastas do MSI, puro (P3)

**Modelo:** opus.

#### Where this fits

- Spec 2b §(c) MSI e §(c2): o caminho de cada arquivo relativo ao `SourceDir` sai das tabelas `Directory`, `Component` e `File`. A S3 (Task 119) mediu 73/73 iguais ao `msiexec /a` com a regra de colisão: versionado vence não versionado; maior `Version`; empate ⇒ menor `Sequence`.
- Nada lê MSI nesta tarefa: a entrada são structs simples.

#### What binds this task

Criar `src/install/msi_layout.rs` (puro, sem crate nova):

```rust
pub struct DirRow { pub directory: String, pub parent: Option<String>, pub default_dir: String }
pub struct CompRow { pub component: String, pub directory: String }
pub struct FileRow { pub file: String, pub component: String, pub file_name: String, pub version: Option<String>, pub sequence: i32 }
#[derive(Debug, PartialEq, Eq)]
pub struct Placed { pub file: String, pub rel: Vec<String> }
pub fn layout(dirs: &[DirRow], comps: &[CompRow], files: &[FileRow]) -> Result<Vec<Placed>, LayoutError>;
```

Regras:
1. **Nome de origem** de um `DefaultDir`: a parte depois de `:` se houver (`destino:origem`), senão o todo; em `curto|longo`, o longo; `.` não acrescenta componente.
2. A raiz é a linha sem `parent` ou com `parent` igual a ela mesma (em geral `TARGETDIR` com `DefaultDir = SourceDir`) e vira `[]`.
3. **Nome do arquivo** de `FileName`: em `curto|longo`, o longo.
4. Cada componente passa por `install::safepath::safe_components`; inválido ⇒ `LayoutError::Unsafe`.
5. Pai ausente, ciclo, profundidade > 64 e componente sem diretório ⇒ `LayoutError::{MissingParent, Cycle, TooDeep, MissingDirectory}`.
6. **Colisão** (mesmo `rel`, sem caixa): fica um só, pela regra medida; versão comparada por partes numéricas (`1.10.0` > `1.9.9`). Os perdedores saem da lista.
7. Saída ordenada por `sequence`.

**Files:**
- Create: `src/install/msi_layout.rs`, `tests/install_msi_layout.rs`
- Modify: `src/install/mod.rs`

Testes:
1. `source_name_after_colon_and_long_form`: `DefaultDir` = `PFILES:Program Files`, `a~1|App Dir` e `.`.
2. `two_levels_destination_differs_from_source`: três níveis com `destino:origem` diferentes ⇒ só nomes de origem.
3. `targetdir_root_dropped`: arquivo direto na raiz ⇒ `rel = ["x.dll"]`.
4. `collision_rules`: (a) versionado × não versionado; (b) `1.10.0` × `1.9.9`; (c) mesma versão, sequências 7 e 3 ⇒ fica a 3; (d) nomes que só diferem na caixa colidem.
5. `cycle_missing_parent_depth`.
6. `unsafe_name_refused`: `DefaultDir` = `..:..` e `FileName` = `a|../y` ⇒ `Unsafe`.
7. `three_real_vectors`: 3 MSI reais pequenos recomendados em `.claude/tasks/task-119-report.md`. Extraia as linhas `Directory`/`Component`/`File` de 3 arquivos de cada um com a crate `msi` num example temporário sobre `target/probe-2b/corpus-msi/` (apague o example depois) e **cole as linhas e os caminhos esperados no teste**, comparando com a árvore do `msiexec /a` em `target/probe-2b/msi-admin/`. O teste não lê MSI.

#### Checks beyond the steps

1. Mutação: inverter destino e origem em `DefaultDir` ⇒ testes 1/2 **PEGA**.
2. Mutação: empate de versão escolhe a maior sequência ⇒ teste 4c **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): pure MSI source layout with the msiexec /a collision rule`

---

### Task 125: Extrator MSI com `cab` vendorizado sequencial (P4)

**Modelo:** opus.

#### Where this fits

- Task 124: `msi_layout::layout(dirs, comps, files) -> Vec<Placed>`.
- `cab` 0.6.0 (S3): `Cabinet::read_file` recria o decodificador a cada arquivo, O(n²) por pasta (webex > 577 s contra 16 s sequencial). A sonda usou uma cópia com patch em `target/probe-2b/s3/cab-0.6.0` (`Cabinet::for_each_file`: um decodificador por pasta, arquivos em ordem de offset), conferida igual ao `read_file` em 2.616 arquivos. `cab` lê MSZIP e LZX (SHA iguais na S1), **sem Quantum**.
- `msi` 0.10.0 (S1): lê tabelas e streams de MSI real; escreve MSI (útil para testes; o Windows Installer recusa o MSI gerado, o nosso leitor não depende disso).
- S3: 73/73 OLE/CFB; `extract_dir` existe nos 62 que têm, 3 só sem caixa e 4 com `/`.
- `ExtractError::Unsupported` (Task 116), `write_entry`, `Budget`, `Mapper` (`extract.rs`).

#### What binds this task

1. **Vendorizar** `cab` 0.6.0 em `vendor/cab-0.6.0/` (cópia do registry, `LICENSE` incluída) com o patch `for_each_file` da sonda (diff mínimo, LF) e `[patch.crates-io] cab = { path = "vendor/cab-0.6.0" }`; `cab = "=0.6.0"` e `msi = "=0.10.0"` em `[dependencies]`. O patch ganha um comentário de cabeçalho com o motivo e a data. Se o gate formatar ou lintar `vendor/`, exclua-o e registre como.
2. **`src/install/extract_msi.rs`**, `extract_msi(file, root, mapper, limits)`:
   - Abrir **só leitura** com `msi::Package::open`.
   - Recusar com `ExtractError::Unsupported(<motivo>)` (a Task 129 usa isso para oferecer a reserva): gabinete externo (`Media.Cabinet` sem `#`), MSI sem compressão, gabinete Quantum, transform obrigatório. Um motivo distinto e testado para cada.
   - `Directory`, `Component`, `File` ⇒ `msi_layout::layout`; erro ⇒ `ExtractError::MsiLayout(String)`. Todos os nomes validados antes de gravar o primeiro byte.
   - **`extract_dir`:** compare sem caixa e aceite `/` e `\` (normalize antes do `Mapper`).
   - Para cada `#gabinete` da `Media`: abrir o stream e `Cabinet::for_each_file` em ordem; o nome no gabinete é a chave `File`; achar o `Placed`, passar pelo `mapper` e por `write_entry`. Arquivo do gabinete sem linha `File` e perdedores de colisão: lidos, cobrados no orçamento e não gravados. `File` sem dado em gabinete nenhum ⇒ `MsiLayout("arquivo X sem dados")`.
   - D4-1: `for_each_file` dentro de `catch_unwind`, como na Task 123.
3. Despacho em `extract_one` para `ArchiveKind::Msi`: farejar o cabeçalho OLE `D0 CF 11 E0 A1 B1 1A E1`; outro conteúdo ⇒ `Unsupported("conteúdo não é MSI")`. **Não** mude `supported()`.

**Files:**
- Create: `vendor/cab-0.6.0/**`, `src/install/extract_msi.rs`, `tests/install_msi.rs`, `tests/fixtures/msi/` (gabinetes pequenos gerados com `makecab` **uma vez** e versionados; comando no relatório)
- Modify: `Cargo.toml`, `Cargo.lock`, `src/install/mod.rs`, `src/install/extract.rs`

Testes (MSI montado no teste com a crate `msi`, gabinete da fixture como stream):
1. `extracts_msi_tree`: 2 diretórios aninhados (`destino:origem`), 3 arquivos ⇒ árvore e SHA-256 iguais.
2. `two_cabinets`: arquivos em `#cab1` e `#cab2`.
3. `external_cabinet_unsupported` e `uncompressed_unsupported`, com motivos distintos.
4. `unsafe_layout_writes_nothing`: `FileName` com `..` ⇒ erro, pasta pai sem nada novo.
5. `extract_dir_case_and_slash`: `extract_dir = "pfiles/App"` com a pasta `PFiles\app` ⇒ extrai.
6. `collision_loser_not_written`.
7. `sequential_reader_is_linear`: gabinete com 200 arquivos numa pasta ⇒ os bytes lidos do stream comprimido ficam em no máximo 2× o tamanho dele (leitor que conta bytes).
8. `truncated_cab_is_error`.

#### Checks beyond the steps

1. Mutação: aceitar `Cabinet` sem `#` ⇒ teste 3 **PEGA**.
2. Mutação: comparar `extract_dir` com caixa ⇒ teste 5 **PEGA**.
3. Mutação: no patch, recriar o decodificador por arquivo ⇒ teste 7 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): MSI extraction from embedded cabinets without running the installer`

---

### Task 126: `.tar.xz` e `.nupkg` (D3)

**Modelo:** sonnet.

#### Where this fits

- Decisão do usuário: D3-1 + `.tar.xz`, porque a S1 mostrou `lzma_rust2::XzReader::new(reader, true)` (feature `xz`) lendo `tar -cJf` do GNU tar e do bsdtar.
- `ArchiveKind::{TarXz, Nupkg}` (Task 116); `extract_tar_gz` em `extract.rs` usa `tar::Archive` sobre `flate2::read::GzDecoder`; `extract_zip`.

#### What binds this task

1. `lzma-rust2 = { version = "=0.22.0", default-features = false, features = ["xz"] }` (mesma versão que o `sevenz-rust2` trouxe; nenhuma duplicata em `cargo tree -d`).
2. `extract_tar_gz` vira `extract_tar(reader: impl Read, ...)`, usado com `GzDecoder` (TarGz) e `XzReader` (TarXz). O `.tar.gz` não muda (os testes existentes passam sem alteração).
3. `Nupkg` ⇒ `extract_zip`.
4. `supported()` passa a `true` para `TarXz` e `Nupkg`; o teste `only_2a_kinds_supported_for_now` vira `supported_kinds` com a lista nova.

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `src/install/extract.rs`, `src/install/archive.rs`, `tests/install_archive.rs`
- Test: `tests/install_extract.rs`

Testes:
1. `extracts_tar_xz`: `.tar.xz` gerado no teste (`tar::Builder` + o escritor xz da crate, se houver; senão uma fixture pequena gerada uma vez com `tar -cJf`, versionada, comando no relatório).
2. `tar_xz_symlink_refused`.
3. `extracts_nupkg`.
4. `manifest_and_extractor_agree` (Task 116) passa com a lista nova.

#### Checks beyond the steps

1. Mutação: `TarXz` volta a `supported() == false` ⇒ `manifest_and_extractor_agree` **PEGA**.
2. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): .tar.xz and .nupkg downloads`

---

### Task 127: Fila do Windows Installer (`msi.lock`) e lançador do `msiexec /a`

**Modelo:** opus.

#### Where this fits

- Decisão do usuário (2026-10-16): reserva `msiexec /a` com **fila**, para que instalações paralelas não travem. Spec 2b §(c2) itens 1–8; exceção no `CLAUDE.md` ("Exceção da fase 2b (reserva MSI)").
- Medido na S3: matar o cliente não cancela (o serviço terminou o gedit em 254 s segurando o mutex, e os 5 seguintes deram 1618); sem UAC.
- `install.lock` (`src/install/ops.rs:267` `lock_blocking`) já serializa as operações do wingetrs entre processos. A ponte (`src/tui/jobs.rs:257` `run_bridge`) roda o winget **fora** dele, e o winget roda instaladores MSI que disputam o mesmo mutex.

#### What binds this task

1. **`src/install/msi_queue.rs` (puro):**

```rust
pub enum Attempt { Done, Busy1618, Failed(i32) }
pub struct Policy { pub poll: Duration, pub wait_max: Duration, pub retries: [Duration; 3] }
/// poll 2 s, wait_max 600 s, retries 5 s, 15 s, 45 s.
pub const POLICY: Policy;
pub enum Status { WaitingLock, WaitingInstaller, Running { attempt: u8 }, Retrying { in_secs: u64 } }
/// Waits while `busy()` (Global\_MSIExecute), retries 1618 per `policy`, reports each state.
pub fn run_queued(policy: &Policy, busy: &mut dyn FnMut() -> bool,
                  launch: &mut dyn FnMut() -> io::Result<Attempt>,
                  sleep: &mut dyn FnMut(Duration), status: &mut dyn FnMut(Status))
                  -> Result<(), MsiQueueError>;
```

   `MsiQueueError::{InstallerBusy, Failed(i32), Io(String)}`, mensagens "o Windows Installer está ocupado por outro programa" e "o msiexec terminou com código {0}".
2. **`msi.lock`:** `Layout::msi_lock()` = `<raiz>\msi.lock`, trava exclusiva de arquivo entre processos com o mesmo mecanismo do `install.lock` (reuse a função existente); espera sem limite reportando `Status::WaitingLock`.
3. **`src/install/msiexec_win.rs`** (`#[cfg(windows)]`):
   - `installer_busy()`: `OpenMutexW(SYNCHRONIZE, false, "Global\\_MSIExecute")`; abriu ⇒ `true` (feche o handle); `ERROR_FILE_NOT_FOUND` ⇒ `false`. Nunca cria o mutex.
   - `msiexec_admin(msi, target_dir, log) -> io::Result<Attempt>`: `msiexec.exe /a "<msi>" /qn TARGETDIR="<target>" /l*v "<log>"`; espera o processo **sem nunca matá-lo** (depois de 10 min, um aviso no log a cada 60 s); 0 e 3010 ⇒ `Done` (3010 com aviso no log); 1618 ⇒ `Busy1618`; outro ⇒ `Failed`. Log em `<raiz>\log\msiexec-<app>-<n>.log`.
4. **A ponte entra na fila:** o job da ponte com `Tool::Winget` pega o `msi.lock` antes de lançar o winget e solta ao terminar, mostrando "aguardando outro instalador" enquanto espera. `Tool::Scoop` não pega.
5. Esta tarefa **não** liga a reserva ao executor (Task 129).

**Files:**
- Create: `src/install/msi_queue.rs`, `src/install/msiexec_win.rs`, `tests/install_msi_queue.rs`
- Modify: `src/install/mod.rs`, `src/install/layout.rs`, `src/tui/jobs.rs`, `Cargo.toml` (só features do crate `windows`, se faltar `Win32_System_Threading`)
- Test: `tests/install_msi_queue.rs`, `tests/tui_jobs.rs`

Testes (sem `msiexec` nem mutex real; `busy`, `launch` e `sleep` falsos):
1. `waits_while_installer_busy`: `busy` verdadeiro 3 vezes ⇒ 3 `sleep(poll)`, `WaitingInstaller` reportado, depois `Running`.
2. `busy_too_long_fails`: `busy` sempre ⇒ `InstallerBusy` depois de `wait_max / poll` consultas.
3. `retries_1618_with_backoff`: `Busy1618` duas vezes e `Done` ⇒ `sleep(5 s)`, `sleep(15 s)`, `Ok`.
4. `gives_up_after_three_1618`: quatro `Busy1618` ⇒ `InstallerBusy`, sleeps 5, 15, 45.
5. `failed_code_is_error`: `Failed(1603)` ⇒ erro sem retry.
6. `msi_lock_is_exclusive_across_threads`: a segunda thread só entra depois que a primeira solta (prove pela ordem dos eventos, não por relógio).
7. `bridge_winget_takes_msi_lock` (`tui_jobs`, executor falso): com o `msi.lock` preso, o job do winget espera e reporta o estado; `Tool::Scoop` não espera.
8. `installer_busy_live` com `#[ignore = "live: lê o mutex global do Windows Installer"]`.

#### Checks beyond the steps

1. Mutação: não reconsultar `busy` depois do primeiro `sleep` ⇒ teste 1 **PEGA**.
2. Mutação: tratar 1618 como `Failed` ⇒ teste 3 **PEGA**.
3. Mutação: a ponte não pegar o `msi.lock` ⇒ teste 7 **PEGA**.
4. `grep -n "TerminateProcess\|\.kill()" src/install/msiexec_win.rs` vazio.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): Windows Installer queue (msi.lock, mutex wait, 1618 retry) shared with bridge winget jobs`

---

### Task 128: Ligar 7z e MSI no manifesto, no plano e no executor (P5 + P6)

**Modelo:** opus.

#### Where this fits

- Tasks 123 (7z), 125 (MSI), 126 (xz, nupkg); `ArchiveKind::supported()` (Task 116).
- `src/install/executor.rs:586`: `extract::extract(...)` é chamado **síncrono** dentro do `async fn execute_install` (`:344`); o maior 7z do corpus leva ~10,5 s (S2), o que trava um worker do runtime.
- `src/install/plan.rs` (~:222): verbo `ArchiveKind::verb`, `Display` de `Step::Extract`.
- Snapshots `tests/snapshots/install_planner__*.snap` da 2a **não mudam**.

#### What binds this task

1. `supported()` passa a `true` para `SevenZip` e `Msi`.
2. **Executor:** a extração roda em `tokio::task::spawn_blocking`, com o handle sempre aguardado e pânico propagado com `resume_unwind` (padrão de `download_one` em `src/install/download.rs`). Log "extraindo <nome>" antes e o tempo depois.
3. **Plano:** o `Display` de `Step::Extract` mostra o motor: `(7z)`, `(MSI, sem executar o instalador)`, `(tar.xz)`, `(zip)` para nupkg. Zip, tar.gz e cópia ficam como hoje.
4. **Ciclo completo**, com os fakes dos testes de executor existentes (`grep -ln "execute_install" tests`).
5. Pânico do decodificador (leitor injetado, como na Task 123) ⇒ erro com rollback, sem travar.

**Files:**
- Modify: `src/install/archive.rs`, `src/install/plan.rs`, `src/install/executor.rs`, `tests/install_archive.rs`
- Test: o arquivo de testes de executor existente, `tests/install_planner.rs` + snapshots **novos** (`insta`) para 7z e MSI

Testes:
1. `sevenzip_manifest_resolves`: manifesto mínimo com `.7z` ⇒ `resolve` ok e plano com `(7z)`.
2. `msi_manifest_resolves`: `.msi` + `extract_dir` ⇒ plano com `(MSI, sem executar o instalador)`.
3. `rar_still_unsupported`.
4. `install_remove_7z_leaves_nothing` e `install_remove_msi_leaves_nothing`.
5. `corrupt_7z_rolls_back_identically` (lista da raiz antes e depois).
6. `extract_runs_off_the_runtime`: runtime `current_thread`, extração de um 7z de ~32 MiB e um temporizador que precisa avançar durante ela (técnica de `cached_lookup_does_not_block_the_runtime`, Task 121).
7. `decoder_panic_rolls_back`.

#### Checks beyond the steps

1. Mutação: extrair sem `spawn_blocking` ⇒ teste 6 **PEGA**.
2. Mutação: `.7z` volta a `supported() == false` ⇒ teste 1 **PEGA**.
3. `git diff --stat tests/snapshots` só com snapshots **novos**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): install 7z, MSI, tar.xz and nupkg apps from Scoop`

---

### Task 129: Reserva `msiexec /a` no executor, com confirmação por item

**Modelo:** opus.

#### Where this fits

- Task 127: `msi_queue::run_queued`, `msiexec_win::{installer_busy, msiexec_admin}`, `Layout::msi_lock()`.
- Task 125: `ExtractError::Unsupported(<motivo>)` do MSI (gabinete externo, sem compressão, Quantum, transform).
- Spec 2b §(c2) itens 1, 2, 6, 7; exceção no `CLAUDE.md`.
- TUI: `Modal::Confirm { items: Vec<ConfirmItem>, notes, .. }` (Task 110, `src/tui/app.rs`); `Modal::ConfirmQuit` (`src/tui/app.rs:311`).

#### What binds this task

1. O executor, ao receber `Unsupported` do extrator de MSI, **não** cai sozinho na reserva: faz o rollback normal e devolve `ExecError::NeedsMsiexec { app, source_index, reason }`.
2. **CLI:** `wingetrs install x --allow-msiexec` usa a reserva nesse caso. Sem a flag, a mensagem é: "este MSI precisa do Windows Installer (<motivo>); rode de novo com --allow-msiexec. O msiexec /a não pode ser cancelado depois de começar e grava arquivos em C:\Windows\Installer." `--yes` **não** autoriza.
3. **TUI:** `NeedsMsiexec` abre a confirmação daquele item com a nota "Usa o Windows Installer (msiexec /a). Não pode ser cancelado depois de começar; grava arquivos em C:\Windows\Installer.". `Continuar` reenfileira o item autorizado; `Cancelar` encerra o item como erro.
4. **Execução:** `TARGETDIR` = `<staging>\_msiexec\SourceDir`, dentro da raiz; `run_queued(POLICY, installer_busy, msiexec_admin, ...)`. A árvore de `SourceDir` passa pelo mesmo `mapper` (`extract_dir` sem caixa e com `/`), `safepath` e orçamento da extração própria; o `.msi` administrativo que o `msiexec` deixa na raiz do `TARGETDIR` é apagado.
5. **Sem cancelamento:** enquanto o `msiexec` roda, o job fica `uncancellable`; sair da TUI abre o `ConfirmQuit` com "O Windows Installer ainda está trabalhando; o wingetrs vai esperar ele terminar." e a TUI espera o job.
6. **Equivalência ao vivo:** teste `#[ignore = "live: msiexec"]` com 3 MSI pequenos (vetores da Task 119): extração própria × reserva ⇒ mesma árvore (caminhos + SHA-256).

**Files:**
- Modify: `src/install/executor.rs`, `src/install/ops.rs`, `src/cli/mod.rs`, `src/tui/app.rs`, `src/tui/jobs.rs`
- Test: o arquivo de testes de executor existente, `tests/cli_offline.rs`, `tests/tui_state.rs`, `tests/tui_jobs.rs`, `tests/install_msi_live.rs`

Testes (o lançador entra por injeção, como os outros efeitos de `OpsCtx::effects`; nada de `msiexec`):
1. `unsupported_msi_needs_msiexec`: sem autorização ⇒ `NeedsMsiexec` e rollback.
2. `allowed_msiexec_extracts_via_fake`: lançador falso grava uma árvore em `SourceDir` ⇒ instalado; o `.msi` administrativo não vai para a pasta do app.
3. `yes_does_not_allow_msiexec` (CLI).
4. `integrity_error_never_falls_back`: hash errado ou nome inseguro ⇒ erro direto, nunca `NeedsMsiexec`.
5. `tui_confirms_msiexec_per_item` e `quit_waits_for_uncancellable_job`.
6. Equivalência ao vivo (ignorado).

#### Checks beyond the steps

1. Mutação: cair na reserva sem autorização ⇒ teste 1 **PEGA**.
2. Mutação: `--yes` autorizar ⇒ teste 3 **PEGA**.
3. Mutação: oferecer reserva para erro de integridade ⇒ teste 4 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): msiexec /a fallback for unsupported MSI, confirmed per item and queued`

---

### Task 130: Oráculo da 2b por sub-caso (P7)

**Modelo:** sonnet.

#### Where this fits

- `examples/oracle_2a.rs` (Task 40): `resolve` + `plan_install` sobre os manifestos Scoop sem script do `catalog.db` local (`SELECT source, id, raw FROM packages WHERE kind = 'scoop'`, aberto só leitura); agrupa os não suportados por motivo.
- Spec 2b, tabela de cobertura estimada (7z + msi = 5,0% do Main, 7,9% do Extras).

#### What binds this task

1. Contar por sub-caso de download: zip, tar.gz, tar.xz, nupkg, 7z, msi, plain e os não suportados por motivo.
2. Imprimir a cobertura por bucket (Main, Extras) nas colunas "2a" (só formatos da 2a) e "2b".
3. Nada é baixado nem instalado.

**Files:**
- Modify: `examples/oracle_2a.rs`

Testes: rodar `cargo run --release --example oracle_2a -- tests/fixtures/scoop <catalog.db>` e colar a tabela real no relatório.

#### Checks beyond the steps

1. `unsup-7z` passa a dar "ok" (cole as linhas antes e depois).
2. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(examples): phase 2b coverage by download sub-case`

---

### Task 131: Prova ao vivo, NOTICE e fechamento da 2b (P8)

**Modelo:** opus. **Precisa do usuário** para a prova ao vivo.

#### What binds this task

1. `NOTICE` na raiz com as atribuições Apache-2.0 (`sevenz-rust2`, `lzma-rust2`), BSD-3-Clause (`encoding_rs`) e o `cab` vendorizado (MIT, com o patch); o release passa a anexar o `NOTICE`.
2. Prova ao vivo (`--release`, com autorização): instalar e remover 2 apps 7z, 2 apps MSI (um com `extract_dir`), 1 `.tar.xz`, 1 `.nupkg` e um MSI pela reserva com `--allow-msiexec`; conferir que nada sobra na raiz e que a ponte e a reserva não rodam juntas.
3. `docs/ARMADILHAS.md` com o que a 2b ensinou; spec 2b com status "implementada"; linha da fase 2b no `CLAUDE.md` (orquestrador).

**Files:**
- Create: `NOTICE`, `.claude/tasks/task-131-report.md`

Testes: nenhum novo (prova ao vivo).

#### Checks beyond the steps

1. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `docs: phase 2b live proof and third-party notices`
