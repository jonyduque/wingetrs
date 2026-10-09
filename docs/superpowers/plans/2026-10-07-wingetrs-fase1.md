# wingetrs Fase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Neste repositório o laço por tarefa está em `.claude/skills/run-task/SKILL.md`; cada tarefa vira um brief autocontido via `pwsh -File scripts/brief.ps1 <N>`.

**Goal:** CLI + TUI em Rust que atualiza, busca, mostra e lista pacotes do winget (CDN) e do Scoop, em milissegundos, sem depender do winget instalado.

**Architecture:** Um crate binário com lib (`src/lib.rs`) organizado por responsabilidade: `core` (tipos, versões), `sources/{winget,scoop}` (aquisição e parsing), `catalog` (SQLite próprio + busca fuzzy em memória), `cache`/`http` (disco e rede condicional), `installed` (registro, MSIX, scoop, normalização, correlação), `update`, `cli`, `tui` (Elm-like sobre ratatui).

**Tech Stack:** Rust 1.99 (edition 2024), tokio, reqwest (rustls), rusqlite (bundled), zip, flate2 (zlib-rs), tar, serde/serde_json/serde-saphyr/toml, roxmltree, sha2/base64/hex, fancy-regex, nucleo, ratatui 0.30 + crossterm 0.29, clap 4, winreg + windows 0.62 (só Windows), thiserror/anyhow/tracing; dev: insta, wiremock, tempfile.

**Spec:** `docs/superpowers/specs/2026-10-07-wingetrs-fase1-design.md`

## Global Constraints

- Rust edition 2024; `rust-version = "1.89"` (usa `std::fs::File::try_lock`).
- Dependências: exatamente as do `Cargo.toml` da Task 1. Nenhuma outra sem a tarefa mandar.
- `clippy -D warnings` com `unwrap_used`/`expect_used` negados fora de testes.
- Dado externo nunca causa pânico; erros com `thiserror` por módulo, `anyhow` só em `main.rs`/`cli`.
- Nada escreve em stdout fora de `src/main.rs` e `src/cli/`.
- Código Windows-only em arquivos `*_win.rs` atrás de `#[cfg(windows)] mod ...;`.
- Diretório de dados: `%LOCALAPPDATA%\wingetrs\` (`catalog.db`, `http.json`, `blobs\`, `update.lock`, `log\`, `config.toml`).
- URL do índice: `https://cdn.winget.microsoft.com/cache/source2.msix`; base do CDN: `https://cdn.winget.microsoft.com/cache/`.
- Refresh automático quando a última checagem tem mais de 900 s.
- Buckets padrão do scoop: `main`, `extras`.
- Glifos padrão: `nerd`; alternativas `unicode`, `ascii`.
- Texto ao usuário em português; código e comentários em inglês.

## Review Focus

1. **Primeira execução offline, sem cache:** `wingetrs search x` deve sair com código ≠ 0 e mensagem "sem catálogo local e sem rede" — nunca pânico. Teste na Task 9.
2. **Catálogo lido enquanto outra conexão faz `replace_source`:** o leitor continua vendo o estado anterior (WAL). Teste na Task 5.
3. **Entrada ARP com `DisplayVersion` vazia/lixo ou valores REG_DWORD onde se espera string:** vira versão desconhecida, sem pânico. Teste na Task 11 (filtro puro) e Task 12 (estado).
4. **Nomes não ASCII (CJK, acentos) na busca e na normalização:** busca por "360" acha "360安全卫士"; normalização bate com o corpus golden. Testes nas Tasks 5 e 10.
5. **Terminal minúsculo (20×5) e redimensionamento:** a TUI renderiza sem pânico. Teste na Task 14.

## Ordem e dependências

| Task | Título | Depende de | Tier |
|---|---|---|---|
| 1 | Scaffold do crate, tipos centrais e CLI | — | transcrição |
| 2 | Comparação de versões estilo winget | 1 | transcrição |
| 3 | Decodificador MSZIP e versionData | 1 | transcrição |
| 4 | Manifesto merged do winget | 1 | transcrição |
| 5 | Catálogo SQLite e busca fuzzy | 1 | transcrição |
| 6 | Diretório de dados, cache e HTTP condicional | 1 | transcrição |
| 7 | Fonte winget: índice, integridade e detalhes | 3,4,5,6 | transcrição |
| 8 | Fonte scoop: tarball, manifestos e detalhes | 5,6 | transcrição |
| 9 | Orquestração do update e comandos CLI | 7,8 | transcrição |
| 10 | Porte do NameNormalizer | 1 | **design** |
| 11 | Coleta de instalados (ARP, MSIX, scoop) | 1 | transcrição |
| 12 | Correlação e comando `list` | 2,5,7,10,11 | **design** |
| 13 | TUI: tema, glifos e estado | 5,12 | transcrição |
| 14 | TUI: views, loop de eventos e snapshots | 13 | transcrição |
| 15 | Validação ao vivo: oráculo e benchmarks | 14 | **design** (relatório com evidência) |

---

### Task 1: Scaffold do crate, tipos centrais e esqueleto da CLI

#### Where this fits

Primeira tarefa. Cria o crate que todas as outras usam: `Cargo.toml` travado, `lib.rs` com os módulos vazios, os tipos compartilhados de `core` e o parser da CLI (clap). Nenhuma lógica de rede ou disco.

#### What binds this task

- Edition 2024, `rust-version = "1.89"`.
- Dependências **exatamente** as abaixo (conjunto verificado em crate de sondagem em 2026-10-07, todas compilam juntas no Windows MSVC).
- `clippy -D warnings` com `unwrap_used` e `expect_used` negados no código de produção; permitidos em testes via `clippy.toml`.
- Texto ao usuário em português.

#### The decision this task must get right

- `src/main.rs` é fino: parseia `Cli`, inicializa nada mais, chama `wingetrs::cli::run(cli)` e converte erro em código de saída 1 com mensagem `erro: <cadeia de causas>` no stderr.
- Sem subcomando ⇒ `Command::Tui` implícito (abre a TUI; por ora devolve erro "TUI ainda não implementada").
- Subcomandos ainda não implementados devolvem `anyhow::bail!("comando ainda não implementado: <nome>")`.

#### Traps already paid for that apply here

- `crossterm` precisa da feature `event-stream` (o `EventStream` da Task 14). Adicionar só `ratatui` não basta.
- `flate2` com `zlib-rs`: o decodificador MSZIP (Task 3) precisa de `Decompress::set_dictionary`; verificado funcionando com esse backend.
- `winreg` e `windows` ficam em `[target.'cfg(windows)'.dependencies]` para a lógica pura compilar em qualquer plataforma.

**Files:**

- Create (new): `Cargo.toml`, `clippy.toml`, `rustfmt.toml`, `src/main.rs`, `src/lib.rs`, `src/cli/mod.rs`, `src/core/mod.rs`, `src/core/version.rs` (vazio com `//!`), `src/sources/mod.rs`, `src/sources/winget/mod.rs`, `src/sources/scoop/mod.rs`, `src/catalog/mod.rs`, `src/cache/mod.rs`, `src/http.rs`, `src/installed/mod.rs`, `src/update.rs`, `src/tui/mod.rs`
- Test (new): `tests/cli_parse.rs`

#### Interfaces

- Produces (`src/core/mod.rs`):

```rust
pub mod version;

use serde::Serialize;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Winget,
    Scoop,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str { /* "winget" | "scoop" */ }
}
impl fmt::Display for SourceKind { /* as_str */ }
impl FromStr for SourceKind {
    type Err = String; // "fonte desconhecida: <s>"
    /* case-insensitive "winget" | "scoop" */
}

/// One row of the unified catalog, as loaded for search.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageEntry {
    pub source: SourceKind,
    /// "winget" or "scoop:<bucket>"
    pub source_name: String,
    pub id: String,
    pub name: String,
    pub moniker: Option<String>,
    pub version: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallerSummary {
    pub architecture: String,
    pub installer_type: String,
    pub scope: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageDetails {
    pub source: SourceKind,
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
    pub installers: Vec<InstallerSummary>,
    /// Newest first.
    pub available_versions: Vec<String>,
}
```

- Produces (`src/cli/mod.rs`):

```rust
#[derive(Debug, clap::Parser)]
#[command(name = "wingetrs", version, about = "Gerenciador de pacotes rápido sobre winget e scoop")]
pub struct Cli {
    /// Conjunto de glifos da TUI
    #[arg(long, value_enum, global = true)]
    pub glyphs: Option<GlyphArg>,
    /// Verbosidade (-v, -vv): também loga no stderr
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GlyphArg { Nerd, Unicode, Ascii }

#[derive(Debug, PartialEq, clap::Subcommand)]
pub enum Command {
    /// Atualiza o catálogo local
    Update {
        #[arg(long)]
        force: bool,
        /// Usado pelo refresh em segundo plano: sem saída
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Busca pacotes
    Search {
        query: String,
        #[arg(long)]
        source: Option<SourceKind>,
        #[arg(long)]
        json: bool,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Mostra detalhes de um pacote
    Show {
        id: String,
        #[arg(long)]
        source: Option<SourceKind>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Lista pacotes instalados
    List {
        #[arg(long)]
        upgradable: bool,
        #[arg(long)]
        source: Option<SourceKind>,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(cli: Cli) -> anyhow::Result<()>;
```

`SourceKind` precisa implementar `clap::ValueEnum` **ou** ser parseado via `FromStr` (`value_parser = clap::value_parser!(...)` não serve para enum externo; use `#[arg(long, value_parser = parse_source)]` com `fn parse_source(s: &str) -> Result<SourceKind, String>`). Decisão: função `parse_source` em `cli/mod.rs` que chama `SourceKind::from_str`.

#### Steps

- [ ] **Step 1: `Cargo.toml` literal**

```toml
[package]
name = "wingetrs"
version = "0.1.0"
edition = "2024"
rust-version = "1.89"
description = "Gerenciador de pacotes rápido sobre os catálogos do winget e do scoop"
license = "MIT"

[lib]
path = "src/lib.rs"

[[bin]]
name = "wingetrs"
path = "src/main.rs"

[dependencies]
anyhow = "1.0.104"
base64 = "0.23.1"
clap = { version = "4.6.7", features = ["derive"] }
crossterm = { version = "0.29.0", features = ["event-stream"] }
directories = "6.0.0"
fancy-regex = "0.19.2"
flate2 = { version = "1.1.10", default-features = false, features = ["zlib-rs"] }
futures-util = "0.3.34"
hex = "0.4.3"
nucleo = "0.5.0"
ratatui = "0.30.2"
reqwest = { version = "0.13.5", default-features = false, features = ["rustls", "http2", "stream"] }
roxmltree = "0.21.1"
rusqlite = { version = "0.40.2", features = ["bundled"] }
serde = { version = "1.0.229", features = ["derive"] }
serde-saphyr = "1.3.0"
serde_json = "1.0.151"
sha2 = "0.11.0"
tar = "0.4.46"
thiserror = "2.0.21"
tokio = { version = "1.53.2", features = ["full"] }
toml = "1.1.6"
tracing = "0.1.44"
tracing-appender = "0.2.5"
tracing-subscriber = "0.3.23"
zip = { version = "8.6.0", default-features = false, features = ["deflate"] }

[target.'cfg(windows)'.dependencies]
windows = { version = "0.62.2", features = ["Management_Deployment", "ApplicationModel", "Foundation_Collections"] }
winreg = "0.56.0"

[dev-dependencies]
insta = "1.49.0"
tempfile = "3.27.0"
wiremock = "0.6.5"

[lints.clippy]
unwrap_used = "deny"
expect_used = "deny"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

`clippy.toml`:

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
```

`rustfmt.toml`:

```toml
edition = "2024"
max_width = 100
```

- [ ] **Step 2: Teste de parsing que falha** — `tests/cli_parse.rs`:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use clap::{CommandFactory, Parser};
use wingetrs::cli::{Cli, Command, GlyphArg};
use wingetrs::core::SourceKind;

#[test]
fn clap_definition_is_valid() {
    Cli::command().debug_assert();
}

#[test]
fn no_subcommand_means_tui() {
    let cli = Cli::try_parse_from(["wingetrs"]).unwrap();
    assert_eq!(cli.command, None);
}

#[test]
fn search_with_source_and_limit() {
    let cli = Cli::try_parse_from(["wingetrs", "search", "power toys", "--source", "SCOOP", "-n", "5"]).unwrap();
    assert_eq!(
        cli.command,
        Some(Command::Search { query: "power toys".into(), source: Some(SourceKind::Scoop), json: false, limit: 5 })
    );
}

#[test]
fn unknown_source_is_rejected_with_portuguese_message() {
    let err = Cli::try_parse_from(["wingetrs", "list", "--source", "choco"]).unwrap_err();
    assert!(err.to_string().contains("fonte desconhecida: choco"), "{err}");
}

#[test]
fn glyphs_is_global() {
    let cli = Cli::try_parse_from(["wingetrs", "list", "--glyphs", "ascii"]).unwrap();
    assert_eq!(cli.glyphs, Some(GlyphArg::Ascii));
}

#[test]
fn update_background_flag_is_hidden_but_accepted() {
    let cli = Cli::try_parse_from(["wingetrs", "update", "--background"]).unwrap();
    assert_eq!(cli.command, Some(Command::Update { force: false, background: true }));
    // The root help never lists subcommand flags; inspect the subcommand's own help.
    let mut root = Cli::command();
    let help = root.find_subcommand_mut("update").unwrap().render_long_help().to_string();
    assert!(help.contains("--force"), "sanity: update help rendered:\n{help}");
    assert!(!help.contains("--background"));
}

#[test]
fn source_kind_display_and_parse_round_trip() {
    for k in [SourceKind::Winget, SourceKind::Scoop] {
        assert_eq!(k.to_string().parse::<SourceKind>().unwrap(), k);
    }
}
```

Run: `cargo test --quiet --test cli_parse` → Expected: FAIL (crate ainda não compila / itens não existem).

- [ ] **Step 3: Implementar** `src/lib.rs` (`pub mod cache; pub mod catalog; pub mod cli; pub mod core; pub mod http; pub mod installed; pub mod sources; pub mod tui; pub mod update;`), os módulos vazios com um `//!` de uma linha cada, `src/core/mod.rs` e `src/cli/mod.rs` conforme Interfaces, e `src/main.rs`:

```rust
use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = wingetrs::cli::Cli::parse();
    match wingetrs::cli::run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 4:** `cargo test --quiet --test cli_parse` → PASS; `pwsh -File scripts/verify.ps1` → `[OK]   gate verde`.
- [ ] **Step 5: Commit** `feat(cli): scaffold crate, core types and command parser` (inclui `Cargo.lock`).

#### Checks beyond the steps

- Mutation proof (regra: `--source` é case-insensitive): `pwsh -File scripts/mutate.ps1 -Path src/core/mod.rs -Anchor '<o trecho que normaliza a caixa, ex.: to_ascii_lowercase()>' -Replacement '<mesma expressão sem normalizar, ex.: to_string()>' -TestCommand 'cargo test --quiet --test cli_parse'` → deve sair `[OK] mutação PEGA`. Adapte a âncora ao texto exato que você escreveu e cole-o no relatório.
- `cargo tree -d --depth 1` sem duplicatas de `crossterm` (ratatui e o crate direto devem unificar em 0.29). Cole a saída.

**Commit:** `feat(cli): scaffold crate, core types and command parser`

---

### Task 2: Comparação de versões estilo winget

#### Where this fits

`list` decide "atualização disponível" comparando a versão instalada (texto livre do registro) com a do catálogo. O winget não usa semver; usa `Utility::Version`. Esta tarefa porta só o necessário.

#### What binds this task

- Porte de `docs/reference/winget-cli/AppInstallerSharedLib_Versions.cpp` (classe `Version` e `Version::Part`) e do header `AppInstallerSharedLib_Public_AppInstallerVersions.h`. Leia só essas classes.
- **Fora do escopo:** `ApproximateComparator`, `RawVersion`, `UInt64Version`, `SemanticVersion`, `VersionRange`, `GatedVersion`, `Channel`, `OpenTypeFontVersion`.
- Separador padrão: `"."`. `parse_with(s, split_chars)` aceita outros (teste `.-`).
- Cada função portada tem um comentário de uma linha citando a origem (`// port: Versions.cpp Version::Assign`).

#### The decision this task must get right

`Version` implementa `Ord`/`Eq` de forma que **`"1.0" == "1.0.0"`** (zeros finais não contam) e `latest` > qualquer versão > `unknown`. `Eq` precisa ser consistente com `Ord` (derive manual), senão `BTreeMap`/sort quebram em silêncio.

**Files:**

- Modify: `src/core/version.rs`
- Test (new): `tests/version.rs`

#### Interfaces

- Produces:

```rust
#[derive(Debug, Clone)]
pub struct Version { /* raw: String, parts: Vec<Part>, kind */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub integer: u64,
    pub other: String,
}

impl Version {
    pub fn parse(s: &str) -> Version;                 // split chars "."
    pub fn parse_with(s: &str, split_chars: &str) -> Version;
    pub fn latest() -> Version;
    pub fn unknown() -> Version;
    pub fn is_latest(&self) -> bool;
    pub fn is_unknown(&self) -> bool;
    pub fn is_empty(&self) -> bool;
    pub fn parts(&self) -> &[Part];
    pub fn part_at(&self, index: usize) -> Part;      // zero part when out of range
    pub fn as_str(&self) -> &str;                      // original text
}
impl Default for Version { /* empty */ }
impl PartialEq for Version; impl Eq for Version; impl PartialOrd for Version; impl Ord for Version;
impl std::fmt::Display for Version;
```

#### The body of the tests that are not obvious

Transcreva literalmente (portados de `AppInstallerCLITests_Versions.cpp`):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::core::version::{Part, Version};

fn p(i: u64, o: &str) -> Part {
    Part { integer: i, other: o.to_string() }
}

fn lt(a: &str, b: &str) {
    let (va, vb) = (Version::parse(a), Version::parse(b));
    assert!(va < vb, "{a} < {b}");
    assert!(!(vb < va), "!({b} < {a})");
    assert!(va <= vb && !(vb <= va), "<= {a} {b}");
    assert!(vb > va && !(va > vb), "> {a} {b}");
    assert!(vb >= va && !(va >= vb), ">= {a} {b}");
    assert!(va != vb, "{a} != {b}");
}

fn eq(a: &str, b: &str) {
    let (va, vb) = (Version::parse(a), Version::parse(b));
    assert!(va == vb, "{a} == {b}");
    assert!(va <= vb && va >= vb && !(va < vb) && !(va > vb), "ord {a} {b}");
    assert_eq!(va.cmp(&vb), std::cmp::Ordering::Equal);
}

#[test]
fn parse_basic_with_suffix() {
    assert_eq!(Version::parse("1.2.3.4-alpha").parts(), &[p(1, ""), p(2, ""), p(3, ""), p(4, "-alpha")]);
}

#[test]
fn parse_plus_dash_split() {
    assert_eq!(
        Version::parse_with("1.2.3.4-alpha", ".-").parts(),
        &[p(1, ""), p(2, ""), p(3, ""), p(4, ""), p(0, "alpha")]
    );
}

#[test]
fn parse_whitespace_and_preamble() {
    let four = [p(1, ""), p(2, ""), p(3, ""), p(4, "")];
    assert_eq!(Version::parse("1. 2.3 . 4 ").parts(), &four);
    assert_eq!(Version::parse("v1.2.3.4").parts(), &four);
}

#[test]
fn parse_corners() {
    assert!(Version::parse("").parts().is_empty());
    assert!(Version::parse(".").parts().is_empty());
    assert!(Version::parse(".0").parts().is_empty());
    assert_eq!(Version::parse(".1").parts(), &[p(0, ""), p(1, "")]);
    assert_eq!(Version::parse("version").parts(), &[p(0, "version")]);
    assert_eq!(Version::parse(". 1 ").parts(), &[p(0, ""), p(1, "")]);
    assert_eq!(Version::parse("v1.2a").parts(), &[p(1, ""), p(2, "a")]);
}

#[test]
fn compare_table() {
    lt("1", "2");
    lt("1.0.0", "2.0.0");
    lt("0.0.1", "0.0.2");
    lt("0.0.1-alpha", "0.0.2-alpha");
    lt("0.0.1-beta", "0.0.2-alpha");
    lt("13.9.8", "14.1");
    lt("1-rc", "1");
    lt("1.2-rc", "1.2");
    lt("1.0-rc", "1.0");
    lt("1.0.0-rc", "1");
    lt("22.0.0-rc.1", "22.0.0");
    lt("22.0.0-rc.1", "22.0.0.1");
    lt("22.0.0-rc.1", "22.0.0.1-rc");
    lt("22.0.0-rc.1", "22.0.0-rc.1.1");
    lt("22.0.0-rc.1.1", "22.0.0-rc.1.2");
    lt("22.0.0-rc.1.2", "22.0.0-rc.2");
    eq("1.0", "1.0.0");
    eq("1.2.00.3", "1.2.0.3");
    eq("1.2.003.4", "1.2.3.4");
    eq("01.02.03.04", "1.2.3.4");
    eq("1.2.03-beta", "1.2.3-beta");
    eq("1.0", "1.0 ");
    eq("1.0", "1. 0");
    eq("1.0", "1.0.");
    eq("1.0", "Version 1.0");
    eq("foo1", "bar1");
    lt("v0.0.1", "0.0.2");
    lt("v0.0.1", "v0.0.2");
    lt("1.a2", "1.b1");
    lt("alpha", "beta");
}

#[test]
fn latest_and_unknown() {
    assert!(Version::latest().is_latest());
    assert!(Version::parse("latest").is_latest());
    assert!(Version::parse("LATEST").is_latest());
    assert!(!Version::parse("1.0").is_latest());
    lt("1.0", "latest");
    lt("100", "latest");
    lt("943849587389754876.1", "latest");
    eq("latest", "LATEST");
    assert!(Version::unknown().is_unknown());
    assert!(Version::parse("unknown").is_unknown());
    assert!(Version::parse("UNKNOWN").is_unknown());
    assert!(!Version::parse("1.0").is_unknown());
    lt("unknown", "1.0");
    lt("unknown", "1.fork");
    eq("unknown", "UNKNOWN");
    assert!(Version::unknown() < Version::latest());
}

#[test]
fn empty_and_part_at() {
    assert!(Version::default().is_empty());
    assert!(Version::parse("").is_empty());
    assert!(!Version::parse("1").is_empty());
    assert!(!Version::parse("0").is_empty());
    assert_eq!(Version::default().part_at(0).integer, 0);
    assert_eq!(Version::parse("1").part_at(0).integer, 1);
    assert_eq!(Version::parse("1").part_at(1).integer, 0);
    assert_eq!(Version::parse("1").part_at(9999).integer, 0);
}

