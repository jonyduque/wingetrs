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