#[test]
fn sort_is_consistent_with_eq() {
    let mut v: Vec<Version> = ["2.0", "1.0.0", "latest", "1.0", "unknown", "1.0-rc"]
        .iter()
        .map(|s| Version::parse(s))
        .collect();
    v.sort();
    let s: Vec<&str> = v.iter().map(|x| x.as_str()).collect();
    assert_eq!(s[0], "unknown");
    assert_eq!(s[1], "1.0-rc");
    assert_eq!(&s[4..], &["2.0", "latest"]);
}
```

#### Steps

- [ ] Step 1: crie `tests/version.rs` com o código acima; `cargo test --quiet --test version` → FAIL (não compila).
- [ ] Step 2: implemente `src/core/version.rs` portando `Version::Assign`, `Version::Trim`, `Version::operator<`, `Version::Part::Part`, `Version::Part::operator<`, `IsLatest`, `IsUnknown`. Comparação de `other` como no C++ (case-insensitive conforme a fonte).
- [ ] Step 3: `cargo test --quiet --test version` → PASS; `pwsh -File scripts/verify.ps1` → verde.
- [ ] Step 4: commit.

#### Checks beyond the steps

Três regras, três mutações (adapte a âncora ao texto exato do seu código e cole o comando e a saída):

1. Zeros finais são ignorados: mutate a remoção de partes zero finais (ex.: comente/inverta a condição do trim) → `cargo test --quiet --test version compare_table` deve falhar.
2. Sufixo vazio é maior que sufixo não vazio (`"1-rc" < "1"`): inverta essa comparação → falha esperada.
3. `unknown < qualquer versão`: troque a ordem relativa de `unknown` → falha esperada em `latest_and_unknown`.

**Files:** `src/core/version.rs`, `tests/version.rs`
**Commit:** `feat(core): winget-style version parsing and ordering`

---

### Task 3: Decodificador MSZIP e versionData

#### Where this fits

`show` de um pacote winget baixa `packages/<Id>/<hash8>/versionData.mszyml` do CDN: YAML comprimido no formato MSZIP da Windows Compression API. Esta tarefa decodifica e parseia, sem rede.

#### What binds this task

- Formato (verificado em arquivos reais em 2026-10-07):
  - bytes 0..4 = `0A 51 E5 C0` (assinatura);
  - bytes 8..16 = tamanho descomprimido total, `u64` little-endian;
  - a partir do byte 24: blocos `u32 LE tamanho` + payload, onde payload começa com `"CK"` e o resto é **deflate cru**;
  - cada bloco descomprime até 32 KiB e **usa os últimos 32 KiB já descomprimidos como dicionário**.
- YAML: `sV: 1.0` e `vD: [ { v, rP, s256H } ]`. Campos curtos; `v` pode parecer número (`1.10`) e deve continuar texto.
- URL: `https://cdn.winget.microsoft.com/cache/packages/<Id>/<8 primeiros hex minúsculos do hash>/versionData.mszyml`.

#### The measured evidence of the trap

Decodificando `discord-canary.versionData.mszyml` (4 blocos) **sem** dicionário: bloco 0 ok (32768 B), blocos 1–3 falham com `invalid distance too far back`. Com `flate2::Decompress::new(false)` + `set_dictionary(&out[len-32768..])` decodifica 112103/112103 bytes. Verificado com `flate2` backend `zlib-rs`.

#### The decision this task must get right

Nunca indexar o buffer sem checar limites: arquivo truncado ou corrompido devolve `MszipError`, nunca pânico. O total descomprimido deve bater com o header (`SizeMismatch`).

**Files:**

- Create (new): `src/sources/winget/mszip.rs`, `src/sources/winget/version_data.rs`
- Modify: `src/sources/winget/mod.rs` (`pub mod mszip; pub mod version_data;` e `pub const CDN_BASE: &str = "https://cdn.winget.microsoft.com/cache/";`)
- Test (new): `tests/winget_version_data.rs`

#### Interfaces

```rust
// src/sources/winget/mszip.rs
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MszipError {
    #[error("assinatura MSZIP inválida")]
    BadSignature,
    #[error("dados MSZIP truncados")]
    Truncated,
    #[error("bloco MSZIP sem marcador CK")]
    MissingCk,
    #[error("deflate inválido: {0}")]
    Deflate(String),
    #[error("tamanho descomprimido diverge: esperado {expected}, obtido {actual}")]
    SizeMismatch { expected: usize, actual: usize },
}
pub fn decode(input: &[u8]) -> Result<Vec<u8>, MszipError>;

// src/sources/winget/version_data.rs
#[derive(Debug, Clone, PartialEq)]
pub struct VersionDataEntry { pub version: String, pub rel_path: String, pub sha256: String }
#[derive(Debug, Clone, PartialEq)]
pub struct VersionData { pub entries: Vec<VersionDataEntry> }
#[derive(Debug, thiserror::Error)]
pub enum VersionDataError {
    #[error(transparent)]
    Mszip(#[from] super::mszip::MszipError),
    #[error("versionData não é UTF-8")]
    Utf8,
    #[error("versionData inválido: {0}")]
    Yaml(String),
}
pub fn parse(yaml: &str) -> Result<VersionData, VersionDataError>;
pub fn decode_and_parse(mszyml: &[u8]) -> Result<VersionData, VersionDataError>;
/// "packages/<id>/<hex8>/versionData.mszyml" (relative to CDN_BASE)
pub fn version_data_path(id: &str, hash: &[u8]) -> String;
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::sources::winget::mszip::{decode, MszipError};
use wingetrs::sources::winget::version_data::{decode_and_parse, parse, version_data_path};

const PT: &[u8] = include_bytes!("fixtures/winget/powertoys.versionData.mszyml");
const PT_YAML: &[u8] = include_bytes!("fixtures/winget/powertoys.versionData.yaml");
const DC: &[u8] = include_bytes!("fixtures/winget/discord-canary.versionData.mszyml");
const DC_YAML: &[u8] = include_bytes!("fixtures/winget/discord-canary.versionData.yaml");

#[test]
fn single_block_matches_reference_decoder() {
    assert_eq!(decode(PT).unwrap(), PT_YAML);
}

#[test]
fn multi_block_needs_dictionary_chaining() {
    let out = decode(DC).unwrap();
    assert_eq!(out.len(), 112_103);
    assert_eq!(out, DC_YAML);
}

#[test]
fn truncated_input_is_an_error_not_a_panic() {
    for cut in [0, 3, 10, 23, 24, 27, 30, DC.len() / 2, DC.len() - 1] {
        assert!(decode(&DC[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn bad_signature() {
    let mut b = PT.to_vec();
    b[0] ^= 0xFF;
    assert_eq!(decode(&b), Err(MszipError::BadSignature));
}

#[test]
fn missing_ck_marker() {
    let mut b = PT.to_vec();
    b[28] = b'X'; // first block payload starts at 24 + 4
    assert_eq!(decode(&b), Err(MszipError::MissingCk));
}

#[test]
fn size_mismatch_is_reported() {
    let mut b = PT.to_vec();
    b[8] = b[8].wrapping_add(1); // header claims one byte more
    assert!(matches!(decode(&b), Err(MszipError::SizeMismatch { .. }) | Err(MszipError::Truncated)));
}

#[test]
fn parses_powertoys_versions() {
    let vd = decode_and_parse(PT).unwrap();
    assert_eq!(vd.entries.len(), 115);
    let first = &vd.entries[0];
    assert_eq!(first.version, "0.101.2362.0");
    assert_eq!(first.rel_path, "manifests/m/Microsoft/PowerToys/0.101.2362.0/509b");
    assert_eq!(first.sha256, "345056f4d70d74ef350a0e00a7c5a710657271a882be4fa0bc263af6bb088245");
}

#[test]
fn parses_793_discord_canary_versions() {
    assert_eq!(decode_and_parse(DC).unwrap().entries.len(), 793);
}

#[test]
fn numeric_looking_versions_stay_text() {
    let vd = parse("sV: 1.0\nvD:\n- v: 1.10\n  rP: a/b\n  s256H: ab\n- v: 2\n  rP: c\n  s256H: cd\n").unwrap();
    assert_eq!(vd.entries[0].version, "1.10");
    assert_eq!(vd.entries[1].version, "2");
}

#[test]
fn garbage_yaml_is_an_error() {
    assert!(parse("vD: [unterminated").is_err());
}

#[test]
fn path_uses_first_8_hex_of_hash() {
    let hash = hex::decode("d193b67ac71ce5d456c10f6731447abb6df68ae12ef8691c92af8ed82659e720").unwrap();
    assert_eq!(
        version_data_path("Microsoft.PowerToys", &hash),
        "packages/Microsoft.PowerToys/d193b67a/versionData.mszyml"
    );
}
```

`hex` é dependência normal do crate, então está disponível nos testes de integração.

#### Steps

- [ ] Step 1: teste acima → FAIL.
- [ ] Step 2: implemente `decode` (laço de blocos com checagem de limites; `flate2::Decompress::new(false)`; `set_dictionary` com a janela dos últimos 32 KiB a partir do 2º bloco; `decompress_vec` com capacidade 32 KiB + folga; erro de deflate → `Deflate(msg)`); `parse` com `serde-saphyr` (`#[serde(rename = "vD")]` etc.); `decode_and_parse`; `version_data_path`.
- [ ] Step 3: PASS; gate verde; commit.

#### Checks beyond the steps

1. Encadeamento de dicionário: mutate a linha do `set_dictionary(...)` para não ser chamada (ex.: trocar a condição por `false`) → `cargo test --quiet --test winget_version_data multi_block` deve falhar.
2. Checagem de tamanho total: mutate o retorno de `SizeMismatch` para `Ok(out)` → `size_mismatch_is_reported` falha.

**Commit:** `feat(winget): MSZIP decoder and versionData parsing`

---

### Task 4: Manifesto merged do winget

#### Where this fits

Depois do versionData, `show` baixa `cache/<rP>`: um YAML único `ManifestType: merged`. Esta tarefa parseia esse YAML e o converte em `PackageDetails`.

#### What binds this task

- Chaves em PascalCase; campos desconhecidos ignorados (o schema vai até 1.12 e cresce).
- Herança: `InstallerType` e `Scope` na raiz valem para cada item de `Installers` que não os define.
- `description` em `PackageDetails` = `ShortDescription` se existir, senão `Description`.
- `homepage` = `PackageUrl`, senão `PublisherUrl`.
- Arquivo real tem fim de linha CRLF.

**Files:**

- Create (new): `src/sources/winget/manifest.rs`
- Modify: `src/sources/winget/mod.rs` (`pub mod manifest;`)
- Test (new): `tests/winget_manifest.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MergedManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub package_name: Option<String>,
    pub publisher: Option<String>,
    pub short_description: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub package_url: Option<String>,
    pub publisher_url: Option<String>,
    pub moniker: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub installer_type: Option<String>,
    pub scope: Option<String>,
    #[serde(default)]
    pub installers: Vec<Installer>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Installer {
    pub architecture: String,
    pub installer_type: Option<String>,
    pub scope: Option<String>,
    pub installer_url: String,
    pub installer_sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifesto não é UTF-8")]
    Utf8,
    #[error("manifesto inválido: {0}")]
    Yaml(String),
}

pub fn parse(bytes: &[u8]) -> Result<MergedManifest, ManifestError>;

impl MergedManifest {
    pub fn into_details(self, available_versions: Vec<String>) -> crate::core::PackageDetails;
}
```

`into_details`: `source = Winget`, `name = package_name.unwrap_or(package_identifier.clone())`; `installer_type` de cada installer = próprio, senão raiz, senão `"unknown"`; `scope` = próprio, senão raiz.

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::core::SourceKind;
use wingetrs::sources::winget::manifest::parse;

const PT: &[u8] = include_bytes!("fixtures/winget/powertoys-0.101.2362.0.merged.yaml");

#[test]
fn parses_real_powertoys_manifest() {
    let m = parse(PT).unwrap();
    assert_eq!(m.package_identifier, "Microsoft.PowerToys");
    assert_eq!(m.package_version, "0.101.2362.0");
    assert_eq!(m.installers.len(), 4);
    assert_eq!(m.tags.len(), 10);
}

#[test]
fn details_inherit_root_installer_type_and_keep_scope() {
    let d = parse(PT).unwrap().into_details(vec!["0.101.2362.0".into(), "0.100.2".into()]);
    assert_eq!(d.source, SourceKind::Winget);
    assert_eq!(d.name, "PowerToys");
    assert_eq!(d.publisher.as_deref(), Some("Microsoft Corporation"));
    assert_eq!(d.license.as_deref(), Some("MIT"));
    assert_eq!(d.homepage.as_deref(), Some("https://github.com/microsoft/PowerToys"));
    assert!(d.description.as_deref().unwrap().starts_with("Microsoft PowerToys is a set of utilities"));
    let types: Vec<&str> = d.installers.iter().map(|i| i.installer_type.as_str()).collect();
    assert_eq!(types, ["burn", "burn", "burn", "burn"]);
    let archs: Vec<&str> = d.installers.iter().map(|i| i.architecture.as_str()).collect();
    assert_eq!(archs, ["x64", "x64", "arm64", "arm64"]);
    let scopes: Vec<Option<&str>> = d.installers.iter().map(|i| i.scope.as_deref()).collect();
    assert_eq!(scopes, [Some("user"), Some("machine"), Some("user"), Some("machine")]);
    assert_eq!(d.available_versions, ["0.101.2362.0", "0.100.2"]);
}

#[test]
fn installer_level_type_overrides_root() {
    let y = "PackageIdentifier: A.B\nPackageVersion: 1.0\nInstallerType: exe\nInstallers:\n- Architecture: x86\n  InstallerType: msi\n  InstallerUrl: https://x/a.msi\n  InstallerSha256: AA\n- Architecture: x64\n  InstallerUrl: https://x/b.exe\n  InstallerSha256: BB\n";
    let d = parse(y.as_bytes()).unwrap().into_details(vec![]);
    assert_eq!(d.installers[0].installer_type, "msi");
    assert_eq!(d.installers[1].installer_type, "exe");
    assert_eq!(d.name, "A.B");
    assert_eq!(d.version, "1.0");
}

#[test]
fn minimal_manifest_without_optional_fields() {
    let d = parse(b"PackageIdentifier: X.Y\nPackageVersion: '2'\n").unwrap().into_details(vec![]);
    assert!(d.installers.is_empty());
    assert_eq!(d.installers.len(), 0);
    assert_eq!(d.publisher, None);
}

#[test]
fn invalid_utf8_and_yaml_are_errors() {
    assert!(parse(&[0xFF, 0xFE, 0x00]).is_err());
    assert!(parse(b"Installers: [").is_err());
}
```

#### Steps

- [ ] Step 1: teste → FAIL. Step 2: implemente. Step 3: PASS, gate, commit.

#### Checks beyond the steps

1. Herança do `InstallerType`: mutate o fallback para a raiz (ex.: `.or(root_type.clone())` → `.or(None)`) → `details_inherit_root_installer_type_and_keep_scope` falha.
2. Preferência de `ShortDescription` sobre `Description`: inverta a ordem → o `starts_with` continua passando? Se passar, o teste não pina a regra: acrescente ao teste `installer_level_type_overrides_root` um manifesto com `ShortDescription: curta` e `Description: longa` e asserção `Some("curta")`, e refaça a mutação até `[OK] mutação PEGA`.

**Commit:** `feat(winget): merged manifest parsing into PackageDetails`

---

### Task 5: Catálogo SQLite e busca fuzzy

#### Where this fits

Todas as leituras (`search`, `show`, `list`, TUI) leem o `catalog.db` local; cada fonte é substituída inteira numa transação. A busca roda em memória sobre `PackageEntry` com `nucleo`.

#### What binds this task

- Schema exato abaixo; `PRAGMA journal_mode=WAL` na abertura de arquivo.
- `replace_source` substitui **só** a fonte indicada, numa transação (`DELETE` pacotes e mapeamentos dessa fonte + `INSERT`s), e grava `meta`.
- Chaves de `map_productcode`, `map_upgradecode`, `map_pfn` gravadas em minúsculas; consulta também em minúsculas.
- `map_norm_*` gravadas exatamente como recebidas (o índice já guarda `mozillafirefox(X64)` com `X64` maiúsculo).
- Busca: query vazia → todos por nome (case-insensitive) até `limit`; match exato (case-insensitive) de `id` ou `moniker` vem antes de qualquer fuzzy; demais por score desc, empate por nome.

#### Schema

```sql
CREATE TABLE IF NOT EXISTS meta (
  source TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  source_version TEXT NOT NULL,
  fetched_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS packages (
  rowid INTEGER PRIMARY KEY,
  source TEXT NOT NULL,
  kind TEXT NOT NULL,
  id TEXT NOT NULL,
  name TEXT NOT NULL,
  moniker TEXT,
  version TEXT NOT NULL,
  publisher TEXT,
  description TEXT,
  tags TEXT NOT NULL DEFAULT '',
  commands TEXT NOT NULL DEFAULT '',
  arp_min_version TEXT,
  arp_max_version TEXT,
  winget_hash BLOB,
  raw TEXT,
  UNIQUE (source, id)
);
CREATE TABLE IF NOT EXISTS map_productcode    (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_upgradecode    (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_pfn            (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_norm_name      (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS map_norm_publisher (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS ix_pc  ON map_productcode(key);
CREATE INDEX IF NOT EXISTS ix_uc  ON map_upgradecode(key);
CREATE INDEX IF NOT EXISTS ix_pfn ON map_pfn(key);
CREATE INDEX IF NOT EXISTS ix_nn  ON map_norm_name(key);
CREATE INDEX IF NOT EXISTS ix_np  ON map_norm_publisher(key);
```

`tags` e `commands` gravados separados por um espaço.

**Files:**

- Modify: `src/catalog/mod.rs`
- Create (new): `src/catalog/search.rs`
- Test (new): `tests/catalog.rs`, `tests/search.rs`

#### Interfaces

```rust
// src/catalog/mod.rs
pub mod search;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewPackage {
    pub id: String,
    pub name: String,
    pub moniker: Option<String>,
    pub version: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub commands: Vec<String>,
    pub arp_min_version: Option<String>,
    pub arp_max_version: Option<String>,
    pub winget_hash: Option<Vec<u8>>,
    pub raw: Option<String>,
}

/// (key, package id) pairs; ids refer to NewPackage.id of the same import.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WingetMaps {
    pub productcodes: Vec<(String, String)>,
    pub upgradecodes: Vec<(String, String)>,
    pub pfns: Vec<(String, String)>,
    pub norm_names: Vec<(String, String)>,
    pub norm_publishers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapTable { ProductCode, UpgradeCode, Pfn, NormName, NormPublisher }

#[derive(Debug, Clone, PartialEq)]
pub struct SourceMeta { pub source: String, pub kind: SourceKind, pub source_version: String, pub fetched_at: i64 }

#[derive(Debug, Clone, PartialEq)]
pub struct StoredPackage {
    pub entry: PackageEntry,
    pub arp_min_version: Option<String>,
    pub arp_max_version: Option<String>,
    pub winget_hash: Option<Vec<u8>>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats { pub packages: usize, pub map_rows: usize }

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("erro no catálogo local: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("mapeamento aponta para pacote inexistente: {0}")]
    DanglingMap(String),
}

pub struct Catalog { /* conn: rusqlite::Connection */ }

impl Catalog {
    pub fn open(path: &std::path::Path) -> Result<Catalog, CatalogError>;
    pub fn open_in_memory() -> Result<Catalog, CatalogError>;
    pub fn replace_source(
        &mut self,
        source: &str,
        kind: SourceKind,
        source_version: &str,
        fetched_at: i64,
        packages: &[NewPackage],
        maps: &WingetMaps,
    ) -> Result<ImportStats, CatalogError>;
    pub fn sources(&self) -> Result<Vec<SourceMeta>, CatalogError>;
    pub fn load_entries(&self) -> Result<Vec<PackageEntry>, CatalogError>;
    pub fn get(&self, source: &str, id: &str) -> Result<Option<StoredPackage>, CatalogError>;
    /// Finds by id across sources (case-insensitive); used by `show <id>` without --source.
    pub fn find_by_id(&self, id: &str) -> Result<Vec<StoredPackage>, CatalogError>;
    /// Package ids (winget source) whose map key equals `key` (case rules above).
    pub fn lookup(&self, table: MapTable, key: &str) -> Result<Vec<String>, CatalogError>;
    pub fn is_empty(&self) -> Result<bool, CatalogError>;
}

// src/catalog/search.rs
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit { pub index: usize, pub score: u32, pub name_indices: Vec<u32> }

pub struct SearchIndex { /* entries, matcher */ }
impl SearchIndex {
    pub fn new(entries: Vec<PackageEntry>) -> SearchIndex;
    pub fn entries(&self) -> &[PackageEntry];
    pub fn search(&mut self, query: &str, filter: Option<SourceKind>, limit: usize) -> Vec<SearchHit>;
}
```

`name_indices` são as posições (em chars) do nome que casaram, para sublinhar na TUI; vazio quando o match veio do id/moniker/tags.

#### The body of the tests that are not obvious

`tests/catalog.rs` (new):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::{Catalog, MapTable, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;

fn pkg(id: &str, name: &str, version: &str) -> NewPackage {
    NewPackage { id: id.into(), name: name.into(), version: version.into(), ..Default::default() }
}

#[test]
fn replace_source_only_touches_that_source() {
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source("winget", SourceKind::Winget, "v1", 1, &[pkg("A.A", "Alpha", "1")], &WingetMaps::default()).unwrap();
    c.replace_source("scoop:main", SourceKind::Scoop, "e1", 1, &[pkg("alpha", "alpha", "2")], &WingetMaps::default()).unwrap();
    c.replace_source("winget", SourceKind::Winget, "v2", 2, &[pkg("B.B", "Beta", "1")], &WingetMaps::default()).unwrap();
    let mut ids: Vec<String> = c.load_entries().unwrap().into_iter().map(|e| e.id).collect();
    ids.sort();
    assert_eq!(ids, ["B.B", "alpha"]);
    let metas = c.sources().unwrap();
    assert_eq!(metas.iter().find(|m| m.source == "winget").unwrap().source_version, "v2");
}

#[test]
fn map_keys_are_case_insensitive_for_codes_and_exact_for_norm() {
    let mut c = Catalog::open_in_memory().unwrap();
    let maps = WingetMaps {
        productcodes: vec![("{23170F69-40C1-2701-1900-000001000000}".into(), "7zip.7zip".into())],
        norm_names: vec![("mozillafirefox(X64)".into(), "Mozilla.Firefox".into())],
        ..Default::default()
    };
    c.replace_source("winget", SourceKind::Winget, "v", 1, &[pkg("7zip.7zip", "7-Zip", "26.04"), pkg("Mozilla.Firefox", "Firefox", "157")], &maps).unwrap();
    assert_eq!(c.lookup(MapTable::ProductCode, "{23170f69-40c1-2701-1900-000001000000}").unwrap(), ["7zip.7zip"]);
    assert_eq!(c.lookup(MapTable::NormName, "mozillafirefox(X64)").unwrap(), ["Mozilla.Firefox"]);
    assert!(c.lookup(MapTable::NormName, "mozillafirefox(x64)").unwrap().is_empty());
}

#[test]
fn dangling_map_rolls_back_whole_import() {
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source("winget", SourceKind::Winget, "v1", 1, &[pkg("A.A", "Alpha", "1")], &WingetMaps::default()).unwrap();
    let bad = WingetMaps { pfns: vec![("x_1".into(), "Missing.Id".into())], ..Default::default() };
    assert!(c.replace_source("winget", SourceKind::Winget, "v2", 2, &[pkg("B.B", "Beta", "1")], &bad).is_err());
    let ids: Vec<String> = c.load_entries().unwrap().into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["A.A"], "failed import must leave previous data intact");
}

#[test]
fn reader_keeps_old_snapshot_while_writer_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut w = Catalog::open(&path).unwrap();
    w.replace_source("winget", SourceKind::Winget, "v1", 1, &[pkg("A.A", "Alpha", "1")], &WingetMaps::default()).unwrap();
    let r = Catalog::open(&path).unwrap();
    assert_eq!(r.load_entries().unwrap().len(), 1);
    let many: Vec<NewPackage> = (0..500).map(|i| pkg(&format!("P.{i}"), &format!("P{i}"), "1")).collect();
    w.replace_source("winget", SourceKind::Winget, "v2", 2, &many, &WingetMaps::default()).unwrap();
    assert_eq!(r.load_entries().unwrap().len(), 500, "reader sees the committed state after commit");
}

#[test]
fn tags_and_commands_round_trip() {
    let mut c = Catalog::open_in_memory().unwrap();
    let mut p = pkg("x", "x", "1");
    p.tags = vec!["a".into(), "b-c".into()];
    p.commands = vec!["rg".into()];
    c.replace_source("scoop:main", SourceKind::Scoop, "e", 1, &[p], &WingetMaps::default()).unwrap();
    let e = &c.load_entries().unwrap()[0];
    assert_eq!(e.tags, ["a", "b-c"]);
    assert_eq!(e.commands, ["rg"]);
    assert_eq!(e.source_name, "scoop:main");
    assert_eq!(e.source, SourceKind::Scoop);
}
```

`tests/search.rs` (new):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::search::SearchIndex;
use wingetrs::core::{PackageEntry, SourceKind};

fn e(source: SourceKind, id: &str, name: &str, moniker: Option<&str>) -> PackageEntry {
    PackageEntry {
        source,
        source_name: source.as_str().into(),
        id: id.into(),
        name: name.into(),
        moniker: moniker.map(Into::into),
        version: "1".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
    }
}

fn idx() -> SearchIndex {
    SearchIndex::new(vec![
        e(SourceKind::Winget, "lin-ycv.EverythingPowerToys", "EverythingPowerToys", None),
        e(SourceKind::Winget, "Microsoft.PowerToys", "PowerToys", Some("powertoys")),
        e(SourceKind::Scoop, "powertoys", "powertoys", None),
        e(SourceKind::Winget, "Qihoo.360TotalSecurity", "360安全卫士", None),
        e(SourceKind::Winget, "Zeal.Zeal", "Zeal", None),
    ])
}

#[test]
fn exact_moniker_or_id_wins() {
    let mut i = idx();
    let hits = i.search("powertoys", None, 10);
    let top: Vec<&str> = hits.iter().take(2).map(|h| i.entries()[h.index].id.as_str()).collect();
    assert!(top.contains(&"Microsoft.PowerToys") && top.contains(&"powertoys"), "{top:?}");
    assert_eq!(i.entries()[hits[2].index].id, "lin-ycv.EverythingPowerToys");
}

#[test]
fn source_filter() {
    let mut i = idx();
    let hits = i.search("powertoys", Some(SourceKind::Scoop), 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(i.entries()[hits[0].index].source, SourceKind::Scoop);
}

#[test]
fn empty_query_lists_by_name_case_insensitive() {
    let mut i = idx();
    let names: Vec<String> = i.search("", None, 3).iter().map(|h| i.entries()[h.index].name.clone()).collect();
    assert_eq!(names, ["360安全卫士", "EverythingPowerToys", "PowerToys"]);
}

#[test]
fn non_ascii_names_are_searchable() {
    let mut i = idx();
    let hits = i.search("360", None, 10);
    assert_eq!(i.entries()[hits[0].index].id, "Qihoo.360TotalSecurity");
    let hits = i.search("安全", None, 10);
    assert_eq!(hits.len(), 1);
}

#[test]
fn fuzzy_name_indices_point_at_matched_chars() {
    let mut i = idx();
    let hits = i.search("zel", None, 10);
    let h = &hits[0];
    assert_eq!(i.entries()[h.index].name, "Zeal");
    assert_eq!(h.name_indices, [0, 1, 3]);
}

#[test]
fn limit_is_respected() {
    let mut i = idx();
    assert_eq!(i.search("", None, 2).len(), 2);
}
```

#### Steps

- [ ] Step 1: os dois arquivos de teste → FAIL. Step 2: implemente `Catalog` (rusqlite; `replace_source` com `transaction()`, mapa `id→rowid` após inserir pacotes; id ausente → `DanglingMap` e a transação é descartada) e `SearchIndex` (`nucleo::Matcher` com `Config::DEFAULT`, `Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)`; `pattern.indices(Utf32Str::new(name, &mut buf), &mut matcher, &mut idx)` para o nome; `pattern.score(...)` para id/moniker; tags/commands com metade do score). Step 3: PASS, gate, commit.

API do `nucleo` verificada em 2026-10-07: `Pattern::parse("powt", CaseMatching::Ignore, Normalization::Smart).indices(Utf32Str::new("PowerToys", &mut buf), &mut matcher, &mut idx)` → `Some(105)`, `idx = [0, 1, 2, 5]`.

#### Checks beyond the steps

1. Isolamento por fonte: mutate o `DELETE` para não filtrar por `source` → `replace_source_only_touches_that_source` falha.
2. Rollback: mutate para fazer `commit` antes de validar os mapas (ou ignorar o id ausente) → `dangling_map_rolls_back_whole_import` falha.
3. Prioridade do match exato: remova o bônus de id/moniker exato → `exact_moniker_or_id_wins` falha.

**Commit:** `feat(catalog): sqlite catalog with per-source replace and fuzzy search`

---

### Task 6: Diretório de dados, cache e HTTP condicional

#### Where this fits

Base de I/O para as fontes: onde ficam os arquivos, o estado HTTP (ETag/Last-Modified/checagem), o armazenamento de blobs por hash, o lock de `update` e o cliente HTTP com GET condicional.

#### What binds this task

- Diretório padrão: `directories::BaseDirs::new()?.data_local_dir().join("wingetrs")` (= `%LOCALAPPDATA%\wingetrs`). Todos os componentes recebem `DataDir` (testes usam `tempfile`).
- `http.json` gravado de forma atômica (escreve `http.json.tmp` e renomeia). Arquivo ausente **ou corrompido** ⇒ estado vazio (corrompido gera `tracing::warn!`, nunca erro).
- `BlobStore::put_verified` só grava se o SHA-256 bater; nome do arquivo = hex minúsculo do SHA-256.
- Lock: `std::fs::File::try_lock()` (Rust ≥ 1.89; verificado). `acquire_timeout` tenta a cada 100 ms.
- HTTP: `User-Agent: wingetrs/<versão do crate>`; timeout de conexão 10 s, total 60 s (configuráveis para teste). 304 ⇒ `Fetch::NotModified`. Status ≠ 2xx/304 ⇒ `HttpError::Status`.
- Cabeçalhos guardados: `etag`, `last-modified`, `x-ms-meta-sourceversion`.

**Files:**

- Modify: `src/cache/mod.rs`, `src/http.rs`
- Test (new): `tests/cache.rs`, `tests/http.rs`

#### Interfaces

```rust
// src/cache/mod.rs
#[derive(Debug, Clone)]
pub struct DataDir { root: std::path::PathBuf }
impl DataDir {
    pub fn new(root: impl Into<std::path::PathBuf>) -> DataDir;
    pub fn default_location() -> Result<DataDir, CacheError>;
    pub fn root(&self) -> &std::path::Path;
    pub fn catalog_path(&self) -> std::path::PathBuf;     // root/catalog.db
    pub fn http_state_path(&self) -> std::path::PathBuf;  // root/http.json
    pub fn blobs_dir(&self) -> std::path::PathBuf;        // root/blobs
    pub fn lock_path(&self) -> std::path::PathBuf;        // root/update.lock
    pub fn log_dir(&self) -> std::path::PathBuf;          // root/log
    pub fn config_path(&self) -> std::path::PathBuf;      // root/config.toml
    pub fn tmp_dir(&self) -> std::path::PathBuf;          // root/tmp
    pub fn ensure(&self) -> Result<(), CacheError>;       // creates root, blobs, log, tmp
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HttpEntry {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub source_version: Option<String>,
    /// unix seconds of the last successful check (200 or 304)
    pub checked_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HttpState { pub entries: std::collections::BTreeMap<String, HttpEntry> }
impl HttpState {
    pub fn load(path: &std::path::Path) -> HttpState;
    pub fn save(&self, path: &std::path::Path) -> Result<(), CacheError>;
    pub fn get(&self, url: &str) -> Option<&HttpEntry>;
    pub fn set(&mut self, url: &str, entry: HttpEntry);
}

pub struct BlobStore { dir: std::path::PathBuf }
impl BlobStore {
    pub fn new(dir: std::path::PathBuf) -> BlobStore;
    pub fn get(&self, sha256_hex: &str) -> Option<Vec<u8>>;          // None if missing or content hash differs
    pub fn put_verified(&self, expected_sha256_hex: &str, bytes: &[u8]) -> Result<(), CacheError>;
}

pub struct UpdateLock { _file: std::fs::File }
impl UpdateLock {
    pub fn try_acquire(path: &std::path::Path) -> Result<Option<UpdateLock>, CacheError>;
    pub fn acquire_timeout(path: &std::path::Path, timeout: std::time::Duration) -> Result<Option<UpdateLock>, CacheError>;
}

pub fn now_unix() -> i64;
pub fn sha256_hex(bytes: &[u8]) -> String;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("diretório de dados indisponível")]
    NoDataDir,
    #[error("E/S em {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("hash não confere: esperado {expected}, obtido {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("estado HTTP inválido: {0}")]
    Json(String),
}

// src/http.rs
#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    NotModified,
    Modified { body: Vec<u8>, entry: crate::cache::HttpEntry },
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("sem conexão: {0}")]
    Network(String),
    #[error("tempo esgotado")]
    Timeout,
    #[error("HTTP {0}")]
    Status(u16),
}
impl HttpError { pub fn is_offline(&self) -> bool; } // Network | Timeout

#[derive(Clone)]
pub struct Http { client: reqwest::Client }
impl Http {
    pub fn new() -> Result<Http, HttpError>;
    pub fn with_timeouts(connect: std::time::Duration, total: std::time::Duration) -> Result<Http, HttpError>;
    /// Sends If-None-Match / If-Modified-Since from `prev`; on 200 returns the new entry with checked_at = now.
    pub async fn get_conditional(&self, url: &str, prev: Option<&crate::cache::HttpEntry>) -> Result<Fetch, HttpError>;
    pub async fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;
}
```

#### The body of the tests that are not obvious

`tests/http.rs` (new) (wiremock, runtime tokio):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::Duration;
use wingetrs::cache::HttpEntry;
use wingetrs::http::{Fetch, Http, HttpError};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn first_get_returns_body_and_validators() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/source2.msix"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec())
            .insert_header("ETag", "\"0x1\"")
            .insert_header("Last-Modified", "Wed, 07 Oct 2026 03:02:43 GMT")
            .insert_header("x-ms-meta-SourceVersion", "2026.1007.459.0"))
        .mount(&s).await;
    let h = Http::new().unwrap();
    match h.get_conditional(&format!("{}/source2.msix", s.uri()), None).await.unwrap() {
        Fetch::Modified { body, entry } => {
            assert_eq!(body, b"abc");
            assert_eq!(entry.etag.as_deref(), Some("\"0x1\""));
            assert_eq!(entry.last_modified.as_deref(), Some("Wed, 07 Oct 2026 03:02:43 GMT"));
            assert_eq!(entry.source_version.as_deref(), Some("2026.1007.459.0"));
            assert!(entry.checked_at > 1_700_000_000);
        }
        other => panic!("expected Modified, got {other:?}"),
    }
}

#[tokio::test]
async fn sends_validators_and_maps_304() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/x"))
        .and(header("If-None-Match", "\"0x1\""))
        // wiremock 0.6 splits header values on commas: match the two pieces exactly.
        .and(headers("If-Modified-Since", vec!["Wed", "07 Oct 2026 03:02:43 GMT"]))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&s).await;
    let prev = HttpEntry {
        etag: Some("\"0x1\"".into()),
        last_modified: Some("Wed, 07 Oct 2026 03:02:43 GMT".into()),
        source_version: None,
        checked_at: 0,
    };
    let r = Http::new().unwrap().get_conditional(&format!("{}/x", s.uri()), Some(&prev)).await.unwrap();
    assert_eq!(r, Fetch::NotModified);
}

#[tokio::test]
async fn non_success_status_is_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).mount(&s).await;
    let e = Http::new().unwrap().get(&format!("{}/nope", s.uri())).await.unwrap_err();
    assert!(matches!(e, HttpError::Status(404)), "{e:?}");
    assert!(!e.is_offline());
}

#[tokio::test]
async fn slow_server_times_out() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(1500)))
        .mount(&s).await;
    let h = Http::with_timeouts(Duration::from_millis(200), Duration::from_millis(300)).unwrap();
    let e = h.get(&s.uri()).await.unwrap_err();
    assert!(matches!(e, HttpError::Timeout), "{e:?}");
    assert!(e.is_offline());
}

#[tokio::test]
async fn unreachable_host_is_offline() {
    let h = Http::with_timeouts(Duration::from_millis(300), Duration::from_millis(500)).unwrap();
    let e = h.get("http://127.0.0.1:9/").await.unwrap_err();
    assert!(e.is_offline(), "{e:?}");
}
```

`tests/cache.rs` (new) (prosa → escreva você): `HttpState` salva e recarrega igual; arquivo com `{lixo` carrega vazio; `save` não deixa `.tmp` para trás; `BlobStore::put_verified` com hash errado devolve `HashMismatch` e não cria arquivo; `get` de blob adulterado (sobrescreva o arquivo com outros bytes) devolve `None`; `UpdateLock::try_acquire` duas vezes no mesmo caminho → o segundo devolve `Ok(None)` enquanto o primeiro vive, e `Ok(Some)` depois do `drop`; `acquire_timeout` com 300 ms enquanto outro segura → `Ok(None)` em ≥ 300 ms e < 2 s.

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente. Step 3: PASS, gate, commit.

#### Checks beyond the steps

1. Validadores enviados: mutate a inclusão do header `If-None-Match` (remova a linha) → `sends_validators_and_maps_304` falha.
2. Verificação de blob: mutate `put_verified` para não comparar → teste de hash errado falha.
3. Escrita atômica: mutate `save` para escrever direto em `http.json` sem `.tmp`/rename — se nenhum teste falhar, acrescente um teste que verifique que `http.json.tmp` não existe após `save` **e** que um `http.json` pré-existente válido continua legível se o processo de escrita falhar (simule com diretório somente leitura não é portável; aceite como regra pinada só a ausência do `.tmp`). Cole o resultado real, inclusive se a mutação sobreviver, e explique.

**Commit:** `feat(cache): data dir, http state, blob store, update lock and conditional http`

---

### Task 7: Fonte winget — índice, integridade e detalhes

#### Where this fits

Liga as Tasks 3–6: baixa `source2.msix` com GET condicional, valida o `AppxBlockMap`, lê o `index.db` v2 e produz `NewPackage` + `WingetMaps` para o catálogo; e resolve detalhes (`versionData` → manifesto) com cache de blobs.

#### What binds this task

- `index.db` está em `Public/index.db` dentro do ZIP; o BlockMap em `AppxBlockMap.xml`.
- BlockMap: cada `<Block Hash="base64(sha256)"/>` cobre 65536 bytes **descomprimidos** consecutivos de `Public\index.db`; `b4:FileHash` (se presente) = base64(sha256 do arquivo inteiro). Contagem de blocos deve ser `ceil(len/65536)`. Use `roxmltree`; o atributo do nome do arquivo usa barra invertida: `Public\index.db`.
- Só aceita `metadata.majorVersion == "2"`; outro valor ⇒ `IndexError::UnsupportedVersion(v)`.
- Leitura do índice v2 (tabelas reais, verificadas):
  - `packages(rowid, id, name, moniker, latest_version, arp_min_version, arp_max_version, hash BLOB)`;
  - `tags2(rowid, tag)` + `tags2_map(tag, package)`; `commands2(rowid, command)` + `commands2_map(command, package)`;
  - `pfns2(pfn, package)`, `productcodes2(productcode, package)`, `upgradecodes2(upgradecode, package)`, `norm_names2(norm_name, package)`, `norm_publishers2(norm_publisher, package)` — `package` = `packages.rowid`.
- O `index.db` vai para `DataDir::tmp_dir()/index-<pid>.db`, é lido e apagado.
- Detalhes: versionData em `blobs/` pela chave `"vd-" + id + "-" + hex8` não é hash de conteúdo — **decisão:** versionData não usa `BlobStore` (não há hash conhecido); é baixado sempre (≈5 KB) e só o manifesto vai para `BlobStore` com o `s256H`.
- Escolha da versão: `version = Some(v)` procura entrada com `v` igual (texto exato); ausente ⇒ `WingetError::VersionNotFound`. `None` ⇒ maior pela ordem de `core::version::Version`.

**Files:**

- Create (new): `src/sources/winget/index.rs`, `src/sources/winget/details.rs`
- Modify: `src/sources/winget/mod.rs`
- Test (new): `tests/winget_index.rs`, `tests/winget_details.rs`

#### Interfaces

```rust
// src/sources/winget/mod.rs
pub const SOURCE2_URL: &str = "https://cdn.winget.microsoft.com/cache/source2.msix";
pub const SOURCE_NAME: &str = "winget";

#[derive(Debug, thiserror::Error)]
pub enum WingetError {
    #[error(transparent)] Http(#[from] crate::http::HttpError),
    #[error(transparent)] Index(#[from] index::IndexError),
    #[error(transparent)] VersionData(#[from] version_data::VersionDataError),
    #[error(transparent)] Manifest(#[from] manifest::ManifestError),
    #[error(transparent)] Cache(#[from] crate::cache::CacheError),
    #[error("versão {0} não encontrada")] VersionNotFound(String),
    #[error("manifesto com hash divergente")] ManifestIntegrity,
}

pub enum WingetFetch {
    NotModified,
    Modified { import: index::WingetImport, entry: crate::cache::HttpEntry },
}

/// `base_url` is SOURCE2_URL in production, the mock server in tests.
pub async fn fetch_index(http: &crate::http::Http, data: &crate::cache::DataDir, url: &str, prev: Option<&crate::cache::HttpEntry>) -> Result<WingetFetch, WingetError>;

// src/sources/winget/index.rs
#[derive(Debug, Clone, PartialEq)]
pub struct WingetImport { pub packages: Vec<crate::catalog::NewPackage>, pub maps: crate::catalog::WingetMaps }

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("pacote MSIX inválido: {0}")] Zip(String),
    #[error("arquivo ausente no MSIX: {0}")] MissingEntry(&'static str),
    #[error("AppxBlockMap inválido: {0}")] BlockMap(String),
    #[error("integridade: bloco {block} do índice não confere")] Integrity { block: usize },
    #[error("integridade: hash do arquivo do índice não confere")] FileHash,
    #[error("formato de índice v{0} não suportado")] UnsupportedVersion(String),
    #[error("índice ilegível: {0}")] Sqlite(String),
    #[error("E/S: {0}")] Io(String),
}

/// Returns the validated bytes of Public/index.db.
pub fn validate_msix(msix: &[u8]) -> Result<Vec<u8>, IndexError>;
pub fn read_index(db_path: &std::path::Path) -> Result<WingetImport, IndexError>;

// src/sources/winget/details.rs
pub async fn details(
    http: &crate::http::Http,
    blobs: &crate::cache::BlobStore,
    cdn_base: &str,           // CDN_BASE in production
    id: &str,
    hash: &[u8],
    version: Option<&str>,
) -> Result<crate::core::PackageDetails, WingetError>;
```

`NewPackage` do winget: `id`, `name`, `moniker`, `version = latest_version`, `publisher = None`, `description = None`, `tags`, `commands`, `arp_min_version`, `arp_max_version`, `winget_hash = Some(hash)`, `raw = None`.

#### Fixture facts (medidos em `tests/fixtures/winget/index-mini.db`)

- 20 pacotes; productcodes2 = 6025, upgradecodes2 = 13, pfns2 = 5, norm_names2 = 29, norm_publishers2 = 22, tags2_map = 289, commands2_map = 12.
- `Python.Python.3.13`: version `3.13.15`, arp_min = arp_max = `3.13.15150.0`.
- `Microsoft.PowerToys`: moniker `powertoys`, hash começa com `d193b67a`.
- `7zip.7zip` norm_names = {`7zip`, `7zip(X64)`}; norm_publishers = {`igorpavlov`}.
- `source2-mini.msix` tem 5 blocos e `b4:FileHash`; `source2-mini-corrupt.msix` tem um byte do meio do `index.db` invertido com o BlockMap original.

#### The body of the tests that are not obvious

`tests/winget_index.rs` (new):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::sources::winget::index::{read_index, validate_msix, IndexError};
use wingetrs::sources::winget::{fetch_index, WingetFetch};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MSIX: &[u8] = include_bytes!("fixtures/winget/source2-mini.msix");
const CORRUPT: &[u8] = include_bytes!("fixtures/winget/source2-mini-corrupt.msix");
const DB: &[u8] = include_bytes!("fixtures/winget/index-mini.db");

#[test]
fn valid_msix_yields_exact_index_bytes() {
    assert_eq!(validate_msix(MSIX).unwrap(), DB);
}

#[test]
fn corrupt_block_is_rejected() {
    match validate_msix(CORRUPT) {
        Err(IndexError::Integrity { block }) => assert_eq!(block, (DB.len() / 2) / 65536),
        other => panic!("expected Integrity, got {other:?}"),
    }
}

#[test]
fn not_a_zip_is_an_error() {
    assert!(matches!(validate_msix(b"hello"), Err(IndexError::Zip(_))));
}

#[test]
fn reads_packages_and_all_maps() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let imp = read_index(&p).unwrap();
    assert_eq!(imp.packages.len(), 20);
    assert_eq!(imp.maps.productcodes.len(), 6025);
    assert_eq!(imp.maps.upgradecodes.len(), 13);
    assert_eq!(imp.maps.pfns.len(), 5);
    assert_eq!(imp.maps.norm_names.len(), 29);
    assert_eq!(imp.maps.norm_publishers.len(), 22);
    let tag_total: usize = imp.packages.iter().map(|p| p.tags.len()).sum();
    let cmd_total: usize = imp.packages.iter().map(|p| p.commands.len()).sum();
    assert_eq!((tag_total, cmd_total), (289, 12));
    let py = imp.packages.iter().find(|p| p.id == "Python.Python.3.13").unwrap();
    assert_eq!(py.version, "3.13.15");
    assert_eq!(py.arp_max_version.as_deref(), Some("3.13.15150.0"));
    let pt = imp.packages.iter().find(|p| p.id == "Microsoft.PowerToys").unwrap();
    assert_eq!(hex::encode(&pt.winget_hash.as_ref().unwrap()[..4]), "d193b67a");
    let mut seven: Vec<&str> = imp.maps.norm_names.iter().filter(|(_, id)| id == "7zip.7zip").map(|(k, _)| k.as_str()).collect();
    seven.sort();
    assert_eq!(seven, ["7zip", "7zip(X64)"]);
}

#[test]
fn unsupported_major_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute("update metadata set value='3' where name='majorVersion'", []).unwrap();
    drop(c);
    assert!(matches!(read_index(&p), Err(IndexError::UnsupportedVersion(v)) if v == "3"));
}

#[tokio::test]
async fn fetch_index_end_to_end_and_304() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/source2.msix"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(MSIX.to_vec()).insert_header("ETag", "\"e1\""))
        .up_to_n_times(1).mount(&s).await;
    Mock::given(method("GET")).and(path("/source2.msix"))
        .respond_with(ResponseTemplate::new(304)).mount(&s).await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let url = format!("{}/source2.msix", s.uri());
    let http = Http::new().unwrap();
    let entry = match fetch_index(&http, &data, &url, None).await.unwrap() {
        WingetFetch::Modified { import, entry } => {
            assert_eq!(import.packages.len(), 20);
            entry
        }
        WingetFetch::NotModified => panic!("expected Modified"),
    };
    assert!(matches!(fetch_index(&http, &data, &url, Some(&entry)).await.unwrap(), WingetFetch::NotModified));
    let leftovers = std::fs::read_dir(data.tmp_dir()).unwrap().count();
    assert_eq!(leftovers, 0, "temporary index.db must be removed");
}

#[tokio::test]
async fn fetch_index_corrupt_download_is_integrity_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_bytes(CORRUPT.to_vec())).mount(&s).await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let r = fetch_index(&Http::new().unwrap(), &data, &s.uri(), None).await;
    assert!(r.is_err());
}
```

`tests/winget_details.rs` (new):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::BlobStore;
use wingetrs::http::Http;
use wingetrs::sources::winget::details::details;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VD: &[u8] = include_bytes!("fixtures/winget/powertoys.versionData.mszyml");
const MANIFEST: &[u8] = include_bytes!("fixtures/winget/powertoys-0.101.2362.0.merged.yaml");
const HASH_HEX: &str = "d193b67ac71ce5d456c10f6731447abb6df68ae12ef8691c92af8ed82659e720";

async fn server(manifest: &[u8]) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/packages/Microsoft.PowerToys/d193b67a/versionData.mszyml"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(VD.to_vec())).mount(&s).await;
    Mock::given(method("GET")).and(path("/manifests/m/Microsoft/PowerToys/0.101.2362.0/509b"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(manifest.to_vec())).mount(&s).await;
    s
}

#[tokio::test]
async fn latest_details_with_all_versions() {
    let s = server(MANIFEST).await;
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::new(dir.path().to_path_buf());
    let d = details(&Http::new().unwrap(), &blobs, &format!("{}/", s.uri()), "Microsoft.PowerToys", &hex::decode(HASH_HEX).unwrap(), None).await.unwrap();
    assert_eq!(d.version, "0.101.2362.0");
    assert_eq!(d.available_versions.len(), 115);
    assert_eq!(d.available_versions[0], "0.101.2362.0");
    assert_eq!(d.installers.len(), 4);
    assert!(blobs.get("345056f4d70d74ef350a0e00a7c5a710657271a882be4fa0bc263af6bb088245").is_some(), "manifest cached by sha256");
}

#[tokio::test]
async fn tampered_manifest_is_rejected() {
    let mut bad = MANIFEST.to_vec();
    bad[10] ^= 1;
    let s = server(&bad).await;
    let dir = tempfile::tempdir().unwrap();
    let r = details(&Http::new().unwrap(), &BlobStore::new(dir.path().to_path_buf()), &format!("{}/", s.uri()), "Microsoft.PowerToys", &hex::decode(HASH_HEX).unwrap(), None).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn unknown_version_is_reported() {
    let s = server(MANIFEST).await;
    let dir = tempfile::tempdir().unwrap();
    let r = details(&Http::new().unwrap(), &BlobStore::new(dir.path().to_path_buf()), &format!("{}/", s.uri()), "Microsoft.PowerToys", &hex::decode(HASH_HEX).unwrap(), Some("9.9.9")).await;
    assert!(r.unwrap_err().to_string().contains("9.9.9"));
}
```

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente `validate_msix`, `read_index`, `fetch_index`, `details`. Step 3: PASS, gate, commit.

#### Checks beyond the steps

1. Validação por bloco: mutate a comparação de hash de bloco para sempre aceitar → `corrupt_block_is_rejected` falha.
2. Rejeição de versão: mutate a checagem `== "2"` → `unsupported_major_version_is_refused` falha.
3. Verificação do manifesto: mutate a comparação com `s256H` → `tampered_manifest_is_rejected` falha.

**Commit:** `feat(winget): index acquisition with blockmap integrity and package details`

---

### Task 8: Fonte scoop — tarball, manifestos e detalhes

#### Where this fits

Segunda fonte. Baixa o tarball do bucket no codeload (GET condicional por ETag), extrai em memória `*/bucket/*.json` e produz `NewPackage`s; detalhes vêm do JSON guardado em `raw`.

#### What binds this task

- URLs conhecidas: `main` → `https://codeload.github.com/ScoopInstaller/Main/tar.gz/refs/heads/master`; `extras` → `.../ScoopInstaller/Extras/...`; `versions` → `.../ScoopInstaller/Versions/...`. Outro nome ⇒ `ScoopError::UnknownBucket`.
- Nome da fonte no catálogo: `scoop:<bucket>`.
- Só arquivos cujo caminho tem a forma `<qualquer>/bucket/<app>.json` (um nível). JSON inválido ⇒ pula e registra o nome em `skipped`; BOM UTF-8 inicial é tolerado.
- `NewPackage`: `id = name = <app>`; `version`; `description`; `publisher = None`; `tags = []`; `commands` derivados de `bin` (abaixo); `raw = Some(<json original sem BOM>)`.
- `bin` pode ser string, lista de strings, ou lista contendo `[caminho, alias, ...]`. Comando = alias se houver, senão o nome do arquivo sem extensão; tudo em minúsculas, sem duplicatas, na ordem de aparição. `bin` também pode estar dentro de `architecture.<arch>`; considere raiz e todas as arquiteturas.
- Detalhes: `license` string ou objeto `{identifier}`; `homepage`; installers de `architecture` (`64bit`→`x64`, `32bit`→`x86`, `arm64`→`arm64`), `url` string ou lista (primeiro), `installer_type = "scoop"`, `scope = Some("user")`; sem `architecture` ⇒ um installer `neutral` com o `url` da raiz. `available_versions = [version]`.

#### Fixture facts (`tests/fixtures/scoop/Main-mini.tar.gz`)

Prefixo `Main-master/`; 8 manifestos válidos (`7zip` 26.04, `aria2` 1.37.0-1, `ffmpeg` 9.0.2, `gh` 2.102.0, `git` 2.56.0.2, `nodejs-lts` 24.21.0, `python` 3.14.8, `ripgrep` 15.2.0) + `broken.json` (JSON truncado) + `README.md`. `7zip.bin = ["7z.exe","7zG.exe","7zFM.exe"]`; `python.bin` começa com `["python.exe","python3"]`; `7zip.license = {"identifier": "BSD-2-Clause, BSD-3-Clause, LGPL-2.1-or-later", ...}` (comece a asserção por `starts_with("BSD-2-Clause")`); `7zip.architecture` tem `64bit`, `32bit`, `arm64`.

**Files:**

- Create (new): `src/sources/scoop/manifest.rs`, `src/sources/scoop/bucket.rs`
- Modify: `src/sources/scoop/mod.rs`
- Test (new): `tests/scoop.rs`

#### Interfaces

```rust
// src/sources/scoop/mod.rs
pub mod bucket;
pub mod manifest;

#[derive(Debug, thiserror::Error)]
pub enum ScoopError {
    #[error("bucket desconhecido: {0}")] UnknownBucket(String),
    #[error("tarball inválido: {0}")] Tar(String),
    #[error("manifesto scoop inválido: {0}")] Json(String),
    #[error(transparent)] Http(#[from] crate::http::HttpError),
}

pub fn source_name(bucket: &str) -> String; // "scoop:<bucket>"

// src/sources/scoop/manifest.rs
pub fn parse(app: &str, json: &[u8]) -> Result<crate::catalog::NewPackage, ScoopError>;
pub fn details_from_raw(app: &str, raw: &str) -> Result<crate::core::PackageDetails, ScoopError>;

// src/sources/scoop/bucket.rs
#[derive(Debug, Clone, PartialEq)]
pub struct BucketImport { pub packages: Vec<crate::catalog::NewPackage>, pub skipped: Vec<String> }
pub fn bucket_url(bucket: &str) -> Result<&'static str, ScoopError>;
pub fn read_tarball(tar_gz: &[u8]) -> Result<BucketImport, ScoopError>;
pub enum ScoopFetch { NotModified, Modified { import: BucketImport, entry: crate::cache::HttpEntry } }
pub async fn fetch_bucket(http: &crate::http::Http, url: &str, prev: Option<&crate::cache::HttpEntry>) -> Result<ScoopFetch, ScoopError>;
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::sources::scoop::bucket::{bucket_url, read_tarball};
use wingetrs::sources::scoop::manifest::{details_from_raw, parse};

const TGZ: &[u8] = include_bytes!("fixtures/scoop/Main-mini.tar.gz");

#[test]
fn tarball_imports_valid_and_skips_broken() {
    let imp = read_tarball(TGZ).unwrap();
    let mut ids: Vec<&str> = imp.packages.iter().map(|p| p.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["7zip", "aria2", "ffmpeg", "gh", "git", "nodejs-lts", "python", "ripgrep"]);
    assert_eq!(imp.skipped, ["broken.json"]);
}

#[test]
fn commands_from_bin_variants() {
    let imp = read_tarball(TGZ).unwrap();
    let get = |id: &str| imp.packages.iter().find(|p| p.id == id).unwrap().clone();
    assert_eq!(get("7zip").commands, ["7z", "7zg", "7zfm"]);
    assert_eq!(get("ripgrep").commands, ["rg"]);
    assert_eq!(get("python").commands[0], "python3");
    assert_eq!(get("7zip").version, "26.04");
}

#[test]
fn bom_is_tolerated() {
    let mut j = vec![0xEF, 0xBB, 0xBF];
    j.extend_from_slice(br#"{"version":"1.0","description":"d","url":"https://x/a.zip","hash":"aa","bin":"a.exe"}"#);
    let p = parse("a", &j).unwrap();
    assert_eq!(p.version, "1.0");
    assert!(!p.raw.as_ref().unwrap().starts_with('\u{feff}'));
}

#[test]
fn details_map_architectures_and_license_object() {
    let imp = read_tarball(TGZ).unwrap();
    let raw = imp.packages.iter().find(|p| p.id == "7zip").unwrap().raw.clone().unwrap();
    let d = details_from_raw("7zip", &raw).unwrap();
    assert!(d.license.as_deref().unwrap().starts_with("BSD-2-Clause"));
    let archs: Vec<&str> = d.installers.iter().map(|i| i.architecture.as_str()).collect();
    assert_eq!(archs, ["x64", "x86", "arm64"]);
    assert!(d.installers.iter().all(|i| i.installer_type == "scoop" && i.scope.as_deref() == Some("user")));
    assert_eq!(d.homepage.as_deref(), Some("https://www.7-zip.org"));
}

#[test]
fn neutral_installer_without_architecture() {
    let raw = r#"{"version":"2","url":["https://x/one.zip","https://x/two.zip"],"hash":["a","b"]}"#;
    let d = details_from_raw("n", raw).unwrap();
    assert_eq!(d.installers.len(), 1);
    assert_eq!(d.installers[0].architecture, "neutral");
    assert_eq!(d.installers[0].url, "https://x/one.zip");
}

#[test]
fn bucket_urls() {
    assert_eq!(bucket_url("main").unwrap(), "https://codeload.github.com/ScoopInstaller/Main/tar.gz/refs/heads/master");
    assert!(bucket_url("choco").is_err());
}
```

Acrescente (escreva você) um teste `fetch_bucket` com `wiremock`: 200 + `ETag` ⇒ `Modified` com 8 pacotes; segunda chamada com o `entry` ⇒ `NotModified` (mock responde 304 quando `If-None-Match` presente).

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente (`flate2::read::GzDecoder` + `tar::Archive`; `serde_json::Value` para o manifesto). Step 3: PASS, gate, commit.

#### Checks beyond the steps

1. Pular inválido: mutate o tratamento de erro de JSON para propagar (`?`) → `tarball_imports_valid_and_skips_broken` falha.
2. Alias de `bin`: mutate para usar sempre o nome do arquivo → `commands_from_bin_variants` falha.

**Commit:** `feat(scoop): bucket tarball import and manifest details`

---

### Task 9: Orquestração do update e comandos CLI

#### Where this fits

Junta tudo em comandos usáveis: `update` (com lock, por fonte em paralelo, falha isolada), `search`, `show` e a configuração (`config.toml`). Inicializa o log em arquivo.

#### What binds this task

- `Config` (TOML em `DataDir::config_path()`; ausente ⇒ padrão; inválido ⇒ erro claro): `scoop_buckets = ["main","extras"]`, `glyphs = "nerd"`, `theme = "dark"`.
- `update`:
  1. `UpdateLock::acquire_timeout(lock, 30 s)`; `None` ⇒ erro `"outra atualização em andamento"`.
  2. Para cada fonte (winget + cada bucket), em paralelo (`tokio::join!`/`futures_util::future::join_all`): se `force == false` e `checked_at` da URL tem < 900 s ⇒ `Outcome::Fresh`, sem rede. Senão GET condicional.
  3. `Modified` ⇒ `Catalog::replace_source` (fonte `winget` ou `scoop:<bucket>`, `source_version` = `x-ms-meta-SourceVersion` ou ETag ou `""`), depois atualiza `HttpState`. `NotModified` ⇒ só atualiza `checked_at`. Erro ⇒ `Outcome::Failed(msg)`, dados antigos intactos.
  4. Salva `http.json` uma vez no fim.
- URLs injetáveis (`Endpoints`) para teste com `wiremock`.
- `search`: carrega `Catalog` → `SearchIndex` → imprime tabela `Nome  Id  Versão  Fonte` (colunas alinhadas, sem cores) ou JSON (`Vec<PackageEntry>`).
- `show`: `find_by_id` (com `--source` filtra); 0 ⇒ erro `"pacote não encontrado: <id>"`; >1 sem `--source` ⇒ erro listando as fontes; winget ⇒ `sources::winget::details::details`; scoop ⇒ `details_from_raw`.
- Sem catálogo (`Catalog::is_empty`) em `search`/`show`/`list` ⇒ roda `update` bloqueante primeiro; se falhar por rede ⇒ erro `"sem catálogo local e sem rede"` (Review Focus 1).
- Catálogo existente mas velho (> 900 s): CLI dispara `current_exe update --background` desacoplado (`std::process::Command` com `creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)` em `#[cfg(windows)]`, `0x00000008 | 0x08000000`) e segue com o cache. Decisão pura `needs_refresh(&HttpState, &[&str], now) -> bool` testada; o spawn não é testado.
- Log: `tracing_subscriber` + `tracing_appender::rolling::daily(log_dir, "wingetrs.log")`; `-v` acrescenta stderr.

**Files:**

- Modify: `src/update.rs`, `src/cli/mod.rs`
- Create (new): `src/config.rs` (e `pub mod config;` em `lib.rs`), `src/cli/output.rs`
- Test (new): `tests/update.rs`, `tests/config.rs`, `tests/cli_offline.rs`

#### Interfaces

```rust
// src/config.rs
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Config { pub scoop_buckets: Vec<String>, pub glyphs: String, pub theme: String }
impl Default for Config { /* ["main","extras"], "nerd", "dark" */ }
impl Config { pub fn load(path: &std::path::Path) -> Result<Config, ConfigError>; }

// src/update.rs
#[derive(Debug, Clone)]
pub struct Endpoints { pub winget_index: String, pub scoop: Vec<(String /*bucket*/, String /*url*/)> }
impl Endpoints { pub fn production(cfg: &crate::config::Config) -> Result<Endpoints, UpdateError>; }

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome { Fresh, Unchanged, Updated { packages: usize, skipped: usize }, Failed(String) }
#[derive(Debug, Clone, PartialEq)]
pub struct SourceReport { pub source: String, pub outcome: Outcome }

pub const MAX_AGE_SECS: i64 = 900;
pub fn needs_refresh(state: &crate::cache::HttpState, urls: &[&str], now: i64) -> bool;
pub async fn run_update(
    data: &crate::cache::DataDir,
    endpoints: &Endpoints,
    http: &crate::http::Http,
    force: bool,
) -> Result<Vec<SourceReport>, UpdateError>;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("outra atualização em andamento")] Busy,
    #[error(transparent)] Cache(#[from] crate::cache::CacheError),
    #[error(transparent)] Catalog(#[from] crate::catalog::CatalogError),
    #[error(transparent)] Scoop(#[from] crate::sources::scoop::ScoopError),
}
```

#### The body of the tests that are not obvious

`tests/update.rs` (new):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::{DataDir, HttpEntry, HttpState};
use wingetrs::catalog::Catalog;
use wingetrs::http::Http;
use wingetrs::update::{needs_refresh, run_update, Endpoints, Outcome};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MSIX: &[u8] = include_bytes!("fixtures/winget/source2-mini.msix");
const TGZ: &[u8] = include_bytes!("fixtures/scoop/Main-mini.tar.gz");

async fn setup() -> (MockServer, Endpoints) {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/source2.msix"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(MSIX.to_vec()).insert_header("ETag", "\"w1\"").insert_header("x-ms-meta-SourceVersion", "2026.1007.459.0"))
        .mount(&s).await;
    Mock::given(method("GET")).and(path("/main.tgz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(TGZ.to_vec()).insert_header("ETag", "\"s1\""))
        .mount(&s).await;
    Mock::given(method("GET")).and(path("/extras.tgz"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s).await;
    let ep = Endpoints {
        winget_index: format!("{}/source2.msix", s.uri()),
        scoop: vec![("main".into(), format!("{}/main.tgz", s.uri())), ("extras".into(), format!("{}/extras.tgz", s.uri()))],
    };
    (s, ep)
}

#[tokio::test]
async fn update_imports_sources_and_isolates_failure() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let rep = run_update(&data, &ep, &Http::new().unwrap(), true).await.unwrap();
    let get = |s: &str| rep.iter().find(|r| r.source == s).unwrap().outcome.clone();
    assert_eq!(get("winget"), Outcome::Updated { packages: 20, skipped: 0 });
    assert_eq!(get("scoop:main"), Outcome::Updated { packages: 8, skipped: 1 });
    assert!(matches!(get("scoop:extras"), Outcome::Failed(_)));
    let cat = Catalog::open(&data.catalog_path()).unwrap();
    assert_eq!(cat.load_entries().unwrap().len(), 28);
    let meta = cat.sources().unwrap();
    assert_eq!(meta.iter().find(|m| m.source == "winget").unwrap().source_version, "2026.1007.459.0");
    let st = HttpState::load(&data.http_state_path());
    assert_eq!(st.get(&ep.winget_index).unwrap().etag.as_deref(), Some("\"w1\""));
}

#[tokio::test]
async fn second_update_without_force_is_fresh_and_offline_safe() {
    let (s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    run_update(&data, &ep, &Http::new().unwrap(), true).await.unwrap();
    drop(s); // server gone: any network call would fail
    let rep = run_update(&data, &ep, &Http::new().unwrap(), false).await.unwrap();
    assert_eq!(rep.iter().find(|r| r.source == "winget").unwrap().outcome, Outcome::Fresh);
    assert_eq!(Catalog::open(&data.catalog_path()).unwrap().load_entries().unwrap().len(), 28);
}

#[tokio::test]
async fn concurrent_update_is_busy() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let _held = wingetrs::cache::UpdateLock::try_acquire(&data.lock_path()).unwrap().unwrap();
    // run_update waits up to 30 s; the test must not take that long, so use the
    // test-only knob described in the brief: WINGETRS_LOCK_TIMEOUT_MS
    unsafe { std::env::set_var("WINGETRS_LOCK_TIMEOUT_MS", "200") };
    let e = run_update(&data, &ep, &Http::new().unwrap(), true).await.unwrap_err();
    assert_eq!(e.to_string(), "outra atualização em andamento");
}

#[test]
fn needs_refresh_rule() {
    let mut st = HttpState::default();
    assert!(needs_refresh(&st, &["u"], 10_000), "never checked");
    st.set("u", HttpEntry { checked_at: 10_000 - 899, ..Default::default() });
    assert!(!needs_refresh(&st, &["u"], 10_000));
    st.set("u", HttpEntry { checked_at: 10_000 - 901, ..Default::default() });
    assert!(needs_refresh(&st, &["u"], 10_000));
}
```

O timeout do lock lê `WINGETRS_LOCK_TIMEOUT_MS` (padrão 30000) — **decisão:** é o único knob de teste por variável de ambiente; documente com um comentário de uma linha.

`tests/cli_offline.rs` (new) (Review Focus 1): rode o binário com `std::process::Command::new(env!("CARGO_BIN_EXE_wingetrs"))`, `LOCALAPPDATA` apontando para um `tempdir` vazio e as variáveis `WINGETRS_WINGET_INDEX_URL=http://127.0.0.1:9/x` e `WINGETRS_SCOOP_BASE_URL=http://127.0.0.1:9` (o `Endpoints::production` respeita essas duas variáveis quando presentes — **decisão** para permitir o teste); `search foo` deve sair com código ≠ 0, stderr contendo `sem catálogo local e sem rede`, e sem `panicked` no stderr.

`tests/config.rs` (new) (escreva você): ausente ⇒ padrão; `scoop_buckets = ["main"]` ⇒ só main; TOML inválido ⇒ `Err` com o caminho na mensagem.

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente `config`, `update`, `cli::run` para `Update`/`Search`/`Show`, saída em `cli/output.rs`, init de log. Step 3: PASS, gate. Step 4: teste manual real (rede): `cargo run --quiet -- update --force` e `cargo run --quiet -- search powertoys -n 5`; cole as saídas no relatório. Step 5: commit.

#### Checks beyond the steps

1. Isolamento de falha: mutate o tratamento de erro por fonte para abortar o update inteiro (`?`) → `update_imports_sources_and_isolates_failure` falha.
2. Regra de frescor: mutate `MAX_AGE_SECS` de 900 para 1000 → `needs_refresh_rule` falha (perturbação pequena, não valor degenerado).
3. Lock: mutate para ignorar o `None` do lock → `concurrent_update_is_busy` falha.

**Commit:** `feat(update): parallel source refresh, config and search/show commands`

---

### Task 10: Porte do NameNormalizer

#### Where this fits

O `list` correlaciona programas instalados que não têm ProductCode/PFN conhecidos pelo nome e publisher **normalizados exatamente como o winget normalizou ao gerar o índice** (`norm_names2`/`norm_publishers2`). Qualquer divergência vira "não correlacionado". É a peça mais delicada do projeto.

#### What binds this task

- Fonte: `docs/reference/winget-cli/AppInstallerCommonCore_NameNormalization.cpp` (528 linhas) e o header `..._Public_winget_NameNormalization.h`. Porte **somente** `NormalizationVersion::Initial` (não o `InitialPreserveWhiteSpace`).
- Regex: `fancy-regex` 0.19 (os 32 padrões do arquivo compilam todos com `RegexBuilder::new(p).case_insensitive(true)` — verificado em 2026-10-07). Compile uma vez (`NameNormalizer::new`).
- Case folding: o índice aplica `Utility::FoldCase` (ICU `u_strFoldCase`) **antes** de normalizar: `Normalize(FoldCase(name), FoldCase(publisher))` (ver `Interface_2_0.cpp`, função `UpdateNormalizedNameAndPublisher`). Use `str::to_lowercase()` como aproximação e meça contra o corpus.
- Chave de nome com arquitetura: `GetNormalizedName(NormalizationField::Architecture)` = nome + sufixo de arquitetura (ex.: `mozillafirefox(X64)`). Sem arquitetura detectada ⇒ só o nome.
- Oráculo: `docs/reference/winget-cli/AppInstallerCLITests_TestData_InputNames.txt`, `..._InputPublishers.txt`, `..._NormalizationInitialIds.txt` (linhas pareadas; esperado = `Publisher + "." + Name` do `Normalize(name, publisher)` **sem** fold).

#### The decision this task must get right

Meta: 100% do corpus golden. Se não atingir, **não afrouxe o teste**: o teste afirma o teto medido (`assert!(passed >= MEASURED)`), lista as 20 primeiras divergências no relatório com causa provável (ex.: diferença ICU × Unicode do Rust em `\p{L}`), e registra follow-up. Nunca `#[ignore]`.

**Files:**

- Create (new): `src/installed/normalize.rs`
- Modify: `src/installed/mod.rs` (`pub mod normalize;`)
- Test (new): `tests/normalize.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture { Unknown, X86, X64, Arm, Arm64 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedName { pub name: String, pub publisher: String, pub architecture: Architecture, pub locale: String }
impl NormalizedName {
    /// GetNormalizedName(NormalizationField::Architecture): name + "(X64)" etc. when known.
    pub fn name_with_arch(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexKeys { pub name: String, pub name_with_arch: Option<String>, pub publisher: String }

pub struct NameNormalizer { /* compiled regexes */ }
impl NameNormalizer {
    pub fn new() -> NameNormalizer;
    pub fn normalize(&self, name: &str, publisher: &str) -> NormalizedName;
    /// Keys as stored in the index: normalize(fold(name), fold(publisher)).
    pub fn index_keys(&self, display_name: &str, publisher: &str) -> IndexKeys;
}
```

`NameNormalizer::new` não pode falhar em runtime: os padrões são constantes verificadas pelo teste `all_patterns_compile`; dentro de `new`, padrão inválido é bug de programação — use um `match` que, no erro, registra `tracing::error!` e usa um regex que nunca casa (`(?!)`), em vez de `expect` (proibido pelo clippy).

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::installed::normalize::{Architecture, NameNormalizer};

const NAMES: &str = include_str!("../docs/reference/winget-cli/AppInstallerCLITests_TestData_InputNames.txt");
const PUBS: &str = include_str!("../docs/reference/winget-cli/AppInstallerCLITests_TestData_InputPublishers.txt");
const IDS: &str = include_str!("../docs/reference/winget-cli/AppInstallerCLITests_TestData_NormalizationInitialIds.txt");

/// Measured pass count of the golden corpus. Raise it, never lower it.
const MEASURED_GOLDEN_PASS: usize = 0; // set to the measured value in Step 3

#[test]
fn golden_corpus_matches_winget() {
    let n = NameNormalizer::new();
    let (names, pubs, ids): (Vec<&str>, Vec<&str>, Vec<&str>) =
        (NAMES.lines().collect(), PUBS.lines().collect(), IDS.lines().collect());
    assert_eq!(names.len(), pubs.len());
    assert_eq!(names.len(), ids.len());
    assert!(names.len() > 900, "corpus must be the full file, got {}", names.len());
    let mut passed = 0;
    let mut first_fail = Vec::new();
    for ((name, publisher), expected) in names.iter().zip(&pubs).zip(&ids) {
        let r = n.normalize(name, publisher);
        let got = format!("{}.{}", r.publisher, r.name);
        if got == *expected {
            passed += 1;
        } else if first_fail.len() < 20 {
            first_fail.push(format!("[{name}] [{publisher}] => {got} (esperado {expected})"));
        }
    }
    eprintln!("golden: {passed}/{} ; primeiras falhas:\n{}", names.len(), first_fail.join("\n"));
    assert!(passed >= MEASURED_GOLDEN_PASS, "regressão: {passed} < {MEASURED_GOLDEN_PASS}");
    assert!(MEASURED_GOLDEN_PASS > 0, "MEASURED_GOLDEN_PASS must be set to the measured value");
}

#[test]
fn architecture_detection() {
    let n = NameNormalizer::new();
    assert_eq!(n.normalize("Name", "").architecture, Architecture::Unknown);
    assert_eq!(n.normalize("Name x86", "").architecture, Architecture::X86);
    assert_eq!(n.normalize("Name x86_64", "").architecture, Architecture::X64);
    assert_eq!(n.normalize("Name (64 bit)", "").architecture, Architecture::X64);
    assert_eq!(n.normalize("Name 32/64 bit", "").architecture, Architecture::Unknown);
    assert_eq!(n.normalize("Fox86", "").architecture, Architecture::Unknown);
}

#[test]
fn locale_detection() {
    let n = NameNormalizer::new();
    assert_eq!(n.normalize("Name", "").locale, "");
    assert_eq!(n.normalize("Name en-US", "").locale, "en-us");
    assert_eq!(n.normalize("Name (es-mx)", "").locale, "es-mx");
    assert_eq!(n.normalize("Names-mx", "").locale, "");
}

#[test]
fn kb_numbers_are_kept() {
    assert_eq!(NameNormalizer::new().normalize("Fix for (KB42)", "").name, "FixforKB42");
}

#[test]
fn name_with_arch_suffix() {
    let n = NameNormalizer::new();
    let r = n.normalize("Name(X64)", "");
    assert_eq!(r.name, "Name");
    assert_eq!(r.name_with_arch(), "Name(X64)");
}

#[test]
fn index_keys_match_real_index_rows() {
    // Values copied from tests/fixtures/winget/index-mini.db (norm_names2/norm_publishers2).
    let n = NameNormalizer::new();
    let k = n.index_keys("Mozilla Firefox (x64 en-US)", "Mozilla");
    assert_eq!(k.name_with_arch.as_deref(), Some("mozillafirefox(X64)"));
    assert_eq!(k.name, "mozillafirefox");
    assert_eq!(k.publisher, "mozilla");
    let k = n.index_keys("7-Zip 24.08 (x64)", "Igor Pavlov");
    assert_eq!(k.name_with_arch.as_deref(), Some("7zip(X64)"));
    assert_eq!(k.publisher, "igorpavlov");
    let k = n.index_keys("Notepad++ (64-bit x64)", "Notepad++ Team");
    assert_eq!(k.name, "notepad");
    assert_eq!(k.publisher, "notepadteam");
}
```

O teste `index_keys_match_real_index_rows` usa nomes de ARP **plausíveis** (não medidos nesta máquina). Se o porte fiel (100% golden) divergir de algum deles, o porte vence: corrija o nome de entrada no teste para um que o golden confirme e registre no relatório.

#### Steps

- [ ] Step 1: teste → FAIL. Step 2: porte `NameNormalizer` (Initial) com comentários `// port: NameNormalization.cpp <função>`. Step 3: rode o golden, ajuste o porte até o máximo; fixe `MEASURED_GOLDEN_PASS` no valor medido. Step 4: gate verde, commit.

#### Checks beyond the steps

1. Mutação no remoção de versão (ex.: tirar `VersionDelimited` da lista de regexes do nome) → golden cai abaixo de `MEASURED_GOLDEN_PASS` → `[OK] mutação PEGA`.
2. Mutação no fold antes de normalizar em `index_keys` (remova `to_lowercase()`) → `index_keys_match_real_index_rows` falha.
3. Relatório: número `passed/total` real colado do `eprintln!` (`cargo test --quiet --test normalize -- --nocapture golden`).

**Commit:** `feat(installed): port winget NameNormalizer (Initial)`

---

### Task 11: Coleta de instalados (ARP, MSIX, scoop)

#### Where this fits

Fonte de verdade do `list`: o que está instalado nesta máquina. Lógica de filtro é pura e testável; leitura do registro e do PackageManager fica em arquivos `_win.rs`.

#### What binds this task

- ARP: `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall` (vista de 64 bits: `KEY_WOW64_64KEY`), `HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall`, `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`. Leitura medida: 109 subchaves HKLM em 5 ms (winreg).
- Filtro (puro, `filter_arp`): descarta sem `DisplayName` (ou vazio após trim); `SystemComponent == 1`; com `ParentKeyName`; `ReleaseType` ∈ {`Update`, `Hotfix`, `Security Update`, `Service Pack`} (case-insensitive). `DisplayVersion` vazia ⇒ `version = None`.
- Valores de registro com tipo inesperado (DWORD onde se espera string, etc.) ⇒ tratados como ausentes, nunca erro (Review Focus 3). No `_win.rs`, leia com `get_raw_value` e converta só `REG_SZ`/`REG_EXPAND_SZ`; DWORD só para `SystemComponent`/`WindowsInstaller`.
- MSI: `WindowsInstaller == 1` e nome da subchave é GUID `{...}` ⇒ `product_code = Some(nome)`. UpgradeCode: varra `HKLM\SOFTWARE\Classes\Installer\UpgradeCodes\<packed>` — cada valor (nome) é um ProductCode **empacotado**; monte mapa productcode→upgradecode desempacotando os dois.
- GUID empacotado: inverte os 8 primeiros hex, depois os 4, depois os 4, e troca os pares nos 16 restantes. `{23170F69-40C1-2701-1900-000001000000}` ⇔ `96F071321C0410729100000010000000`.
- MSIX: `PackageManager::new()?.FindPackagesByUserSecurityId(&HSTRING::new())` (verificado: 222 pacotes em 43 ms). Descarta `IsFramework()`, `IsResourcePackage()`, `SignatureKind() == PackageSignatureKind::System`. `family_name = Id().FamilyName()`, versão `Major.Minor.Build.Revision`, nome = `DisplayName()` (se falhar, `Id().Name()`), publisher = `PublisherDisplayName()` (se falhar, `None`).
- Scoop: raiz = `%SCOOP%` se definido, senão `~/scoop`. Para cada `apps/<app>/current/` com `manifest.json` + `install.json` (`{"bucket": "main", "architecture": "64bit"}`), `version` do manifest. Ausência de scoop ⇒ lista vazia.
- `scan_all()` roda as três coletas em threads (`std::thread::scope`) e devolve erros por coletor sem abortar.

**Files:**

- Modify: `src/installed/mod.rs`
- Create (new): `src/installed/arp.rs` (puro), `src/installed/arp_win.rs`, `src/installed/appx_win.rs`, `src/installed/scoop.rs`, `src/installed/guid.rs`
- Test (new): `tests/installed.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive { MachineX64, MachineX86, User }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSource { Arp(Hive), Msix, Scoop }

#[derive(Debug, Clone, PartialEq)]
pub struct InstalledApp {
    pub source: InstallSource,
    /// ARP subkey name, MSIX family name, or scoop app name
    pub key: String,
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub product_code: Option<String>,
    pub upgrade_code: Option<String>,
    pub family_name: Option<String>,
    pub scoop_bucket: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawArpEntry {
    pub hive: Option<Hive>,
    pub key_name: String,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub display_version: Option<String>,
    pub windows_installer: Option<u32>,
    pub system_component: Option<u32>,
    pub parent_key_name: Option<String>,
    pub release_type: Option<String>,
}

pub fn filter_arp(raw: RawArpEntry, upgrade_codes: &std::collections::HashMap<String, String>) -> Option<InstalledApp>;
pub fn pack_guid(guid: &str) -> Option<String>;     // "{...}" -> 32 hex, uppercase
pub fn unpack_guid(packed: &str) -> Option<String>; // 32 hex -> "{...}", uppercase
pub fn read_scoop(root: &std::path::Path) -> Vec<InstalledApp>;

#[derive(Debug, Default)]
pub struct InstalledScan { pub apps: Vec<InstalledApp>, pub errors: Vec<String>, pub elapsed_ms: u128 }
pub fn scan_all() -> InstalledScan; // on non-Windows: only scoop
```

`hive` em `RawArpEntry` é `Option` só para `Default`; `filter_arp` usa `Hive::MachineX64` se `None`.

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use wingetrs::installed::{filter_arp, pack_guid, read_scoop, unpack_guid, Hive, InstallSource, RawArpEntry};

fn raw(name: &str) -> RawArpEntry {
    RawArpEntry { hive: Some(Hive::MachineX64), key_name: "K".into(), display_name: Some(name.into()), ..Default::default() }
}

#[test]
fn guid_packing_round_trip() {
    assert_eq!(pack_guid("{23170F69-40C1-2701-1900-000001000000}").unwrap(), "96F071321C0410729100000010000000");
    assert_eq!(unpack_guid("96F071321C0410729100000010000000").unwrap(), "{23170F69-40C1-2701-1900-000001000000}");
    assert_eq!(pack_guid("{23170f69-40c1-2701-1900-000001000000}").unwrap(), "96F071321C0410729100000010000000");
    assert!(pack_guid("not-a-guid").is_none());
    assert!(unpack_guid("XYZ").is_none());
}

#[test]
fn filters_system_children_updates_and_nameless() {
    let none = HashMap::new();
    assert!(filter_arp(RawArpEntry { display_name: None, ..raw("x") }, &none).is_none());
    assert!(filter_arp(raw("   "), &none).is_none());
    assert!(filter_arp(RawArpEntry { system_component: Some(1), ..raw("x") }, &none).is_none());
    assert!(filter_arp(RawArpEntry { parent_key_name: Some("P".into()), ..raw("x") }, &none).is_none());
    assert!(filter_arp(RawArpEntry { release_type: Some("security update".into()), ..raw("x") }, &none).is_none());
    assert!(filter_arp(RawArpEntry { system_component: Some(0), ..raw("Keep") }, &none).is_some());
}

#[test]
fn msi_entry_gets_product_and_upgrade_code() {
    let mut up = HashMap::new();
    up.insert("{23170F69-40C1-2701-1900-000001000000}".to_string(), "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}".to_string());
    let app = filter_arp(
        RawArpEntry { key_name: "{23170F69-40C1-2701-1900-000001000000}".into(), windows_installer: Some(1), display_version: Some("19.00.00.0".into()), ..raw("7-Zip 19.00 (x64 edition)") },
        &up,
    ).unwrap();
    assert_eq!(app.product_code.as_deref(), Some("{23170F69-40C1-2701-1900-000001000000}"));
    assert_eq!(app.upgrade_code.as_deref(), Some("{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}"));
    assert_eq!(app.source, InstallSource::Arp(Hive::MachineX64));
    assert_eq!(app.version.as_deref(), Some("19.00.00.0"));
}

#[test]
fn non_msi_key_name_is_still_a_product_code_candidate_and_blank_version_is_none() {
    let app = filter_arp(RawArpEntry { key_name: "Notepad++".into(), display_version: Some("  ".into()), ..raw("Notepad++ (64-bit x64)") }, &HashMap::new()).unwrap();
    assert_eq!(app.product_code.as_deref(), Some("Notepad++"));
    assert_eq!(app.version, None);
}

#[test]
fn scoop_apps_from_directory_layout() {
    let dir = tempfile::tempdir().unwrap();
    let cur = dir.path().join("apps/ripgrep/current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"15.2.0","description":"rg"}"#).unwrap();
    std::fs::write(cur.join("install.json"), r#"{"bucket":"main","architecture":"64bit"}"#).unwrap();
    let broken = dir.path().join("apps/broken/current");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("manifest.json"), "{oops").unwrap();
    let apps = read_scoop(dir.path());
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].key, "ripgrep");
    assert_eq!(apps[0].scoop_bucket.as_deref(), Some("main"));
    assert_eq!(apps[0].version.as_deref(), Some("15.2.0"));
    assert_eq!(apps[0].source, InstallSource::Scoop);
}

#[test]
#[ignore = "live: reads this machine's registry and MSIX packages"]
fn live_scan_is_fast_and_nonempty() {
    let scan = wingetrs::installed::scan_all();
    eprintln!("apps={} errors={:?} elapsed_ms={}", scan.apps.len(), scan.errors, scan.elapsed_ms);
    assert!(scan.apps.len() > 50);
    assert!(scan.elapsed_ms < 500);
}
```

**Decisão (registrada):** `product_code` recebe o nome da subchave **para toda entrada ARP**, não só MSI — o índice guarda chaves não-GUID como `notepad++` e `7-zip` em `productcodes2` (medido). `upgrade_code` só para MSI.

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente puro + `_win.rs`. Step 3: PASS; rode o live `cargo test --quiet --test installed -- --ignored --nocapture live_scan` e cole a linha `apps=...`. Step 4: gate, commit.

#### Checks beyond the steps

1. Filtro de `ParentKeyName`: mutate para ignorar → `filters_system_children_updates_and_nameless` falha.
2. Empacotamento: mutate a troca de pares (ex.: não trocar) → `guid_packing_round_trip` falha.
3. Product code para não-MSI: mutate a condição para exigir `windows_installer == Some(1)` → `non_msi_key_name_...` falha.

**Commit:** `feat(installed): collect ARP, MSIX and scoop installs`

---

### Task 12: Correlação e comando `list`

#### Where this fits

Liga instalado ↔ catálogo e decide se há atualização. Entrega `wingetrs list [--upgradable] [--source] [--json]`.

#### What binds this task

- Ordem de regras (a primeira que acerta decide; spec §5.4):
  1. `ProductCode`: `catalog.lookup(MapTable::ProductCode, key)` com o `product_code`.
  2. `UpgradeCode`: `lookup(UpgradeCode, upgrade_code)`.
  3. `PackageFamilyName`: `lookup(Pfn, family_name)`.
  4. `NameAndPublisher`: `keys = normalizer.index_keys(name, publisher.unwrap_or(""))`; candidatos por nome = `lookup(NormName, name_with_arch)` se houver arquitetura **e** der resultado, senão `lookup(NormName, name)`; resultado = interseção com `lookup(NormPublisher, publisher)`.
  5. `ScoopManifest`: app scoop ⇒ `catalog.get("scoop:<bucket>", app)`.
- Candidatos deduplicados. 1 ⇒ `Matched`; >1 ⇒ `Ambiguous` (nunca escolher); 0 em todas ⇒ `Unmatched`.
- Estado de atualização (função pura `update_state`):
  - versão instalada ausente, vazia ou `Version::is_unknown` ⇒ `Unknown`;
  - com `arp_max_version`: instalada ≥ `arp_max` ⇒ `UpToDate`; senão `Available { latest }`;
  - sem faixa ARP: instalada < latest ⇒ `Available`; senão `UpToDate`.
- Comparação sempre com `core::version::Version`.
- Saída CLI: tabela `Nome  Id  Instalada  Disponível  Fonte` + seções "Ambíguos" e "Não correlacionados" (só sem `--upgradable`); `--json` serializa `Vec<ListRow>`.

**Files:**

- Create (new): `src/installed/correlate.rs`
- Modify: `src/installed/mod.rs`, `src/cli/mod.rs`, `src/cli/output.rs`
- Test (new): `tests/correlate.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Rule { ProductCode, UpgradeCode, PackageFamilyName, NameAndPublisher, ScoopManifest }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Correlation {
    Matched { source: String, package_id: String, rule: Rule },
    Ambiguous { candidates: Vec<String>, rule: Rule },
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum UpdateState { UpToDate, Available { latest: String }, Unknown }

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListRow {
    pub name: String,
    pub installed_version: Option<String>,
    pub correlation: Correlation,
    pub latest: Option<String>,
    pub state: UpdateState,
}

pub fn update_state(installed: Option<&str>, latest: &str, arp_min: Option<&str>, arp_max: Option<&str>) -> UpdateState;
pub fn correlate(
    apps: &[crate::installed::InstalledApp],
    catalog: &crate::catalog::Catalog,
    normalizer: &crate::installed::normalize::NameNormalizer,
) -> Result<Vec<ListRow>, crate::catalog::CatalogError>;
```

`InstalledApp` precisa de `serde::Serialize`? Não — `ListRow` não o carrega (decisão: só nome e versão vão para o JSON).

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::Catalog;
use wingetrs::core::SourceKind;
use wingetrs::installed::correlate::{correlate, update_state, Correlation, Rule, UpdateState};
use wingetrs::installed::normalize::NameNormalizer;
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::sources::winget::index::read_index;

const DB: &[u8] = include_bytes!("fixtures/winget/index-mini.db");

fn catalog() -> Catalog {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let imp = read_index(&p).unwrap();
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source("winget", SourceKind::Winget, "v", 1, &imp.packages, &imp.maps).unwrap();
    c
}

fn arp(key: &str, name: &str, publisher: &str, version: &str) -> InstalledApp {
    InstalledApp {
        source: InstallSource::Arp(Hive::MachineX64),
        key: key.into(),
        name: name.into(),
        publisher: Some(publisher.into()),
        version: Some(version.into()),
        product_code: Some(key.into()),
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
    }
}

#[test]
fn product_code_match_is_case_insensitive() {
    let rows = correlate(&[arp("{23170F69-40C1-2701-1900-000001000000}", "7-Zip 19.00 (x64 edition)", "Igor Pavlov", "19.00.00.0")], &catalog(), &NameNormalizer::new()).unwrap();
    assert_eq!(rows[0].correlation, Correlation::Matched { source: "winget".into(), package_id: "7zip.7zip".into(), rule: Rule::ProductCode });
    assert_eq!(rows[0].state, UpdateState::Available { latest: "26.04".into() });
}

#[test]
fn package_family_name_match() {
    let mut a = arp("x", "Windows Terminal", "Microsoft Corporation", "1.25.2733.0");
    a.source = InstallSource::Msix;
    a.product_code = None;
    a.family_name = Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe".into());
    let rows = correlate(&[a], &catalog(), &NameNormalizer::new()).unwrap();
    assert_eq!(rows[0].correlation, Correlation::Matched { source: "winget".into(), package_id: "Microsoft.WindowsTerminal".into(), rule: Rule::PackageFamilyName });
    assert_eq!(rows[0].state, UpdateState::UpToDate);
}

#[test]
fn name_and_publisher_with_arch_fallback() {
    let mut a = arp("Mozilla Firefox 150.0 (x64 en-US)", "Mozilla Firefox (x64 en-US)", "Mozilla", "150.0");
    a.product_code = None;
    let rows = correlate(&[a], &catalog(), &NameNormalizer::new()).unwrap();
    assert_eq!(rows[0].correlation, Correlation::Matched { source: "winget".into(), package_id: "Mozilla.Firefox".into(), rule: Rule::NameAndPublisher });
}

#[test]
fn unknown_app_is_unmatched_and_never_upgradable() {
    let rows = correlate(&[arp("Zzz", "Totally Unknown Thing", "Nobody", "")], &catalog(), &NameNormalizer::new()).unwrap();
    assert_eq!(rows[0].correlation, Correlation::Unmatched);
    assert_eq!(rows[0].state, UpdateState::Unknown);
}

#[test]
fn update_state_table() {
    assert_eq!(update_state(Some("3.13.15150.0"), "3.13.15", Some("3.13.15150.0"), Some("3.13.15150.0")), UpdateState::UpToDate);
    assert_eq!(update_state(Some("3.13.14150.0"), "3.13.15", Some("3.13.15150.0"), Some("3.13.15150.0")), UpdateState::Available { latest: "3.13.15".into() });
    assert_eq!(update_state(Some("1.0"), "1.0.0", None, None), UpdateState::UpToDate);
    assert_eq!(update_state(Some("1.0"), "1.1", None, None), UpdateState::Available { latest: "1.1".into() });
    assert_eq!(update_state(Some("2.0"), "1.1", None, None), UpdateState::UpToDate);
    assert_eq!(update_state(None, "1.1", None, None), UpdateState::Unknown);
    assert_eq!(update_state(Some("  "), "1.1", None, None), UpdateState::Unknown);
    assert_eq!(update_state(Some("unknown"), "1.1", None, None), UpdateState::Unknown);
}

#[test]
fn ambiguous_is_never_resolved_silently() {
    // Two packages sharing a product code key.
    let mut c = Catalog::open_in_memory().unwrap();
    let p = |id: &str| wingetrs::catalog::NewPackage { id: id.into(), name: id.into(), version: "1".into(), ..Default::default() };
    let maps = wingetrs::catalog::WingetMaps { productcodes: vec![("dup".into(), "A.A".into()), ("dup".into(), "B.B".into())], ..Default::default() };
    c.replace_source("winget", SourceKind::Winget, "v", 1, &[p("A.A"), p("B.B")], &maps).unwrap();
    let rows = correlate(&[arp("dup", "Something", "X", "1")], &c, &NameNormalizer::new()).unwrap();
    match &rows[0].correlation {
        Correlation::Ambiguous { candidates, rule } => {
            let mut c = candidates.clone();
            c.sort();
            assert_eq!(c, ["A.A", "B.B"]);
            assert_eq!(*rule, Rule::ProductCode);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    assert_eq!(rows[0].state, UpdateState::Unknown);
}

#[test]
#[ignore = "live: correlates this machine's installs against the real catalog in %LOCALAPPDATA%"]
fn live_list_runs_under_budget() {
    let t = std::time::Instant::now();
    let data = wingetrs::cache::DataDir::default_location().unwrap();
    let cat = Catalog::open(&data.catalog_path()).unwrap();
    let scan = wingetrs::installed::scan_all();
    let rows = correlate(&scan.apps, &cat, &NameNormalizer::new()).unwrap();
    let matched = rows.iter().filter(|r| matches!(r.correlation, Correlation::Matched { .. })).count();
    eprintln!("rows={} matched={} elapsed_ms={}", rows.len(), matched, t.elapsed().as_millis());
    assert!(t.elapsed().as_millis() < 500);
}
```

Se `name_and_publisher_with_arch_fallback` falhar porque o porte (Task 10) normaliza esse nome de outra forma, investigue com o golden; o teste só muda se o golden provar que a entrada não é realista — registre no relatório.

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente `correlate`, `update_state`, `list` na CLI. Step 3: PASS; rode o live (`-- --ignored --nocapture live_list`) e `cargo run --quiet --release -- list` e cole as linhas de resumo. Step 4: gate, commit.

#### Checks beyond the steps

1. Ordem das regras: mutate para avaliar `NameAndPublisher` antes de `ProductCode` → `product_code_match_is_case_insensitive` deve falhar (a regra reportada muda). Se não falhar, o teste não pina a ordem: acrescente um caso onde nome e productcode apontam para pacotes diferentes.
2. Faixa ARP: mutate `arp_max` para comparar com `latest` → `update_state_table` falha.
3. Ambíguo: mutate para pegar o primeiro candidato → `ambiguous_is_never_resolved_silently` falha.

**Commit:** `feat(list): correlate installs with catalog and report upgrades`

---

### Task 13: TUI — tema, glifos e estado

#### Where this fits

Base da TUI sem desenhar nada ainda: tokens de cor, conjuntos de glifos, detecção de profundidade de cor e o estado Elm-like (`App`, `Msg`, `Cmd`, `update`) testável sem terminal.

#### What binds this task

- Tokens (spec §7.3): `accent`, `ok`, `warn`, `muted`, `alert`, `err`, `src_winget`, `src_scoop`, mais `fg`, `fg_dim`, `bg_selected`, `border`, `border_focus`. Tema escuro e claro.
- Profundidade: `COLORTERM` ∈ {`truecolor`, `24bit`} ⇒ RGB; senão `Color::Indexed` (256). `NO_COLOR` definido (qualquer valor não vazio) ⇒ sem cores, mas negrito/itálico/sublinhado mantidos.
- Glifos (codepoints Font Awesome 4 da Nerd Font, decididos):

| Uso | `nerd` | `unicode` | `ascii` |
|---|---|---|---|
| winget | `\u{f17a}` | `●` | `W` |
| scoop | `\u{f1b3}` | `◆` | `S` |
| buscar | `\u{f002}` | `⌕` | `/` |
| instalado/ok | `\u{f00c}` | `✓` | `+` |
| atualização | `\u{f062}` | `↑` | `^` |
| desconhecido | `\u{f128}` | `?` | `?` |
| ambíguo | `\u{f071}` | `⚠` | `!` |
| refresh | `\u{f021}` | `⟳` | `~` |
| offline | `\u{f127}` | `⊘` | `x` |
| licença | `\u{f24e}` | `§` | `L` |
| publisher | `\u{f007}` | `@` | `@` |
| id | `\u{f1b2}` | `#` | `#` |
| seleção | `▶` | `▶` | `>` |

- Spinner: `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` (unicode/nerd), `|/-\` (ascii).
- `App` não faz I/O: `update(&mut App, Msg) -> Vec<Cmd>`; I/O vira `Cmd` executado pelo loop (Task 14).

**Files:**

- Create (new): `src/tui/theme.rs`, `src/tui/app.rs`
- Modify: `src/tui/mod.rs`
- Test (new): `tests/tui_state.rs`

#### Interfaces

```rust
// src/tui/theme.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSet { Nerd, Unicode, Ascii }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth { TrueColor, Ansi256, NoColor }
pub struct Glyphs { pub winget: &'static str, pub scoop: &'static str, pub search: &'static str, pub ok: &'static str,
    pub upgrade: &'static str, pub unknown: &'static str, pub ambiguous: &'static str, pub refresh: &'static str,
    pub offline: &'static str, pub license: &'static str, pub publisher: &'static str, pub id: &'static str,
    pub selected: &'static str, pub spinner: &'static [&'static str] }
impl Glyphs { pub fn for_set(set: GlyphSet) -> &'static Glyphs; }
pub struct Theme { /* ratatui::style::Style per token */ }
impl Theme {
    pub fn new(dark: bool, depth: ColorDepth) -> Theme;
    pub fn accent(&self) -> ratatui::style::Style; /* ...one fn per token... */
    pub fn name(&self) -> Style;        // bold
    pub fn description(&self) -> Style; // italic
    pub fn dim(&self) -> Style;         // DIM modifier
    pub fn matched(&self) -> Style;     // accent + underlined
}
pub fn detect_depth(colorterm: Option<&str>, no_color: Option<&str>) -> ColorDepth;

// src/tui/app.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab { Search, Installed, Updates }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus { List, Details }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter { All, Winget, Scoop }

pub enum Msg {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Tick,
    CatalogLoaded(Vec<crate::core::PackageEntry>),
    InstalledLoaded(Vec<crate::installed::correlate::ListRow>),
    DetailsLoaded { key: String, result: Result<crate::core::PackageDetails, String> },
    RefreshStarted,
    RefreshFinished(Vec<crate::update::SourceReport>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd { Quit, Refresh { force: bool }, LoadDetails { source: String, id: String }, ScanInstalled }

pub struct App {
    pub tab: Tab, pub focus: Focus, pub filter: SourceFilter, pub query: String,
    pub selected: usize, pub scroll: usize,
    pub search: crate::catalog::search::SearchIndex,
    pub hits: Vec<crate::catalog::search::SearchHit>,
    pub installed: Vec<crate::installed::correlate::ListRow>,
    pub details: std::collections::HashMap<String, Result<crate::core::PackageDetails, String>>,
    pub refreshing: bool, pub offline: bool, pub index_age_secs: Option<i64>,
    pub spinner_frame: usize, pub show_help: bool, pub status: Option<String>,
    pub glyphs: GlyphSet, pub size: (u16, u16),
}
impl App {
    pub fn new(entries: Vec<crate::core::PackageEntry>, glyphs: GlyphSet) -> App;
    /// rows visible in the current tab after filters
    pub fn visible_len(&self) -> usize;
    pub fn selected_key(&self) -> Option<(String, String)>; // (source_name, id)
}
pub fn update(app: &mut App, msg: Msg) -> Vec<Cmd>;
```

Chave de `details`: `format!("{source_name}/{id}")` (ex.: `winget/Microsoft.PowerToys`). `selected_key()` devolve `(source_name, id)`.

Teclas (spec §7.2): texto digitado vai para `query` (re-busca, `selected = 0`); `Backspace` apaga; `Down`/`Up` (e `j`/`k` **somente** quando `query` está vazia — senão são texto); `PageDown`/`PageUp` (10 linhas); `Home`/`End`; `Tab`/`BackTab` trocam aba; `Enter` ⇒ `Focus::Details`; `Esc`: se `Details` ⇒ `List`; senão se `query` não vazia ⇒ limpa; senão ⇒ `Cmd::Quit`; `Ctrl+C` ⇒ `Quit`; `q` com `query` vazia ⇒ `Quit`; `Ctrl+R` ⇒ `Refresh { force: true }` (decisão: `r` puro é texto de busca); `Ctrl+S` alterna filtro; `?` com `query` vazia alterna ajuda. Mudança de seleção ⇒ `Cmd::LoadDetails` se o detalhe não está em cache.

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::tui::app::{update, App, Cmd, Focus, Msg, SourceFilter, Tab};
use wingetrs::tui::theme::{detect_depth, ColorDepth, GlyphSet, Glyphs};

fn entry(source: SourceKind, id: &str, name: &str) -> PackageEntry {
    PackageEntry { source, source_name: if source == SourceKind::Winget { "winget".into() } else { "scoop:main".into() },
        id: id.into(), name: name.into(), moniker: None, version: "1".into(), publisher: None, description: None, tags: vec![], commands: vec![] }
}
fn app() -> App {
    App::new(vec![
        entry(SourceKind::Winget, "Microsoft.PowerToys", "PowerToys"),
        entry(SourceKind::Scoop, "ripgrep", "ripgrep"),
        entry(SourceKind::Winget, "Zeal.Zeal", "Zeal"),
    ], GlyphSet::Nerd)
}
fn key(c: KeyCode) -> Msg { Msg::Key(KeyEvent::new(c, KeyModifiers::NONE)) }
fn ctrl(c: char) -> Msg { Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)) }

#[test]
fn typing_filters_and_resets_selection() {
    let mut a = app();
    update(&mut a, key(KeyCode::Down));
    assert_eq!(a.selected, 1);
    for c in "zeal".chars() { update(&mut a, key(KeyCode::Char(c))); }
    assert_eq!(a.query, "zeal");
    assert_eq!(a.selected, 0);
    assert_eq!(a.visible_len(), 1);
}

#[test]
fn j_k_are_text_when_query_is_not_empty() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('j')));
    assert_eq!(a.query, "", "j moves when query is empty");
    assert_eq!(a.selected, 1);
    update(&mut a, key(KeyCode::Char('z')));
    update(&mut a, key(KeyCode::Char('j')));
    assert_eq!(a.query, "zj");
}

#[test]
fn escape_ladder_and_quit() {
    let mut a = app();
    update(&mut a, key(KeyCode::Enter));
    assert_eq!(a.focus, Focus::Details);
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
    assert_eq!(a.focus, Focus::List);
    update(&mut a, key(KeyCode::Char('x')));
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
    assert_eq!(a.query, "");
    assert_eq!(update(&mut a, key(KeyCode::Esc)), vec![Cmd::Quit]);
    assert_eq!(update(&mut app(), ctrl('c')), vec![Cmd::Quit]);
}

#[test]
fn tabs_cycle_and_filter_toggles() {
    let mut a = app();
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Installed);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Updates);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Search);
    update(&mut a, key(KeyCode::BackTab));
    assert_eq!(a.tab, Tab::Updates);
    let mut b = app();
    update(&mut b, ctrl('s'));
    assert_eq!(b.filter, SourceFilter::Winget);
    assert_eq!(b.visible_len(), 2);
}

#[test]
fn selection_change_requests_details_once() {
    let mut a = app();
    let cmds = update(&mut a, key(KeyCode::Down));
    assert!(matches!(cmds.as_slice(), [Cmd::LoadDetails { .. }]), "{cmds:?}");
    let (source, id) = a.selected_key().unwrap();
    update(&mut a, Msg::DetailsLoaded { key: format!("{source}/{id}"), result: Err("x".into()) });
    update(&mut a, key(KeyCode::Up));
    let cmds = update(&mut a, key(KeyCode::Down));
    assert!(cmds.is_empty(), "cached details must not be reloaded: {cmds:?}");
}

#[test]
fn ctrl_r_refreshes_and_selection_is_clamped() {
    let mut a = app();
    assert_eq!(update(&mut a, ctrl('r')), vec![Cmd::Refresh { force: true }]);
    for _ in 0..10 { update(&mut a, key(KeyCode::Down)); }
    assert_eq!(a.selected, 2);
    update(&mut a, key(KeyCode::Home));
    assert_eq!(a.selected, 0);
}

#[test]
fn depth_detection() {
    assert_eq!(detect_depth(Some("truecolor"), None), ColorDepth::TrueColor);
    assert_eq!(detect_depth(Some("24bit"), None), ColorDepth::TrueColor);
    assert_eq!(detect_depth(None, None), ColorDepth::Ansi256);
    assert_eq!(detect_depth(Some("truecolor"), Some("1")), ColorDepth::NoColor);
    assert_eq!(detect_depth(None, Some("")), ColorDepth::Ansi256, "empty NO_COLOR is ignored");
}

#[test]
fn glyph_sets_are_distinct_and_ascii_is_ascii() {
    let a = Glyphs::for_set(GlyphSet::Ascii);
    for g in [a.winget, a.scoop, a.search, a.ok, a.upgrade, a.unknown, a.ambiguous, a.refresh, a.offline, a.license, a.publisher, a.id, a.selected] {
        assert!(g.is_ascii(), "{g:?}");
    }
    assert_eq!(Glyphs::for_set(GlyphSet::Nerd).winget, "\u{f17a}");
    assert_eq!(Glyphs::for_set(GlyphSet::Unicode).upgrade, "↑");
}
```

`Cmd` precisa de `#[derive(Debug, Clone, PartialEq)]` (já na interface).

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente. Step 3: PASS, gate, commit.

#### Checks beyond the steps

1. `j`/`k` como texto: mutate a guarda `query.is_empty()` → `j_k_are_text_when_query_is_not_empty` falha.
2. Cache de detalhes: mutate para sempre emitir `LoadDetails` → `selection_change_requests_details_once` falha.
3. `NO_COLOR` vazio ignorado: mutate para `is_some()` → `depth_detection` falha.

**Commit:** `feat(tui): theme tokens, glyph sets and elm-style state`

---

### Task 14: TUI — views, loop de eventos e snapshots

#### Where this fits

Desenha o layout da spec §7.1 a partir de `App` (função pura `view`), roda o loop `tokio::select!` e executa os `Cmd` em tasks. `wingetrs` sem argumentos abre a TUI.

#### What binds this task

- Layout (spec §7.1): barra de abas no título (contador de atualizações na aba Atualizações), campo de busca, lista à esquerda (≈ 55%), detalhes à direita, barra de status em itálico (`<visíveis> de <total> · ↑↓ mover · Tab aba · Enter detalhes · Ctrl+R refresh · ? ajuda`), idade do índice + spinner/offline no canto superior direito.
- Linha da lista: `<seleção> <glifo fonte colorido> <nome em negrito com match sublinhado>  <versão>`; na aba Atualizações `antiga → nova` (antiga dim, nova `ok`+negrito).
- O painel de detalhes mostra **imediatamente** os campos do `PackageEntry` selecionado (nome, id, versão, publisher, descrição) e acrescenta licença/instaladores/versões quando `PackageDetails` chega.
- Registre `tracing::info!(first_frame_ms = ...)` após o primeiro `draw` (medido desde o início de `run`); a Task 15 lê essa linha do log.
- Detalhes: nome negrito; id e publisher dim com glifos; licença; descrição em itálico (quebra de linha); instaladores `arquitetura · tipo · escopo`; estado de instalação quando houver `ListRow`. Detalhe carregando ⇒ spinner + "carregando…"; erro ⇒ estilo `err`.
- Bordas `BorderType::Rounded`; painel com foco usa `border_focus`.
- **Pequeno:** com largura < 60 esconde o painel de detalhes; com altura < 8 mostra só a lista; nunca pânico (Review Focus 5).
- Loop: `ratatui::init()` / `ratatui::restore()` (restaura também em pânico); `crossterm::event::EventStream` (`futures_util::StreamExt::next`); canal `tokio::sync::mpsc::unbounded_channel::<Msg>()`; tick de 100 ms só enquanto `refreshing` ou há detalhe carregando; redesenha só quando `update` mudou algo (compare um contador de geração no `App`).
- `Cmd::Refresh` ⇒ `update::run_update` numa task; `Cmd::ScanInstalled` ⇒ `spawn_blocking(scan_all + correlate)`; `Cmd::LoadDetails` ⇒ winget `details` / scoop `details_from_raw` com debounce de 150 ms (descarta se a seleção mudou).
- Ao abrir: carrega catálogo (se vazio, mostra "baixando catálogo…" e dispara `Refresh{force:false}`), dispara `ScanInstalled` e, se `needs_refresh`, `Refresh{force:false}`.
- Glifos: `--glyphs` > `config.toml` > `nerd`.

**Files:**

- Create (new): `src/tui/view.rs`, `src/tui/run.rs`
- Modify: `src/tui/mod.rs`, `src/cli/mod.rs` (sem subcomando ⇒ `tui::run`)
- Test (new): `tests/tui_view.rs` + snapshots em `tests/snapshots/` (gerados pelo `insta`)

#### Interfaces

```rust
// src/tui/view.rs
pub fn view(app: &crate::tui::app::App, theme: &crate::tui::theme::Theme, frame: &mut ratatui::Frame);
// src/tui/run.rs
pub fn run(glyphs: crate::tui::theme::GlyphSet) -> anyhow::Result<()>; // builds its own tokio runtime
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::tui::app::{update, App, Msg};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;

fn entries() -> Vec<PackageEntry> {
    let e = |s: SourceKind, id: &str, name: &str, v: &str| PackageEntry {
        source: s, source_name: if s == SourceKind::Winget { "winget".into() } else { "scoop:main".into() },
        id: id.into(), name: name.into(), moniker: None, version: v.into(), publisher: None,
        description: Some("Utilitários para power users".into()), tags: vec![], commands: vec![] };
    vec![
        e(SourceKind::Winget, "Microsoft.PowerToys", "PowerToys", "0.101.2362.0"),
        e(SourceKind::Winget, "lin-ycv.EverythingPowerToys", "EverythingPowerToys", "0.91.1"),
        e(SourceKind::Scoop, "powertoys", "powertoys", "0.101.2"),
    ]
}

fn render(app: &App, w: u16, h: u16) -> String {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| view(app, &theme, f)).unwrap();
    format!("{}", t.backend())
}

#[test]
fn search_screen_snapshot() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    for c in "powert".chars() {
        update(&mut a, Msg::Key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char(c), crossterm::event::KeyModifiers::NONE)));
    }
    insta::assert_snapshot!(render(&a, 100, 16));
}

#[test]
fn empty_catalog_snapshot() {
    insta::assert_snapshot!(render(&App::new(vec![], GlyphSet::Ascii), 80, 12));
}

#[test]
fn offline_snapshot() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.offline = true;
    a.index_age_secs = Some(3 * 3600);
    insta::assert_snapshot!(render(&a, 100, 12));
}

#[test]
fn tiny_terminals_never_panic() {
    let a = App::new(entries(), GlyphSet::Nerd);
    for (w, h) in [(1, 1), (10, 3), (20, 5), (59, 7), (60, 8), (200, 60)] {
        let _ = render(&a, w, h);
    }
}

#[test]
fn narrow_terminal_hides_details_panel() {
    let a = App::new(entries(), GlyphSet::Ascii);
    let s = render(&a, 50, 12);
    assert!(!s.contains("Utilitários"), "details panel must be hidden below 60 columns:\n{s}");
    let s = render(&a, 100, 12);
    assert!(s.contains("Utilitários"), "details panel visible at 100 columns:\n{s}");
}
```

Snapshots com `GlyphSet::Ascii` e `ColorDepth::NoColor` para serem estáveis em qualquer fonte. Revise cada `.snap.new` antes de aceitar (`cargo insta` não está instalado: renomeie `.snap.new` para `.snap` após ler); cole no relatório o snapshot `search_screen_snapshot`.

#### Steps

- [ ] Step 1: testes → FAIL. Step 2: implemente `view` e `run`. Step 3: aceite snapshots após inspeção; PASS. Step 4: teste manual: `cargo run --release` — abra, digite, troque abas, `Ctrl+R`, redimensione a janela, saia com `Esc`; descreva no relatório o que viu (inclua tempo até o primeiro frame medido com `tracing` em `log/`). Step 5: gate, commit.

#### Checks beyond the steps

1. Esconder detalhes < 60 colunas: mutate o limite para `< 40` → `narrow_terminal_hides_details_panel` falha.
2. Sublinhado do match: mutate o estilo `matched()` para não sublinhar e confira se algum teste falha; snapshots em `NoColor` não carregam estilo — acrescente um teste que inspeciona `TestBackend::buffer()` e afirma `Modifier::UNDERLINED` na célula da 1ª letra casada de "PowerToys" após digitar "powert". Refaça a mutação até `[OK] mutação PEGA`.

**Commit:** `feat(tui): layout, event loop and snapshot tests`

---

### Task 15: Validação ao vivo — oráculo do winget e benchmarks

#### Where this fits

Fecha a fase 1 medindo os critérios de sucesso da spec §9.2 nesta máquina, com evidência. Não adiciona funcionalidade; pode ajustar desempenho se uma meta falhar.

#### What binds this task

- Metas: `search` com cache quente < 100 ms; `list` < 500 ms; primeiro frame da TUI com cache < 50 ms; `update` sem mudanças (304) < 300 ms; correlação ≥ 90% do que o `winget list` identifica.
- Medição pelo binário release (`cargo build --release`), mediana de 10 execuções após 2 de aquecimento, via `scripts/bench.ps1` (criar). Medir o comando inteiro (processo), não funções internas — o critério é o tempo que o usuário espera.
- Oráculo: `winget list --source winget --disable-interactivity` (texto; colunas Nome/ID/Versão/Disponível/Origem). Conte os IDs que o winget mostra; conte quantos desses o `wingetrs list --json` mostra como `Matched` com o mesmo `package_id`. Divergências: listar até 30 com a causa provável.
- **Toda número no relatório vem de comando colado.** "não medido" é resposta válida; número inventado não.
- Meta não atingida: registre o teto medido, a causa (perfil com `tracing` spans) e uma tarefa de follow-up no relatório. Não afrouxe metas.

**Files:**

- Create (new): `scripts/bench.ps1`, `docs/superpowers/reports/2026-10-07-fase1-validacao.md`
- Test (new): nenhum novo (os `#[ignore]` das Tasks 11/12 são executados e colados)

#### Steps

- [ ] Step 1: `scripts/bench.ps1` — parâmetros `-Exe`, `-Args`, `-Runs 10`, `-Warmup 2`; imprime min/mediana/max em ms usando `[Diagnostics.Stopwatch]` em volta de `& $Exe @Args | Out-Null`.
- [ ] Step 2: `cargo build --release`; `target/release/wingetrs update --force` (aquece); rode bench de `search powertoys`, `list`, `update` (sem `--force`, após 1 s, para pegar o caminho Fresh/304 — documente qual caminho foi medido) e o mesmo para `winget search powertoys --source winget` e `winget list`.
- [ ] Step 3: primeiro frame: leia o span `first_frame_ms` do log (a Task 14 registra) após abrir/fechar a TUI 5 vezes; cole as linhas.
- [ ] Step 4: oráculo; produza a tabela de divergências.
- [ ] Step 5: relatório `docs/superpowers/reports/2026-10-07-fase1-validacao.md` com: tabela meta × medido × winget, comandos exatos, saídas coladas, divergências do oráculo, follow-ups. Commit.

#### Checks beyond the steps

Esta tarefa não entrega código de produto: **no mutation proof** (declare explicitamente no relatório). A verificação é o orquestrador reexecutar um dos benchmarks e obter mediana dentro de ±30% da colada.

**Commit:** `docs(report): phase 1 live validation and benchmarks`

---

## Lote de correção pós-validação (decidido pelo usuário em 2026-10-08)

Origem: `docs/superpowers/reports/2026-10-07-fase1-validacao.md` (F1, F2). Decisões do usuário: corrigir as duas metas que falharam; pacotes MSIX de framework **incluídos e mostrados** (paridade com o winget, sem ocultar).

### Task 16: `search` abaixo de 100 ms

#### Where this fits

`wingetrs search` mede 192–206 ms de mediana (meta da spec §9.2: < 100 ms). Medido na Task 15: `Catalog::load_entries` lê as 19 478 linhas **incluindo a coluna `raw` (5,6 MB de JSON do scoop) e `winget_hash`**, em ~90–105 ms; `nucleo` ~17–22 ms; drop das estruturas ~10–13 ms; piso do processo ~44 ms.

#### What binds this task

- Meta: mediana < 100 ms com `pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'search powertoys'` (10 execuções, 2 de aquecimento), catálogo real em `%LOCALAPPDATA%\wingetrs`, binário release.
- `load_entries` é usado por `cli` (search) e `tui::run` (2 lugares); todos só precisam de `PackageEntry` (sem `raw`, `winget_hash`, faixas ARP).
- Não mude a interface pública de `Catalog` além de acrescentar; `get`/`find_by_id` continuam devolvendo `StoredPackage` completo.
- Medir antes de otimizar além do item 1: cada mudança extra precisa de número antes/depois colado.

#### The decision this task must get right

1. `load_entries` passa a selecionar só as colunas de `PackageEntry` (`source, kind, id, name, moniker, version, publisher, description, tags, commands`) com uma função de linha própria. Isso é obrigatório.
2. Se ainda ≥ 100 ms, perfile com `tracing` (spans existentes ou novos, `-v`) as etapas do processo `search` (init de log, DataDir/Config, `needs_refresh`/spawn de refresh em segundo plano, abrir catálogo, load, SearchIndex, busca, saída, drop) e ataque a maior. Permitido: pular a desalocação no caminho CLI de `search` (`std::mem::forget` do índice depois de imprimir, com comentário de uma linha explicando o motivo). Proibido: cache em disco novo, mudança de schema, dependência nova.
3. A busca não muda de semântica: os testes existentes de `tests/search.rs` e `tests/catalog.rs` continuam passando sem alteração.

#### Traps already paid for that apply here

- Medir pelo processo inteiro (bench.ps1), não por função — a Task 15 mostrou piso de ~44 ms fora da busca.
- O refresh em segundo plano só dispara com catálogo velho (> 900 s); meça nos dois estados e diga qual foi medido.

#### The body of the tests that are not obvious

Em `tests/catalog.rs` (new test fn):

```rust
#[test]
fn load_entries_does_not_need_raw_column() {
    // A package with a huge raw payload must load with the same entry and without raw.
    let mut c = Catalog::open_in_memory().unwrap();
    let mut p = pkg("big", "Big", "1");
    p.raw = Some("x".repeat(1_000_000));
    c.replace_source("scoop:main", SourceKind::Scoop, "e", 1, &[p], &WingetMaps::default()).unwrap();
    let e = c.load_entries().unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].id, "big");
    assert_eq!(c.get("scoop:main", "big").unwrap().unwrap().raw.as_deref().map(str::len), Some(1_000_000));
}
```

#### Checks beyond the steps

- Mutation proof: `pwsh -File scripts/mutate.ps1 -Path src/catalog/mod.rs -Anchor '<o SELECT novo de load_entries>' -Replacement '<o SELECT com PKG_COLS>' -TestCommand 'cargo test --quiet --test catalog'` — **esperado: sobrevive** (é otimização, não regra observável por teste); cole a saída e diga isso. A prova desta tarefa é o benchmark.
- Benchmark antes e depois (3 rodadas de bench.ps1 cada, catálogo fresco e velho), colados. Se a meta não for atingida: teto medido, causa e follow-up — nunca afrouxe a meta.

**Files:**
- Modify: `src/catalog/mod.rs`, `src/cli/mod.rs` (só se o perfil apontar), `src/tui/run.rs` (só se necessário)
- Test: `tests/catalog.rs`

**Commit:** `perf(search): load only search columns from the catalog`

### Task 17: Incluir pacotes MSIX de framework no `list`

#### Where this fits

O oráculo da Task 15 (95/116 = 81,9%, meta ≥ 90%) perdeu 15 IDs porque `src/installed/appx_win.rs` descarta `IsFramework()`: `Microsoft.VCLibs.14`, `Microsoft.VCLibs.Desktop.14`, `Microsoft.UI.Xaml.2.6/2.7/2.8`, `Microsoft.DotNet.Native.Runtime`, `Microsoft.WindowsAppRuntime.1.1 … 1.8` e `.2`. O winget os lista. Decisão do usuário: **incluir e mostrar** (sem ocultar, sem flag nova).

#### What binds this task

- Remova o descarte de `IsFramework()`; **continue** descartando `IsResourcePackage()` e `SignatureKind == System`.
- Deduplicação: vários pacotes MSIX do mesmo `FamilyName` coexistem (arquiteturas x86/x64/arm64 e versões lado a lado do mesmo framework). Mantenha **uma** `InstalledApp` por `FamilyName`, a de **maior versão** (`core::version::Version`). A regra fica numa função pura em `src/installed/mod.rs`, não no `_win.rs`.
- Nome de exibição: se `DisplayName()` falhar, vier vazio ou começar com `ms-resource:`, use `Id().Name()`.
- Orçamento: `list` continua < 500 ms (medido 400 ms antes desta tarefa). A coleta MSIX já é paralela em 8 threads; meça `list` com bench.ps1 antes e depois e cole.
- Meta da spec: oráculo ≥ 90%. Reexecute a comparação com `winget list --source winget --disable-interactivity` (somente leitura) pelo mesmo método da Task 15 (seção "Oráculo" do relatório de validação) e cole a nova taxa.

#### The body of the tests that are not obvious

Funções puras a criar (assinaturas decididas), exportadas em `wingetrs::installed`:

```rust
/// Keeps one MSIX app per family name: the highest version (winget-style ordering).
pub fn dedupe_msix_by_family(apps: Vec<InstalledApp>) -> Vec<InstalledApp>;
/// "ms-resource:..." or blank display names fall back to the package id name.
pub fn msix_display_name(display: Option<&str>, id_name: &str) -> String;
```

Testes em `tests/installed.rs`:

```rust
#[test]
fn msix_frameworks_are_deduped_by_family_keeping_highest_version() {
    use wingetrs::installed::{dedupe_msix_by_family, InstallSource, InstalledApp};
    let m = |fam: &str, v: &str| InstalledApp {
        source: InstallSource::Msix, key: fam.into(), name: fam.into(), publisher: None,
        version: Some(v.into()), product_code: None, upgrade_code: None,
        family_name: Some(fam.into()), scoop_bucket: None,
    };
    let out = dedupe_msix_by_family(vec![
        m("Microsoft.VCLibs.140.00_8wekyb3d8bbwe", "14.0.33519.0"),
        m("Microsoft.VCLibs.140.00_8wekyb3d8bbwe", "14.0.33728.0"),
        m("Microsoft.VCLibs.140.00_8wekyb3d8bbwe", "14.0.30704.0"),
        m("Microsoft.UI.Xaml.2.8_8wekyb3d8bbwe", "8.2310.30001.0"),
    ]);
    assert_eq!(out.len(), 2);
    let vc = out.iter().find(|a| a.key.starts_with("Microsoft.VCLibs")).unwrap();
    assert_eq!(vc.version.as_deref(), Some("14.0.33728.0"));
}

#[test]
fn msix_display_name_falls_back_on_resource_strings() {
    use wingetrs::installed::msix_display_name;
    assert_eq!(msix_display_name(Some("ms-resource:AppName"), "Microsoft.VCLibs.140.00"), "Microsoft.VCLibs.140.00");
    assert_eq!(msix_display_name(Some("  "), "X.Y"), "X.Y");
    assert_eq!(msix_display_name(None, "X.Y"), "X.Y");
    assert_eq!(msix_display_name(Some("Windows Terminal"), "Microsoft.WindowsTerminal"), "Windows Terminal");
}
```

#### Checks beyond the steps

- Mutation proofs (`-TestCommand 'cargo test --quiet --test installed'`): (1) dedupe mantendo a **primeira** em vez da maior versão ⇒ PEGA; (2) remover o fallback de `ms-resource:` ⇒ PEGA.
- Live: `cargo test --quiet --release --test installed -- --ignored --nocapture live_scan` e `cargo test --quiet --release --test correlate -- --ignored --nocapture live_list`, colados.
- Oráculo: taxa nova colada, com a lista das divergências restantes.

**Files:**
- Modify: `src/installed/appx_win.rs`, `src/installed/mod.rs`
- Test: `tests/installed.rs`

**Commit:** `feat(list): include MSIX framework packages, one per family`

### Task 18: `search` da CLI com leitura e busca fundidas, antecipação e abertura barata

#### Where this fits

Decidido pelo usuário em 2026-10-08 após perfil por etapa (orquestrador, release, mediana de 15, catálogo real de 19 528 entradas):

| Etapa | ms |
|---|---|
| `Catalog::open` + `is_empty` (`COUNT(*)` + `CREATE ... IF NOT EXISTS` a cada abertura) | 6,5 |
| `load_entries` paralelo (hoje) | 42,7 |
| ↳ 1 thread: percorrer linhas sem ler | 12,2 |
| ↳ 1 thread: ler colunas como `&str`, sem alocar | 27,0 |
| ↳ 1 thread: decodificar e alocar `PackageEntry` (como hoje) | 78,5 |
| ↳ alocações por load | ~223 mil `String`s (106 mil são tags/comandos) |
| busca nucleo 8 threads / 1 thread | 10,3 / 19,4 |
| piso do processo (hello-world Rust no mesmo bench) | 38 |

O gargalo é alocação (o heap padrão do Windows serializa as threads), não o SQLite. Meta da spec §9.2: `search` < 100 ms; hoje 107 ms.

#### What binds this task

- Só o caminho **CLI `search`** muda de estratégia. A TUI continua com `load_entries` + `SearchIndex` em memória (ela precisa do catálogo inteiro). `SearchIndex::search` não muda de comportamento.
- **Mesmo ranking, bit a bit**: a nova busca em disco deve produzir exatamente a mesma lista (ids, ordem, `score`, `name_indices`) que `SearchIndex::new(catalog.load_entries()?).search(q, filter, limit)`. Para isso, a pontuação de uma entrada vira **uma função compartilhada** usada pelos dois caminhos.
- Sem dependência nova, sem mudança de schema, sem cache novo em disco.
- Meta: `pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'search powertoys'` mediana < 100 ms (estimativa 50–60 ms). Bench antes/depois, 3 rodadas, colados.

#### The decision this task must get right

1. **Pontuação compartilhada** em `src/catalog/search.rs`:

```rust
/// Text fields of one catalog row, borrowed (no allocation needed to score it).
pub struct EntryView<'a> {
    pub source: SourceKind,
    pub id: &'a str,
    pub name: &'a str,
    pub moniker: Option<&'a str>,
    pub tags: &'a str,      // space-separated, as stored in the catalog
    pub commands: &'a str,  // space-separated, as stored in the catalog
}
/// Score of one entry for a parsed query, or None when it does not match.
/// Same rules as today: name (with indices), id, moniker, tags/commands at half weight, EXACT_BONUS.
pub fn score_entry(view: &EntryView<'_>, q: &Query, matcher: &mut Matcher, buf: &mut Vec<char>) -> Option<(u32, Vec<u32>)>;
/// Parsed query reused across entries (trimmed text, lowercase, nucleo Pattern).
pub struct Query { /* text, lower, pattern */ }
impl Query { pub fn new(text: &str) -> Query; pub fn is_empty(&self) -> bool; }
```

`scan` (caminho em memória) passa a montar `EntryView` a partir de `PackageEntry` e chamar `score_entry`. Tags/commands: iterar `split(' ')` sobre o texto — em memória, junte com `' '` não é aceitável (alocaria); para `PackageEntry` use um helper que itera `tags.iter()` — então `score_entry` recebe os campos via um trait/closure ou duas variantes que compartilham o núcleo. **Decisão:** o núcleo é `fn score_fields<'a>(source, id, name, moniker, extra: impl Iterator<Item = &'a str>, q, matcher, buf)`; `EntryView` (disco) passa `tags.split(' ').chain(commands.split(' ')).filter(|s| !s.is_empty())`, e `PackageEntry` (memória) passa `tags.iter().chain(commands.iter()).map(String::as_str)`.

2. **Busca em disco fundida** em `src/catalog/search.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Found { pub entry: PackageEntry, pub score: u32, pub name_indices: Vec<u32> }

/// Parallel scan of the catalog file: each worker reads its rowid range as borrowed text,
/// scores rows in place and keeps only (rowid, score, name_indices, lowercase name);
/// then the top `limit` rows are materialized with one query.
pub fn search_on_disk(path: &Path, query: &str, filter: Option<SourceKind>, limit: usize)
    -> Result<Vec<Found>, CatalogError>;
```

- Fase 1 (paralela, até 8 workers, mesmas faixas de rowid de `load_entries`): conexão RO + mmap por worker; `SELECT rowid, kind, id, name, moniker, tags, commands FROM packages WHERE rowid BETWEEN ?1 AND ?2 ORDER BY rowid`; ler com `row.get_ref(i)?.as_str()` (sem `String`). Aplicar filtro de fonte e `score_entry`. Guardar só os acertos: `(rowid, score, name_indices, name.to_lowercase())`.
- Concatenar os resultados dos workers **na ordem das faixas** (= ordem de rowid) e ordenar **exatamente** como `SearchIndex`: query vazia ⇒ por nome minúsculo (sort estável); senão score desc, empate por nome minúsculo (sort estável). Truncar em `limit`.
- Fase 2: `SELECT {ENTRY_COLS}, rowid FROM packages WHERE rowid IN (…)` para os rowids finais, montar `PackageEntry` com o decodificador existente e devolver `Found` na ordem da fase 1.
- Consistência: `PRAGMA data_version` antes da fase 1 e depois da fase 2 (mesma conexão-mestre); se mudou, refazer uma vez (mesma regra de `load_entries`).
- Catálogo inexistente ou sem linhas ⇒ `Ok(vec![])`.

3. **Abertura barata** em `src/catalog/mod.rs`:

```rust
/// Read-only open for an existing catalog: no schema DDL, no journal_mode write.
pub fn open_read_only(path: &Path) -> Result<Catalog, CatalogError>;
```

e `is_empty` passa a `SELECT EXISTS(SELECT 1 FROM packages)`. `ensure_catalog` (CLI) usa `open_read_only` quando o arquivo existe; arquivo ausente ⇒ segue o caminho atual (update bloqueante via `open`). Nunca chame `open_read_only` antes de o arquivo existir.

4. **Antecipação** em `src/cli/mod.rs`: em `run`, logo após obter `DataDir` e **antes** de `init_logging`/`Config::load`/runtime/`ensure_catalog`, se o comando é `Search` e `catalog.db` existe, dispare `std::thread::spawn(move || search_on_disk(&path, &query, source, limit))`. Depois que `ensure_catalog` confirmar catálogo não vazio, faça `join` e use o resultado. Se `ensure_catalog` precisou rodar o update (catálogo vazio) ou a thread devolveu erro, rode `search_on_disk` de novo, em série. Saída (tabela/JSON) idêntica à de hoje. Remova o `mem::forget` do índice nesse caminho (não há mais índice); mantenha só se o perfil mostrar custo de drop relevante.

#### Traps already paid for that apply here

- O heap padrão do Windows serializa alocações entre threads (78,5 ms em 1 thread viraram só 42,7 com 8): o ganho vem de **não alocar** por linha, não de mais threads.
- Rowids não são contíguos após `replace_source` (faixas desbalanceadas, sem perda) — os testes de borda da Task 16 continuam valendo; reutilize a partição.
- Medir pelo processo inteiro (bench.ps1), não por função.

#### The body of the tests that are not obvious

Em `tests/search.rs` (new test fn) — equivalência contra o caminho em memória num catálogo em disco com dados reais e sintéticos:

```rust
fn disk_catalog_with_many(n: usize) -> (tempfile::TempDir, std::path::PathBuf) {
    use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    let names = ["PowerToys", "Power Automate", "powertoys", "Zeal", "360安全卫士", "Visual Studio Code", "Café Läuft", "7-Zip"];
    let mk = |i: usize| NewPackage {
        id: format!("Pub{}.App{i}", i % 37),
        name: format!("{} {i}", names[i % names.len()]),
        moniker: (i % 5 == 0).then(|| format!("m{i}")),
        version: "1".into(),
        tags: vec![format!("tag{}", i % 11), "power".into()],
        commands: vec![format!("cmd{}", i % 13)],
        ..Default::default()
    };
    let winget: Vec<NewPackage> = (0..n).map(mk).collect();
    c.replace_source("winget", wingetrs::core::SourceKind::Winget, "v", 1, &winget, &WingetMaps::default()).unwrap();
    let scoop: Vec<NewPackage> = (n..n + n / 3).map(mk).collect();
    c.replace_source("scoop:main", wingetrs::core::SourceKind::Scoop, "e", 1, &scoop, &WingetMaps::default()).unwrap();
    drop(c);
    (dir, path)
}

#[test]
fn search_on_disk_matches_in_memory_search_exactly() {
    use wingetrs::catalog::Catalog;
    use wingetrs::catalog::search::{search_on_disk, SearchIndex};
    let (_d, path) = disk_catalog_with_many(6000);
    let mut idx = SearchIndex::new(Catalog::open(&path).unwrap().load_entries().unwrap());
    for (q, f, limit) in [
        ("powertoys", None, 20),
        ("power", None, 50),
        ("power", Some(SourceKind::Scoop), 50),
        ("安全", None, 10),
        ("cafe", None, 10),
        ("m10", None, 5),         // exact moniker bonus
        ("tag3", None, 30),       // tags at half weight
        ("", None, 25),           // empty query: alphabetical
        ("zzzz-no-match", None, 10),
    ] {
        let mem: Vec<(String, u32, Vec<u32>)> = idx.search(q, f, limit).into_iter()
            .map(|h| (idx.entries()[h.index].id.clone(), h.score, h.name_indices)).collect();
        let disk: Vec<(String, u32, Vec<u32>)> = search_on_disk(&path, q, f, limit).unwrap().into_iter()
            .map(|x| (x.entry.id, x.score, x.name_indices)).collect();
        assert_eq!(disk, mem, "query {q:?} filter {f:?} limit {limit}");
    }
}

#[test]
fn search_on_disk_missing_or_empty_catalog_is_empty() {
    use wingetrs::catalog::search::search_on_disk;
    let dir = tempfile::tempdir().unwrap();
    assert!(search_on_disk(&dir.path().join("nope.db"), "x", None, 10).unwrap().is_empty());
    let p = dir.path().join("empty.db");
    drop(wingetrs::catalog::Catalog::open(&p).unwrap());
    assert!(search_on_disk(&p, "x", None, 10).unwrap().is_empty());
}
```

Em `tests/catalog.rs` (new test fn):

```rust
#[test]
fn open_read_only_does_not_write_and_sees_data() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("c.db");
    let mut w = Catalog::open(&p).unwrap();
    w.replace_source("winget", SourceKind::Winget, "v", 1, &[pkg("A.A", "Alpha", "1")], &WingetMaps::default()).unwrap();
    drop(w);
    let r = Catalog::open_read_only(&p).unwrap();
    assert!(!r.is_empty().unwrap());
    assert_eq!(r.load_entries().unwrap().len(), 1);
    assert!(Catalog::open_read_only(&dir.path().join("missing.db")).is_err(), "must not create a file");
    assert!(!dir.path().join("missing.db").exists());
}
```

#### Checks beyond the steps

- Mutation proofs (`-TestCommand 'cargo test --quiet --test search'`): (1) no ordenamento da busca em disco, trocar o desempate por nome por desempate por rowid ⇒ PEGA; (2) no `score_fields`, peso de tags `s / 2` → `s` ⇒ PEGA (os dois caminhos usam a mesma função, então o teste de tags do `tests/search.rs` existente ou o novo deve pegar); (3) `open_read_only` com flag de criação ⇒ `open_read_only_does_not_write_and_sees_data` PEGA (`-TestCommand 'cargo test --quiet --test catalog'`).
- Bench `search powertoys` (3 rodadas, antes/depois) e `search ""` (lista alfabética, pior caso de alocação de nomes) colados.
- `cargo run --release -- search powertoys` e `--json` com saída idêntica à anterior (diff colado).

**Files:**
- Modify: `src/catalog/search.rs`, `src/catalog/mod.rs`, `src/cli/mod.rs`
- Test: `tests/search.rs`, `tests/catalog.rs`

**Commit:** `perf(search): fused on-disk scan, early start and cheap read-only open`

### Task 19: Arquivo de busca compacto (`search.idx`)

#### Where this fits

Decidido pelo usuário em 2026-10-08. Perfil do orquestrador (release, mediana): após a Task 18 o `search` custa **66–70 ms acima de um processo vazio** (`bench.ps1 -Baseline target/release/examples/noop.exe`). O que sobra está no SQLite: abrir conexão + primeira consulta ao `catalog.db` custa 11–15 ms (medido também em Python), ler as linhas ~24 ms numa thread, e 8 conexões paralelas competem no `prepare` (4 → 16–20 ms). Solução: o `update` grava, ao lado do catálogo, um arquivo binário só com os campos da busca; o `search` da CLI lê esse arquivo inteiro (`std::fs::read`, ~2–3 MB) e pontua em paralelo, sem SQLite.

#### What binds this task

- Arquivo: `DataDir::search_index_path()` = `<root>/search.idx` (acrescente o método em `src/cache/mod.rs`).
- Sem dependência nova (nada de memmap2/bincode): leitura com `std::fs::read`, formato próprio little-endian descrito abaixo.
- **Ranking idêntico** ao caminho em memória e a `search_on_disk`: use `Query`, `EntryView` e `score_entry` existentes em `src/catalog/search.rs`; ordenação idêntica (estável; vazia ⇒ nome minúsculo; senão score desc, desempate por nome minúsculo; entradas na ordem do catálogo, `rowid`).
- Arquivo **ausente, inválido, truncado ou de versão diferente ⇒ fallback silencioso** para `search_on_disk` (com `tracing::warn!` quando inválido). Dado externo nunca causa pânico: toda leitura com checagem de limites.
- Saída do `search` (tabela, `--json`, `--source`) idêntica à atual.
- A TUI **não** muda nesta tarefa.
- Meta: `pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'search powertoys' -Runs 15 -Baseline target/release/examples/noop.exe` com **delta median < 30 ms** (hoje 66–70). Antes/depois, 2 rodadas cada, colados. Build do baseline: `cargo build --release --example noop`.

#### Formato `search.idx` v1 (decidido)

```
offset 0  : magic  b"WRSIDX\0\x01"          (8 bytes; último byte = versão do formato = 1)
offset 8  : u32 entry_count
offset 12 : u32 reserved = 0
offset 16 : u64 body_len                    (bytes do corpo que segue)
offset 24 : u32 offsets[entry_count]        (início de cada registro, relativo ao começo do corpo)
...       : corpo: registros concatenados
fim       : magic b"WRSIDX\0\x01" de novo (trailer: detecta truncamento)
```

Registro (na ordem `rowid` do catálogo): `u8 kind` (0 = winget, 1 = scoop) seguido de 9 campos de texto, cada um `u32 len` + bytes UTF-8, ou `len = 0xFFFF_FFFF` para `None`: `source_name, id, name, moniker, version, publisher, description, tags, commands` (tags/commands como no catálogo: palavras separadas por espaço).

Validação na leitura: magic inicial e final, versão = 1, `24 + 4*entry_count + body_len + 8 == file_len`, cada offset `< body_len` e crescente, cada campo dentro do registro, `std::str::from_utf8` em cada campo usado (erro ⇒ arquivo inválido ⇒ fallback).

#### Interfaces (decididas)

```rust
// src/catalog/index_file.rs  (pub mod index_file; em catalog/mod.rs)
pub const MAGIC: &[u8; 8] = b"WRSIDX\0\x01";

#[derive(Debug, thiserror::Error)]
pub enum IndexFileError {
    #[error("índice de busca inválido: {0}")]
    Invalid(&'static str),
    #[error("E/S no índice de busca: {0}")]
    Io(String),
    #[error(transparent)]
    Catalog(#[from] super::CatalogError),
}

/// Serializes the catalog's search columns (load_entries order) and writes `path` atomically
/// (write `path.tmp`, then rename; retry the rename up to 5 times, 20 ms apart, because a
/// concurrent reader on Windows can make it fail transiently).
pub fn write(catalog: &super::Catalog, path: &std::path::Path) -> Result<usize, IndexFileError>;

/// Parsed, validated view over the bytes of a search.idx (borrows `bytes`).
pub struct IndexView<'a> { /* bytes, offsets */ }
impl<'a> IndexView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<IndexView<'a>, IndexFileError>;
    pub fn len(&self) -> usize;
    pub fn view(&self, i: usize) -> Result<(EntryView<'a>, RecordRest<'a>), IndexFileError>;
    pub fn entry(&self, i: usize) -> Result<PackageEntry, IndexFileError>; // materializes one record
}
/// Remaining fields of a record not needed for scoring.
pub struct RecordRest<'a> { pub source_name: &'a str, pub version: &'a str, pub publisher: Option<&'a str>, pub description: Option<&'a str> }

/// Reads `path` and searches it in parallel (same chunking idea as SearchIndex: up to 8 workers
/// over contiguous record ranges, each with its own nucleo Matcher). Ok(None) when the file is
/// missing; Err when it is invalid (caller falls back).
pub fn search(path: &std::path::Path, query: &str, filter: Option<SourceKind>, limit: usize)
    -> Result<Option<Vec<super::search::Found>>, IndexFileError>;
```

(`EntryView` e `Found` são os tipos já existentes em `src/catalog/search.rs`.)

#### Integração

1. **Update** (`src/update.rs`, `run_update`): depois do último `replace_source`/salvar `http.json`, ainda sob o lock: se alguma fonte teve `Outcome::Updated` **ou** o `search.idx` não existe, chame `index_file::write` (abrindo o catálogo). Falha ao gravar o índice ⇒ `tracing::warn!` e segue (o `search` cai no fallback); não transforma o update em erro.
2. **CLI `search`** (`src/cli/mod.rs`): a thread antecipada (guard `EarlySearch` da Task 18) passa a tentar `index_file::search` primeiro; `Ok(None)`/`Err` ⇒ `search_on_disk` como hoje.
3. **`ensure_catalog` no caminho do `search`:** se `search.idx` existe e é válido, não é preciso abrir o SQLite para checar "catálogo vazio" — índice com `entry_count > 0` prova que há catálogo. Mantenha o disparo de refresh em segundo plano pelo `http.json` (barato). Se o índice não existe ⇒ caminho atual.
4. **Concorrência:** `fs::read` lê e fecha o arquivo (o Rust no Windows abre com share read/write/delete), então o rename do writer não deve falhar por causa do leitor; mesmo assim, faça o retry do rename e teste-o.

#### Traps already paid for that apply here

- Gargalo medido é abrir/preparar SQLite e alocar `String` por linha — o caminho novo não pode reintroduzir `PackageEntry` por registro durante a varredura; materialize só os `limit` finais (`IndexView::entry`).
- Ordem dos registros = ordem de `load_entries` (rowid); o desempate estável depende disso.
- Medir com `-Baseline` (a carga da máquina variou o piso de 38 a 67 ms na mesma sessão).

#### The body of the tests that are not obvious

`tests/index_file.rs` (new):

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::index_file::{self, IndexView, MAGIC};
use wingetrs::catalog::search::{search_on_disk, SearchIndex};
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;

fn catalog(n: usize) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    let names = ["PowerToys", "Power Automate", "powertoys", "Zeal", "360安全卫士", "Visual Studio Code", "Café Läuft", "7-Zip"];
    let mk = |i: usize| NewPackage {
        id: format!("Pub{}.App{i}", i % 37),
        name: format!("{} {i}", names[i % names.len()]),
        moniker: (i % 5 == 0).then(|| format!("m{i}")),
        version: format!("1.{i}"),
        publisher: (i % 3 == 0).then(|| "Pub".to_string()),
        description: (i % 4 == 0).then(|| format!("desc {i} ✓")),
        tags: vec![format!("tag{}", i % 11), "power".into()],
        commands: vec![format!("cmd{}", i % 13)],
        ..Default::default()
    };
    let w: Vec<NewPackage> = (0..n).map(mk).collect();
    c.replace_source("winget", SourceKind::Winget, "v", 1, &w, &WingetMaps::default()).unwrap();
    let s: Vec<NewPackage> = (n..n + n / 3).map(mk).collect();
    c.replace_source("scoop:main", SourceKind::Scoop, "e", 1, &s, &WingetMaps::default()).unwrap();
    drop(c);
    (dir, path)
}

#[test]
fn round_trip_every_entry() {
    let (d, db) = catalog(500);
    let idx = d.path().join("search.idx");
    let cat = Catalog::open(&db).unwrap();
    assert_eq!(index_file::write(&cat, &idx).unwrap(), 666);
    let bytes = std::fs::read(&idx).unwrap();
    let v = IndexView::parse(&bytes).unwrap();
    let mem = cat.load_entries().unwrap();
    assert_eq!(v.len(), mem.len());
    for (i, e) in mem.iter().enumerate() {
        assert_eq!(&v.entry(i).unwrap(), e, "record {i}");
    }
    assert!(!idx.with_extension("idx.tmp").exists());
}

#[test]
fn index_search_matches_in_memory_and_sqlite_exactly() {
    let (d, db) = catalog(6000);
    let idx = d.path().join("search.idx");
    let cat = Catalog::open(&db).unwrap();
    index_file::write(&cat, &idx).unwrap();
    let mut mem = SearchIndex::new(cat.load_entries().unwrap());
    for (q, f, limit) in [
        ("powertoys", None, 20), ("power", None, 50), ("power", Some(SourceKind::Scoop), 50),
        ("安全", None, 10), ("cafe", None, 10), ("m10", None, 5), ("tag3", None, 30),
        ("", None, 25), ("zzzz-no-match", None, 10),
    ] {
        let a: Vec<(String, u32, Vec<u32>)> = mem.search(q, f, limit).into_iter()
            .map(|h| (mem.entries()[h.index].id.clone(), h.score, h.name_indices)).collect();
        let b: Vec<(String, u32, Vec<u32>)> = index_file::search(&idx, q, f, limit).unwrap().unwrap().into_iter()
            .map(|x| (x.entry.id, x.score, x.name_indices)).collect();
        let c: Vec<(String, u32, Vec<u32>)> = search_on_disk(&db, q, f, limit).unwrap().into_iter()
            .map(|x| (x.entry.id, x.score, x.name_indices)).collect();
        assert_eq!(b, a, "index vs memory: {q:?} {f:?}");
        assert_eq!(c, a, "sqlite vs memory: {q:?} {f:?}");
    }
}

#[test]
fn missing_file_is_none_and_damaged_files_are_errors_never_panics() {
    let (d, db) = catalog(300);
    let idx = d.path().join("search.idx");
    assert!(index_file::search(&idx, "x", None, 10).unwrap().is_none());
    index_file::write(&Catalog::open(&db).unwrap(), &idx).unwrap();
    let good = std::fs::read(&idx).unwrap();
    // truncations
    for cut in [0, 7, 8, 23, 24, good.len() / 2, good.len() - 8, good.len() - 1] {
        assert!(IndexView::parse(&good[..cut]).is_err(), "cut {cut}");
    }
    // wrong version byte
    let mut bad = good.clone();
    bad[7] = 2;
    assert!(IndexView::parse(&bad).is_err());
    // count claims more entries than exist
    let mut bad = good.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(IndexView::parse(&bad).is_err());
    // deterministic byte flips anywhere: parse/search may fail, but must never panic
    let mut x: u32 = 0x9E37_79B9;
    for _ in 0..500 {
        x ^= x << 13; x ^= x >> 17; x ^= x << 5;
        let mut b = good.clone();
        let pos = (x as usize) % b.len();
        b[pos] ^= (x >> 24) as u8 | 1;
        if let Ok(v) = IndexView::parse(&b) {
            for i in 0..v.len() { let _ = v.view(i); let _ = v.entry(i); }
        }
    }
    assert_eq!(&good[..8], MAGIC);
}
```

Em `tests/update.rs` (new test fn), usando o `setup()` existente:

```rust
#[tokio::test]
async fn update_writes_search_index_and_rebuilds_it_when_missing() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    run_update(&data, &ep, &Http::new().unwrap(), true, wingetrs::update::LockWait::from_env()).await.unwrap();
    let bytes = std::fs::read(data.search_index_path()).unwrap();
    assert_eq!(wingetrs::catalog::index_file::IndexView::parse(&bytes).unwrap().len(), 28);
    std::fs::remove_file(data.search_index_path()).unwrap();
    // Fresh (no network needed): the missing index alone must trigger a rebuild.
    run_update(&data, &ep, &Http::new().unwrap(), false, wingetrs::update::LockWait::from_env()).await.unwrap();
    assert!(data.search_index_path().exists());
}
```

Confira a assinatura real de `run_update`/`LockWait` com `rust-analyzer symbols < src/update.rs` e ajuste só a chamada, nunca as asserções.

#### Checks beyond the steps

- Mutation proofs (`-TestCommand 'cargo test --quiet --test index_file'`): (1) remover a checagem do trailer ⇒ PEGA (truncamento `good.len() - 8`); (2) trocar o desempate por nome por desempate por índice no `search` do arquivo ⇒ PEGA; (3) no update, remover a condição "índice ausente" ⇒ `update_writes_search_index_and_rebuilds_it_when_missing` PEGA (`--test update`).
- `cargo run --release -- update` (sem `--force`; a condição "índice ausente" deve gerar o `search.idx` real) e depois `search powertoys`, `search powertoys --json`, `search "" -n 5`, `search powertoys --source scoop`: saída idêntica à do binário anterior (diff colado). Tamanho do `search.idx` real colado.
- Bench com `-Baseline` antes/depois (2 rodadas cada), colado.

**Files:**
- Create (new): `src/catalog/index_file.rs`, `tests/index_file.rs`
- Modify: `src/catalog/mod.rs`, `src/cache/mod.rs`, `src/update.rs`, `src/cli/mod.rs`
- Test: `tests/update.rs`

**Commit:** `perf(search): compact search.idx written by update and read by search`

---

## Fechamento das pendências da fase 1 (decidido pelo usuário em 2026-10-09)

Ordem: Task 20 sozinha (mede tempo) → Tasks 22 e 23 em paralelo → Task 21 (o oráculo dela depende da normalização da 22). Benchmarks sempre com `pwsh -File scripts/bench.ps1 ... -Baseline target/release/examples/noop.exe` (build: `cargo build --release --example noop`), máquina ociosa.

### Task 20: `list` — medir por coletor e otimizar o maior custo

#### Where this fits

`list` mede 455–464 ms de mediana com máximos ocasionais > 500 ms (meta da spec §9.2: < 500 ms). Quebra conhecida da Task 12 (release, antes dos frameworks MSIX): abrir catálogo 12 ms, `scan_all` 285 ms, `NameNormalizer::new` 60 ms, correlação 65 ms. A Task 17 tentou reduzir chamadas `DisplayName` do MSIX e **não teve ganho medido** (revertida em 9192e7b): o palpite estava errado.

#### What binds this task

- **Medir antes de mudar.** Adicione spans `tracing` (nível info) por etapa: `scan_arp` (por hive), `scan_upgrade_codes`, `scan_msix` (enumeração e leitura de propriedades separadas), `scan_scoop`, `open_catalog`, `normalizer_new`, `correlate`, `output`. Rode `target/release/wingetrs.exe list -v` 10× e cole a mediana por etapa (os spans saem no stderr com `-v`).
- Só depois otimize **a maior etapa medida**. Candidatas permitidas (escolha pelo número, não pelo palpite): `NameNormalizer` construído em paralelo com a coleta (thread desde o início do `list`); abrir o catálogo e preparar statements em paralelo com a coleta; leitura de registro mais enxuta (ler só os valores necessários com uma chamada por valor, sem enumerar todos); reduzir a disputa das 8 threads do MSIX.
- **Saída idêntica:** `target/release/wingetrs.exe list --json` antes/depois com o mesmo conjunto de linhas (ordene e compare; cole o resultado).
- Meta: mediana de `list` com `-Baseline` **delta < 350 ms** (≈ 390 ms absolutos numa máquina ociosa). Antes/depois, 2 rodadas cada, colados. Meta não atingida: teto medido + causa + follow-up; nunca afrouxe.
- Sem dependência nova.

#### Traps already paid for that apply here

- Palpites sobre MSIX já falharam uma vez: decida pelo span.
- O live test `live_list_runs_under_budget` roda em `--release`; mantenha o limite.

#### Checks beyond the steps

- Esta tarefa é de desempenho: **no mutation proof** para a otimização (declare). Se você criar função pura nova (ex.: escolha de valores do registro), ela leva teste e mutação normais.
- Tabela de spans antes/depois colada no relatório.

**Files:**
- Modify: `src/installed/*.rs`, `src/cli/mod.rs` (só o caminho `list`), `src/installed/correlate.rs` (só se o span apontar)
- Test: `tests/installed.rs`, `tests/correlate.rs` (se criar função pura)

**Commit:** `perf(list): measure per-collector spans and cut the largest cost`

### Task 21: Desempate de correlações ambíguas

#### Where this fits

O oráculo (Task 17) tem 110/116; 5 das 6 divergências são `Ambiguous` (medido, relatório de validação, seção Divergências):

| winget | wingetrs | Causa |
|---|---|---|
| Google.AndroidStudio | Ambiguous [AndroidStudio, .Beta, .Canary] (ProductCode) | ProductCode compartilhado entre canais |
| Gyan.FFmpeg | Ambiguous [FFmpeg, .Essentials, .Shared] (NameAndPublisher) | nome/publisher iguais entre variantes |
| RubyInstallerTeam.RubyWithDevKit.4.0 | Ambiguous [Ruby.3.4, RubyWithDevKit.3.4] (ProductCode) | instalado 3.4.9-1; o winget provavelmente usa o próprio registro de instalação |
| Devolutions.UniGetUI | Ambiguous [UniGetUI, MartiCliment.UniGetUI.Pre-Release] (ProductCode) | ProductCode compartilhado com pré-release |
| wez.wezterm | Ambiguous [wezterm, wezterm.nightly] (ProductCode) | ProductCode compartilhado com nightly |
| MSYS2.MSYS2 | Unmatched | catálogo sem ProductCode; nome normalizado `msys2installer` vs ARP `MSYS2` |

#### What binds this task

- **Regra de ouro (spec §5.4): nunca escolher em silêncio um candidato que pode estar errado.** Um desempate só resolve quando o critério é objetivo; senão continua `Ambiguous`.
- Pesquise primeiro como o winget resolve: (a) leia no winget-cli (via `gh api repos/microsoft/winget-cli/contents/...` ou `gh search code --repo microsoft/winget-cli`) o código de correlação de pacotes instalados com o catálogo (procure `CorrelateForInstalled`, `ARPCorrelation`, `GetMatchingPackages`, `IsCompatible`, "tracking catalog"); salve o trecho relevante em `docs/reference/winget-cli/` com o commit no nome; (b) verifique se o winget mantém um **catálogo de rastreamento** local (pacotes instalados via winget) legível em modo somente leitura (ex.: sob `%LOCALAPPDATA%\Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\`), e qual o formato. **Somente leitura**: nunca escreva em arquivos do winget.
- Regras candidatas, na ordem, cada uma aplicada só sobre os candidatos já empatados:
  1. Registro de rastreamento do winget (se existir e for legível): o ID que o winget instalou vence.
  2. Faixa ARP: manter só candidatos cuja faixa `[arp_min_version, arp_max_version]` contém a versão instalada (quando a faixa existe).
  3. Canal: se exatamente um candidato tem ID que é prefixo dos demais seguido de `.` e um sufixo de canal conhecido (`Beta`, `Canary`, `Dev`, `Nightly`, `Preview`, `Pre-Release`, `Insiders`, `Essentials`, `Shared`), **e** o nome exibido instalado não contém esse sufixo, o ID base vence.
  4. Senão: continua `Ambiguous`.
- Cada regra é uma função pura testável (sem I/O); a leitura do rastreamento fica em `*_win.rs` se usar APIs do Windows, ou em módulo próprio se for só SQLite.
- `ListRow` passa a registrar a regra de desempate usada (`tiebreak: Option<String>` ou enum) para depuração no `--json`.
- Meta: oráculo ≥ 112/116 sem nenhum falso positivo novo (nenhum ID que o winget mostra como outro). Reexecute o oráculo como na Task 15/17 e cole.

#### The body of the tests that are not obvious

Escreva com dados sintéticos (catálogo em memória com `replace_source`), um teste por regra, e um teste de **não-resolução**: dois candidatos sem faixa ARP, sem sufixo de canal reconhecido e sem rastreamento ⇒ continua `Ambiguous` com os dois. Inclua o caso `Gyan.FFmpeg` × `.Essentials` × `.Shared` com nome exibido "FFmpeg" ⇒ `Gyan.FFmpeg`, e o caso inverso (nome exibido "FFmpeg Essentials") ⇒ não escolhe o base.

#### Checks beyond the steps

- Mutation proofs (`--test correlate`): uma por regra (ex.: inverter a condição de contém da faixa ARP; remover a exigência de o nome exibido não conter o sufixo).
- Oráculo antes/depois colado com a lista de divergências restantes.

**Files:**
- Modify: `src/installed/correlate.rs`, `src/installed/mod.rs`
- Create (new): `src/installed/tiebreak.rs` (regras puras) e, se houver rastreamento legível, `src/installed/winget_tracking.rs`
- Test: `tests/correlate.rs`

**Commit:** `feat(list): objective tie-breaks for ambiguous correlations`

### Task 22: Normalização Unicode fiel ao winget (NFKC e case folding completo)

#### Where this fits

O porte do `NameNormalizer` (Task 10) passa 1137/1137 no corpus golden, mas o relatório registrou duas aproximações: `Utility::Normalize` (normalização Unicode, linha 35 de `docs/reference/winget-cli/AppInstallerCommonCore_NameNormalization.cpp`) não é aplicada, e `FoldCase` (linhas 135, 200, 366) é aproximado por `str::to_lowercase`. Diferenças reais: `ß`→`ss`, `ﬁ`→`fi`, sigma final, caracteres fullwidth (`ＡＢＣ`), `İ`.

#### What binds this task

- Confirme no código do winget qual forma de normalização `Utility::Normalize` usa e como `FoldCase` é implementado: busque `Normalize(` e `FoldCase(` em `src/AppInstallerSharedLib` via `gh api repos/microsoft/winget-cli/contents/...?ref=<commit em docs/reference/winget-cli/SOURCE_COMMIT.txt>`; salve os arquivos em `docs/reference/winget-cli/` como os demais. Implemente a forma que o código mostrar (a expectativa é NFKC + ICU `u_strFoldCase` com opção default — confirme, não assuma).
- Dependência nova **autorizada**: `unicode-normalization` (verifique a versão atual no crates.io e que compila com o conjunto atual). Case folding completo sem crate: gere uma tabela Rust a partir de `https://www.unicode.org/Public/UCD/latest/ucd/CaseFolding.txt` (status `C` + `F`), com um script em `scripts/gen_casefold.py` e o arquivo gerado `src/installed/casefold_table.rs` commitado (cabeçalho com a versão Unicode e a URL). `fold_case(s)` usa a tabela; caracteres fora dela passam inalterados.
- Onde o porte chama `to_lowercase` em lugar de `FoldCase`, troque por `fold_case`. Aplique a normalização Unicode onde o C++ aplica.
- O golden **continua 1137/1137** (`MEASURED_GOLDEN_PASS` não pode cair).

#### The body of the tests that are not obvious

Em `tests/normalize.rs`:

```rust
#[test]
fn full_case_folding_and_compatibility_forms() {
    use wingetrs::installed::normalize::{fold_case, NameNormalizer};
    assert_eq!(fold_case("Straße"), "strasse");
    assert_eq!(fold_case("ﬁle"), "file");
    assert_eq!(fold_case("ΟΔΟΣ"), "οδοσ");
    let n = NameNormalizer::new();
    // Fullwidth letters normalize to the same key as ASCII.
    assert_eq!(n.index_keys("ＶＬＣ media player", "VideoLAN").name, n.index_keys("VLC media player", "VideoLAN").name);
    assert_eq!(n.index_keys("Straße App", "X").name, n.index_keys("Strasse App", "X").name);
}
```

Se o código do winget mostrar que alguma dessas expectativas não vale (ex.: NFKC não aplicado ao nome), o código do winget vence: ajuste a asserção correspondente, cite a linha do C++ no comentário do teste e registre no relatório.

#### Checks beyond the steps

- Mutation proofs (`--test normalize`): (1) trocar `fold_case` por `to_lowercase` em `index_keys` ⇒ PEGA; (2) remover a normalização Unicode ⇒ PEGA (pelo caso fullwidth).
- Golden: linha `golden: 1137/1137` colada.
- Tempo: `NameNormalizer::new` + `index_keys` não podem piorar o `list` mais que 10 ms (medir com o span da Task 20 se já existir, ou com `cargo test --release ... live_list`).

**Files:**
- Modify: `src/installed/normalize.rs`, `Cargo.toml`
- Create (new): `src/installed/casefold_table.rs`, `scripts/gen_casefold.py`
- Test: `tests/normalize.rs`

**Commit:** `feat(normalize): NFKC and full Unicode case folding as in winget`

### Task 23: Ajuda da CLI completa e em português

#### Where this fits

`wingetrs <cmd> --help` mostra opções sem descrição (`--force`, `--source`, `--json`, `--upgradable`, `--version`, `<QUERY>`, `<ID>`) e textos do clap em inglês ("Print help", "Print version", "Usage", "Commands", "Options", "possible values", "help: Print this message or the help of the given subcommand(s)").

#### What binds this task

- Toda opção e argumento ganha descrição curta em português (doc comment no `Cli`/`Command`).
- Textos embutidos do clap em português: desabilite as flags padrão (`disable_help_flag`, `disable_version_flag`, `disable_help_subcommand`) e declare as suas (`-h/--help` "Mostra a ajuda", `-V/--version` "Mostra a versão"); use `help_template` com cabeçalhos "Uso:", "Comandos:", "Opções:", "Argumentos:".
- `--glyphs` só aparece na ajuda raiz (deixe de ser `global = true`, ou esconda nos subcomandos), porque só afeta a TUI.
- Exemplos no `after_help` de cada subcomando (ex.: `wingetrs search powertoys --source scoop -n 5`).
- Os testes de `tests/cli_parse.rs` continuam passando; ajuste-os só se a mudança de `--glyphs` exigir, mantendo a intenção.

#### The body of the tests that are not obvious

Snapshot com `insta` da ajuda de cada comando (`Cli::command().render_long_help()` e de cada subcomando via `find_subcommand_mut`) e um teste que garante que nenhuma opção ficou sem descrição:

```rust
#[test]
fn every_argument_has_a_portuguese_description() {
    use clap::CommandFactory;
    let mut root = wingetrs::cli::Cli::command();
    root.build();
    let mut cmds = vec![root.clone()];
    cmds.extend(root.get_subcommands().cloned());
    for c in cmds {
        for a in c.get_arguments() {
            let help = a.get_help().map(|h| h.to_string()).unwrap_or_default();
            assert!(!help.trim().is_empty(), "{} --{:?} sem descrição", c.get_name(), a.get_id());
        }
    }
    let text = wingetrs::cli::Cli::command().render_long_help().to_string();
    for en in ["Print help", "Usage:", "Options:", "Commands:"] {
        assert!(!text.contains(en), "texto em inglês: {en}");
    }
}
```

#### Checks beyond the steps

- Mutation proof (`--test cli_parse`): apagar a descrição de uma opção (ex.: o doc comment de `json` em `List`) ⇒ PEGA.
- Cole no relatório a saída real de `wingetrs --help` e `wingetrs search --help`.

**Files:**
- Modify: `src/cli/mod.rs`
- Test: `tests/cli_parse.rs`, `tests/snapshots/*`

**Commit:** `feat(cli): complete Portuguese help with examples`
