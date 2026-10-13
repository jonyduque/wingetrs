# wingetrs Fase 2a (Scoop: instalar, remover, atualizar) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Neste repositório o laço por tarefa está em `.claude/skills/run-task/SKILL.md`; cada tarefa vira um brief autocontido via `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-10-wingetrs-fase2a.md`.

**Goal:** `wingetrs install|uninstall|upgrade|bucket|repair` para apps do Scoop com manifestos simples, numa raiz própria, com plano puro, execução com staging e rollback, limpeza de resíduos e seleção interativa por setas.

**Architecture:** `src/install/` separa decisão (puro: manifesto → `InstallPlan`; resíduos → classificação) de efeito (executor sobre traits `Fs`/`EnvStore`/`Shortcuts`/`Trash`/`RegistryScan`, com implementações reais em `*_win.rs`). Estado próprio em `state.db`. CLI e TUI consomem a mesma API e o mesmo componente de seleção.

**Tech Stack:** Rust 1.99 (edition 2024), crates da fase 1 + `sha1` 0.11.0, `md-5` 0.11.0, `junction` 2.1.0, e features novas do `windows` 0.62.2 (`Win32_UI_Shell`, `Win32_System_Com`, `Win32_UI_WindowsAndMessaging`, `Win32_Foundation`, `Win32_Storage_FileSystem`, `Win32_UI_Shell_Common`, `Win32_System_Com_StructuredStorage`) — todas verificadas juntas num crate de sondagem em 2026-10-10.

**Spec:** `docs/superpowers/specs/2026-10-10-wingetrs-fase2a-scoop-design.md`

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes de cada tarefa.
- Raiz: `%LOCALAPPDATA%\wingetrs` (`DataDir::from_env()`); apps em `apps\<app>\<versão>`, `current` = junction, `persist\<app>`, `shims\`, `backups\`, `state.db`, `install.lock`.
- Fora da raiz o wingetrs escreve **somente**: `HKCU\Environment` (entrada `…\wingetrs\shims` e variáveis dos manifestos), `%APPDATA%\Microsoft\Windows\Start Menu\Programs\wingetrs Apps\`, a Lixeira, e chaves `HKCU` de resíduo confirmadas. Nunca `HKLM` nem `C:\Program Files`.
- Testes **nunca** tocam `HKCU\Environment` real nem o Menu Iniciar real: as implementações reais leem `WINGETRS_TEST_ENV_KEY` (ex.: `Software\wingetrs-test\env-<pid>`) e `WINGETRS_TEST_SHORTCUT_DIR` quando definidas, e testes ao vivo as definem.
- Nível suportado: manifestos sem `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`, `installer`, `uninstaller`, `psmodule`, `innosetup`; downloads `.zip`, `.tar.gz`/`.tgz`, ou arquivo solto (inclui `.exe`); demais extensões (`.7z`, `.msi`, `.rar`, `.nupkg`, `.tar.xz`, `.tar.bz2`) ⇒ `NãoSuportado`. Manifesto sem hash para alguma URL ⇒ `NãoSuportado`.
- Hashes: `sha256` (padrão, sem prefixo), `sha1:`, `sha512:`, `md5:` — comparação case-insensitive em hex.
- Lançador de shim: kiennq/scoop-better-shimexe **v3.2.2** (MIT). URLs `https://github.com/kiennq/scoop-better-shimexe/releases/download/v3.2.2/shimexe-{x86_64|aarch64|x86}.zip`; SHA-256 dos zips: x86_64 `27ee2cffa9d6f5f72f39a4f60d23b6ac3390db843a3b2eefb52d64daa80a8440`, aarch64 `a5896c99527ec8f9effe445c19a2d9cf2a482e6eb36d72660612b41b50d3a250`, x86 `05a1e35861ad8d7033e544e5ea058e22dd2208edd53dd9ed9da9867a24fe8375`; `shim.exe` dentro do zip x86_64: `caf80022ea9ea756a4bf18f37fd85615b65c8a5d45ec7f6cad1f91c844ada921` (161 280 bytes). Medido: overhead do shim ≈ 48 ms por execução (dois processos), aceito.
- Formato `.shim` (do scoop, `lib/core.ps1`): linha `path = "<caminho absoluto>"` e, se houver, `args = <args>`; wrappers `.cmd` (`@rem <path>` + `@"<path>" <args> %*`) para `.bat`/`.cmd`, e `.cmd` + `.ps1` para `.ps1`, `.cmd` com `java -jar` para `.jar` — ver Task 30.
- Mensagens ao usuário em português; código em inglês. Regras ⚙ do `CLAUDE.md` valem (gate barra).
- Sem dependência fora das listadas acima.

## Review Focus

1. **Zip/tar com caminho malicioso** (`../`, absoluto `C:\…`, `\\?\`) ⇒ extração recusada, nada escrito fora do staging. Teste na Task 26.
2. **`Path` do usuário com `%VAR%`, `REG_EXPAND_SZ`, `;` duplicado, entradas com caixa/barra final diferentes** ⇒ só a nossa entrada muda; o resto fica byte a byte. Teste na Task 27 (funções puras) e Task 32 (registro real em chave de teste).
3. **App aberto durante upgrade/uninstall** ⇒ para antes de mudar qualquer coisa, aponta o arquivo. Teste na Task 33 (detecção) e Task 37 (fluxo).
4. **Duas instalações simultâneas** ⇒ a segunda espera o `install.lock` (CLI) ou recebe `Ocupado` (TUI/`--no-wait`). Teste na Task 37.
5. **Nome de atalho com subpasta ou caracteres inválidos** (`7-Zip\7-Zip File Manager`, `a:b?`) ⇒ subpastas criadas dentro de `wingetrs Apps`, caracteres inválidos trocados por `_`, nunca escapando para fora. Teste na Task 28 (plano) e Task 32 (real).

## Ordem, dependências e tier

| Task | Título | Depende de | Tier |
|---|---|---|---|
| 24 | Manifesto scoop tipado e checagem de suporte | — | transcrição |
| 25 | Hashes e download com cache | — | transcrição |
| 26 | Extração (zip, tar.gz, solto) segura | — | transcrição |
| 27 | Layout da raiz, traits de efeito, fakes e PATH puro | — | transcrição |
| 28 | Planejador puro (`InstallPlan`) | 24, 27 | **design** |
| 29 | Estado (`state.db`, `install.json`) | 27 | transcrição |
| 30 | Shims: lançador, PE subsystem, wrappers | 25, 27 | transcrição |
| 31 | Executor com staging e rollback | 25, 26, 28, 29, 30 | **design** |
| 32 | `EnvStore` e `Shortcuts` reais (Windows) | 27 | **design** |
| 33 | `Trash`, `RegistryScan` e detecção de arquivo em uso (Windows) | 27 | transcrição |
| 34 | Resíduos: classificação e plano de limpeza | 27, 29 | **design** |
| 35 | Resolução do alvo e seletor por setas | — | transcrição |
| 36 | Buckets: `add/list/rm` | — | transcrição |
| 37 | Operações: install/upgrade/uninstall/repair com deps e lock | 28–35 | **design** |
| 38 | CLI: comandos, confirmação, dry-run, progresso | 36, 37 | transcrição |
| 39 | TUI: teclas, confirmação, progresso | 37 | transcrição |
| 40 | Ciclo completo, prova ao vivo e benchmark | 38, 39 | **design** (relatório com evidência) |

Paralelismo seguro: {24, 25, 26, 27, 35, 36} não compartilham arquivos (cada uma cria os seus) — exceto `src/install/mod.rs` e `Cargo.toml`: **a Task 27 cria `src/install/mod.rs` com todos os `pub mod` vazios e o `Cargo.toml` com as dependências novas; as demais só preenchem seus arquivos.** Por isso a 27 roda primeiro e sozinha.

---

### Task 27: Layout da raiz, traits de efeito, fakes e PATH puro

#### Where this fits

Fundação da fase 2a. Cria o módulo `install` (com os submódulos vazios que as outras tarefas preenchem), as dependências novas, o layout de caminhos da raiz, os traits pelos quais todo efeito colateral passa (para testar sem tocar o Windows), fakes em memória desses traits, um utilitário de "fotografia" de diretório para provar que nada mudou, e as funções puras de edição do `Path`.

#### What binds this task

- `Cargo.toml`: acrescente exatamente `sha1 = "0.11.0"`, `md-5 = "0.11.0"`, `junction = "2.1.0"` (em `[target.'cfg(windows)'.dependencies]` o `junction`) e as features do `windows` listadas em Global Constraints, além das que já existem.
- `src/install/mod.rs` declara: `pub mod download; pub mod executor; pub mod extract; pub mod layout; pub mod manifest; pub mod path_edit; pub mod plan; pub mod planner; pub mod residue; pub mod shim; pub mod snapshot; pub mod state; pub mod target; pub mod traits;` e, com `#[cfg(windows)]`, `pub mod env_win; pub mod shortcut_win; pub mod trash_win; pub mod registry_win; pub mod fs_win;`. Cada arquivo que não é desta tarefa é criado com só um `//!` de uma linha. `src/lib.rs` ganha `pub mod install;`.
- O `Fs` real é o sistema de arquivos (std) restrito a um diretório raiz (testes usam `tempdir`); junction via crate `junction` no Windows. **Env, atalhos, Lixeira e registro** são traits com fakes, porque são estado global do usuário.
- Edição do `Path` é **pura** e preserva tudo que não é a entrada alvo, byte a byte.

**Files:**
- Create (new): `src/install/mod.rs`, `src/install/layout.rs`, `src/install/traits.rs`, `src/install/path_edit.rs`, `src/install/snapshot.rs`, demais submódulos vazios listados acima
- Modify: `Cargo.toml`, `src/lib.rs`, `src/cache/mod.rs` (método `DataDir::install_layout()`)
- Test (new): `tests/install_foundation.rs`

#### Interfaces

```rust
// src/install/layout.rs
#[derive(Debug, Clone)]
pub struct Layout { root: std::path::PathBuf }
impl Layout {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Layout;
    pub fn root(&self) -> &std::path::Path;
    pub fn apps(&self) -> PathBuf;                         // root/apps
    pub fn app_dir(&self, app: &str) -> PathBuf;           // root/apps/<app>
    pub fn version_dir(&self, app: &str, v: &str) -> PathBuf;
    pub fn staging_dir(&self, app: &str, v: &str) -> PathBuf; // root/apps/<app>/.staging-<v>
    pub fn current(&self, app: &str) -> PathBuf;           // root/apps/<app>/current
    pub fn persist_dir(&self, app: &str) -> PathBuf;       // root/persist/<app>
    pub fn shims(&self) -> PathBuf;                        // root/shims
    pub fn launcher(&self) -> PathBuf;                     // root/shims/.launcher/shim.exe
    pub fn backups(&self) -> PathBuf;                      // root/backups
    pub fn state_db(&self) -> PathBuf;                     // root/state.db
    pub fn install_lock(&self) -> PathBuf;                 // root/install.lock
}
// src/cache/mod.rs
impl DataDir { pub fn install_layout(&self) -> crate::install::layout::Layout; } // Layout::new(self.root())

// src/install/traits.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvValue { pub value: String, pub expand: bool } // expand = REG_EXPAND_SZ
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EffectError(pub String);
pub trait EnvStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<EnvValue>, EffectError>;
    fn set(&self, name: &str, value: &EnvValue) -> Result<(), EffectError>;
    fn remove(&self, name: &str) -> Result<(), EffectError>;
    fn broadcast(&self);
}
pub trait Shortcuts: Send + Sync {
    /// `link` is the full .lnk path (already under the shortcut root).
    fn create(&self, link: &Path, target: &Path, args: Option<&str>, icon: Option<&Path>) -> Result<(), EffectError>;
    fn remove(&self, link: &Path) -> Result<(), EffectError>;
    fn root(&self) -> PathBuf; // Start Menu\Programs\wingetrs Apps (or test dir)
}
pub trait Trash: Send + Sync { fn recycle(&self, path: &Path) -> Result<(), EffectError>; }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegHit { pub key: String, pub value_name: Option<String>, pub data: String }
pub trait RegistryScan: Send + Sync {
    /// Values under each HKCU-relative root whose string data contains `needle` (case-insensitive).
    fn values_containing(&self, roots: &[&str], needle: &str) -> Result<Vec<RegHit>, EffectError>;
    /// Direct subkeys of `parent` (HKCU-relative) whose name equals one of `names` (case-insensitive).
    fn subkeys_named(&self, parent: &str, names: &[&str]) -> Result<Vec<String>, EffectError>;
    fn export(&self, key: &str, file: &Path) -> Result<(), EffectError>;
    fn delete_value(&self, key: &str, value_name: &str) -> Result<(), EffectError>;
    fn delete_key(&self, key: &str) -> Result<(), EffectError>;
}
pub struct Effects<'a> { pub env: &'a dyn EnvStore, pub shortcuts: &'a dyn Shortcuts, pub trash: &'a dyn Trash, pub registry: &'a dyn RegistryScan }
pub mod fake { /* FakeEnv, FakeShortcuts, FakeTrash, FakeRegistry: Mutex<BTreeMap<..>> state; each has `dump()` returning a comparable snapshot (BTreeMap/Vec) */ }

// src/install/path_edit.rs
/// Appends `entry` unless an equivalent entry exists (case-insensitive, ignoring one trailing '\').
/// Never touches other entries: the result is `existing` + (";" if existing non-empty and not ending in ';') + entry.
pub fn path_add(existing: &str, entry: &str) -> String;
/// Removes every entry equivalent to `entry`; all other bytes (including empty segments, %VARS%) stay.
pub fn path_remove(existing: &str, entry: &str) -> String;
pub fn path_contains(existing: &str, entry: &str) -> bool;

// src/install/snapshot.rs
/// Deterministic picture of a directory tree: relative path -> (kind, sha256 of content or link target).
pub fn snapshot(root: &Path) -> std::io::Result<std::collections::BTreeMap<String, String>>;
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::path_edit::{path_add, path_contains, path_remove};
use wingetrs::install::snapshot::snapshot;

const ENTRY: &str = r"C:\Users\u\AppData\Local\wingetrs\shims";

#[test]
fn path_add_appends_once_and_keeps_everything_else() {
    let user = r"%USERPROFILE%\bin;;C:\Tools\;%JAVA_HOME%\bin";
    let out = path_add(user, ENTRY);
    assert_eq!(out, format!(r"{user};{ENTRY}"));
    assert_eq!(path_add(&out, ENTRY), out, "idempotent");
    assert_eq!(path_add(&out, &ENTRY.to_uppercase()), out, "case-insensitive");
    assert_eq!(path_add(&out, &format!(r"{ENTRY}\")), out, "trailing backslash");
    assert_eq!(path_add("", ENTRY), ENTRY);
    assert_eq!(path_add("A;", ENTRY), format!("A;{ENTRY}"), "no doubled separator");
}

#[test]
fn path_remove_only_removes_equivalent_entries() {
    let user = format!(r"%USERPROFILE%\bin;;{ENTRY};C:\Tools\;{}\;%JAVA_HOME%\bin", ENTRY.to_lowercase());
    let out = path_remove(&user, ENTRY);
    assert_eq!(out, r"%USERPROFILE%\bin;;C:\Tools\;%JAVA_HOME%\bin");
    assert!(!path_contains(&out, ENTRY));
    assert_eq!(path_remove(&out, ENTRY), out);
    assert_eq!(path_remove(ENTRY, ENTRY), "");
}

#[test]
fn snapshot_detects_content_and_structure_changes() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("a/b")).unwrap();
    std::fs::write(d.path().join("a/b/f.txt"), b"1").unwrap();
    let s1 = snapshot(d.path()).unwrap();
    assert_eq!(s1, snapshot(d.path()).unwrap());
    std::fs::write(d.path().join("a/b/f.txt"), b"2").unwrap();
    assert_ne!(s1, snapshot(d.path()).unwrap());
    std::fs::write(d.path().join("a/b/f.txt"), b"1").unwrap();
    std::fs::create_dir(d.path().join("empty")).unwrap();
    assert_ne!(s1, snapshot(d.path()).unwrap(), "empty dirs count");
}
```

Escreva também (prosa): `Layout` produz os caminhos do contrato acima a partir de uma raiz qualquer; cada fake (`FakeEnv`, `FakeShortcuts`, `FakeTrash`, `FakeRegistry`) guarda estado em `Mutex<…>` e `dump()` muda quando o estado muda e volta igual quando revertido.

#### Checks beyond the steps

1. Idempotência do `path_add`: mutate a checagem de equivalência (ex.: comparar com `==` sensível à caixa) ⇒ PEGA.
2. `path_remove` preserva segmentos vazios: mutate para filtrar segmentos vazios ⇒ PEGA.
3. `snapshot` conta diretórios vazios: mutate para listar só arquivos ⇒ PEGA.

**Commit:** `feat(install): layout, effect traits, fakes, snapshot and pure PATH editing`

---

### Task 24: Manifesto scoop tipado e checagem de suporte

#### Where this fits

Lê o JSON do scoop (já guardado em `packages.raw` do catálogo) num tipo forte para o planejador, resolve a variante por arquitetura e diz, com motivo, se a fase 2a suporta o manifesto.

#### What binds this task

- Campos: `version`, `url` (string | lista), `hash` (string | lista), `extract_dir` (string | lista, paralela às URLs), `extract_to` (idem), `bin`, `shortcuts`, `persist`, `env_add_path`, `env_set`, `depends`, `architecture.{64bit,32bit,arm64}` (cada uma pode sobrescrever `url`, `hash`, `extract_dir`, `extract_to`, `bin`, `shortcuts`, `env_add_path`, `env_set`) — campos da arquitetura **substituem** os da raiz.
- `bin`: string | lista de (string | `[caminho, alias?, args?]`).
- `shortcuts`: lista de `[alvo, nome, args?, ícone?]`.
- `persist`: string | lista de (string | `[origem, destino]`).
- `depends`: string | lista; cada item pode ser `bucket/app`.
- URL com `#/nome` ⇒ arquivo baixado é renomeado para `nome`.
- Suporte (Global Constraints): primeira razão encontrada vira `Unsupported { reason }` em português, ex.: `"usa post_install (PowerShell): suportado na fase 2c"`, `"download .7z: suportado na fase 2b"`, `"sem hash para verificar o download"`.
- Seleção de arquitetura: preferência pela máquina (`x86_64` ⇒ 64bit, 32bit; `aarch64` ⇒ arm64, 64bit, 32bit); manifesto sem `architecture` serve a qualquer uma.

#### Fixture facts (`tests/fixtures/scoop/manifests/`, reais do catálogo)

`ripgrep__main__ripgrep.json`: version 15.2.0, `bin "rg.exe"`, 64bit url `…/ripgrep-15.2.0-x86_64-pc-windows-msvc.zip`, hash `71b2fef860abe467217a538ff31de02f5258807c0129f771846f87bd029aafc5`, extract_dir `ripgrep-15.2.0-x86_64-pc-windows-msvc`. `bin-alias__main__alist.json`: `bin [["alist.exe","alist","--force-bin-dir"]]`, hashes `md5:…`, persist `["data","log"]`. `env__main__android-cli.json`: `env_set {"ANDROID_HOME":"$dir\\Sdk", …}`, `env_add_path ["Sdk\\cmdline-tools\\latest\\bin","Sdk\\platform-tools"]`, url `.exe` direto. `persist-file-and-dir__main__i2p-zero.json`: `extract_dir "i2p-zero-win.v1.20\\router"`, `bin [["bin\\launch.bat","i2p-zero"]]`, persist `["conf","i2p.base"]`. `shortcuts-persist__main__danser-go.json`: `shortcuts [["danser.exe","danser-go"]]`, persist `"settings"`, bin lista de dois. `url-rename__main__agent-browser.json`: url `…-win32-x64.exe#/agent-browser.exe`. `sha1__main__astyle.json` (`sha1:`), `sha512__main__antigravity-cli.json` (`sha512:`), `depends__main__apimtemplate.json` (`depends "azure-cli"`), `tar-gz__main__air.json` (`.tar.gz`), `bat-bin__main__alass.json` (`alass.bat`), `ps1-bin__main__concfg.json` (`bin\\concfg.ps1`). Não suportados: `unsup-script__main__7zip.json` (`post_install`), `unsup-7z__main__cdrtools.json` (`.7z`), `unsup-msi__main__7zip19.00-helper.json` (`.msi`), `unsup-innosetup__main__boost.json` (`innosetup`, também `pre_install`).

**Files:**
- Modify (stub gerado pela Task 27): `src/install/manifest.rs`
- Test (new): `tests/install_manifest.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Arch { X64, X86, Arm64 }
impl Arch { pub fn key(&self) -> &'static str; /* "64bit" | "32bit" | "arm64" */ pub fn host() -> Arch; }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo { Sha256, Sha1, Sha512, Md5 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashSpec { pub algo: HashAlgo, pub hex: String } // hex lowercase
impl HashSpec { pub fn parse(s: &str) -> Option<HashSpec>; }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source { pub url: String, pub file_name: String, pub hash: HashSpec, pub extract_dir: Option<String>, pub extract_to: Option<String> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinEntry { pub path: String, pub alias: Option<String>, pub args: Option<String> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEntry { pub target: String, pub name: String, pub args: Option<String>, pub icon: Option<String> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistEntry { pub source: String, pub target: String }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub version: String, pub arch: Option<Arch>, pub sources: Vec<Source>,
    pub bin: Vec<BinEntry>, pub shortcuts: Vec<ShortcutEntry>, pub persist: Vec<PersistEntry>,
    pub env_add_path: Vec<String>, pub env_set: Vec<(String, String)>, pub depends: Vec<String>,
}
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ManifestError {
    #[error("manifesto inválido: {0}")] Invalid(String),
    #[error("não suportado: {reason}")] Unsupported { reason: String },
    #[error("sem variante para a arquitetura desta máquina")] NoArch,
}
pub fn resolve(raw_json: &str, host: Arch) -> Result<Resolved, ManifestError>;
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::manifest::{resolve, Arch, HashAlgo, ManifestError};
fn fx(n: &str) -> String { std::fs::read_to_string(format!("tests/fixtures/scoop/manifests/{n}")).unwrap() }

#[test]
fn ripgrep_x64() {
    let r = resolve(&fx("ripgrep__main__ripgrep.json"), Arch::X64).unwrap();
    assert_eq!(r.version, "15.2.0");
    assert_eq!(r.arch, Some(Arch::X64));
    assert_eq!(r.sources.len(), 1);
    let s = &r.sources[0];
    assert!(s.url.ends_with("ripgrep-15.2.0-x86_64-pc-windows-msvc.zip"));
    assert_eq!(s.file_name, "ripgrep-15.2.0-x86_64-pc-windows-msvc.zip");
    assert_eq!(s.hash.algo, HashAlgo::Sha256);
    assert_eq!(s.hash.hex, "71b2fef860abe467217a538ff31de02f5258807c0129f771846f87bd029aafc5");
    assert_eq!(s.extract_dir.as_deref(), Some("ripgrep-15.2.0-x86_64-pc-windows-msvc"));
    assert_eq!(r.bin.len(), 1);
    assert_eq!(r.bin[0].path, "rg.exe");
}

#[test]
fn arch_fallback_and_rename_fragment() {
    let r = resolve(&fx("ripgrep__main__ripgrep.json"), Arch::Arm64).unwrap();
    assert_eq!(r.arch, Some(Arch::Arm64));
    let r = resolve(&fx("url-rename__main__agent-browser.json"), Arch::X64).unwrap();
    assert_eq!(r.sources[0].file_name, "agent-browser.exe");
    assert!(!r.sources[0].url.contains('#'), "fragment is not part of the request URL");
    // only 64bit exists: an x86 host has no variant
    assert_eq!(resolve(&fx("url-rename__main__agent-browser.json"), Arch::X86), Err(ManifestError::NoArch));
}

#[test]
fn bin_alias_args_hash_prefixes_and_persist() {
    let r = resolve(&fx("bin-alias__main__alist.json"), Arch::X64).unwrap();
    assert_eq!(r.bin[0].path, "alist.exe");
    assert_eq!(r.bin[0].alias.as_deref(), Some("alist"));
    assert_eq!(r.bin[0].args.as_deref(), Some("--force-bin-dir"));
    assert_eq!(r.sources[0].hash.algo, HashAlgo::Md5);
    assert_eq!(r.persist.iter().map(|p| (p.source.as_str(), p.target.as_str())).collect::<Vec<_>>(), [("data", "data"), ("log", "log")]);
    assert_eq!(resolve(&fx("sha1__main__astyle.json"), Arch::X64).unwrap().sources[0].hash.algo, HashAlgo::Sha1);
    assert_eq!(resolve(&fx("sha512__main__antigravity-cli.json"), Arch::X64).unwrap().sources[0].hash.algo, HashAlgo::Sha512);
}

#[test]
fn env_and_depends() {
    let r = resolve(&fx("env__main__android-cli.json"), Arch::X64).unwrap();
    assert!(r.env_set.contains(&("ANDROID_HOME".to_string(), r"$dir\Sdk".to_string())));
    assert_eq!(r.env_add_path, [r"Sdk\cmdline-tools\latest\bin", r"Sdk\platform-tools"]);
    assert_eq!(resolve(&fx("depends__main__apimtemplate.json"), Arch::X64).unwrap().depends, ["azure-cli"]);
}

#[test]
fn unsupported_manifests_say_why() {
    let reason = |f: &str| match resolve(&fx(f), Arch::X64) { Err(ManifestError::Unsupported { reason }) => reason, o => panic!("{f}: {o:?}") };
    assert!(reason("unsup-script__main__7zip.json").contains("post_install"));
    assert!(reason("unsup-7z__main__cdrtools.json").contains(".7z"));
    assert!(reason("unsup-msi__main__7zip19.00-helper.json").contains(".msi"));
    let r = reason("unsup-innosetup__main__boost.json");
    assert!(r.contains("innosetup") || r.contains("pre_install"), "{r}");
    let nohash = r#"{"version":"1","url":"https://x/a.zip"}"#;
    assert!(matches!(resolve(nohash, Arch::X64), Err(ManifestError::Unsupported { reason }) if reason.contains("sem hash")));
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    for s in ["", "[]", "{", r#"{"version":1}"#, r#"{"version":"1","url":5,"hash":"x"}"#, r#"{"version":"1","bin":[[]]}"#] {
        assert!(resolve(s, Arch::X64).is_err(), "{s}");
    }
}
```

Acrescente (prosa) um teste que percorre **todos** os arquivos de `tests/fixtures/scoop/manifests/` e afirma que os de prefixo `unsup-` dão `Unsupported` e os demais dão `Ok` (com `Arch::X64`).

#### Checks beyond the steps

1. Arquitetura sobrescreve a raiz: mutate para preferir o campo da raiz ⇒ `ripgrep_x64` PEGA.
2. `#/` renomeia: mutate para usar o basename da URL ⇒ PEGA.
3. Manifesto sem hash recusado: mutate para aceitar ⇒ PEGA.

**Commit:** `feat(install): typed scoop manifest with support check`

---

### Task 25: Hashes e download com cache

#### Where this fits

Baixa os arquivos do plano, reaproveitando o `BlobStore` da fase 1 (cache por SHA-256 em `blobs\`) e verificando o hash declarado no manifesto (sha256/sha1/sha512/md5).

#### What binds this task

- `verify(algo, expected_hex, bytes) -> bool` com `sha2` (256/512), `sha1`, `md-5`.
- Cache: o blob é indexado pelo **sha256 do conteúdo**. Para hashes não-sha256 mantenha um índice `blobs\index.json` `{"<algo>:<hex>": "<sha256>"}` para achar o blob sem baixar. Blob em cache com conteúdo adulterado ⇒ ignorado e rebaixado.
- Download em streaming para `DataDir::tmp_dir()` e só depois move para `blobs\` (nunca blob parcial no cache). Progresso por callback `(baixados, total_opcional)`.
- Vários downloads do mesmo plano em paralelo (`futures_util::future::join_all`), limite 4 simultâneos.
- Falha de hash ⇒ `DownloadError::HashMismatch { url }` e o arquivo temporário é apagado.

**Files:**
- Modify (stub gerado pela Task 27): `src/install/download.rs`
- Test (new): `tests/install_download.rs`

#### Interfaces

```rust
use crate::install::manifest::{HashAlgo, HashSpec, Source};
pub fn verify(spec: &HashSpec, bytes: &[u8]) -> bool;
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error(transparent)] Http(#[from] crate::http::HttpError),
    #[error("hash não confere para {url}")] HashMismatch { url: String },
    #[error("E/S: {0}")] Io(String),
}
pub type Progress<'a> = &'a (dyn Fn(&str /*url*/, u64 /*done*/, Option<u64> /*total*/) + Send + Sync);
/// Returns the local path of each source's verified file (in blobs), same order as `sources`.
pub async fn fetch_all(http: &crate::http::Http, data: &crate::cache::DataDir, sources: &[Source], progress: Progress<'_>) -> Result<Vec<std::path::PathBuf>, DownloadError>;
```

`Http` precisa de um método de streaming: acrescente em `src/http.rs` `pub async fn get_to_file(&self, url: &str, path: &Path, progress: impl Fn(u64, Option<u64>)) -> Result<(), HttpError>` (sem mudar os existentes).

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::download::verify;
use wingetrs::install::manifest::HashSpec;

#[test]
fn verify_all_algorithms_case_insensitive() {
    let b = b"abc";
    for (s, ok) in [
        ("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", true),
        ("BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD", true),
        ("sha1:a9993e364706816aba3e25717850c26c9cd0d89d", true),
        ("md5:900150983cd24fb0d6963f7d28e17f72", true),
        ("sha512:ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f", true),
        ("sha1:a9993e364706816aba3e25717850c26c9cd0d89e", false),
    ] {
        assert_eq!(verify(&HashSpec::parse(s).unwrap(), b), ok, "{s}");
    }
}
```

Escreva também com `wiremock` (prosa): (1) dois sources servidos ⇒ `fetch_all` devolve dois caminhos em `blobs\` com o conteúdo certo; (2) segunda chamada com o mock **desmontado** ⇒ ainda devolve os caminhos (cache, inclusive para um source `md5:` via `index.json`); (3) servidor devolve bytes errados ⇒ `HashMismatch` e nenhum arquivo novo em `blobs\` nem em `tmp\`; (4) blob em cache adulterado ⇒ é rebaixado (mock conta 1 requisição).

#### Checks beyond the steps

1. md5 verificado de verdade: mutate `HashAlgo::Md5 =>` para retornar `true` ⇒ PEGA.
2. Cache não aceita blob adulterado: mutate a verificação do blob em cache ⇒ teste (4) PEGA.
3. Temporário apagado no `HashMismatch`: mutate para não apagar ⇒ teste (3) PEGA.

**Commit:** `feat(install): verified downloads with blob cache`

---

### Task 26: Extração segura (zip, tar.gz, arquivo solto)

#### Where this fits

Põe o conteúdo baixado no diretório de staging, aplicando `extract_dir` (subpasta do arquivo que vira a raiz) e `extract_to` (subpasta de destino). Arquivo que não é zip/tar.gz é copiado como está, com o nome do `Source::file_name`.

#### What binds this task

- Tipos por extensão do `file_name`: `.zip` ⇒ crate `zip`; `.tar.gz`/`.tgz` ⇒ `flate2` + `tar`; qualquer outra ⇒ cópia simples.
- **Segurança (Review Focus 1):** cada entrada do arquivo é normalizada; componente `..`, caminho absoluto, prefixo de drive (`C:`), UNC/`\\?\`, ou link simbólico no tar ⇒ `ExtractError::UnsafePath(entrada)` e **nada** foi escrito fora de `dest` (escreva primeiro numa pasta temporária irmã e só renomeie no fim; em erro, remova-a).
- `extract_dir`: aceita `\` ou `/`; se a pasta não existir no arquivo ⇒ `ExtractError::MissingExtractDir`.
- Vários sources: cada um extrai no mesmo staging, na ordem.

**Files:**
- Modify (stub gerado pela Task 27): `src/install/extract.rs`
- Test (new): `tests/install_extract.rs`

#### Interfaces

```rust
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("caminho inseguro no arquivo: {0}")] UnsafePath(String),
    #[error("pasta {0} não existe no arquivo")] MissingExtractDir(String),
    #[error("arquivo inválido: {0}")] Archive(String),
    #[error("E/S: {0}")] Io(String),
}
/// Extracts (or copies) `file` into `dest` honoring extract_dir/extract_to. `dest` may already have content.
pub fn extract(file: &Path, file_name: &str, extract_dir: Option<&str>, extract_to: Option<&str>, dest: &Path) -> Result<(), ExtractError>;
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Write;
use wingetrs::install::extract::{extract, ExtractError};

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in entries { z.start_file(*n, o).unwrap(); z.write_all(b).unwrap(); }
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn extract_dir_becomes_root() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("pkg-1.0/rg.exe", b"x"), ("pkg-1.0/doc/README", b"r")])).unwrap();
    let dest = d.path().join("out");
    extract(&f, "a.zip", Some("pkg-1.0"), None, &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("rg.exe")).unwrap(), b"x");
    assert!(dest.join("doc/README").exists());
    assert!(!dest.join("pkg-1.0").exists());
}

#[test]
fn traversal_and_absolute_entries_are_refused_and_nothing_escapes() {
    for bad in ["../evil.txt", "a/../../evil.txt", "C:/evil.txt", "/abs.txt", "\\\\?\\C:\\evil.txt"] {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        std::fs::write(&f, zip_with(&[("ok.txt", b"1"), (bad, b"2")])).unwrap();
        let dest = d.path().join("inner").join("out");
        let r = extract(&f, "a.zip", None, None, &dest);
        assert!(matches!(r, Err(ExtractError::UnsafePath(_))), "{bad}: {r:?}");
        assert!(!d.path().join("evil.txt").exists() && !d.path().join("inner").join("evil.txt").exists());
        assert!(!dest.exists(), "{bad}: partial output must be removed");
    }
}

#[test]
fn plain_file_is_copied_with_its_name_and_extract_to() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("blob");
    std::fs::write(&f, b"MZ").unwrap();
    let dest = d.path().join("out");
    extract(&f, "agent-browser.exe", None, Some("bin"), &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("bin/agent-browser.exe")).unwrap(), b"MZ");
}

#[test]
fn missing_extract_dir_is_reported() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("x/rg.exe", b"x")])).unwrap();
    assert!(matches!(extract(&f, "a.zip", Some("nope"), None, &d.path().join("o")), Err(ExtractError::MissingExtractDir(_))));
}
```

Acrescente (prosa) um tar.gz construído em memória (`tar::Builder` + `flate2::write::GzEncoder`) com uma entrada normal e uma de **symlink** ⇒ `UnsafePath`; e um tar.gz válido com `extract_dir` com `\` (`"a\\b"`).

> Nota: `zip::ZipWriter` pode recusar nomes como `../evil.txt` ao escrever; se recusar, construa o zip com o nome adulterado direto nos bytes (escreva um nome neutro do mesmo tamanho e troque os bytes no buffer) — registre no relatório como fez.

#### Checks beyond the steps

1. Recusa de `..`: mutate o teste de componente `ParentDir` ⇒ PEGA.
2. Remoção da saída parcial: mutate para não remover em erro ⇒ PEGA.
3. Symlink no tar recusado: mutate para aceitar ⇒ PEGA.

**Commit:** `feat(install): safe archive extraction`

---

### Task 28: Planejador puro (`InstallPlan`)

#### Where this fits

O coração da 2a: a partir do manifesto resolvido, do layout e do estado atual, produz a lista de passos que o executor aplica, que o `--dry-run` imprime e que a TUI mostra. **Não faz I/O.**

#### What binds this task

- Entrada: `Resolved` (Task 24), `app`, `bucket`, `Layout` (Task 27), `installed: Option<InstalledInfo>` (versão atual, se houver) e `launcher_present: bool`.
- Substituição de variáveis em `env_set`, `env_add_path`, `bin.args`, `shortcuts.args`: `$dir` ⇒ `<apps>\<app>\current`, `$persist_dir` ⇒ `<persist>\<app>`, `$version` ⇒ versão, `$app` ⇒ app, `$architecture` ⇒ `64bit|32bit|arm64`. Caminhos relativos de `env_add_path` são relativos a `current`.
- `bin`: nome do shim = `alias` se houver, senão o nome do arquivo **sem extensão**; tipo pelo sufixo (`.exe`, `.cmd`, `.bat`, `.ps1`, `.jar`, outros ⇒ `.exe`-like com `.shim`). Dois `bin` com o mesmo nome de shim ⇒ `PlanError::ShimConflict`.
- `shortcuts`: o nome pode ter subpastas (`7-Zip\7-Zip File Manager`); cada componente tem `<>:"/|?*` e caracteres de controle trocados por `_`; componentes `.`/`..` ⇒ `_` (Review Focus 5). O `.lnk` fica em `<Shortcuts::root()>\<nome>.lnk` — o planejador recebe `shortcut_root: &Path`.
- `persist`: item é **pasta** se não tiver ponto no último componente, senão **arquivo** (regra do scoop; medido no catálogo: 1 022 pastas, 689 arquivos). Pasta ⇒ junction `<version>\<origem>` → `<persist>\<destino>`; arquivo ⇒ hardlink.
- Ordem dos passos (contrato do executor): `EnsureLauncher` (se `!launcher_present`) → `Download`* → `Extract`* → `Promote` → `Persist`* → `LinkCurrent` → `Shim`* → `Shortcut`* → `EnvSet`* → `EnvAddPath`* → `EnsureShimsOnPath` → `RecordState` → (`RemoveOldVersion` se upgrade sem `keep`).
- Upgrade (`installed = Some(v)` com `v != version`): mesmos passos; `upgrade_from = Some(v)`; persist reaproveita `<persist>\<app>` existente. Mesma versão já instalada ⇒ `PlanError::AlreadyInstalled`.
- `InstallPlan: Display` em português, uma linha por passo, com caminhos completos (é o texto do `--dry-run` e da confirmação). Snapshot `insta` desse texto por fixture.

**Files:**
- Modify (stub gerado pela Task 27): `src/install/plan.rs`, `src/install/planner.rs`
- Test (new): `tests/install_planner.rs`, `tests/snapshots/install_planner__*.snap`

#### Interfaces

```rust
// src/install/plan.rs
use std::path::PathBuf;
use crate::install::manifest::{Arch, Source};
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ShimKind { Exe, Cmd, Bat, Ps1, Jar }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimSpec { pub name: String, pub target: PathBuf /*absolute, under current*/, pub args: Option<String>, pub kind: ShimKind }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec { pub link: PathBuf /*absolute .lnk*/, pub target: PathBuf, pub args: Option<String>, pub icon: Option<PathBuf> }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistKind { Dir, File }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistSpec { pub in_version: PathBuf /*relative*/, pub in_persist: PathBuf /*relative*/, pub kind: PersistKind }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    EnsureLauncher,
    Download { index: usize, source: Source },
    Extract { index: usize },
    Promote,
    Persist(PersistSpec),
    LinkCurrent,
    Shim(ShimSpec),
    Shortcut(ShortcutSpec),
    EnvSet { name: String, value: String },
    EnvAddPath { entry: PathBuf },
    EnsureShimsOnPath,
    RecordState,
    RemoveOldVersion { version: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan { pub app: String, pub bucket: String, pub version: String, pub arch: Option<Arch>, pub upgrade_from: Option<String>, pub steps: Vec<Step> }
impl std::fmt::Display for InstallPlan { /* português */ }

// src/install/planner.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledInfo { pub version: String }
#[derive(Debug, Clone)]
pub struct PlanInput<'a> {
    pub app: &'a str, pub bucket: &'a str, pub manifest: &'a crate::install::manifest::Resolved,
    pub layout: &'a crate::install::layout::Layout, pub shortcut_root: &'a std::path::Path,
    pub installed: Option<InstalledInfo>, pub launcher_present: bool, pub keep_old: bool,
}
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlanError {
    #[error("{app} {version} já está instalado")] AlreadyInstalled { app: String, version: String },
    #[error("dois executáveis com o mesmo nome de atalho de comando: {0}")] ShimConflict(String),
}
pub fn plan_install(input: &PlanInput<'_>) -> Result<InstallPlan, PlanError>;
pub fn substitute(template: &str, app: &str, version: &str, arch: Option<Arch>, layout: &crate::install::layout::Layout) -> String;
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::{Path, PathBuf};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{resolve, Arch};
use wingetrs::install::plan::{PersistKind, ShimKind, Step};
use wingetrs::install::planner::{plan_install, substitute, InstalledInfo, PlanError, PlanInput};

fn fx(n: &str) -> String { std::fs::read_to_string(format!("tests/fixtures/scoop/manifests/{n}")).unwrap() }
fn root() -> Layout { Layout::new(r"C:\W") }
const SC: &str = r"C:\SM\wingetrs Apps";

fn plan(file: &str, installed: Option<&str>, launcher: bool) -> Result<wingetrs::install::plan::InstallPlan, PlanError> {
    let m = resolve(&fx(file), Arch::X64).unwrap();
    let l = root();
    plan_install(&PlanInput { app: "app", bucket: "main", manifest: &m, layout: &l, shortcut_root: Path::new(SC),
        installed: installed.map(|v| InstalledInfo { version: v.into() }), launcher_present: launcher, keep_old: false })
}

#[test]
fn step_order_is_the_executor_contract() {
    let p = plan("shortcuts-persist__main__danser-go.json", None, false).unwrap();
    let kinds: Vec<&str> = p.steps.iter().map(|s| match s {
        Step::EnsureLauncher => "launcher", Step::Download { .. } => "download", Step::Extract { .. } => "extract",
        Step::Promote => "promote", Step::Persist(_) => "persist", Step::LinkCurrent => "current", Step::Shim(_) => "shim",
        Step::Shortcut(_) => "shortcut", Step::EnvSet { .. } => "envset", Step::EnvAddPath { .. } => "envpath",
        Step::EnsureShimsOnPath => "path", Step::RecordState => "state", Step::RemoveOldVersion { .. } => "rmold",
    }).collect();
    assert_eq!(kinds, ["launcher", "download", "extract", "promote", "persist", "current", "shim", "shim", "shortcut", "path", "state"]);
    assert!(!plan("shortcuts-persist__main__danser-go.json", None, true).unwrap().steps.contains(&Step::EnsureLauncher));
}

#[test]
fn shim_names_kinds_and_args() {
    let p = plan("bin-alias__main__alist.json", None, true).unwrap();
    let shims: Vec<_> = p.steps.iter().filter_map(|s| if let Step::Shim(x) = s { Some(x.clone()) } else { None }).collect();
    assert_eq!(shims.len(), 1);
    assert_eq!(shims[0].name, "alist");
    assert_eq!(shims[0].args.as_deref(), Some("--force-bin-dir"));
    assert_eq!(shims[0].kind, ShimKind::Exe);
    assert_eq!(shims[0].target, PathBuf::from(r"C:\W\apps\app\current\alist.exe"));
    let p = plan("persist-file-and-dir__main__i2p-zero.json", None, true).unwrap();
    let s = p.steps.iter().find_map(|s| if let Step::Shim(x) = s { Some(x.clone()) } else { None }).unwrap();
    assert_eq!((s.name.as_str(), s.kind), ("i2p-zero", ShimKind::Bat));
    let p = plan("ps1-bin__main__concfg.json", None, true).unwrap();
    let s = p.steps.iter().find_map(|s| if let Step::Shim(x) = s { Some(x.clone()) } else { None }).unwrap();
    assert_eq!((s.name.as_str(), s.kind), ("concfg", ShimKind::Ps1));
}

#[test]
fn persist_dir_vs_file_by_last_component() {
    let p = plan("persist-file-and-dir__main__i2p-zero.json", None, true).unwrap();
    let ps: Vec<_> = p.steps.iter().filter_map(|s| if let Step::Persist(x) = s { Some((x.in_version.clone(), x.kind)) } else { None }).collect();
    assert_eq!(ps, [(PathBuf::from("conf"), PersistKind::Dir), (PathBuf::from("i2p.base"), PersistKind::File)]);
}

#[test]
fn env_variables_are_substituted() {
    let p = plan("env__main__android-cli.json", None, true).unwrap();
    assert!(p.steps.contains(&Step::EnvSet { name: "ANDROID_HOME".into(), value: r"C:\W\apps\app\current\Sdk".into() }));
    assert!(p.steps.contains(&Step::EnvAddPath { entry: PathBuf::from(r"C:\W\apps\app\current\Sdk\platform-tools") }));
    assert_eq!(substitute(r"$persist_dir\x;$version;$app;$architecture", "app", "9", Some(Arch::X64), &root()), r"C:\W\persist\app\x;9;app;64bit");
}

#[test]
fn shortcut_names_are_sanitized_and_contained() {
    let raw = r#"{"version":"1","url":"https://x/a.zip","hash":"aa","shortcuts":[["a.exe","7-Zip\\7-Zip File Manager"],["a.exe","bad:na?me"],["a.exe","..\\..\\escape"]]}"#;
    let m = resolve(raw, Arch::X64).unwrap();
    let l = root();
    let p = plan_install(&PlanInput { app: "app", bucket: "main", manifest: &m, layout: &l, shortcut_root: Path::new(SC), installed: None, launcher_present: true, keep_old: false }).unwrap();
    let links: Vec<PathBuf> = p.steps.iter().filter_map(|s| if let Step::Shortcut(x) = s { Some(x.link.clone()) } else { None }).collect();
    assert_eq!(links[0], PathBuf::from(r"C:\SM\wingetrs Apps\7-Zip\7-Zip File Manager.lnk"));
    assert_eq!(links[1], PathBuf::from(r"C:\SM\wingetrs Apps\bad_na_me.lnk"));
    assert!(links[2].starts_with(SC) && !links[2].to_string_lossy().contains(".."), "{:?}", links[2]);
}

#[test]
fn upgrade_and_same_version() {
    let p = plan("ripgrep__main__ripgrep.json", Some("14.0.0"), true).unwrap();
    assert_eq!(p.upgrade_from.as_deref(), Some("14.0.0"));
    assert_eq!(p.steps.last(), Some(&Step::RemoveOldVersion { version: "14.0.0".into() }));
    assert!(matches!(plan("ripgrep__main__ripgrep.json", Some("15.2.0"), true), Err(PlanError::AlreadyInstalled { .. })));
}

#[test]
fn display_snapshots_for_every_supported_fixture() {
    for e in std::fs::read_dir("tests/fixtures/scoop/manifests").unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        if name.starts_with("unsup-") { continue; }
        let Ok(m) = resolve(&fx(&name), Arch::X64) else { continue };
        let l = root();
        let p = plan_install(&PlanInput { app: "app", bucket: "main", manifest: &m, layout: &l, shortcut_root: Path::new(SC), installed: None, launcher_present: false, keep_old: false }).unwrap();
        insta::assert_snapshot!(name.trim_end_matches(".json"), p.to_string());
    }
}
```

#### Checks beyond the steps

1. Ordem dos passos: mutate trocando `LinkCurrent` para antes de `Persist` ⇒ PEGA.
2. Sanitização: mutate removendo a troca de `:` ⇒ PEGA; mutate deixando `..` ⇒ PEGA.
3. Pasta vs. arquivo no persist: mutate invertendo a regra ⇒ PEGA.
4. `AlreadyInstalled`: mutate para planejar mesmo assim ⇒ PEGA.

**Commit:** `feat(install): pure install planner`

---

### Task 29: Estado (`state.db`, `install.json`)

#### Where this fits

Registra o que o wingetrs instalou e cada artefato criado (para desfazer exatamente) e escreve o `install.json` no formato do scoop.

#### What binds this task

- `state.db` (SQLite próprio, WAL): tabelas `apps(app PRIMARY KEY, bucket, version, arch, installed_at INTEGER, manifest_sha256)` e `artifacts(app, kind, target, previous)`; `kind ∈ {shim, shortcut, persist, path, env}`. Gravação de um app = uma transação (apaga artefatos antigos daquele app, insere os novos).
- `install.json` em `<version>\install.json`: `{"bucket": "...", "architecture": "64bit", "version": "...", "wingetrs": 1}` (o leitor da fase 1 lê `bucket`/`architecture`).
- Leitor de instalados do wingetrs: `scan_wingetrs(layout) -> Vec<InstalledApp>` (tipo da fase 1, `InstallSource` ganha a variante `Wingetrs`) lendo `apps\<app>\current\install.json` + versão do `state.db` quando houver; app com `current` quebrado ⇒ `InstalledApp` com `version: None` e marca `broken` no `ListRow` (Task 37 usa).

**Files:**
- Modify (stub gerado pela Task 27): `src/install/state.rs`, `src/installed/mod.rs` (variante `InstallSource::Wingetrs`)
- Test (new): `tests/install_state.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ArtifactKind { Shim, Shortcut, Persist, Path, Env }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact { pub kind: ArtifactKind, pub target: String, pub previous: Option<String> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRecord { pub app: String, pub bucket: String, pub version: String, pub arch: Option<String>, pub installed_at: i64, pub manifest_sha256: String }
pub struct StateDb { /* conn */ }
impl StateDb {
    pub fn open(path: &Path) -> Result<StateDb, StateError>;
    pub fn record(&mut self, app: &AppRecord, artifacts: &[Artifact]) -> Result<(), StateError>;
    pub fn get(&self, app: &str) -> Result<Option<(AppRecord, Vec<Artifact>)>, StateError>;
    pub fn remove(&mut self, app: &str) -> Result<(), StateError>;
    pub fn apps(&self) -> Result<Vec<AppRecord>, StateError>;
    /// Apps (other than `app`) whose recorded artifacts include a PATH entry equal to `entry`.
    pub fn path_users(&self, app: &str, entry: &str) -> Result<Vec<String>, StateError>; // apps other than `app` using `entry`
}
pub fn write_install_json(version_dir: &Path, bucket: &str, arch: Option<&str>, version: &str) -> std::io::Result<()>;
pub fn scan_wingetrs(layout: &crate::install::layout::Layout) -> Vec<crate::installed::InstalledApp>;
```

Testes (prosa): gravar/ler/remover; regravar substitui artefatos (não duplica); `path_users`; `scan_wingetrs` em `tempdir` com um app válido (junction `current`) e um com `current` apontando para pasta apagada (aparece com `version: None`); `StateDb::open` em arquivo inexistente cria o schema.

#### Checks beyond the steps

1. Regravação substitui artefatos: mutate removendo o `DELETE` prévio ⇒ PEGA.
2. App quebrado aparece: mutate para pular apps sem `install.json` legível ⇒ PEGA.

**Commit:** `feat(install): install state database and install.json`

---

### Task 30: Shims — lançador, subsistema PE, wrappers

#### Where this fits

Cria os comandos no PATH: para `.exe` (e qualquer não-script), copia o lançador kiennq como `<shim>.exe` + `<shim>.shim`; para scripts, gera wrappers como o scoop.

#### What binds this task

- `ensure_launcher(http, data, layout)`: se `layout.launcher()` não existe, baixa o zip da arquitetura da máquina (Global Constraints), verifica o SHA-256 do zip, extrai `shim.exe` para `layout.launcher()` (via `extract` da Task 26 e `download` da Task 25).
- `.exe` (e `Exe`-like): **hardlink** de `launcher()` para `shims\<name>.exe` (cópia se o hardlink falhar) e `shims\<name>.shim` com `path = "<target absoluto>"` + `args = <args>` se houver, UTF-8 sem BOM, `\r\n`.
- **Subsistema PE:** se o alvo é GUI (subsistema 2), o `<name>.exe` do shim é uma **cópia** (não hardlink) com o subsistema trocado para 2 — o scoop faz isso para não abrir console. Leitura e escrita do campo: `e_lfanew` = u32 LE em 0x3C; assinatura `PE\0\0` em `e_lfanew`; campo `Subsystem` = u16 LE em `e_lfanew + 24 + 68` (igual para PE32 e PE32+). Arquivo curto/inválido ⇒ trate como console (não modifica).
- `.cmd`/`.bat`: `shims\<name>.cmd` = `@rem <target>\r\n@"<target>" <args> %*`.
- `.ps1`: `shims\<name>.ps1` (`& "<target>" <args> @args`) e `shims\<name>.cmd` que chama `pwsh.exe` se existir senão `powershell.exe`, com `-noprofile -ex unrestricted -file "<target>" <args> %*`.
- `.jar`: `shims\<name>.cmd` = `@rem <target>\r\n@pushd <pasta do jar>\r\n@java -jar "<target>" <args> %*\r\n@popd`.
- `remove_shim(layout, name)` apaga todos os arquivos `shims\<name>.{exe,shim,cmd,ps1}` que existirem.
- Retorna a lista de arquivos criados (vira artefato `Shim` no estado).

**Files:**
- Modify (stub gerado pela Task 27): `src/install/shim.rs`
- Test (new): `tests/install_shim.rs`

#### Interfaces

```rust
pub fn pe_subsystem(bytes: &[u8]) -> Option<u16>;
pub fn set_pe_subsystem(bytes: &mut [u8], subsystem: u16) -> bool;
pub async fn ensure_launcher(http: &crate::http::Http, data: &crate::cache::DataDir, layout: &crate::install::layout::Layout) -> Result<(), ShimError>;
pub fn create_shim(layout: &crate::install::layout::Layout, spec: &crate::install::plan::ShimSpec) -> Result<Vec<std::path::PathBuf>, ShimError>;
pub fn remove_shim(layout: &crate::install::layout::Layout, name: &str) -> Result<(), ShimError>;
#[derive(Debug, thiserror::Error)]
pub enum ShimError { #[error("lançador de shims indisponível: {0}")] Launcher(String), #[error("E/S: {0}")] Io(String) }
```

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::shim::{pe_subsystem, set_pe_subsystem};

/// Minimal PE-looking buffer: MZ header with e_lfanew = 0x80, "PE\0\0", optional header subsystem at +24+68.
fn fake_pe(subsystem: u16) -> Vec<u8> {
    let mut b = vec![0u8; 0x80 + 24 + 96];
    b[0] = b'M'; b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let off = 0x80 + 24 + 68;
    b[off..off + 2].copy_from_slice(&subsystem.to_le_bytes());
    b
}

#[test]
fn reads_and_patches_subsystem() {
    let mut b = fake_pe(3);
    assert_eq!(pe_subsystem(&b), Some(3));
    assert!(set_pe_subsystem(&mut b, 2));
    assert_eq!(pe_subsystem(&b), Some(2));
    let off = 0x80 + 24 + 68;
    assert_eq!(&b[off..off + 2], &[2, 0]);
}

#[test]
fn invalid_or_short_files_are_not_touched() {
    for mut b in [vec![], b"MZ".to_vec(), { let mut x = fake_pe(3); x[0x80] = b'X'; x }, { let mut x = fake_pe(3); x[0x3C..0x40].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes()); x }] {
        let before = b.clone();
        assert_eq!(pe_subsystem(&b), None);
        assert!(!set_pe_subsystem(&mut b, 2));
        assert_eq!(b, before);
    }
}
```

Testes de arquivos (prosa, em `tempdir` com um `launcher()` falso = `fake_pe(3)` gravado): shim `.exe` de alvo console ⇒ `<n>.exe` é hardlink/cópia idêntica ao lançador e `<n>.shim` tem exatamente `path = "<alvo>"\r\nargs = --x\r\n`; alvo GUI (`fake_pe(2)` como alvo) ⇒ `<n>.exe` com subsistema 2 e o lançador original continua 3; `.bat`, `.ps1` e `.jar` geram os wrappers com o texto exato acima; `remove_shim` apaga todos. `ensure_launcher` com `wiremock` servindo um zip cujo SHA-256 **não** confere ⇒ `ShimError::Launcher` e `launcher()` não existe (o hash fixo das Global Constraints é usado em produção; o teste injeta URL e hash via um parâmetro interno `LauncherSource` — exponha `ensure_launcher_from(http, data, layout, url, sha256)` e faça `ensure_launcher` chamá-la com as constantes).

#### Checks beyond the steps

1. Offset do subsistema: mutate `68` para `66` ⇒ PEGA.
2. Lançador original intocado no caso GUI: mutate para editar o próprio lançador ⇒ PEGA.
3. Hash do lançador verificado: mutate para pular a verificação ⇒ PEGA.

**Commit:** `feat(install): shims with kiennq launcher, PE subsystem and script wrappers`

---

### Task 31: Executor com staging e rollback

#### Where this fits

Aplica um `InstallPlan` e um `UninstallPlan`. Cada passo executado registra o seu "desfazer"; qualquer falha desfaz tudo em ordem inversa. `current` só muda no fim da instalação, então a versão anterior continua funcionando até lá.

#### What binds this task

- `execute_install(plan, ctx) -> Result<InstallReport, ExecError>`; `ctx` contém `Layout`, `DataDir`, `Http`, `Effects` (Task 27), `StateDb`, e um `hook: &dyn StepHook` com `fn before(&self, index: usize, step: &Step) -> Result<(), ExecError>` (produção: no-op; testes: falha no passo k).
- Desfazer por passo: `Download` ⇒ nada (cache é compartilhado); `Extract`/`Promote` ⇒ remover staging/versão criada; `Persist` ⇒ remover junction/hardlink criados **e** remover em `persist\` só o que este passo criou (se `persist\<app>\x` já existia, nunca é apagado); `LinkCurrent` ⇒ restaurar a junction anterior (ou remover); `Shim` ⇒ `remove_shim`; `Shortcut` ⇒ `Shortcuts::remove`; `EnvSet` ⇒ restaurar o valor anterior lido antes (ou remover); `EnvAddPath`/`EnsureShimsOnPath` ⇒ `path_remove` da entrada **se este passo a adicionou**; `RecordState` ⇒ restaurar o registro anterior; `RemoveOldVersion` ⇒ é o último passo e só roda após todos os outros; falha nele ⇒ registra aviso, não desfaz a instalação.
- `PersistKind` do plano é palpite (regra do ponto no último componente). Como no `persist_data` do scoop, **o disco decide**: se a origem existe na versão extraída, use o tipo real dela (pasta ⇒ junction, arquivo ⇒ hardlink); só quando não existe nem na versão nem em `persist\` vale o `PersistKind` do plano (pasta ⇒ cria pasta vazia; arquivo ⇒ arquivo vazio). Teste: persist `data.d` que é pasta no zip ⇒ junction.
- `Persist` (primeira instalação): se a pasta/arquivo existe na versão extraída e **não** existe em `persist\`, **move** para `persist\` e cria o link; se não existe em nenhum, cria vazio em `persist\` (pasta ou arquivo de 0 bytes) e o link; se existe em `persist\`, remove o da versão e cria o link (preserva dados).
- Desinstalação (`UninstallPlan`, montado a partir do `state.db`): remove shims, atalhos, env (restaurando `previous`), entrada do PATH se `path_users` vazio, `apps\<app>`, `persist\<app>` só com `purge`, e o registro. Antes de qualquer remoção chama `fs_win::files_in_use(dir)` (Task 33) ⇒ se houver, `ExecError::InUse(caminho)` sem mudar nada.
- `InstallReport { undone: Vec<String> }` em falha vira parte da mensagem ("desfeito: …").

**Files:**
- Modify (stub gerado pela Task 27): `src/install/executor.rs`, `src/install/plan.rs` (`UninstallPlan`)
- Test (new): `tests/install_executor.rs`

#### Interfaces

```rust
pub trait StepHook: Send + Sync { fn before(&self, index: usize, step: &crate::install::plan::Step) -> Result<(), ExecError>; }
pub struct NoHook; impl StepHook for NoHook { /* Ok(()) */ }
pub struct ExecCtx<'a> {
    pub layout: &'a crate::install::layout::Layout, pub data: &'a crate::cache::DataDir, pub http: &'a crate::http::Http,
    pub effects: crate::install::traits::Effects<'a>, pub state: &'a mut crate::install::state::StateDb,
    pub hook: &'a dyn StepHook, pub progress: crate::install::download::Progress<'a>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan { pub app: String, pub purge: bool, pub artifacts: Vec<crate::install::state::Artifact>, pub remove_path_entry: bool }
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("falha no passo {index} ({step}): {message}; desfeito: {undone:?}")] Failed { index: usize, step: String, message: String, undone: Vec<String> },
    #[error("arquivo em uso: {0} — feche o programa e tente de novo")] InUse(String),
    #[error("{0}")] Hook(String),
}
pub async fn execute_install(plan: &crate::install::plan::InstallPlan, ctx: &mut ExecCtx<'_>) -> Result<(), ExecError>;
pub fn execute_uninstall(plan: &UninstallPlan, ctx: &mut ExecCtx<'_>) -> Result<(), ExecError>;
```

#### The body of the tests that are not obvious

O teste central (Review: "nunca meio instalado") — usa um bucket local: um zip real pequeno servido por `wiremock`, `Layout` em `tempdir`, fakes de env/atalhos, e o lançador falso já presente (`launcher_present: true`, arquivo `fake_pe(3)` em `layout.launcher()`).

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
// helpers (escreva no próprio arquivo): `fixture_zip()` -> Vec<u8> com "pkg/tool.exe" (bytes fake_pe(3)),
// "pkg/conf/default.ini"; `manifest_json(url, sha256)` com extract_dir "pkg", bin "tool.exe",
// shortcuts [["tool.exe","Tool"]], persist ["conf"], env_set {"TOOL_HOME":"$dir"}, env_add_path ["."].
// `setup()` -> (MockServer, TempDir, Layout, DataDir, FakeEnv, FakeShortcuts, FakeTrash, FakeRegistry, StateDb, InstallPlan)

struct FailAt(usize);
impl wingetrs::install::executor::StepHook for FailAt {
    fn before(&self, i: usize, _s: &wingetrs::install::plan::Step) -> Result<(), wingetrs::install::executor::ExecError> {
        if i == self.0 { Err(wingetrs::install::executor::ExecError::Hook(format!("injected at {i}"))) } else { Ok(()) }
    }
}

#[tokio::test]
async fn failure_at_any_step_restores_the_initial_state_exactly() {
    let probe = setup().await;
    let n = probe.plan.steps.len();
    assert!(n >= 10, "plan must exercise every step kind: {n}");
    for k in 0..n {
        let s = setup().await; // fresh world each time
        let before_fs = wingetrs::install::snapshot::snapshot(s.layout.root()).unwrap();
        let before_env = s.env.dump();
        let before_sc = s.shortcuts.dump();
        let before_state = s.state.apps().unwrap();
        let mut ctx = s.ctx_with_hook(&FailAt(k));
        let err = wingetrs::install::executor::execute_install(&s.plan, &mut ctx).await.unwrap_err();
        assert!(err.to_string().contains(&format!("injected at {k}")), "{err}");
        drop(ctx);
        // blobs/ may keep the verified download (shared cache); everything else must be identical
        let mut after_fs = wingetrs::install::snapshot::snapshot(s.layout.root()).unwrap();
        let mut b = before_fs.clone();
        after_fs.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        b.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        assert_eq!(after_fs, b, "filesystem differs after failing at step {k}");
        assert_eq!(s.env.dump(), before_env, "env differs after step {k}");
        assert_eq!(s.shortcuts.dump(), before_sc, "shortcuts differ after step {k}");
        assert_eq!(s.state.apps().unwrap(), before_state, "state differs after step {k}");
    }
}

#[tokio::test]
async fn full_install_then_uninstall_leaves_nothing_but_persist() {
    let s = setup().await;
    let before_env = s.env.dump();
    let mut ctx = s.ctx_with_hook(&wingetrs::install::executor::NoHook);
    wingetrs::install::executor::execute_install(&s.plan, &mut ctx).await.unwrap();
    assert!(s.layout.current("app").join("tool.exe").exists());
    assert!(s.layout.persist_dir("app").join("conf/default.ini").exists(), "persist moved out of the version dir");
    assert_eq!(s.env.get_raw("TOOL_HOME").as_deref(), Some(s.layout.current("app").to_str().unwrap()));
    let up = s.uninstall_plan(false);
    wingetrs::install::executor::execute_uninstall(&up, &mut ctx).unwrap();
    assert!(!s.layout.app_dir("app").exists());
    assert!(s.layout.persist_dir("app").exists(), "persist kept without --purge");
    assert_eq!(s.env.dump(), before_env);
    assert!(s.shortcuts.dump().is_empty());
}
```

Acrescente (prosa): (a) `--purge` remove `persist\app`; (b) **upgrade**: instalar v1, escrever `persist\app\conf\user.ini`, instalar v2 ⇒ `user.ini` intacto, `current` aponta para v2, v1 removida (e mantida com `keep_old`); (c) falha injetada durante o upgrade ⇒ `current` ainda aponta para v1 e v1 funciona; (d) `EnvSet` sobre variável pré-existente ⇒ uninstall restaura o valor anterior.

#### Checks beyond the steps

1. Rollback de `LinkCurrent`: mutate para não restaurar a junction anterior ⇒ teste (c) PEGA.
2. Persist pré-existente nunca apagado no rollback: mutate o desfazer de `Persist` para remover sempre ⇒ teste de upgrade com falha PEGA (escreva-o).
3. `EnvSet` restaura valor anterior: mutate para remover em vez de restaurar ⇒ teste (d) PEGA.
4. Rollback completo: mutate pulando o desfazer de `Shim` ⇒ `failure_at_any_step…` PEGA.

**Commit:** `feat(install): executor with staging and full rollback`

---

### Task 32: `EnvStore` e `Shortcuts` reais (Windows)

#### Where this fits

Implementações reais em `env_win.rs` e `shortcut_win.rs`. É onde está o maior risco ao seu Windows: o `Path` do usuário.

#### What binds this task

- `RegEnvStore`: chave `HKCU\Environment`, **ou** `HKCU\<WINGETRS_TEST_ENV_KEY>` quando a variável existir (testes). Lê o tipo do valor: `REG_EXPAND_SZ` ⇒ `expand: true`, `REG_SZ` ⇒ `false`; **grava com o mesmo tipo** (para `Path`, se não existia, `REG_EXPAND_SZ`). Lê e grava o texto **sem expandir** (`RegGetValueW` com `RRF_NOEXPAND`, ou `winreg` lendo raw bytes UTF-16 — nunca `ExpandEnvironmentStrings`).
- `broadcast()`: `SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, "Environment", SMTO_ABORTIFHUNG, 2000)` (verificado na sondagem); com `WINGETRS_TEST_ENV_KEY` definida, **não** transmite.
- `ComShortcuts`: raiz `%APPDATA%\Microsoft\Windows\Start Menu\Programs\wingetrs Apps` (via `SHGetKnownFolderPath(FOLDERID_Programs)`), ou `WINGETRS_TEST_SHORTCUT_DIR`; cria subpastas do `link`; `IShellLinkW` + `IPersistFile::Save` (verificado); `CoInitializeEx` por thread (`COINIT_APARTMENTTHREADED`), tolerando `RPC_E_CHANGED_MODE`; `remove` apaga o `.lnk` e as subpastas que ficaram vazias **dentro** da raiz (nunca a raiz em si além de `wingetrs Apps` quando vazia).

**Files:**
- Modify (stub gerado pela Task 27): `src/install/env_win.rs`, `src/install/shortcut_win.rs`
- Test (new): `tests/install_win_effects.rs` (testes `#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]`)

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]
use wingetrs::install::env_win::RegEnvStore;
use wingetrs::install::traits::{EnvStore, EnvValue};

fn test_key() -> String {
    let k = format!(r"Software\wingetrs-test\env-{}-{}", std::process::id(), line!());
    unsafe { std::env::set_var("WINGETRS_TEST_ENV_KEY", &k) };
    k
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn expand_sz_is_preserved_verbatim() {
    let k = test_key();
    let env = RegEnvStore::new();
    let user = EnvValue { value: r"%USERPROFILE%\bin;;C:\Tools\".into(), expand: true };
    env.set("Path", &user).unwrap();
    assert_eq!(env.get("Path").unwrap(), Some(user.clone()), "read back without expansion");
    let added = wingetrs::install::path_edit::path_add(&user.value, r"C:\W\shims");
    env.set("Path", &EnvValue { value: added.clone(), expand: true }).unwrap();
    let raw = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(&k).unwrap().get_raw_value("Path").unwrap();
    assert_eq!(raw.vtype, winreg::enums::RegType::REG_EXPAND_SZ);
    assert_eq!(env.get("Path").unwrap().unwrap().value, added);
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey_all(&k).unwrap();
}
```

Acrescente (prosa, também `#[ignore = "live: …"]`): `REG_SZ` existente continua `REG_SZ` após `set`; `remove` apaga o valor; atalho criado em `tempdir` via `WINGETRS_TEST_SHORTCUT_DIR` com subpasta, `.lnk` existe e o alvo resolvido (`IShellLinkW::GetPath`) é o esperado; `remove` apaga o `.lnk` e a subpasta vazia. Um teste **não-ignorado** garante que, sem `WINGETRS_TEST_ENV_KEY`, `RegEnvStore::key_path()` é `Environment` (para o teste não depender do registro real, `key_path` é função pura).

#### Checks beyond the steps

1. Tipo preservado: mutate para gravar sempre `REG_SZ` ⇒ live PEGA (rode com `-- --ignored`; cole a saída).
2. Sem expansão: mutate lendo com expansão ⇒ live PEGA.
3. Broadcast desligado em teste: inspeção + teste puro de `should_broadcast()`.

**Commit:** `feat(install): real environment and shortcut effects for Windows`

---

### Task 33: `Trash`, `RegistryScan` e arquivo em uso (Windows)

#### Where this fits

Implementações reais do envio à Lixeira, da varredura de resíduos no registro do usuário e da detecção de arquivos em uso antes de remover/atualizar.

#### What binds this task

- `ShellTrash::recycle(path)`: `IFileOperation` com `FOF_ALLOWUNDO | FOF_NO_UI | FOFX_RECYCLEONDELETE` (verificado).
- `RegScanner`: só `HKCU`; `values_containing(roots, needle)` percorre recursivamente as raízes dadas (limite de profundidade 8) comparando dados `REG_SZ`/`REG_EXPAND_SZ` case-insensitive; `subkeys_named`; `export` via `reg.exe export "HKCU\<key>" "<file>" /y` (processo, sem shell); `delete_value`/`delete_key`. Com `WINGETRS_TEST_ENV_KEY` definida, toda chave é relativa a ela (testes).
- `files_in_use(dir) -> Vec<PathBuf>`: para cada `.exe`/`.dll` sob `dir`, tenta abrir com `share_mode(0)` (sem compartilhamento); `ERROR_SHARING_VIOLATION` ⇒ em uso.
- Desempenho: `values_containing` nas raízes da spec (§8) em < 300 ms nesta máquina — meça com `tracing` span e cole.

**Files:**
- Modify (stub gerado pela Task 27): `src/install/trash_win.rs`, `src/install/registry_win.rs`, `src/install/fs_win.rs`
- Test (new): `tests/install_win_scan.rs` (live `#[ignore]` para registro/Lixeira; `files_in_use` roda normal)

Testes (prosa): `files_in_use` com um arquivo aberto sem compartilhamento por outro handle no próprio teste ⇒ listado; fechado ⇒ vazio. Live: criar `HKCU\<test>\Classes\x\shell\open\command` com valor apontando para `C:\W\apps\app\x.exe` ⇒ `values_containing` acha; `export` gera `.reg` que contém a chave; `delete_key` remove. Lixeira (live): arquivo em `tempdir` some do disco após `recycle`.

#### Checks beyond the steps

1. Comparação case-insensitive: mutate para sensível à caixa ⇒ live PEGA.
2. `files_in_use` detecta violação: mutate para abrir com compartilhamento total ⇒ PEGA.

**Commit:** `feat(install): recycle bin, HKCU residue scan and in-use detection`

---

### Task 34: Resíduos — classificação e plano de limpeza

#### Where this fits

Depois de um `uninstall`, decide (puro) quais resíduos sugerir e com qual confiança, e executa a limpeza escolhida de forma recuperável.

#### What binds this task

- Entradas: `app`, pasta removida (`apps\<app>\`), `homepage` do manifesto, os hits de `RegistryScan` nas raízes da spec §8, os `.lnk` encontrados (Área de Trabalho, Menu Iniciar, Inicialização) com seus alvos, pastas candidatas em `%APPDATA%`/`%LOCALAPPDATA%` e subchaves em `HKCU\Software`, e `other_installs: bool` (o mesmo app aparece no `list` como scoop/winget).
- Alta confiança (pré-marcado): hit cujo dado contém a pasta removida; `.lnk` cujo alvo está nela.
- Baixa confiança (desmarcado): pasta/chave com nome igual (case-insensitive) ao `app` ou ao dono do repositório quando `homepage` é `https://github.com/<dono>/…`.
- `other_installs == true` ⇒ todos os itens de baixa confiança ganham a nota "pode ser de outra instalação" e **nunca** pré-marcados; itens de alta confiança continuam pré-marcados (apontam para a pasta removida, não podem ser de outra instalação).
- Execução: pastas ⇒ `Trash::recycle`; chaves/valores ⇒ `RegistryScan::export` para `backups\<AAAAMMDD-HHMMSS>\<app>.reg` antes de `delete_*`; export falhou ⇒ não apaga aquela chave.

**Files:**
- Modify (stub gerado pela Task 27): `src/install/residue.rs`
- Test (new): `tests/install_residue.rs`

#### Interfaces

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidueKind { RegValue { key: String, value_name: Option<String> }, RegKey { key: String }, Folder { path: std::path::PathBuf }, Shortcut { path: std::path::PathBuf } }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence { High, Low }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residue { pub kind: ResidueKind, pub confidence: Confidence, pub preselected: bool, pub note: Option<String> }
pub struct ResidueInput<'a> {
    pub app: &'a str, pub removed_dir: &'a std::path::Path, pub homepage: Option<&'a str>,
    pub reg_hits: &'a [crate::install::traits::RegHit], pub shortcuts: &'a [(std::path::PathBuf, std::path::PathBuf)],
    pub folders: &'a [std::path::PathBuf], pub software_keys: &'a [String], pub other_installs: bool,
}
pub fn classify(input: &ResidueInput<'_>) -> Vec<Residue>;
pub fn clean(selected: &[Residue], app: &str, effects: &crate::install::traits::Effects<'_>, backups: &std::path::Path) -> Vec<Result<(), String>>;
```

Testes (prosa, com `FakeRegistry`/`FakeTrash`): hit apontando para a pasta ⇒ `High`+pré-marcado; pasta `%APPDATA%\ripgrep` ⇒ `Low`+desmarcado; homepage `https://github.com/BurntSushi/ripgrep` ⇒ `HKCU\Software\BurntSushi` `Low`; `other_installs` ⇒ nota e nunca pré-marcado; `clean` exporta antes de apagar (ordem verificada no `FakeRegistry`); export falhou ⇒ chave intacta.

#### Checks beyond the steps

1. `other_installs` nunca pré-marca baixa confiança: mutate ⇒ PEGA.
2. Export antes de apagar: mutate invertendo a ordem ⇒ PEGA.
3. Alta confiança exige a pasta removida no dado: mutate para casar só pelo nome do app ⇒ PEGA.

**Commit:** `feat(install): residue classification and recoverable cleanup`

---

### Task 35: Resolução do alvo e seletor por setas

#### Where this fits

Converte o que o usuário digitou (`ripgrep`, `"Visual Studio Code"`, `extras/vscode`, `--bucket`) em candidatos e, havendo mais de um, mostra a lista para escolher com setas — o mesmo componente na CLI (inline) e na TUI.

#### What binds this task

- Candidatos vêm do catálogo (fase 1): ID exato (case-insensitive) ⇒ moniker/nome exato ⇒ senão `NotFound { suggestions }` com até 5 do fuzzy (nunca instala por fuzzy). `bucket/id` e `--bucket` restringem a `scoop:<bucket>`. Candidatos do winget entram com `selectable: false` e nota "instalação na fase 2d".
- `Picker` é estado puro: `items`, `selected`, `checked` (modo múltiplo, usado pelos resíduos), teclas `Up/Down/Home/End/Enter/Esc/Space`; pula itens não selecionáveis.
- CLI: `pick_inline(items) -> Option<usize>` desenha com `crossterm` **sem tela alternativa** (raw mode, cursor, redesenho das linhas da lista, restaura tudo ao sair, inclusive em erro); só em stdin/stdout TTY. Sem TTY ⇒ não chama o seletor.
- Estilos via `tui::theme` (tokens) e glifos via `Glyphs`.

**Files:**
- Modify (stub gerado pela Task 27): `src/install/target.rs`
- Create (new): `src/cli/picker.rs`, `src/tui/picker.rs` (estado puro compartilhado fica em `src/tui/picker.rs`; `src/cli/picker.rs` só desenha inline)
- Test (new): `tests/install_target.rs`

#### The body of the tests that are not obvious

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use wingetrs::tui::picker::{PickOutcome, Picker, PickerItem};

fn item(id: &str, selectable: bool) -> PickerItem { PickerItem { label: id.into(), detail: String::new(), selectable, note: None } }
fn k(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }

#[test]
fn arrows_skip_unselectable_and_enter_picks() {
    let mut p = Picker::single(vec![item("a", true), item("winget", false), item("b", true)]);
    assert_eq!(p.selected(), 0);
    assert_eq!(p.key(k(KeyCode::Down)), PickOutcome::Pending);
    assert_eq!(p.selected(), 2, "skips the winget row");
    assert_eq!(p.key(k(KeyCode::Down)), PickOutcome::Pending);
    assert_eq!(p.selected(), 2, "stays at the end");
    assert_eq!(p.key(k(KeyCode::Up)), PickOutcome::Pending);
    assert_eq!(p.selected(), 0);
    assert_eq!(p.key(k(KeyCode::Enter)), PickOutcome::Picked(vec![0]));
}

#[test]
fn esc_cancels_and_multi_mode_toggles_with_space() {
    let mut p = Picker::single(vec![item("a", true)]);
    assert_eq!(p.key(k(KeyCode::Esc)), PickOutcome::Cancelled);
    let mut m = Picker::multi(vec![item("a", true), item("b", true), item("c", true)], &[true, false, false]);
    m.key(k(KeyCode::Down));
    m.key(k(KeyCode::Char(' ')));
    m.key(k(KeyCode::Up));
    m.key(k(KeyCode::Char(' ')));
    assert_eq!(m.key(k(KeyCode::Enter)), PickOutcome::Picked(vec![1]));
}
```

Testes de resolução (prosa) com um `Catalog` em memória: `ripgrep` exato; `RIPGREP`; `main/ripgrep`; `--bucket extras` sem o app ⇒ `NotFound`; nome igual em main e extras ⇒ 2 candidatos; ID existente só no winget ⇒ candidato `selectable: false`; termo inexistente ⇒ `NotFound` com sugestões, sem candidato.

#### Checks beyond the steps

1. Pular não selecionável: mutate ⇒ PEGA.
2. Nunca instalar por fuzzy: mutate retornando o 1º fuzzy como candidato ⇒ PEGA.

**Commit:** `feat(install): target resolution and arrow-key picker`

---

### Task 36: Buckets — `add/list/rm`

#### Where this fits

Permite acrescentar buckets do GitHub além de main/extras, generalizando a URL do codeload que a fase 1 já usa.

#### What binds this task

- Conhecidos por nome (de `buckets.json` oficial do scoop): main, extras, versions, nerd-fonts, java, games, nirsoft, sysinternals, php, nonportable ⇒ `ScoopInstaller/<Nome>` exceto `nirsoft` ⇒ `ScoopInstaller/Nirsoft`, `sysinternals` ⇒ `niheaven/scoop-sysinternals`, `nerd-fonts` ⇒ `matthewjberger/scoop-nerd-fonts`, `java` ⇒ `ScoopInstaller/Java`, `games` ⇒ `Calinou/scoop-games`, `php` ⇒ `ScoopInstaller/PHP`, `nonportable` ⇒ `ScoopInstaller/Nonportable`, `versions` ⇒ `ScoopInstaller/Versions`. **Antes de codificar, confirme cada um** baixando `https://raw.githubusercontent.com/ScoopInstaller/Scoop/master/buckets.json` (somente leitura) e use o que o arquivo disser; registre no relatório.
- URL aceita: `owner/repo`, `https://github.com/owner/repo(.git)?`. Outro host ⇒ erro "só buckets do GitHub nesta versão".
- Config em `config.toml` (`[[buckets]] name, repo`), mantendo `scoop_buckets` da fase 1 como lista de nomes conhecidos (migração: nome conhecido sem entrada ⇒ usa o repo conhecido).
- `bucket add` de repo não conhecido ⇒ exige confirmação (a CLI passa `confirmed: bool`); `bucket rm` recusa se `state.db` tem app desse bucket (lista quais) e remove a fonte `scoop:<nome>` do catálogo.
- `Endpoints::production` passa a montar as URLs a partir da lista configurada.

**Files:**
- Create (new): `src/buckets.rs`
- Modify: `src/lib.rs`, `src/config.rs`, `src/update.rs`, `src/sources/scoop/bucket.rs`
- Test (new): `tests/buckets.rs`

Testes (prosa): parse de `owner/repo` e URLs (com/sem `.git`, barra final); host não-GitHub ⇒ erro; nome conhecido resolve o repo; `add` desconhecido sem confirmação ⇒ erro específico; `rm` com app instalado ⇒ erro listando; `Endpoints` inclui o bucket adicionado (URL do codeload `https://codeload.github.com/<owner>/<repo>/tar.gz/refs/heads/master` — branch padrão: usar `HEAD` em vez de `master` se o codeload aceitar `refs/heads/HEAD`? **Não**: confirme com uma requisição `curl -sI` a `https://codeload.github.com/ScoopInstaller/Main/tar.gz/HEAD` e use a forma que responder 200; registre).

#### Checks beyond the steps

1. Host não-GitHub recusado: mutate ⇒ PEGA.
2. `rm` com app instalado recusado: mutate ⇒ PEGA.

**Commit:** `feat(buckets): add, list and remove GitHub buckets`

---

### Task 37: Operações — install/upgrade/uninstall/repair com dependências e lock

#### Where this fits

A API que CLI e TUI chamam: junta resolução (35), manifesto (24), planejador (28), executor (31), estado (29), resíduos (34) e efeitos reais (32/33), com dependências, `install.lock` e integração com o `list`.

#### What binds this task

- `install.lock` (`UpdateLock` da fase 1 sobre `layout.install_lock()`): CLI espera até 30 s (`WINGETRS_LOCK_TIMEOUT_MS` para teste); TUI usa `NoWait` ⇒ `Ocupado` (Review Focus 4).
- Dependências: resolve `depends` recursivamente pelo catálogo (`bucket/app` ou `app`); ciclo ⇒ `DependencyCycle(cadeia)`; dependência não suportada ⇒ falha antes de qualquer download; já instalada pelo wingetrs ⇒ pula; plano final = dependências (ordem topológica) + alvo.
- `upgrade --all`: todos os apps do `state.db` com versão mais nova no catálogo (comparação `core::version`).
- `uninstall`: recusa se outro app do wingetrs depende dele (lista quem); `files_in_use` antes (Review Focus 3); depois executa e devolve os resíduos classificados (a CLI/TUI confirmam e chamam `clean`).
- `repair`: reconstrói `state.db` a partir de `apps\*\current\install.json` e de `shims\*.shim` (alvos que apontam para `apps\<app>\`), marcando artefatos de PATH se `…\wingetrs\shims` está no `Path`.
- `list`: `scan_all` (fase 1) inclui `scan_wingetrs`; correlação de app `Wingetrs` usa a mesma regra do scoop (`scoop:<bucket>` + app); `ListRow` mostra a origem "wingetrs"; app quebrado ⇒ estado "quebrado".
- Toda a orquestração recebe `Effects` por parâmetro: testes usam fakes; produção monta as reais.
- **Broadcast de ambiente (da Task 31):** o executor nunca chama `EnvStore::broadcast()`. `apply` e `uninstall` chamam `ctx.effects.env.broadcast()` **uma vez** ao final, se algum plano mexeu em env/PATH (sucesso ou rollback que tocou env). Teste com `FakeEnv`: dois planos com env ⇒ contador de broadcast = 1; plano sem env ⇒ 0.
- **Hash do manifesto (da Task 31):** o executor grava `AppRecord.manifest_sha256` vazio porque o `InstallPlan` não o carrega; `ops` calcula o SHA-256 do JSON do manifesto usado e o põe no plano (campo novo `InstallPlan.manifest_sha256: String`, preenchido pelo planejador a partir de um parâmetro) — teste: após `apply`, o registro tem o hash do JSON da fixture.
- **Arquivo em uso (da Task 31):** `ExecCtx.in_use` é injetado; `OpsCtx` ganha `pub in_use: &'a (dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync)` e a produção (CLI/TUI) passa `install::fs_win::files_in_use`.

**Files:**
- Create (new): `src/install/ops.rs` (e `pub mod ops;` em `src/install/mod.rs`)
- Modify: `src/installed/mod.rs`, `src/installed/correlate.rs`, `src/cli/output.rs` (coluna origem)
- Test (new): `tests/install_ops.rs`

#### Interfaces

```rust
pub struct OpsCtx<'a> { pub data: &'a crate::cache::DataDir, pub http: &'a crate::http::Http, pub catalog: &'a crate::catalog::Catalog, pub effects: crate::install::traits::Effects<'a>, pub wait: crate::update::LockWait, pub progress: crate::install::download::Progress<'a> }
pub enum Resolution { One(Candidate), Many(Vec<Candidate>), NotFound { suggestions: Vec<String> } }
pub fn resolve_target(ctx: &OpsCtx<'_>, input: &str, bucket: Option<&str>) -> Result<Resolution, OpsError>;
pub fn plan_for(ctx: &OpsCtx<'_>, c: &Candidate, keep_old: bool) -> Result<Vec<crate::install::plan::InstallPlan>, OpsError>; // deps + target
pub async fn apply(ctx: &OpsCtx<'_>, plans: &[crate::install::plan::InstallPlan]) -> Result<(), OpsError>;
pub fn plan_uninstall(ctx: &OpsCtx<'_>, app: &str, purge: bool) -> Result<crate::install::executor::UninstallPlan, OpsError>;
pub fn uninstall(ctx: &OpsCtx<'_>, plan: &crate::install::executor::UninstallPlan) -> Result<Vec<crate::install::residue::Residue>, OpsError>;
pub fn upgradable(ctx: &OpsCtx<'_>) -> Result<Vec<(String, String, String)>, OpsError>; // app, from, to
pub fn repair(ctx: &OpsCtx<'_>) -> Result<RepairReport, OpsError>;
#[derive(Debug, thiserror::Error)]
pub enum OpsError { /* NotFound, Ambiguous, Unsupported, DependencyCycle(Vec<String>), DependencyUnsupported{app,reason}, HasDependents(Vec<String>), InUse(String), Busy, Plan(..), Exec(..), Manifest(..), Catalog(..), State(..) — mensagens em português */ }
```

Testes (prosa, com catálogo em memória contendo manifestos das fixtures e fakes): dependência resolvida em ordem; ciclo (A→B→A) ⇒ `DependencyCycle(["a","b","a"])`; dependência não suportada ⇒ falha sem download (mock com `.expect(0)`); lock ocupado + `NoWait` ⇒ `Busy`; uninstall com dependente ⇒ `HasDependents`; `repair` reconstrói o `state.db` apagado após uma instalação (mesmos apps e artefatos de shim); `list` mostra o app instalado com origem wingetrs.

#### Checks beyond the steps

1. Ciclo detectado: mutate ⇒ PEGA.
2. Dependência não suportada aborta antes do download: mutate ⇒ PEGA (mock `expect(0)`).
3. `HasDependents`: mutate ⇒ PEGA.

**Commit:** `feat(install): operations with dependencies, lock, repair and list integration`

---

### Task 38: CLI — comandos, confirmação, dry-run, progresso

#### What binds this task

- Comandos novos (ajuda em português, como a Task 23): `install <alvo> [--bucket B] [--dry-run] [--yes]`, `uninstall <alvo> [--purge] [--dry-run] [--yes]`, `upgrade [<alvo>|--all] [--keep] [--dry-run] [--yes]`, `bucket add <nome> [<url>] [--yes]`, `bucket list`, `bucket rm <nome>`, `repair`.
- Ambiguidade: TTY ⇒ `pick_inline`; sem TTY ou `--yes` ⇒ erro listando candidatos com `bucket/id` (spec §5).
- Confirmação: imprime o plano (`Display`) e pergunta `Continuar? [s/N]` (TTY); `--yes` pula; `--dry-run` imprime e sai **sem tocar no disco** (teste: snapshot da raiz idêntico).
- Progresso de download numa linha (`\r`), só em TTY.
- Resíduos após `uninstall`: TTY ⇒ `pick_inline` em modo múltiplo com os pré-marcados; `--yes` ⇒ só alta confiança.
- Efeitos reais: `RegEnvStore`, `ComShortcuts`, `ShellTrash`, `RegScanner`.

**Files:**
- Modify: `src/cli/mod.rs`, `src/cli/output.rs`, `src/cli/picker.rs` (gerado na Task 35)
- Test: `tests/cli_parse.rs` (novos comandos), `tests/cli_install.rs` (new: binário + `wiremock` + `LOCALAPPDATA` em `tempdir` + `WINGETRS_TEST_ENV_KEY`/`WINGETRS_TEST_SHORTCUT_DIR`)

Testes (prosa): parse de todos os comandos e flags; `install --dry-run --yes` não altera a raiz (snapshot); `install --yes` de app ambíguo sem TTY ⇒ código ≠ 0 e stderr com `main/x` e `extras/x`; ciclo `install --yes` → `uninstall --yes` com bucket falso ⇒ raiz final igual à inicial (exceto `blobs`, `catalog.db`, `search.idx`, `http.json`, `state.db` vazio, `log`).

#### Checks beyond the steps

1. `--dry-run` não toca: mutate executando o plano no dry-run ⇒ PEGA.
2. `--yes` sem TTY não escolhe sozinho: mutate escolhendo o 1º ⇒ PEGA.

**Commit:** `feat(cli): install, uninstall, upgrade, bucket and repair commands`

---

### Task 39: TUI — teclas, confirmação, progresso

#### What binds this task

- Teclas (com `query` vazia, como `j/k` hoje): `i` instalar o selecionado (só scoop; winget ⇒ status "instalação na fase 2d"), `u` remover (aba Instalados, app do wingetrs), `U` atualizar (aba Atualizações; `U` com seleção vazia ⇒ todos com confirmação).
- Novo estado modal no `App`: `Confirm { plan_text, action }`, `Picking(Picker)`, `Running { label, progress }`, `Residues(Picker)`. `Enter` confirma, `Esc` cancela.
- Execução em task tokio com `install.lock` em `NoWait` (`Busy` ⇒ status "outra instalação em andamento"); progresso por `Msg::InstallProgress`; fim ⇒ `Msg::InstallFinished(Result)` e reescaneia instalados.
- `view` desenha os modais (snapshots `insta` com `GlyphSet::Ascii`/`NoColor`): confirmação com o plano, seletor de candidatos, progresso, lista de resíduos.

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/run.rs`
- Test: `tests/tui_state.rs`, `tests/tui_view.rs`, `tests/snapshots/*`

Testes (prosa): `i` em scoop abre `Confirm` (via `Cmd::PlanInstall` → `Msg::Planned`); `i` em winget não abre e mostra status; `Esc` no `Confirm` volta; `Busy` mostra status certo; snapshots dos 4 modais; terminal 20×5 com modal aberto não entra em pânico.

#### Checks beyond the steps

1. winget não instalável: mutate permitindo ⇒ PEGA.
2. Modal fecha com `Esc`: mutate ⇒ PEGA.

**Commit:** `feat(tui): install, uninstall and upgrade with confirmation and progress`

---

### Task 40: Ciclo completo, prova ao vivo e benchmark

#### What binds this task

- Relatório `docs/superpowers/reports/2026-10-10-fase2a-validacao.md` com **saídas coladas**; "não medido" é aceitável, número inventado não.
- **Prova ao vivo** (o orquestrador pede autorização ao usuário antes): com `LOCALAPPDATA` real, `wingetrs install ripgrep --yes`; terminal novo `rg --version`; `wingetrs uninstall ripgrep --yes`; verificar: `apps\ripgrep` não existe, `shims\rg.*` não existe, `Path` do usuário igual ao valor anterior (salvo antes com `reg query HKCU\Environment /v Path`), nenhum `.lnk` novo. Sem autorização ⇒ registre "não executado".
- Benchmarks (`bench.ps1 -Baseline`, máquina ociosa, `leftovers.ps1` limpo antes): `install --yes` de app com download em cache (meta < 1 s); `uninstall --yes`; `install --dry-run`; tempo do planejador (span).
- Oráculo de compatibilidade: os 19 fixtures + uma amostra de 50 manifestos "simples" do catálogo real passam por `resolve` + `plan_install` sem erro (só planeja; nada instala) — cole a contagem.

#### Checks beyond the steps

Tarefa de relatório: **no mutation proof** (declare no relatório). A verificação é o orquestrador reexecutar um benchmark com `-Baseline` e conferir a prova ao vivo.

**Files:**
- Create (new): `docs/superpowers/reports/2026-10-10-fase2a-validacao.md`

**Commit:** `docs(report): phase 2a validation`


---

## Desempenho pós-validação (Tasks 41–43)

Origem: prova ao vivo da Task 40 (relatório §1). Medido: `install` com cache e efeitos reais ~1000 ms, dos quais o broadcast `WM_SETTINGCHANGE` custa 438–568 ms (2,5 s com janela lenta); `upgrade --all` (CLI `src/cli/install_cmd.rs` fn `upgrade`, ~linhas 407–434) e a TUI (`src/tui/jobs.rs` `run_action`, `Action::Apply`) chamam `ops::apply` **uma vez por app**, então pegam o lock e podem fazer broadcast uma vez por app; o executor baixa cada `Step::Download` em sequência (`src/install/executor.rs` ~433), então o paralelismo de `download::fetch_all` (`MAX_PARALLEL = 4`) nunca é usado. O log em arquivo (`src/cli/mod.rs` `init_logging`) não tem filtro: o `update --background` grava TRACE do `h2` (9,5 MB num dia) e os arquivos diários nunca são apagados.

### Task 41: Lote numa só aplicação (um lock, um broadcast)

#### Where this fits

`upgrade --all` e a atualização em lote da TUI passam a aplicar todos os planos numa única chamada de `ops::apply`: um `install.lock`, um broadcast no fim, falha no meio relatada como `PartlyApplied` com os apps já aplicados.

#### What binds this task

- Nova função pura `pub fn merge_groups(groups: Vec<Vec<InstallPlan>>) -> Vec<InstallPlan>` em `src/install/ops.rs`: concatena na ordem dos grupos e, dentro de cada grupo, na ordem original; um plano cujo `app` (comparação sem caixa) já apareceu antes é descartado (dependência comum a dois grupos aparece uma vez, na primeira posição).
- Nova `pub async fn apply_groups(ctx: &OpsCtx<'_>, groups: Vec<Vec<InstallPlan>>) -> Result<Vec<InstallPlan>, OpsError>` = `merge_groups` + **uma** chamada a `apply`; devolve os planos aplicados (para a CLI imprimir "Atualizado: app versão" de cada um). Não reimplemente nada de `apply`.
- CLI `upgrade` (`src/cli/install_cmd.rs`): substitui o laço `for g in &groups { ops::apply(..) }` por `apply_groups`; imprime "Atualizado: {app} {versão}" para cada plano devolvido. Texto do plano e do `--dry-run` não mudam.
- TUI `run_action` (`src/tui/jobs.rs`, `Action::Apply(groups)`): usa `apply_groups`; o resumo continua "Instalado: a, b".
- Falha no 2º app de um lote depois do 1º aplicado ⇒ `OpsError::PartlyApplied { applied: ["<1º>"], .. }` (hoje a CLI perde essa informação porque cada grupo é uma chamada).
- Nada muda em `install` de um alvo só (já é um grupo).
- **Raízes no `revalidate`** (`src/install/ops.rs` ~523): hoje só o **último** plano é tratado como alvo (`i < last && upgrade_from.is_none()` ⇒ dependência instalada por outro meio é pulada). Num lote mesclado, o alvo de cada grupo pode estar no meio. `apply_groups` deve passar o conjunto de alvos (o último plano de cada grupo, sem caixa) e o `revalidate` só pula **não-alvos**; um alvo que divergiu do `state.db` é sempre `StateChanged`. `apply(ctx, plans)` mantém a assinatura e o comportamento (alvo = último).

**Files:**
- Modify: `src/install/ops.rs`, `src/cli/install_cmd.rs`, `src/tui/jobs.rs`
- Test: `tests/install_ops.rs`, `tests/tui_jobs.rs`

Testes (literais em prosa; use o `World` existente de cada arquivo):
1. `merge_groups_keeps_order_and_drops_repeated_apps` (`tests/install_ops.rs`): grupos `[[leaf, a], [LEAF, b], [c]]` (planos reais via `w.plans`, ou construídos) ⇒ apps `["leaf", "a", "b", "c"]`.
2. `apply_groups_broadcasts_once_for_two_env_apps` (`tests/install_ops.rs`): dois apps independentes com `env: true` (campo `App.env` do teste), `apply_groups(ctx, vec![plans_a, plans_b])` ⇒ `w.broadcasts() == 1` e ambos no `state.db`.
3. `apply_groups_reports_what_was_applied_when_a_later_app_fails` (`tests/install_ops.rs`): app `a` normal; app `b` cujo zip servido é lixo com o SHA-256 **certo** (o hash confere, a extração falha) ⇒ `Err(PartlyApplied { applied == ["a"] })`, `a` no `state.db`, `b` não, `apps\b` não existe.
5. `apply_groups_refuses_a_target_installed_meanwhile_even_in_the_middle` (`tests/install_ops.rs`): planos de dois alvos novos `a` e `b` (grupos `[[a],[b]]`); antes de aplicar, instale `a` por fora (`w.install("a")`) ⇒ `apply_groups` devolve `Err(StateChanged("a"))` e `b` não foi instalado.
4. `run_action_applies_all_groups_with_one_broadcast` (`tests/tui_jobs.rs`): `Action::Apply(vec![g1, g2])` com dois apps que mexem em env ⇒ um broadcast (amplie o `World` do arquivo com env se preciso).

#### Checks beyond the steps

1. Dedupe de `merge_groups`: mutate removendo o filtro de repetidos ⇒ PEGA (teste 1).
2. Um broadcast: mutate fazendo `apply_groups` chamar `apply` por grupo ⇒ PEGA (teste 2).
3. TUI usa o lote: mutate voltando `run_action` ao laço por grupo ⇒ PEGA (teste 4).
4. Alvo no meio: mutate fazendo o `revalidate` voltar a usar só `i < last` ⇒ PEGA (teste 5).

**Commit:** `perf(ops): apply a batch of upgrades under one lock and one broadcast`

---

### Task 42: Downloads do lote em paralelo antes de aplicar

#### Where this fits

Depende da Task 41 (lote numa só `apply`). Com o lote inteiro em mãos, `apply` baixa todas as fontes de todos os planos de uma vez (até 4 simultâneas) antes de mexer em qualquer coisa; o executor depois encontra tudo no cache de blobs. Ganho esperado: tempo de download do lote cai da soma para perto do maior (na prova ao vivo: ~4,6 s → ~3,4 s para ripgrep+delta+cpu-z); bônus: download que falha aborta o lote **antes** de qualquer mudança.

#### What binds this task

- `src/install/download.rs`: nova `pub async fn fetch_each(http: &Http, data: &DataDir, sources: &[Source], progress: Progress<'_>) -> Vec<Result<PathBuf, DownloadError>>` — mesma ordem de `sources`, mesmo semáforo `MAX_PARALLEL` para **todas** as fontes (não um por plano), cache consultado antes, índice `index.json` gravado **uma vez** no fim com as entradas novas que deram certo. `fetch_all` passa a ser um invólucro de `fetch_each` (primeiro erro), comportamento inalterado.
- `src/install/ops.rs` `apply`: depois do lock e de `revalidate`, coleta `(app, source)` de todo `Step::Download` de todos os planos restantes e chama `fetch_each` **uma vez**; qualquer erro ⇒ retorna `OpsError::Download { app, error }` (variante nova, mensagem `"falha ao baixar {app}: {error}"`) **antes de executar qualquer plano** — sem `PartlyApplied`, sem broadcast, raiz intacta. Os downloads ficam **dentro** do lock (o `index.json` dos blobs não é seguro entre processos).
- O executor não muda: o `Step::Download` dele acha o blob no cache. O `EnsureLauncher` continua como está.
- Progresso: o callback já é chamado por URL; `LineProgress` (CLI) agrega por URL e `ProgressThrottle` (TUI) já é atômico — confirme que nada assume um download por vez; se assumir, corrija e teste.
- Concorrência: `join_all` sobre futures que pegam permissão do semáforo — nenhuma `spawn` (sem thread/tarefa órfã). Se usar `spawn`, aplique a regra de join/`resume_unwind` do CLAUDE.md.

**Files:**
- Modify: `src/install/download.rs`, `src/install/ops.rs`
- Test: `tests/install_download.rs`, `tests/install_ops.rs`

Testes:
1. `fetch_each_keeps_order_and_reports_each_failure` (`tests/install_download.rs`): fontes `[ok1, 404, ok2]` (wiremock) ⇒ `[Ok, Err, Ok]` com os caminhos de `ok1`/`ok2` em `blobs`.
2. `apply_downloads_the_batch_in_parallel` (`tests/install_ops.rs`): três apps independentes, cada zip servido com `ResponseTemplate::set_delay(Duration::from_millis(400))`; `apply` dos três planos concatenados ⇒ tempo total < 1000 ms (sequencial seria ≥ 1200 ms) e os três instalados.
3. `apply_fails_before_any_change_when_a_download_fails` (`tests/install_ops.rs`): app `a` ok, app `b` com 404 ⇒ `Err(OpsError::Download { app == "b", .. })`, `state.db` sem apps, `apps\a` não existe, `w.broadcasts() == 0`. (Antes desta tarefa o mesmo cenário instalaria `a` e daria `PartlyApplied`.)
4. Os testes da Task 41 continuam verdes (o teste 3 dela falha na extração, não no download — de propósito).

#### Checks beyond the steps

1. Paralelismo: mutate trocando a chamada única de `fetch_each` por um laço sequencial por plano ⇒ PEGA (teste 2).
2. Falha antes de mudar: mutate ignorando os erros de `fetch_each` em `apply` ⇒ PEGA (teste 3).
3. Ordem: mutate devolvendo os resultados de `fetch_each` em ordem de conclusão ⇒ PEGA (teste 1).
4. Bench do orquestrador: `upgrade --all` não é medível sem apps desatualizados; o orquestrador mede com raiz temporária e mock lento se precisar — implementador só reporta os tempos do teste 2.

**Commit:** `perf(ops): download every source of a batch in parallel before applying`

---

### Task 43: Log em arquivo filtrado e com retenção

#### Where this fits

Independente das Tasks 41–42 (só `src/cli/mod.rs` e, se preciso, um módulo novo pequeno). Reduz disco e CPU do `update --background`.

#### What binds this task

- Filtro do arquivo de log: `tracing_subscriber::filter::Targets` (sem crate nova; `Targets` está no `tracing-subscriber` já presente): alvo `wingetrs` em `DEBUG`, todo o resto em `WARN`. Função pura `pub fn file_log_filter() -> Targets` testável.
- Camada de stderr (`-v`/`-vv`): mesmo princípio — `wingetrs` em INFO/DEBUG conforme `verbose`, resto em `WARN` (hoje `-vv` despeja DEBUG do `h2`/`hyper`).
- Retenção: na inicialização do log, `pub fn prune_logs(dir: &Path, keep: usize)` apaga os arquivos mais antigos com nome `wingetrs.log.AAAA-MM-DD` além dos `keep` mais novos (ordem pelo nome, que é a data); ignora qualquer outro arquivo; erro de E/S só vira `tracing::warn!`, nunca falha o comando. `keep = 14`.
- Nada mais muda no formato das linhas.

**Files:**
- Modify: `src/cli/mod.rs` (funções novas ao lado de `init_logging`)
- Test: no próprio módulo (`#[cfg(test)] mod tests` em `src/cli/mod.rs`)

Testes:
1. `file_filter_keeps_wingetrs_debug_and_drops_dependency_noise`: `file_log_filter().would_enable(target, &level)` ⇒ `("wingetrs::install::ops", DEBUG)` true; `("wingetrs", TRACE)` false; `("h2::proto::streams", TRACE)` false; `("hyper_util::client", DEBUG)` false; `("rustls", WARN)` true.
2. `prune_logs_keeps_the_newest_and_ignores_other_files`: tempdir com 16 arquivos `wingetrs.log.2026-09-01` … `wingetrs.log.2026-09-16` e um `outro.txt` ⇒ após `prune_logs(dir, 14)` restam `…09-03` … `…09-16` e `outro.txt`.
3. `prune_logs_on_a_missing_dir_is_a_no_op`.

#### Checks beyond the steps

1. Filtro: mutate trocando o default `WARN` por `TRACE` ⇒ PEGA (teste 1).
2. Retenção: mutate trocando `keep` por `keep + 1` dentro de `prune_logs` ⇒ PEGA (teste 2).
3. O orquestrador roda `wingetrs update` real e confere que o log do dia não ganha linhas `h2`/`hyper` de nível < WARN.

**Commit:** `fix(log): filter the file log to wingetrs and keep 14 days`

## Robustez e medição (Tasks 44–47)

Origem: revisão pós-Task 43. `spawn_boot`/`spawn_reload` da TUI descartam o handle de `spawn_blocking` (pânico engolido) e `spawn_boot` troca erro de catálogo por lista vazia; não há spans por etapa em install/update (relatório da fase 2a §3: "não medido (sem spans)"); `upgrade` carrega o catálogo inteiro por app; `update --background` é redisparado a cada comando depois de uma falha.

### Task 44: Tarefas de fundo da TUI aguardadas e erro de catálogo visível

#### Where this fits

Independente das outras desta leva (só TUI e scripts do gate). Hoje, em `src/tui/run.rs`:

- `spawn_boot` (~:176–192) chama `tokio::task::spawn_blocking(..)` e descarta o handle: se o trabalho entra em pânico, o tokio guarda o pânico num handle que ninguém lê. Pior: `Catalog::open(..).and_then(load_entries).unwrap_or_default()` (:179–181) transforma um catálogo ilegível (corrompido, travado) numa lista **vazia sem aviso**.
- `spawn_reload` (~:233–243) também descarta o handle (o erro só vai para o log).
- `spawn_scan` (~:245–281) aguarda o handle, mas **engole** o pânico: `Err(e)` com `e.is_panic()` vira `Msg::InstalledFailed("pânico interno")` (:270–276).

A TUI já tem o mecanismo certo para trabalho bloqueante: `spawn_job` (`src/tui/run.rs` ~385–398) aguarda o handle via `jobs::join_job` (`src/tui/jobs.rs` ~45), que transforma pânico em `Msg::Panicked`; o laço chama `jobs::propagate` (:132), que faz `resume_unwind`; `run` (:65–73) restaura o terminal e re-lança. Esta tarefa põe boot, reload e scan nesse caminho e cria a regra mecânica que impede a volta do handle descartado.

**Por que re-lançar e não mostrar "pânico interno":** a regra do CLAUDE.md manda propagar com `resume_unwind`; pânico é bug, e mostrar status com resultado parcial é exatamente "pânico engolido". O laço já restaura o terminal antes de re-lançar, então o usuário vê a mensagem do pânico num terminal sadio.

#### What binds this task

1. **Boot pelo canal de mensagens.** Remova o `oneshot`/`struct Boot`/variável `booting` do `event_loop`. Nova variante em `src/tui/app.rs`:
   ```rust
   /// Startup load finished: the catalog (or why it could not be read), its age and
   /// whether the startup refresh is due.
   Booted {
       catalog: Result<Vec<PackageEntry>, String>,
       age: Option<i64>,
       needs_refresh: bool,
   },
   ```
   `spawn_boot(ctx)` vira `spawn_job(ctx, |c, _| boot_msg(c))`, com `boot_msg` em `run.rs` montando `Msg::Booted { catalog: jobs::read_catalog(&c.data.catalog_path()), age: index_age(&c.data), needs_refresh: startup_needs_refresh(c) }`.
2. **`update` trata `Booted`** (função pura, testável): `app.status = None`; `app.index_age_secs = age`; `Ok(entries)` ⇒ mesmo efeito de `CatalogLoaded` (`SearchIndex::new`, `research`, `clamp`); `Err(e)` ⇒ **mantém** o índice atual e põe `app.status = Some(format!("falha ao ler o catálogo: {e}"))`. Devolve `vec![Cmd::ScanInstalled]` e acrescenta `Cmd::Refresh { force: false }` quando `catalog` é `Err`, ou `Ok` vazio, ou `needs_refresh` — nessa ordem (é a ordem atual: `spawn_scan` depois `spawn_refresh`, run.rs :124–127).
3. **Nova variante `Msg::CatalogFailed(String)`**: `update` mantém o índice atual e põe `app.status = Some(format!("falha ao ler o catálogo: {e}"))`. `spawn_reload` vira `spawn_job(ctx, |c, _| match jobs::read_catalog(&c.data.catalog_path()) { Ok(e) => Msg::CatalogLoaded(e), Err(e) => Msg::CatalogFailed(e) })`.
4. **Nova `pub fn read_catalog(path: &Path) -> Result<Vec<PackageEntry>, String>` em `src/tui/jobs.rs`**: `Catalog::open(path).and_then(|c| c.load_entries()).map_err(|e| e.to_string())`. Nunca devolve vazio no lugar de erro.
5. **`spawn_scan` via `spawn_job`**: o corpo atual (thread de tracking juntada em todos os caminhos, `catch_unwind` + `resume_unwind`) fica igual dentro da closure, que devolve `Msg::InstalledLoaded(rows)` ou, em erro de correlação, `Msg::InstalledFailed(e.to_string())`. O ramo `is_panic() ⇒ "pânico interno"` desaparece (o `join_job` leva o pânico ao laço).
6. **Regra mecânica em `scripts/check_rules.ps1`**: uma linha de código de produção (fora de `#[cfg(test)]` e de comentário, como as regras atuais) que contém `spawn_blocking(` só passa se o handle estiver **ligado**: `let <nome> =` na mesma linha (nome que **não** começa com `_`; `let _ =` e `let _h =` também descartam/soltam o handle), ou a linha de código anterior terminar em `let <nome> =` (forma do rustfmt em `src/update.rs` :159–160), ou `.await` na mesma linha, ou a linha seguinte começar com `.await`. Senão: violação `[spawn_blocking sem handle]`. **Atenção ao índice −1:** em PowerShell `$lines[-1]` é a **última** linha; a checagem da linha anterior exige `$i -gt 0`.
7. **O instrumento é provado no menor caso** (ARMADILHAS "Instrumento que para de vigiar em silêncio"): `check_rules.ps1` ganha `param([string]$Path = 'src')` (o gate continua chamando sem argumento) e um novo `scripts/check_rules_selftest.ps1` planta cada caso num diretório temporário próprio, roda `check_rules.ps1 -Path <dir>` e compara o exit. O `verify.ps1` roda o selftest logo depois de `rules`.

Fora de escopo (registrar no relatório, não implementar): `spawn_refresh` (:205–231) e `spawn_details` (:284–298) descartam o handle de `tokio::spawn`; um pânico em `run_update` deixaria `app.refreshing` preso. A regra mecânica desta tarefa é só para `spawn_blocking`.

**Files:**
- Modify: `src/tui/run.rs`, `src/tui/app.rs`, `src/tui/jobs.rs`, `scripts/check_rules.ps1`, `scripts/verify.ps1`
- Create (new): `scripts/check_rules_selftest.ps1`
- Test: `tests/tui_state.rs`, `tests/tui_jobs.rs`

Código do bloco novo em `scripts/check_rules.ps1` (dentro do `for`, depois do `foreach ($r in $rules)`, portanto já depois dos `continue` de teste e de comentário):

```powershell
        # Task 44: a dropped spawn_blocking handle lets tokio swallow the worker panic.
        # Bound = `let <name> =` (name not starting with `_`) on this line or ending the previous
        # line, or `.await` on this line or starting the next one.
        if ($line -match '\bspawn_blocking\s*\(') {
            $bound = ($line -match '\blet\s+(mut\s+)?[A-Za-z]\w*\s*(:[^=]+)?=.*spawn_blocking') -or
                     ($line -match 'spawn_blocking.*\.await')
            # $i -gt 0: $lines[-1] is the LAST line in PowerShell, not "no line".
            if (-not $bound -and $i -gt 0) {
                $bound = $lines[$i - 1] -match '\blet\s+(mut\s+)?[A-Za-z]\w*\s*(:[^=]+)?=\s*$'
            }
            if (-not $bound -and $i + 1 -lt $lines.Count) {
                $bound = $lines[$i + 1] -match '^\s*\.await'
            }
            if (-not $bound) {
                $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'spawn_blocking sem handle', $line.Trim()
            }
        }
```

E no topo, logo depois do bloco `<# ... #>`: `[CmdletBinding()] param([string]$Path = 'src')`; troque `Get-ChildItem -Recurse -File -Path src` por `-Path $Path`.

`scripts/check_rules_selftest.ps1` (literal):

```powershell
#Requires -Version 7.0
<#
.SYNOPSIS
    Proves check_rules.ps1 still sees violations: plants each case in its own temp dir, runs the
    checker on it and compares the exit code. Exit 0 = every case behaved, 1 = the instrument went blind.
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$checker = Join-Path $PSScriptRoot 'check_rules.ps1'
$bound = @(
    'async fn a() -> u8 {',
    '    let h = tokio::task::spawn_blocking(|| 1);',
    '    let lock =',
    '        tokio::task::spawn_blocking(|| 2)',
    '            .await;',
    '    let r = spawn_blocking(|| 3).await;',
    '    let job: JoinHandle<u8> = tokio::task::spawn_blocking(|| 4);',
    '    // tokio::task::spawn_blocking(|| 5);',
    '    tokio::task::spawn_blocking(|| 6)',
    '        .await',
    '}'
) -join "`n"
$cases = @(
    @{ Name = 'arquivo vazio'; Text = ''; Want = 0 },
    @{ Name = 'uma linha, caminho completo'; Text = 'fn f() { tokio::task::spawn_blocking(|| 1); }'; Want = 1 },
    @{ Name = 'uma linha, nome curto'; Text = 'fn f() { spawn_blocking(|| 1); }'; Want = 1 },
    @{ Name = 'let _ ='; Text = 'let _ = tokio::task::spawn_blocking(|| 1);'; Want = 1 },
    @{ Name = 'let _h ='; Text = 'let _h = tokio::task::spawn_blocking(|| 1);'; Want = 1 },
    @{ Name = '1a linha nao olha a ultima'; Text = "tokio::task::spawn_blocking(|| 1);`nlet x ="; Want = 1 },
    @{ Name = 'forma do spawn_boot'; Text = "fn f(c: u8) {`n    let c = c;`n    tokio::task::spawn_blocking(move || {`n        let _ = c;`n    });`n}"; Want = 1 },
    @{ Name = 'formas ligadas'; Text = $bound; Want = 0 },
    @{ Name = 'regra antiga viva'; Text = 'fn f() { println!("x"); }'; Want = 1 }
)
$base = Join-Path ([IO.Path]::GetTempPath()) ("check_rules_selftest-" + [guid]::NewGuid())
$bad = @()
try {
    $n = 0
    foreach ($c in $cases) {
        $n++
        $dir = Join-Path $base "case$n"
        New-Item -ItemType Directory -Force $dir | Out-Null
        [IO.File]::WriteAllText((Join-Path $dir 'x.rs'), $c.Text)
        pwsh -NoProfile -File $checker -Path $dir *> $null
        if ($LASTEXITCODE -ne $c.Want) { $bad += "  [$($c.Name)] exit $LASTEXITCODE, esperado $($c.Want)" }
    }
} finally {
    Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
if ($bad.Count -gt 0) { $bad; exit 1 }
"[OK] check_rules: $($cases.Count) casos plantados"
exit 0
```

Em `scripts/verify.ps1`, logo depois do passo `rules`:
`@{ Name = 'rules-selftest'; Cmd = { pwsh -NoProfile -File (Join-Path $PSScriptRoot 'check_rules_selftest.ps1') 2>&1 } },`

Ordem TDD: (a) acrescente a regra e o selftest **antes** de mexer em `run.rs`; `pwsh -File scripts/check_rules.ps1` deve sair 1 apontando `src/tui/run.rs` nas linhas de `spawn_boot` e `spawn_reload` — cole essa saída no relatório; (b) depois corrija a TUI e veja sair 0.

Testes (corpos literais):

1. `boot_with_an_unreadable_catalog_keeps_entries_shows_the_error_and_refreshes` (`tests/tui_state.rs`):
   ```rust
   #[test]
   fn boot_with_an_unreadable_catalog_keeps_entries_shows_the_error_and_refreshes() {
       let mut a = app();
       let before = a.search.entries().len();
       assert!(before > 0);
       let cmds = update(
           &mut a,
           Msg::Booted {
               catalog: Err("file is not a database".into()),
               age: None,
               needs_refresh: false,
           },
       );
       let status = a.status.clone().unwrap_or_default();
       assert!(status.contains("falha ao ler o catálogo"), "{status}");
       assert!(status.contains("file is not a database"), "{status}");
       assert_eq!(a.search.entries().len(), before);
       assert_eq!(cmds, vec![Cmd::ScanInstalled, Cmd::Refresh { force: false }]);
   }
   ```
2. `boot_with_a_populated_fresh_catalog_loads_it_and_does_not_refresh` (`tests/tui_state.rs`):
   ```rust
   #[test]
   fn boot_with_a_populated_fresh_catalog_loads_it_and_does_not_refresh() {
       let mut a = App::new(Vec::new(), GlyphSet::Nerd);
       a.status = Some("carregando catálogo…".into());
       let cmds = update(
           &mut a,
           Msg::Booted {
               catalog: Ok(vec![
                   entry(SourceKind::Scoop, "ripgrep", "ripgrep"),
                   entry(SourceKind::Winget, "Zeal.Zeal", "Zeal"),
               ]),
               age: Some(42),
               needs_refresh: false,
           },
       );
       assert_eq!(a.search.entries().len(), 2);
       assert_eq!(a.status, None);
       assert_eq!(a.index_age_secs, Some(42));
       assert_eq!(cmds, vec![Cmd::ScanInstalled]);
   }
   ```
3. `boot_refreshes_when_the_catalog_is_empty_or_stale` (`tests/tui_state.rs`):
   ```rust
   #[test]
   fn boot_refreshes_when_the_catalog_is_empty_or_stale() {
       let mut a = App::new(Vec::new(), GlyphSet::Nerd);
       let cmds = update(
           &mut a,
           Msg::Booted { catalog: Ok(vec![]), age: None, needs_refresh: false },
       );
       assert_eq!(cmds, vec![Cmd::ScanInstalled, Cmd::Refresh { force: false }]);
       let mut b = App::new(Vec::new(), GlyphSet::Nerd);
       let cmds = update(
           &mut b,
           Msg::Booted {
               catalog: Ok(vec![entry(SourceKind::Scoop, "ripgrep", "ripgrep")]),
               age: Some(5000),
               needs_refresh: true,
           },
       );
       assert_eq!(cmds, vec![Cmd::ScanInstalled, Cmd::Refresh { force: false }]);
   }
   ```
4. `a_failed_reload_keeps_the_entries_and_shows_the_error` (`tests/tui_state.rs`):
   ```rust
   #[test]
   fn a_failed_reload_keeps_the_entries_and_shows_the_error() {
       let mut a = app();
       let before = a.search.entries().len();
       assert!(before > 0);
       update(&mut a, Msg::CatalogFailed("database is locked".into()));
       assert_eq!(a.search.entries().len(), before);
       let status = a.status.clone().unwrap_or_default();
       assert!(status.contains("falha ao ler o catálogo"), "{status}");
       assert!(status.contains("database is locked"), "{status}");
   }
   ```
5. `read_catalog_reports_an_unreadable_file_instead_of_an_empty_list` e `read_catalog_of_a_good_catalog_lists_its_entries` (`tests/tui_jobs.rs`):
   ```rust
   #[test]
   fn read_catalog_reports_an_unreadable_file_instead_of_an_empty_list() {
       let dir = tempfile::tempdir().unwrap();
       let path = dir.path().join("catalog.db");
       std::fs::write(&path, vec![b'x'; 4096]).unwrap();
       let err = jobs::read_catalog(&path).unwrap_err();
       assert!(!err.is_empty());
   }

   #[test]
   fn read_catalog_of_a_good_catalog_lists_its_entries() {
       let dir = tempfile::tempdir().unwrap();
       let path = dir.path().join("catalog.db");
       let mut c = Catalog::open(&path).unwrap();
       c.replace_source(
           "scoop:main",
           SourceKind::Scoop,
           "v",
           1,
           &[NewPackage {
               id: "ripgrep".into(),
               name: "ripgrep".into(),
               version: "1".into(),
               ..Default::default()
           }],
           &WingetMaps::default(),
       )
       .unwrap();
       drop(c);
       assert_eq!(jobs::read_catalog(&path).unwrap().len(), 1);
   }
   ```
   (Acrescente os `use` que faltarem: `wingetrs::catalog::{Catalog, NewPackage, WingetMaps}`, `wingetrs::core::SourceKind`.)
6. Os testes existentes de pânico continuam valendo para o novo caminho: `a_worker_panic_reaches_the_main_loop_and_is_reraised` (`tests/tui_jobs.rs`) e `update_reraises_a_worker_panic` (`tests/tui_state.rs`).
7. `pwsh -File scripts/check_rules_selftest.ps1` ⇒ `[OK] check_rules: 9 casos plantados`, exit 0 (colar no relatório).

#### Checks beyond the steps

Rode cada uma com `pwsh -File scripts/mutate.ps1 -Path <arquivo> -Anchor '<texto exato>' -Replacement '<texto>' -TestCommand '<comando>'`; todas devem ser PEGAS.

1. Erro de catálogo vira vazio: no ramo `Err` do `Booted` em `src/tui/app.rs`, troque o tratamento por o de `Ok(Vec::new())` (índice vazio, status `None`) ⇒ PEGA (teste 1). `-TestCommand 'cargo test --quiet --test tui_state boot_'`.
2. Refresh no boot: troque a condição do `Cmd::Refresh` por só `needs_refresh` ⇒ PEGA (testes 1 e 3).
3. Reload que falha apaga a lista: faça `CatalogFailed` também zerar `app.search` ⇒ PEGA (teste 4).
4. `read_catalog` engolindo: troque o `map_err(..)` por `.or_else(|_| Ok(Vec::new()))` ⇒ PEGA (teste 5).
5. Instrumento, índice −1: em `scripts/check_rules.ps1` troque `-not $bound -and $i -gt 0` por `-not $bound` ⇒ PEGA (`-TestCommand 'pwsh -NoProfile -File scripts/check_rules_selftest.ps1'`, caso "1a linha nao olha a ultima").
6. Instrumento, `let _h`: troque `[A-Za-z]\w*\s*(:[^=]+)?=.*spawn_blocking` por `\w+\s*(:[^=]+)?=.*spawn_blocking` ⇒ PEGA (casos `let _ =`/`let _h =`).
7. Instrumento, falso positivo: apague o bloco que olha `.await` na linha seguinte ⇒ PEGA (caso "formas ligadas" passa a sair 1).
8. A regra vigia o código real: em `src/tui/run.rs`, reintroduza um `tokio::task::spawn_blocking(move || { .. });` sem `let` (por exemplo, volte `spawn_reload` ao corpo antigo) ⇒ `-TestCommand 'pwsh -NoProfile -File scripts/check_rules.ps1'` sai 1 ⇒ PEGA.
9. Pânico do scan: sem prova de mutação automática (cola em `run.rs`, sem harness). Verificação por grep: `grep -rn "pânico interno" src` não acha nada, e `spawn_scan` usa `spawn_job`.

**Commit:** `fix(tui): await background catalog jobs and show catalog errors`

**Foco extra do revisor:** (a) a TUI agora **termina** com pânico se a coleta de instalados entrar em pânico (antes mostrava status) — é a regra do CLAUDE.md, mas confira que `run` restaura o terminal nesse caminho (`run.rs` :65–73); (b) o primeiro desenho do catálogo passa a esperar `index_age` + `startup_needs_refresh` (abre o catálogo de novo, ~11–15 ms segundo ARMADILHAS) — aceitável, não medido; (c) rode o selftest no checkout de base para ver que o caso "forma do spawn_boot" é o mesmo formato de `run.rs` :177–191.

---

### Task 45: Spans por etapa com duração no log em arquivo

#### Where this fits

Independente da Task 44 (arquivos disjuntos). Precede as Tasks 46 e 47 (mesmos arquivos e é o instrumento que a 46 usa para medir). **Tarefa de medição: nenhuma otimização.** Hoje só existem spans em `src/install/registry_win.rs` (~92) e `installed::timed` (`src/installed/mod.rs` :107, que já cobre o `list`); o relatório da fase 2a §3 registra "Divisão interna … **não medido** (sem spans)".

O log em arquivo é montado em `init_logging` (`src/cli/mod.rs` ~376–397), filtrado por `file_log_filter()` (~322: `wingetrs` em DEBUG, resto em WARN). A camada `fmt` só registra span se pedirmos eventos de span: `tracing_subscriber::fmt::format::FmtSpan::CLOSE` faz cada span, ao fechar, gravar uma linha `close time.busy=… time.idle=…` (sem crate nova; `tracing-subscriber` 0.3 já está no `Cargo.toml`).

#### What binds this task

1. **Nível e alvo:** todo span de etapa é `tracing::debug_span!` criado em código do crate (alvo `wingetrs::…`), portanto aceito por `file_log_filter()`. Nada em TRACE.
2. **Camada do arquivo:** extraia `fn file_layer<S, W>(writer: W) -> impl tracing_subscriber::Layer<S>` (com `S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>`, `W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static`) = `fmt::layer().with_writer(writer).with_ansi(false).with_span_events(FmtSpan::CLOSE).with_filter(file_log_filter())`; `init_logging` usa `file_layer(writer)`.
3. **Stderr:** `fn stderr_span_events(verbose: u8) -> FmtSpan` = `FmtSpan::CLOSE` se `verbose > 1`, senão `FmtSpan::NONE`; a camada de stderr usa `.with_span_events(stderr_span_events(verbose))`. Resultado: `-vv` mostra as durações no terminal, `-v` não (os spans são DEBUG e o `-v` é INFO, mas os spans INFO de `installed::timed` poluiriam o `-v`).
4. **Spans de etapa (nomes exatos, um por etapa, nunca por item de catálogo):**
   - `src/install/ops.rs` `apply_with_targets`: `install.lock` (no `lock_async(..)`, via `.instrument(span)`), `install.revalidate` (`span.in_scope(..)`), `install.prefetch` (`.instrument`, campo `plans = plans.len()`), `install.execute` **um por plano** (`.instrument` no `executor::execute_install(..)`, campo `app = %plan.app`), `install.broadcast` (`in_scope` no `broadcast_if_changed`).
   - `src/install/ops.rs` `uninstall`: `uninstall.lock`, `uninstall.execute`, `uninstall.broadcast`, `uninstall.residue_scan` (no `scan_residues`).
   - `src/install/ops.rs`: `upgrade.list` (corpo de `upgradable`), `target.resolve` (corpo de `resolve_target`), `plan.build` (corpo de `plan_for`).
   - `src/install/executor.rs`: em `execute_install`, cada passo de `main` roda dentro de `debug_span!("install.step", kind = step_kind(step), index = i)` (`.instrument` no `run_step(..)`); os passos de limpeza (`RemoveOldVersion`) usam o mesmo span com `in_scope`. Nova `fn step_kind(step: &Step) -> &'static str`: `EnsureLauncher`→`"launcher"`, `Download`→`"download"`, `Extract`→`"extract"`, `Promote`→`"promote"`, `Persist`→`"persist"`, `LinkCurrent`→`"link"`, `Shim`→`"shim"`, `Shortcut`→`"shortcut"`, `EnvSet`→`"env"`, `EnvAddPath`→`"path"`, `EnsureShimsOnPath`→`"shims_path"`, `RecordState`→`"state"`, `RemoveOldVersion`→`"remove_old"`.
   - `src/update.rs` `run_update`: `update.lock`, `update.fetch` **um por fonte** (cada future do `join_all` com `.instrument(debug_span!("update.fetch", source = %job.source))`, inclusive as puladas), `update.catalog_write` (corpo de `apply`, campo `source`), `update.state_save` (`state.save`), `update.search_idx` (`write_search_index`).
   - `update.parse`: em `src/sources/winget/mod.rs` `fetch_index` em volta de `validate_msix` + gravação + `read_index`; em `src/sources/scoop/bucket.rs` `fetch_bucket` em volta de `read_tarball`. Só existe quando houve corpo para ler (304/erro não abre span).
   - Spans assíncronos usam `tracing::Instrument` (nunca `span.enter()` atravessando `.await`).
5. **Busca:** o corpo da busca roda antes do log existir (`run`, ~824–844), então span ali se perde. `EarlyOutcome` ganha `via: &'static str` (`"idx"` quando `index_file::search` respondeu, `"catalog"` quando caiu para `search_on_disk`) e `elapsed_ms: f64`; `log_warning` passa a registrar também `tracing::debug!(stage = "search", via = out.via, ms = out.elapsed_ms, "stage")`.
6. `list`: nada novo (já medido por `installed::timed`). Não remova nem mude `timed`.

**Files:**
- Modify: `src/cli/mod.rs`, `src/install/ops.rs`, `src/install/executor.rs`, `src/update.rs`, `src/sources/winget/mod.rs`, `src/sources/scoop/bucket.rs`
- Create (new): `tests/spans/mod.rs` (camada de captura compartilhada; `mod spans;` nos dois arquivos de teste)
- Test: `tests/install_ops.rs`, `tests/update.rs`, módulos `#[cfg(test)]` de `src/cli/mod.rs`

`tests/spans/mod.rs` (literal):

```rust
#![allow(dead_code)]
//! Test layer that records every span that closed: (name, `kind` field, target, level).
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

#[derive(Clone, Default)]
pub struct Closed(pub Arc<Mutex<Vec<(String, String, String, tracing::Level)>>>);

struct Kind(String);

struct KindVisitor(Option<String>);

impl tracing::field::Visit for KindVisitor {
    fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
        if f.name() == "kind" {
            self.0 = Some(v.to_string());
        }
    }
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        if f.name() == "kind" {
            self.0 = Some(format!("{v:?}").trim_matches('"').to_string());
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for Closed
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let mut v = KindVisitor(None);
        attrs.record(&mut v);
        if let (Some(span), Some(k)) = (ctx.span(id), v.0) {
            span.extensions_mut().insert(Kind(k));
        }
    }

    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            let kind = span
                .extensions()
                .get::<Kind>()
                .map(|k| k.0.clone())
                .unwrap_or_default();
            let m = span.metadata();
            self.0.lock().unwrap().push((
                m.name().to_string(),
                kind,
                m.target().to_string(),
                *m.level(),
            ));
        }
    }
}

impl Closed {
    /// `name` or `name:kind`, in closing order.
    pub fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(n, k, _, _)| if k.is_empty() { n.clone() } else { format!("{n}:{k}") })
            .collect()
    }

    pub fn count(&self, name: &str) -> usize {
        self.names().iter().filter(|n| n.as_str() == name).count()
    }

    /// Installs this layer as the thread's default subscriber (tests are `current_thread`).
    pub fn install(&self) -> tracing::subscriber::DefaultGuard {
        use tracing_subscriber::layer::SubscriberExt;
        tracing::subscriber::set_default(tracing_subscriber::registry().with(self.clone()))
    }
}
```

(Os testes `#[tokio::test]` destes arquivos são `current_thread`: as futures rodam na thread do teste, então o `set_default` vê os spans. Nenhum span de etapa pode ser criado dentro de `spawn_blocking`.)

Testes:

1. `install_emits_one_span_per_stage_and_per_app` (`tests/install_ops.rs`) — dois planos (dependência + alvo), só a dependência com `env`, para que um span por plano não coincida com um span por lote:
   ```rust
   #[tokio::test]
   async fn install_emits_one_span_per_stage_and_per_app() {
       let mut w = World::new().await;
       w.add(App { env: true, ..app("leaf", &[]) }).await;
       w.add(app("top", &["leaf"])).await;
       let plans = w.plans("top").unwrap();
       assert_eq!(plans.len(), 2);
       let closed = spans::Closed::default();
       let _g = closed.install();
       apply(&w.ctx(), &plans).await.unwrap();
       for once in ["install.lock", "install.revalidate", "install.prefetch", "install.broadcast"] {
           assert_eq!(closed.count(once), 1, "{once}: {:?}", closed.names());
       }
       assert_eq!(closed.count("install.execute"), 2, "{:?}", closed.names());
       for per_app in ["install.step:download", "install.step:extract", "install.step:shim", "install.step:state"] {
           assert_eq!(closed.count(per_app), 2, "{per_app}: {:?}", closed.names());
       }
       assert_eq!(closed.count("install.step:env"), 1, "só leaf tem env_set");
       let filter = wingetrs::cli::file_log_filter();
       for (name, _, target, level) in closed.0.lock().unwrap().iter() {
           if name.starts_with("install.") {
               assert!(filter.would_enable(target, level), "{name} ({target}, {level}) não chega ao log");
           }
       }
   }
   ```
2. `uninstall_emits_its_stage_spans` (`tests/install_ops.rs`): `w.add(app("leaf", &[]))`, `w.install("leaf")`, `plan_uninstall`; depois `let _g = closed.install();` e `uninstall(&w.ctx(), &p).unwrap()` ⇒ `uninstall.lock`, `uninstall.execute`, `uninstall.broadcast`, `uninstall.residue_scan` com `count == 1` cada.
3. `planning_spans_wrap_resolve_and_plan` (`tests/install_ops.rs`): com `_g` ativo, `w.plans("top")` (que chama `resolve_target` e `plan_for`) e `upgradable(&w.ctx())` ⇒ `target.resolve`, `plan.build` e `upgrade.list` com `count == 1`.
4. `update_emits_a_fetch_span_per_source_and_parse_only_for_downloaded_ones` (`tests/update.rs`, com `mod spans;` no topo), usando o `setup()` existente (winget 200, main 200, extras 500 — três fontes, só duas com corpo):
   ```rust
   #[tokio::test]
   async fn update_emits_a_fetch_span_per_source_and_parse_only_for_downloaded_ones() {
       let (_s, ep) = setup().await;
       let dir = tempfile::tempdir().unwrap();
       let data = DataDir::new(dir.path());
       data.ensure().unwrap();
       let closed = spans::Closed::default();
       let _g = closed.install();
       run_update(&data, &ep, &Http::new().unwrap(), true, LockWait::from_env())
           .await
           .unwrap();
       assert_eq!(closed.count("update.lock"), 1, "{:?}", closed.names());
       assert_eq!(closed.count("update.fetch"), 3, "{:?}", closed.names());
       assert_eq!(closed.count("update.parse"), 2, "extras falha antes de ler");
       assert_eq!(closed.count("update.catalog_write"), 2);
       assert_eq!(closed.count("update.state_save"), 1);
       assert_eq!(closed.count("update.search_idx"), 1);
   }
   ```
5. `file_layer_logs_the_duration_of_a_closed_wingetrs_span` (módulo `log_tests` de `src/cli/mod.rs`):
   ```rust
   #[derive(Clone, Default)]
   struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
   impl std::io::Write for Buf {
       fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
           self.0.lock().unwrap().extend_from_slice(b);
           Ok(b.len())
       }
       fn flush(&mut self) -> std::io::Result<()> {
           Ok(())
       }
   }

   #[test]
   fn file_layer_logs_the_duration_of_a_closed_wingetrs_span() {
       use tracing_subscriber::layer::SubscriberExt;
       let buf = Buf::default();
       let w = buf.clone();
       let sub = tracing_subscriber::registry().with(file_layer(move || w.clone()));
       tracing::subscriber::with_default(sub, || {
           let _a = tracing::debug_span!(target: "wingetrs::install::ops", "install.prefetch").entered();
           let _b = tracing::debug_span!(target: "h2::proto", "noise").entered();
       });
       let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
       let line = out
           .lines()
           .find(|l| l.contains("install.prefetch") && l.contains("close"))
           .unwrap_or_else(|| panic!("sem linha de fechamento: {out}"));
       assert!(line.contains("time.busy"), "{line}");
       assert!(!out.contains("noise"), "{out}");
   }

   #[test]
   fn stderr_shows_span_durations_only_with_vv() {
       use tracing_subscriber::fmt::format::FmtSpan;
       assert_eq!(stderr_span_events(1), FmtSpan::NONE);
       assert_eq!(stderr_span_events(2), FmtSpan::CLOSE);
   }
   ```
6. `early_search_reports_which_path_answered` (módulo `early_tests` de `src/cli/mod.rs`) — os dois caminhos, para que um `via` fixo falhe:
   ```rust
   #[test]
   fn early_search_reports_which_path_answered() {
       use crate::catalog::{NewPackage, WingetMaps};
       let dir = tempfile::tempdir().unwrap();
       let (idx, cat) = (dir.path().join("search.idx"), dir.path().join("catalog.db"));
       let mut c = Catalog::open(&cat).unwrap();
       c.replace_source(
           "scoop:main",
           SourceKind::Scoop,
           "v",
           1,
           &[NewPackage {
               id: "ripgrep".into(),
               name: "ripgrep".into(),
               version: "1".into(),
               ..Default::default()
           }],
           &WingetMaps::default(),
       )
       .unwrap();
       let out = search_early(&idx, &cat, "ripgrep", None, 10);
       assert_eq!(out.via, "catalog");
       index_file::write(&c, &idx).unwrap();
       let out = search_early(&idx, &cat, "ripgrep", None, 10);
       assert_eq!(out.via, "idx");
       assert_eq!(out.result.unwrap().len(), 1);
   }
   ```
7. Cole no relatório uma linha real de fechamento do log de um teste manual (`wingetrs -vv search ripgrep` numa raiz temporária ou o resultado do teste 5 com `--nocapture`), mostrando o formato `… install.prefetch…: close time.busy=… time.idle=…`.

Como o orquestrador lê as durações (depois do merge; raiz isolada como no relatório da fase 2a §2):

```
$S = '<scratchpad>'
$env:LOCALAPPDATA = "$S\la"; $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\t45'; $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
pwsh -File scripts/leftovers.ps1
target\release\wingetrs.exe install ripgrep --yes
$log = Get-ChildItem "$S\la\wingetrs\log\wingetrs.log.*" | Sort-Object Name | Select-Object -Last 1
Select-String -Path $log.FullName -Pattern '(install|uninstall|update|target|plan|upgrade)\.[a-z_]+.*close time\.busy' | Select-Object -Last 40
Select-String -Path $log.FullName -Pattern 'stage="search"|stage=search' | Select-Object -Last 3
```

(Alternativa sem arquivo: `target\release\wingetrs.exe -vv install ripgrep --yes 2>&1 | Select-String 'close time.busy'`; o stderr pode ter códigos ANSI. O nome do arquivo diário usa a data UTC do `tracing-appender`, daí o `Select-Object -Last 1`.) Para spans assíncronos (`install.lock`, `install.prefetch`, `install.execute`, `install.step`, `update.fetch`), a duração de parede é `time.busy + time.idle`; para os síncronos, `time.busy`.

#### Checks beyond the steps

1. Fechamento no arquivo: em `file_layer`, remova `.with_span_events(FmtSpan::CLOSE)` ⇒ PEGA (teste 5). `pwsh -File scripts/mutate.ps1 -Path src/cli/mod.rs -Anchor '<linha do with_span_events em file_layer>' -Replacement '' -TestCommand 'cargo test --quiet --lib file_layer_logs'`.
2. Um span por plano: mova o `install.execute` para envolver o laço inteiro (um span por lote) ⇒ PEGA (teste 1, `count == 2`).
3. Nível: troque `debug_span!("install.step"` por `trace_span!("install.step"` ⇒ PEGA (teste 1, asserção do filtro).
4. Parse só com corpo: tire o span `update.parse` de `fetch_bucket` ⇒ PEGA (teste 4, `count == 2`); ponha o `update.parse` envolvendo o `fetch_one` inteiro ⇒ PEGA (`count` vira 3).
5. Resíduos: tire o span `uninstall.residue_scan` ⇒ PEGA (teste 2).
6. `-vv`: faça `stderr_span_events` devolver `CLOSE` para `verbose > 0` ⇒ PEGA (teste 5b).
7. Caminho da busca: fixe `via: "idx"` ⇒ PEGA (teste 6).
8. Nenhuma otimização: o diff não muda ordem de chamadas nem o que é lido/escrito (revisor confere; não há mutação para isso).

**Commit:** `feat(log): spans per stage with durations in the file log`

**Foco extra do revisor:** (a) nenhum `span.enter()`/`entered()` vivo atravessando `.await` (use `Instrument`); (b) nenhum span por pacote em laço quente (`replace_source`, `load_entries`, parse de manifesto) — o log do `update --background` não pode crescer por pacote (Task 43 acabou de reduzir); (c) o teste 1 depende de `current_thread` — se alguém trocar o flavor, o teste falha (não passa em silêncio), confira; (d) `time.busy` nos spans assíncronos subestima o tempo de parede — está documentado acima, não "conserte" com `Instant` manual.

---

### Task 46: `upgrade` busca a linha do app instalado em vez de carregar o catálogo

#### Where this fits

Depois da Task 45 (mesmos `src/install/ops.rs`/`tests/install_ops.rs`; usa os spans `target.resolve`/`upgrade.list` para medir) e da Task 44 (`src/tui/jobs.rs`). Hoje, para cada app escolhido, `upgrade` (`src/cli/install_cmd.rs` ~407–417) e `plan_upgrade_all` da TUI (`src/tui/jobs.rs` ~102–128) chamam `ops::resolve_target(ctx, app, Some(bucket))` (`src/install/ops.rs` ~329), que chama `target::resolve` (`src/install/target.rs` ~47), que faz `catalog.load_entries()` (:61–62) — **o catálogo inteiro** (dezenas de milhares de linhas, várias conexões) — para achar uma linha cujo `(bucket, id)` já sabemos pelo `state.db`.

Pré-condição que torna a troca segura: os apps só chegam aqui depois de `ops::upgradable` (~832–853), que já exige `ctx.catalog.get(&source_name(&rec.bucket), &rec.app)` = `Some` (busca **exata**, indexada por `UNIQUE (source, id)` — `src/catalog/mod.rs` :33 e :457). Logo a mesma busca exata acha o candidato.

`upgradable` em si fica como está: lê o `raw` só dos apps instalados, uma busca indexada por app (não "de tudo").

#### What binds this task

1. Nova função em `src/install/ops.rs`:
   ```rust
   /// Upgrade target of an installed app: its exact catalog row `(scoop:<bucket>, app)`, one
   /// indexed lookup instead of `resolve_target`'s full catalog load. Never matches by name.
   pub fn resolve_installed(ctx: &OpsCtx<'_>, app: &str, bucket: &str) -> Result<Resolution, OpsError>
   ```
   Corpo: `ctx.catalog.get(&crate::sources::scoop::source_name(bucket), app)?` ⇒ `Some(p)` ⇒ `Resolution::One(target::candidate(&p.entry))`; `None` ⇒ `Resolution::NotFound { suggestions: Vec::new() }`. Torne `candidate` em `src/install/target.rs` `pub(crate)` (mesma construção de `Candidate`: `selectable`/`note` idênticos). Span `debug_span!("target.resolve_installed")` em volta (convenção da Task 45).
2. CLI `upgrade` (`src/cli/install_cmd.rs`): no laço que monta `groups`, `ops::resolve_installed(ctx, app, &rec.bucket)?` no lugar de `ops::resolve_target(ctx, app, Some(&rec.bucket))?`; mensagens e saída iguais.
3. TUI `plan_upgrade_all` (`src/tui/jobs.rs`): idem; o bucket vem do `state.db` (já lido ali). App sem linha no `state.db` ⇒ `Err(format!("{app}: não está no registro do wingetrs"))` em vez de passar `None`.
4. `resolve_target` não muda (continua servindo `install <texto do usuário>` e `plan_install` da TUI, que precisam de nome/moniker/sugestões).
5. **Dois caminhos para o mesmo resultado ⇒ teste de equivalência** (CLAUDE.md): no domínio da pré-condição, `resolve_installed(app, bucket)` devolve o **mesmo `Candidate`** que `resolve_target(app, Some(bucket))`.
6. Divergência intencional e documentada: fora da pré-condição, `resolve_target` cai para casamento por **nome** (`target.rs` :79–91) e poderia "atualizar" outro pacote; `resolve_installed` nunca faz isso.

**Files:**
- Modify: `src/install/ops.rs`, `src/install/target.rs`, `src/cli/install_cmd.rs`, `src/tui/jobs.rs`
- Test: `tests/install_ops.rs`

Testes:

1. `resolve_installed_matches_resolve_target_for_every_installed_shape` (`tests/install_ops.rs`) — mesmo id em dois buckets (um caminho que ignore o bucket dá o outro), id com maiúscula (um caminho que baixe a caixa erra), e um pacote cujo **nome** é o id de outro:
   ```rust
   #[tokio::test]
   async fn resolve_installed_matches_resolve_target_for_every_installed_shape() {
       let mut w = World::new().await;
       w.add(app("ripgrep", &[])).await;
       w.add(app("Git", &[])).await;
       w.add(app("vscode", &[])).await;
       w.catalog
           .replace_source(
               "scoop:extras",
               SourceKind::Scoop,
               "v",
               1,
               &[
                   NewPackage {
                       id: "vscode".into(),
                       name: "vscode".into(),
                       version: "2".into(),
                       ..Default::default()
                   },
                   NewPackage {
                       id: "zed".into(),
                       name: "ripgrep".into(),
                       version: "1".into(),
                       ..Default::default()
                   },
               ],
               &WingetMaps::default(),
           )
           .unwrap();
       let ctx = w.ctx();
       for (bucket, id) in [
           ("main", "ripgrep"),
           ("main", "Git"),
           ("main", "vscode"),
           ("extras", "vscode"),
           ("extras", "zed"),
       ] {
           match (
               resolve_target(&ctx, id, Some(bucket)).unwrap(),
               resolve_installed(&ctx, id, bucket).unwrap(),
           ) {
               (Resolution::One(old), Resolution::One(new)) => assert_eq!(old, new, "{bucket}/{id}"),
               (old, new) => panic!("{bucket}/{id}: antigo {old:?}, novo {new:?}"),
           }
       }
       for (bucket, id) in [("main", "zed"), ("main", "nada")] {
           assert!(
               matches!(resolve_target(&ctx, id, Some(bucket)).unwrap(), Resolution::NotFound { .. }),
               "{bucket}/{id}"
           );
           assert!(
               matches!(resolve_installed(&ctx, id, bucket).unwrap(), Resolution::NotFound { .. }),
               "{bucket}/{id}"
           );
       }
   }
   ```
2. `resolve_installed_never_matches_by_name` (`tests/install_ops.rs`), mesmo catálogo do teste 1 (extraia um helper se quiser): `resolve_target(&ctx, "ripgrep", Some("extras"))` ⇒ `One(c)` com `c.entry.id == "zed"` (casamento por nome — o motivo de não usá-lo no upgrade); `resolve_installed(&ctx, "ripgrep", "extras")` ⇒ `NotFound`.
3. Os testes de upgrade existentes continuam verdes (`upgradable_lists_apps_with_a_newer_catalog_version` em `tests/install_ops.rs`, os de `tests/tui_jobs.rs` e `tests/cli_install.rs`).

Medição (orquestrador; A/B contra a base num worktree temporário, como manda a regra de desempenho). `upgrade --dry-run` é repetível (não muda nada) desde que exista um app desatualizado numa raiz isolada:

```
$S = '<scratchpad>'
# raiz isolada com o catálogo real copiado só-leitura (relatório da fase 2a §2)
New-Item -ItemType Directory -Force "$S\la\wingetrs" | Out-Null
sqlite3 "file:$env:LOCALAPPDATA/wingetrs/catalog.db?mode=ro" ".backup '$S/la/wingetrs/catalog.db'"
$env:LOCALAPPDATA = "$S\la"; $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\t46'; $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
target\release\wingetrs.exe install ripgrep --yes
sqlite3 "$S\la\wingetrs\state.db" "UPDATE apps SET version='0.0.1' WHERE app='ripgrep'"
target\release\wingetrs.exe update      # http.json novo: nenhum update em segundo plano durante o bench
pwsh -File scripts/leftovers.ps1
pwsh -File scripts/bench.ps1 -Exe "$S\base\target\release\wingetrs.exe" -Args 'upgrade ripgrep --dry-run' -Runs 15 -Baseline target/release/examples/noop.exe
pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'upgrade ripgrep --dry-run' -Runs 15 -Baseline target/release/examples/noop.exe
```

(`$S\base` = `git worktree add` no SHA anterior à tarefa + `cargo build --release` lá; remova o worktree e a raiz no fim. Os dois binários precisam da Task 45 para a leitura por span abaixo, então a base é o commit da 45.) Leitura por etapa, nas duas versões: `target\release\wingetrs.exe -vv upgrade ripgrep --dry-run 2>&1 | Select-String 'target\.resolve|upgrade\.list|plan\.build'` — o `target.resolve` da base contra o `target.resolve_installed` da nova é o ganho isolado do ruído de processo. Ganho esperado ≈ tempo de `load_entries` do catálogo real: **não medido**. Se o delta do bench não for maior que o ruído entre execuções da base, vale o número do span; se nenhum dos dois mostrar ganho, a tarefa é revertida (regra do CLAUDE.md).

#### Checks beyond the steps

1. Bucket respeitado: em `resolve_installed`, troque a busca por `ctx.catalog.find_by_id(app)?.into_iter().next()` (ignora o bucket) ⇒ PEGA (teste 1, `extras/vscode` vira o de `main`).
2. Caixa exata: passe `&app.to_lowercase()` ao `get` ⇒ PEGA (teste 1, `main/Git`).
3. Candidato idêntico: em `target::candidate`, troque `selectable: scoop` por `selectable: true` **só numa cópia** usada por `resolve_installed` (se o implementador duplicou a construção em vez de reusar) ⇒ deve PEGAR; se reusou `candidate`, registre "mutação não aplicável: construção única".
4. Uso pelos chamadores: a troca `resolve_target` ⇄ `resolve_installed` em `upgrade`/`plan_upgrade_all` é **mutação equivalente** (mesmo resultado por construção — é o que o teste 1 prova); sem prova de mutação. Verificação: `grep -n "resolve_target" src/cli/install_cmd.rs src/tui/jobs.rs` mostra só os usos de `install`/`plan_install`, e o ganho aparece no bench/span acima.

**Commit:** `perf(upgrade): look up the installed app's catalog row instead of loading the catalog`

**Foco extra do revisor:** (a) a pré-condição (`upgradable` usa `get` exato) é o que garante a equivalência — se alguém mudar `upgradable` para busca sem caixa, o upgrade de um app com id de caixa diferente passaria a dar `NotFound`; confira que o teste 1 cobre `Git`; (b) `plan_upgrade_all` deixou de aceitar bucket `None` — mensagem em português; (c) número sem bench A/B ou sem span = "não medido", não "mais rápido".

---

### Task 47: `update --background` com intervalo depois de cada tentativa e sem esperar o lock

#### Where this fits

Depois da Task 45 (mesmos `src/update.rs`, `src/cli/mod.rs`, `tests/update.rs`); independente da 46. Hoje:

- `refresh_if_stale` (`src/cli/mod.rs` ~427–432) dispara `spawn_background_update` sempre que `needs_refresh` (`src/update.rs` ~80) é verdadeiro.
- `checked_at` só é gravado quando a fonte respondeu (304 em ~197–201; 200 + gravação no catálogo em `apply` ~272–276). Fonte que falha — offline, HTTP 500, catálogo travado (`apply` ~278) — fica sem `checked_at` e `needs_refresh` continua verdadeiro: **cada** `search`/`list`/`show`/`install` dispara um processo novo. Mesmo online, comandos em sequência rápida disparam um processo cada até o primeiro gravar `http.json`.
- O `update --background` usa `LockWait::from_env()` (~591) = espera até **30 s** pelo `update.lock`; o 2º processo fica vivo esperando e depois refaz a fonte que falhou.

**Desenho escolhido (o mais simples que mata a tempestade):** registrar a **tentativa** no momento do disparo, num arquivo próprio, e não disparar de novo por `RETRY_SECS = 300` s; e o `--background` nunca espera o lock (`NoWait`, sai em silêncio se outro update está rodando). Por que não registrar a falha no `http.json`: (1) o `http.json` é gravado pelo `run_update` sob o lock — o processo do comando gravando ali correria contra o update em andamento (escrita perdida); (2) registrar no disparo cobre também o caso "online, vários comandos antes do 1º terminar" e o processo que morre sem gravar nada. Por que intervalo fixo e não exponencial: o exponencial precisa de contador de falhas persistido e não muda a ordem de grandeza (offline com uso contínuo: no máximo 12 processos/hora, cada um sai em < 1 s com conexão recusada); `MAX_AGE_SECS` (900) continua mandando quando tudo dá certo.

#### What binds this task

1. `src/cache/mod.rs`: `pub fn refresh_attempt_path(&self) -> PathBuf` = `self.root.join("refresh.attempt")` (conteúdo: segundos Unix em texto).
2. `src/update.rs`: `pub const RETRY_SECS: i64 = 300;` e a regra pura
   ```rust
   /// A detached refresh is due when a source is stale and the last attempt is at least
   /// RETRY_SECS old (or in the future: the clock went back, never block forever).
   pub fn background_due(state: &HttpState, urls: &[&str], last_attempt: Option<i64>, now: i64) -> bool {
       needs_refresh(state, urls, now)
           && last_attempt.is_none_or(|t| t > now || now - t >= RETRY_SECS)
   }
   ```
3. `src/update.rs`: `pub fn claim_background_refresh(data: &DataDir, urls: &[&str], now: i64) -> bool` — carrega `HttpState`, lê `refresh_attempt_path()` (ausente ou ilegível ⇒ `None`), e se `background_due` grava `now` no arquivo e devolve `true`. Falha ao gravar ⇒ `tracing::warn!` e ainda devolve `true` (problema de disco não pode impedir a atualização). Nunca entra em pânico.
4. `src/cli/mod.rs` `refresh_if_stale`: `if claim_background_refresh(data, &all_urls(ep), now_unix()) { spawn_background_update(); }`.
5. `src/cli/mod.rs` `Command::Update { force, background }`: com `background`, `LockWait::NoWait`, e `Err(UpdateError::Busy)` ⇒ `Ok(())` (só `tracing::debug!`). Sem `background`, nada muda (espera `LockWait::from_env()` e `Busy` continua erro).
6. A TUI não muda (o refresh dela é no próprio processo, com `NoWait`, `src/tui/run.rs` ~219).

**Files:**
- Modify: `src/cache/mod.rs`, `src/update.rs`, `src/cli/mod.rs`
- Test: `tests/update.rs`, `tests/cli_offline.rs`

Testes (sem `sleep`; tempo injetado; rede só por wiremock ou porta 9 recusada):

1. `background_refresh_backs_off_after_an_attempt` (`tests/update.rs`) — valores de fronteira dos dois lados:
   ```rust
   #[test]
   fn background_refresh_backs_off_after_an_attempt() {
       let stale = HttpState::default();
       let now = 10_000;
       assert!(background_due(&stale, &["u"], None, now), "nunca tentou");
       assert!(!background_due(&stale, &["u"], Some(now - (RETRY_SECS - 1)), now));
       assert!(background_due(&stale, &["u"], Some(now - RETRY_SECS), now));
       assert!(background_due(&stale, &["u"], Some(now + 60), now), "relógio voltou: não trava");
       let mut fresh = HttpState::default();
       fresh.set("u", HttpEntry { checked_at: now - 10, ..Default::default() });
       assert!(!background_due(&fresh, &["u"], None, now), "catálogo novo: nada a fazer");
   }
   ```
2. `claim_records_the_attempt_and_refuses_a_second_claim_inside_the_window` (`tests/update.rs`):
   ```rust
   #[test]
   fn claim_records_the_attempt_and_refuses_a_second_claim_inside_the_window() {
       let dir = tempfile::tempdir().unwrap();
       let data = DataDir::new(dir.path());
       data.ensure().unwrap();
       let urls = ["http://127.0.0.1:9/x"];
       assert!(claim_background_refresh(&data, &urls, 10_000));
       let read = || std::fs::read_to_string(data.refresh_attempt_path()).unwrap();
       assert_eq!(read().trim(), "10000");
       assert!(!claim_background_refresh(&data, &urls, 10_060));
       assert_eq!(read().trim(), "10000", "recusa não regrava");
       assert!(claim_background_refresh(&data, &urls, 10_000 + RETRY_SECS));
       assert_eq!(read().trim(), (10_000 + RETRY_SECS).to_string());
   }
   ```
3. `unreadable_attempt_file_does_not_block_the_refresh` (`tests/update.rs`): grave `"lixo"` em `refresh_attempt_path()` ⇒ `claim_background_refresh(&data, &urls, 10_000)` é `true` e o arquivo passa a conter `10000`.
4. `a_failed_source_does_not_respawn_on_every_command` (`tests/update.rs`) — o cenário do bug, com o `setup()` existente (extras responde 500):
   ```rust
   #[tokio::test]
   async fn a_failed_source_does_not_respawn_on_every_command() {
       let (_s, ep) = setup().await;
       let dir = tempfile::tempdir().unwrap();
       let data = DataDir::new(dir.path());
       data.ensure().unwrap();
       run_update(&data, &ep, &Http::new().unwrap(), true, LockWait::from_env())
           .await
           .unwrap();
       let urls: Vec<&str> = std::iter::once(ep.winget_index.as_str())
           .chain(ep.scoop.iter().map(|(_, u)| u.as_str()))
           .collect();
       let now = wingetrs::cache::now_unix();
       assert!(
           needs_refresh(&HttpState::load(&data.http_state_path()), &urls, now),
           "extras falhou: continua velho"
       );
       assert!(claim_background_refresh(&data, &urls, now));
       for later in [1, 30, RETRY_SECS - 1] {
           assert!(!claim_background_refresh(&data, &urls, now + later), "+{later}s");
       }
   }
   ```
5. `background_update_gives_up_at_once_when_another_update_holds_the_lock` e `foreground_update_still_waits_for_the_lock` (`tests/cli_offline.rs`) — timeout de 10 s contra limite de 3 s, para que "espera e depois sai 0" não passe:
   ```rust
   fn update_with_lock_held(args: &[&str], timeout_ms: &str) -> (std::process::Output, std::time::Duration) {
       let dir = tempfile::tempdir().unwrap();
       let root = dir.path().join("wingetrs");
       std::fs::create_dir_all(&root).unwrap();
       let _held = wingetrs::cache::UpdateLock::try_acquire(&root.join("update.lock"))
           .unwrap()
           .unwrap();
       let t = std::time::Instant::now();
       let out = Command::new(env!("CARGO_BIN_EXE_wingetrs"))
           .args(args)
           .env("LOCALAPPDATA", dir.path())
           .env("WINGETRS_LOCK_TIMEOUT_MS", timeout_ms)
           .env("WINGETRS_WINGET_INDEX_URL", "http://127.0.0.1:9/x")
           .env("WINGETRS_SCOOP_BASE_URL", "http://127.0.0.1:9")
           .output()
           .unwrap();
       (out, t.elapsed())
   }

   #[test]
   fn background_update_gives_up_at_once_when_another_update_holds_the_lock() {
       let (out, took) = update_with_lock_held(&["update", "--background"], "10000");
       assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
       assert!(took < std::time::Duration::from_secs(3), "{took:?}");
   }

   #[test]
   fn foreground_update_still_waits_for_the_lock() {
       let (out, took) = update_with_lock_held(&["update"], "1500");
       assert!(!out.status.success());
       assert!(took >= std::time::Duration::from_millis(1400), "{took:?}");
   }
   ```

#### Checks beyond the steps

1. Fronteira: em `background_due`, troque `now - t >= RETRY_SECS` por `now - t > RETRY_SECS` ⇒ PEGA (teste 1). `pwsh -File scripts/mutate.ps1 -Path src/update.rs -Anchor 'now - t >= RETRY_SECS' -Replacement 'now - t > RETRY_SECS' -TestCommand 'cargo test --quiet --test update background_refresh_backs_off'`.
2. Relógio que volta: apague `t > now ||` ⇒ PEGA (teste 1).
3. Registro da tentativa: em `claim_background_refresh`, não grave o arquivo ⇒ PEGA (testes 2 e 4).
4. Regra de idade ainda vale: troque o corpo de `background_due` por só a parte do `last_attempt` ⇒ PEGA (teste 1, caso `fresh`).
5. `--background` sem espera: volte para `LockWait::from_env()` ⇒ PEGA (teste 5a, demora ~10 s).
6. Só o `--background`: use `NoWait` para todo `update` ⇒ PEGA (teste 5b).
7. Arquivo ilegível: faça a leitura de `refresh.attempt` falhar como "tentativa agora" (`Some(now)`) ⇒ PEGA (teste 3).

**Commit:** `fix(update): back off background refresh after an attempt and never wait for the lock`

**Foco extra do revisor:** (a) dois comandos simultâneos podem ainda ler o arquivo antes de qualquer um gravar e disparar dois processos — aceito: o 2º sai na hora pelo `NoWait`; confira que não foi "resolvido" com lock de arquivo extra no caminho quente do `search`; (b) o caminho quente do `search` ganhou uma leitura de arquivo pequeno só quando `needs_refresh` já é verdadeiro — confira a ordem (`needs_refresh` primeiro, arquivo depois) para não ler disco à toa no caso comum; (c) os testes de `tests/cli_offline.rs` que já existem disparam `update --background` reais contra a porta 9 / wiremock — depois desta tarefa, no máximo um por raiz temporária; rode `scripts/leftovers.ps1` depois do gate.

## Correções restantes (Tasks 48–52)

### Task 48: O plano de uninstall diz o que acontecerá com a entrada do PATH

#### Where this fits

Defeito visto na validação da fase 2a (`docs/superpowers/reports/2026-10-10-fase2a-validacao.md`, "Pendências"): `uninstall` imprime "Desfazer entrada no PATH: …\shims" mesmo quando outro app ainda usa a pasta de shims, e o executor corretamente **mantém** a entrada (refcount em `src/install/executor.rs:767-783` via `StateDb::path_users`, `src/install/state.rs:194`). O texto vem de `src/cli/output.rs:250` (`_ => format!("Desfazer {}: {}", …)`), sem consultar o refcount. A TUI tem o mesmo problema e ainda imprime `Debug` (`src/tui/jobs.rs:92-93`: `"  - {:?} {}"`, ex.: `- Path C:\…\shims`). O plano é montado por `UninstallPlan::from_state` (`src/install/plan.rs:95-110`), que já recebe o `StateDb`.

#### What binds this task

- `UninstallPlan` (`src/install/plan.rs`) ganha `pub path_users: Vec<(String, Vec<String>)>`: para cada artefato `ArtifactKind::Path` do app, se `state.path_users(app, &a.target)` não for vazio, um par `(a.target.clone(), users)`. Preenchido em `from_state`, na ordem dos artefatos. Sem consulta nova ao banco além de `path_users`.
- Nova `pub fn artifact_lines(&self) -> Vec<String>` em `impl UninstallPlan` (`src/install/plan.rs`), uma linha por artefato, **na ordem de `artifacts`**:
  - `Path` com usuários: `"Manter entrada no PATH: {target} (ainda usada por: {a, b})"` (nomes unidos por `", "`).
  - `Path` sem usuários: `"Desfazer entrada no PATH: {target}"` (texto atual, inalterado).
  - `Persist` com `purge`: `"Apagar {target} (dados persistentes)"`; sem `purge`: `"Manter {target} (dados persistentes; use --purge)"` (texto atual).
  - Demais tipos: `"Desfazer {tipo}: {target}"` com os nomes atuais de `artifact_kind` (`src/cli/output.rs:215-224`), que se mudam para `plan.rs`.
- `uninstall_plan_text` (`src/cli/output.rs:227`) passa a ser: cabeçalho + `Apagar <app_dir>` + `artifact_lines()` numeradas. Saída idêntica à de hoje quando nenhum PATH é compartilhado.
- `plan_remove` (`src/tui/jobs.rs:89`) usa `artifact_lines()` (`"  - {linha}\n"`) em vez de `{:?}`; a linha final sobre dados persistentes continua.
- O executor **não muda**: ele reconfere o refcount na execução (o plano é uma previsão feita fora do lock; o texto descreve o estado no momento do plano).
- `tests/tui_state.rs:575` constrói `UninstallPlan` literal: acrescente `path_users: vec![]`.

**Files:**
- Modify: `src/install/plan.rs`, `src/cli/output.rs`, `src/tui/jobs.rs`
- Test: `tests/install_state.rs`, `tests/tui_jobs.rs`, `tests/tui_state.rs` (só o literal)

Testes (use os helpers `rec`/`art` de `tests/install_state.rs`; caminhos-texto sintéticos):
1. `uninstall_plan_keeps_a_shared_path_entry_and_undoes_its_own` (`tests/install_state.rs`): `a` grava `[Path "C:\R\shims", Path "C:\R\apps\a\current\bin", Shim "rg"]`; `b` grava `[Path r"c:\r\shims\"]` (equivalência de caminho); `c` grava `[Env "C:\R\shims"]` (outro tipo, não conta). `UninstallPlan::from_state(&db, "a", false)` ⇒ `path_users == [("C:\R\shims", ["b"])]`; `artifact_lines()` ⇒ exatamente `["Manter entrada no PATH: C:\R\shims (ainda usada por: b)", "Desfazer entrada no PATH: C:\R\apps\a\current\bin", "Desfazer atalho de linha de comando: rg"]`. (Um plano com uma entrada mantida **e** uma desfeita: "sempre Manter" e "sempre Desfazer" falham.)
2. `uninstall_plan_lists_every_other_user_of_the_entry` (`tests/install_state.rs`): `a`, `b`, `c` gravam `Path "C:\R\shims"` ⇒ linha de `a` termina em `"(ainda usada por: b, c)"`.
3. `uninstall_plan_text_is_unchanged_without_shared_entries` (`tests/install_state.rs`): só `a` com `[Path "C:\R\shims", Persist "conf"]`, `purge = false`; `wingetrs::cli::output::uninstall_plan_text(&plan, &Layout::new(r"C:\R"))` ⇒ igual ao literal `"Desinstalar a 1\n  1. Apagar C:\R\apps\a\n  2. Desfazer entrada no PATH: C:\R\shims\n  3. Manter conf (dados persistentes; use --purge)\n"` (confira o formato do cabeçalho com `rec("a","1")`).
4. `uninstall_plan_text_says_kept_when_shared` (`tests/install_state.rs`): cenário do teste 1 ⇒ o texto da CLI contém `"Manter entrada no PATH: C:\R\shims (ainda usada por: b)"` e não contém `"Desfazer entrada no PATH: C:\R\shims"`.
5. `plan_remove_text_uses_the_plan_lines` (`tests/tui_jobs.rs`, `World` existente; o fluxo que instala `leaf` já existe na l. ~190–207): depois de instalar `leaf`, `jobs::plan_remove(&ctx, "leaf")` ⇒ `Planned::Ready { text, .. }` com `text` contendo `"Desfazer entrada no PATH: "` e **sem** `"Path "` nem `"Shim "` (o `Debug` antigo).

#### Checks beyond the steps

1. Refcount no texto: mutate trocando o ramo "Manter" por "Desfazer" em `artifact_lines` ⇒ PEGA (testes 1, 4).
2. Só `Path` conta: mutate fazendo `from_state` consultar `path_users` para todo artefato ⇒ PEGA (teste 1: `Shim "rg"` não pode virar "Manter"; e `path_users` teria 2 pares).
3. Lista completa: mutate usando só `users[0]` ⇒ PEGA (teste 2).
4. CLI usa a função comum: mutate devolvendo `uninstall_plan_text` ao `match` antigo ⇒ PEGA (teste 4).
5. TUI usa a função comum: mutate voltando `plan_remove` para `{:?}` ⇒ PEGA (teste 5).

**Commit:** `fix(uninstall): plan text says when a shared PATH entry is kept`

---

### Task 49: `bin` com argumentos extras, alias e argumentos validados

#### Where this fits

`resolve` recusa item de `bin` com mais de 3 elementos (`src/install/manifest.rs:322`: `tuple_items(field(root, variant, "bin"), "bin", 3)`; `:189` devolve `Invalid("bin: item com 4 elementos")`). Isso derruba `main/zigup` (`["zigup.exe","zigup","--path-link","\"$persist_dir\\zig\\zig.exe\""]`, Task 40/`examples/oracle_2a.rs`). Nos buckets locais em `~/scoop/buckets` há 3 manifestos assim (contagem nesta sessão: itens de 2 elementos 725, 3 elementos 329, 4 elementos 2 — `zigup`, `wgcf` —, 5 elementos 1 — `stdiscosrv`).

Comportamento do scoop (conferido no código instalado, `~/scoop/apps/scoop/current`, commit `e6aa3b366bdee8ed138c1e0f7b85192ebdd35d0f`): `lib/install.ps1:179` faz `$target, $name, $arg = shim_def $_` — a atribuição múltipla do PowerShell põe **todos** os elementos a partir do 3º em `$arg` (array); `lib/core.ps1:989/1003/1019/…` interpolam `$arg` em string (`"args = $arg"`, `"… $arg %*"`), e o PowerShell junta array interpolado com `$OFS` = **um espaço**. `substitute` (`$dir`, `$persist_dir`, `$original_dir`) é aplicado a cada elemento. Alias vazio: `core.ps1:953` `if (!$name) { $name = strip_ext (fname $path) }`.

Duas lacunas de segurança achadas no mesmo trecho (regra "Todo caminho vindo de manifesto passa por `install::safepath`"):
- O alias vira nome de arquivo em `shims\` e **não** é validado no planejador (`src/install/planner.rs:229`). Só `shim::create_shim` recusa (`src/install/shim.rs:147-155`), na execução, depois do download — e `make_shim` (`src/install/executor.rs:651-665`) já leu (e no rollback regrava) `shims\<alias>.{exe,shim,cmd,ps1}` antes disso, com `alias = "..\\x"` fora de `shims\`.
- `args` vai cru para a linha `args = …` do `.shim` (`src/install/shim.rs:195-196`). Um `\r`/`\n` no argumento injeta uma linha `path = …` no arquivo que o lançador lê. Nenhum manifesto local tem caractere de controle em args (contado: 0).

#### What binds this task

- `src/install/manifest.rs`: o item de `bin` aceita **qualquer** número de elementos ≥ 1 (lista vazia continua `Invalid`; elemento não-texto continua `Invalid`). `BinEntry.args` = elementos a partir do 3º unidos com `" "` (um espaço, como o `$OFS` do scoop), `None` quando há só 1 ou 2 elementos. `shortcuts` (máx. 4) e `persist` (máx. 2) não mudam.
- Planejador (`src/install/planner.rs`, laço de `m.bin`):
  - alias `Some("")` ⇒ usa `stem(path)` (como o scoop).
  - o nome final do shim precisa ser **um** componente seguro: mesma regra de `shim::check_name` (sem `/` nem `\`, `safepath::safe_components(name)` com exatamente 1 componente) ⇒ senão `PlanError::UnsafePath(name)`. Não duplique a regra: torne `check_name` (ou um predicado `pub(crate) fn is_safe_shim_name`) reutilizável e chame-o dos dois lugares.
  - `args` (já substituído) com qualquer caractere de controle (`char::is_control`) ⇒ nova `PlanError::UnsafeArgs(String)` com mensagem `"argumento inseguro no manifesto: {0}"` (mostre o texto com `{:?}`).
- Substituição de variáveis continua por `substitute` sobre o texto já unido (equivalente a substituir elemento a elemento, porque o separador é espaço).
- Nada muda no executor nem em `shim.rs` além de expor o predicado.

**Files:**
- Modify: `src/install/manifest.rs`, `src/install/planner.rs`, `src/install/shim.rs` (só visibilidade do predicado)
- Test: `tests/install_manifest.rs`, `tests/install_planner.rs`

Testes (JSON literal no teste; **não** crie fixture nova — `tests/install_planner.rs:229-250` gera snapshot `insta` para cada arquivo de `tests/fixtures/scoop/manifests`):
1. `bin_with_four_elements_joins_the_extra_args` (`tests/install_manifest.rs`): `{"version":"1","url":"https://x/z.zip","hash":"<64 hex>","bin":[["zigup.exe","zigup","--path-link","\"$persist_dir\\zig\\zig.exe\""]]}` ⇒ `bin[0].alias == Some("zigup")`, `bin[0].args == Some("--path-link \"$persist_dir\\zig\\zig.exe\"")`.
2. `bin_with_five_elements_joins_all_three` (`tests/install_manifest.rs`): `["s.exe","s","-a 1","-b 2","-c 3"]` ⇒ `args == Some("-a 1 -b 2 -c 3")`.
3. `bin_with_three_and_two_elements_is_unchanged` (`tests/install_manifest.rs`): `["a.exe","a","--x"]` ⇒ `args == Some("--x")`; `["b.exe","b"]` ⇒ `args == None`; `"c.exe"` ⇒ alias `None`, args `None`.
4. `garbage_is_an_error_not_a_panic` (existente) continua verde com `"bin":[[]]`; acrescente `r#"{"version":"1","bin":[["a.exe","a","x",5]]}"#` (não-texto) à lista.
5. `zigup_args_are_substituted` (`tests/install_planner.rs`, use o helper de plano do arquivo com JSON literal): o manifesto do teste 1 ⇒ `Step::Shim` com `name == "zigup"` e `args == Some(format!("--path-link \"{}\\zig\\zig.exe\"", layout.persist_dir("app").display()))`.
6. `empty_alias_falls_back_to_the_file_stem` (`tests/install_planner.rs`): `[["tools\\run.exe",""]]` ⇒ shim `name == "run"`.
7. `unsafe_shim_alias_is_refused_by_the_planner` (`tests/install_planner.rs`): para cada alias em `["..\\evil", "a/b", "CON", "x."]` ⇒ `Err(PlanError::UnsafePath(_))`; e o alias `"ok"` no mesmo manifesto ⇒ `Ok` (garante que a recusa vem do alias).
8. `control_characters_in_args_are_refused` (`tests/install_planner.rs`): `[["a.exe","a","--x\r\npath = C:\\evil.exe"]]` ⇒ `Err(PlanError::UnsafeArgs(_))`; com `"--x path = C:\\evil.exe"` (sem quebra) ⇒ `Ok`.

#### Checks beyond the steps

1. Limite removido: mutate voltando o máximo de `bin` para 3 ⇒ PEGA (teste 1).
2. Separador: mutate trocando `join(" ")` por `join("")` ⇒ PEGA (testes 1, 2).
3. Só o 3º elemento: mutate pegando apenas o elemento de índice 2 ⇒ PEGA (testes 1, 2).
4. Alias vazio: mutate removendo o fallback para `stem` ⇒ PEGA (teste 6).
5. Alias inseguro: mutate removendo a checagem do nome no planejador ⇒ PEGA (teste 7).
6. Controle em args: mutate removendo a checagem de `is_control` ⇒ PEGA (teste 8).
7. Orquestrador: rodar `cargo run --release --example oracle_2a -- tests/fixtures/scoop/manifests %LOCALAPPDATA%\wingetrs\catalog.db 50` e conferir que `zigup` sai da lista de falhas (2365 → 2366 planejam, se o catálogo não mudou).

**Commit:** `fix(manifest): accept bin entries with extra args and validate shim names`

---

### Task 50: Limites contra bomba de descompressão na extração

#### Where this fits

Follow-up aberto desde a Task 26 (`.claude/tasks/progress.md`, linha da Task 26: "Follow-up: limite contra zip bomb"; `.claude/tasks/task-26-report.md:36`). Hoje `write_entry` copia sem limite (`src/install/extract.rs:184`: `io::copy(src, &mut out)?`) e `extract_zip`/`extract_tar_gz` (`:189`, `:216`) não contam entradas nem bytes. O hash do manifesto não protege: um mantenedor de bucket (ou um bucket de terceiro) publica o hash do arquivo malicioso.

Medição desta sessão (90 arquivos zip/tar.gz/nupkg em `%LOCALAPPDATA%\wingetrs\blobs` e `~/scoop/cache`, tamanhos declarados): razão descompactado/compactado do **arquivo inteiro** no máximo 7,5 (terminal-icons .nupkg; go 3,0; tesseract-languages 2,0); maior total 668 MB (tesseract-languages, 335 MB compactado); maior número de entradas 15 639 (go 1.27.1). Uma **entrada** isolada chega a razão 230 (go, arquivo de teste cheio de zeros) — por isso **não** há limite de razão por entrada.

#### What binds this task

- Novo `pub struct Limits { pub ratio: u64, pub floor: u64, pub ceiling: u64, pub max_entries: u64 }` em `src/install/extract.rs` e `pub const DEFAULT_LIMITS: Limits = Limits { ratio: 100, floor: 1 << 30, ceiling: 32 << 30, max_entries: 200_000 }`. Justificativa no comentário de uma linha: razão 100 = 13× o maior medido (7,5); piso 1 GiB cobre qualquer arquivo pequeno de texto; teto 32 GiB = 48× o maior total medido; 200 000 entradas = 12× o maior medido (15 639).
- Orçamento de bytes **por arquivo compactado**: `budget = clamp(tamanho_do_arquivo × ratio, floor, ceiling)` (tamanho por `fs::metadata(file).len()`, aritmética com `saturating_mul`).
- Nova `pub fn extract_all_with(sources: &[ExtractSource<'_>], dest: &Path, limits: &Limits) -> Result<(), ExtractError>`; `extract_all` vira `extract_all_with(sources, dest, &DEFAULT_LIMITS)`; `extract` (usado pelo lançador de shims) também usa `DEFAULT_LIMITS`. Assinaturas atuais preservadas.
- Novas variantes: `ExtractError::TooLarge { limit: u64 }` — `"arquivo descompactado passa de {limit} bytes (possível bomba de descompressão)"` — e `ExtractError::TooManyEntries { limit: u64 }` — `"arquivo com mais de {limit} entradas (possível bomba de descompressão)"`.
- **zip**: no laço de validação que já existe (antes de escrever qualquer coisa, `src/install/extract.rs:192-201`), `zip.len() > max_entries` ⇒ `TooManyEntries`; soma dos `size()` declarados (`by_index_raw`) > `budget` ⇒ `TooLarge`. Nada é escrito nesses casos.
- **zip e tar.gz**: a cópia passa por um leitor que conta os bytes **realmente** escritos (soma do arquivo inteiro, não por entrada) e falha com `TooLarge` ao passar do orçamento — cabeçalho declarado pode mentir; o tar.gz não tem diretório central. tar.gz: contador de entradas (todas as entradas iteradas, inclusive as puladas) > `max_entries` ⇒ `TooManyEntries`.
- Falha no meio ⇒ comportamento atual de erro: o `TempGuard` apaga a pasta temporária, `dest` intacto (já testado para outros erros; acrescente o teste 5).
- Arquivo solto (sem extração) não tem limite (é copiado byte a byte do blob já verificado).

**Files:**
- Modify: `src/install/extract.rs`
- Test: `tests/install_extract.rs` (use `zip_with`/`targz_with` existentes, l. 5 e 19)

Testes (fixtures geradas em memória; cada uma falha **só** na checagem que testa — a mesma entrada passa com o limite folgado):
1. `zip_over_the_byte_budget_is_refused_before_writing` : zip com uma entrada `big.bin` de 2 MiB de zeros (comprime para poucos KiB); `Limits { ratio: 100, floor: 1 << 20, ceiling: 1 << 40, max_entries: 10 }` ⇒ `Err(TooLarge { limit: 1 << 20 })` e `dest` não existe; o **mesmo** zip com `floor: 4 << 20` ⇒ `Ok` e `big.bin` tem 2 MiB.
2. `ratio_raises_the_budget_above_the_floor`: zip de 3 entradas de 1 MiB de bytes pseudoaleatórios incompressíveis (xorshift32 com semente fixa, `x ^= x << 13; x ^= x >> 17; x ^= x << 5`, um byte por iteração — **não** use `i * constante`, que tem período 256 e comprime; total 3 MiB, arquivo ≈ 3 MiB) com `floor: 1 << 20, ratio: 2` ⇒ `Ok` (orçamento ≈ 6 MiB > 3 MiB); com `ratio: 0` (orçamento = piso 1 MiB) ⇒ `TooLarge`. (Prova que o orçamento usa o tamanho do arquivo, não só o piso.)
3. `zip_with_too_many_entries_is_refused`: zip com 11 entradas de 1 byte; `max_entries: 10` ⇒ `Err(TooManyEntries { limit: 10 })`; `max_entries: 11` ⇒ `Ok`.
4. `tar_gz_over_the_byte_budget_is_refused`: tar.gz com `big.bin` de 2 MiB de zeros; `floor: 1 << 20` ⇒ `TooLarge`; `floor: 4 << 20` ⇒ `Ok`. (O tar não tem tamanho declarado separado: só o contador pega.)
5. `tar_gz_with_too_many_entries_is_refused`: 11 entradas; `max_entries: 10` ⇒ `TooManyEntries`, `dest` não existe e não sobra `.wingetrs-extract-*` no pai; `11` ⇒ `Ok`.
6. `budget_is_for_the_whole_archive_not_per_entry`: **tar.gz** (no zip a checagem prévia dos tamanhos declarados pegaria antes do contador) com duas entradas de 600 KiB de zeros, `floor: 1 << 20` ⇒ `TooLarge` (cada uma cabe sozinha, a soma não); com `floor: 2 << 20` ⇒ `Ok`.
7. `default_limits_are_the_measured_ones`: `DEFAULT_LIMITS` == os quatro números acima (pina a decisão; o revisor confere o comentário de justificativa).
8. **Cabeçalho mentiroso (exploratório, opus)**: gere um zip de `big.bin` com 2 MiB de zeros e altere nos bytes o tamanho descompactado declarado (diretório central, offset 24 do registro `PK\x01\x02`; cabeçalho local, offset 22 de `PK\x03\x04`) para 1 KiB. Rode com `floor: 1 << 20`. Se o crate `zip` 8.6 já recusar a entrada sozinho (erro de tamanho/CRC), registre isso no relatório e afirme `Err(_)` sem escrever mais que o orçamento; se ele entregar os 2 MiB, o contador tem que dar `TooLarge`. O relatório diz qual dos dois aconteceu.

#### Checks beyond the steps

1. Contador: mutate removendo o leitor que conta bytes ⇒ PEGA (testes 4 e 6). Zip com as duas defesas removidas (soma declarada **e** contador) ⇒ PEGA (teste 1). Remover **só** a soma declarada do zip é mutação equivalente no resultado observável (o contador dá o mesmo `TooLarge` e o `TempGuard` apaga o que foi escrito): o relatório registra isso como "mutação equivalente" (ARMADILHAS, "Mutação equivalente existe") — a soma fica por não escrever nada antes de recusar.
2. Orçamento por arquivo inteiro: mutate zerando o contador a cada entrada ⇒ PEGA (teste 6).
3. Razão: mutate ignorando `ratio` (orçamento = `floor`) ⇒ PEGA (teste 2).
4. Entradas: mutate trocando `>` por `>=` na comparação de entradas ⇒ PEGA (testes 3 e 5: 11 entradas com `max_entries: 11` precisam passar).
5. tar: mutate removendo o contador de entradas do tar ⇒ PEGA (teste 5).
6. Padrões: mutate trocando `ratio: 100` por `ratio: 10` ⇒ PEGA (teste 7).
7. Orquestrador: nenhuma regressão nos testes ao vivo de instalação (`cargo test --quiet --release -- --ignored`, se autorizado) — os pacotes reais medidos ficam ≥ 13× abaixo de cada limite.

**Commit:** `fix(extract): refuse decompression bombs by size and entry count`

---

### Task 51: Reinstalar uma versão que ficou no disco (keep_old e sobras)

#### Where this fits

Follow-up aberto da Task 31 (`.claude/tasks/task-31-report.md:73`: "downgrade to a version kept with keep_old (Promote refuses an existing version dir)"; também no ledger, linha da Task 31). O defeito, preciso:

- `Step::Promote` (`src/install/executor.rs:483-488`) faz `if fs::symlink_metadata(&vdir).is_ok() { return Err("<vdir> já existe") }`.
- `apps\<app>\<versão>` pode existir sem ser a versão instalada em dois casos reais: (a) `upgrade --keep` de 1.0 para 2.0 deixa `apps\app\1.0` (o planejador omite `RemoveOldVersion` com `keep_old`, `src/install/planner.rs:265-271`); (b) `RemoveOldVersion` que falhou só vira aviso (`src/install/executor.rs:364-379`) e deixa a pasta.
- Voltar a essa versão — `install main/app` quando o bucket reverteu para 1.0, ou instalar de outro bucket que está em 1.0: o planejador gera upgrade de 2.0 para 1.0 (`planner.rs:191-199`, versão diferente ⇒ `upgrade_from`) — falha **depois** de baixar e extrair, no passo "promover versão", com uma mensagem que não diz o que fazer. O único remédio hoje é `uninstall` + `install`.
- O scoop, nesse caso, instala por cima da pasta existente (`lib/install.ps1:48` `ensure (versiondir …)`) e, com `--force` na mesma versão, move a antiga para `_<versão>.old` (`libexec/scoop-update.ps1:364-375`).

#### What binds this task

(Decisão do usuário, 2026-10-11: substituir com desfazer.)
- `Step::Promote`: se `vdir` existe **e** não é o alvo atual de `current` (compare com `link::target(current)` normalizado; se for o alvo, mantenha o erro atual), renomeie `vdir` para `apps\<app>\.replaced-<versão>-<pid>-<seq>` (mesmo estilo de `.staging-<v>` de `src/install/layout.rs:26`; `seq` de um `AtomicU64` como os temporários existentes), empilhe um novo `Undo::RestoreAside { aside, original }` (rollback renomeia de volta) **antes** do `rename` de staging, e siga o fluxo atual.
- Depois do sucesso (mesma fase de `remove_upgrade_leftovers`/`RemoveOldVersion`, `executor.rs:361-380`): apaga a pasta `.replaced-*`; falha ⇒ `tracing::warn!`, nunca erro (mesma política).
- Ordem do rollback: o `Undo::RemovePath(vdir)` da versão nova roda antes do `RestoreAside` (pilha LIFO já garante, se empilhar na ordem acima — teste 2 prova).
- O texto do plano não muda (o planejador é puro em relação ao disco e não sabe se a pasta existe); a pasta substituída é cópia antiga — os dados do usuário estão em `persist\`.
- Nada muda em `planner.rs`, `ops.rs` ou `target.rs`.

**Files:**
- Modify: `src/install/executor.rs`
- Test: `tests/install_executor.rs` (use `setup`, `install`, `v2_plan`, `plan_from`, `ctx_with_hook`, `FailAt` e `current_target` existentes; o manifesto da v1 é o que `setup` monta — reaproveite a mesma url/sha)

Testes:
1. `reinstalling_a_kept_version_replaces_its_folder` : instala 1.0; `v2_plan(&s, true)` (keep_old) e instala; escreve `apps\app\1.0\stale.txt`; planeja de novo o manifesto 1.0 (`plan_from`, agora `upgrade_from == Some("2.0")`) e executa ⇒ `Ok`; `current_target` termina em `1.0`; `apps\app\1.0\tool.exe` existe e `stale.txt` **não**; nenhuma entrada `.replaced-*` em `apps\app`; `apps\app\2.0` foi removida (plano sem keep_old); `state.db` registra `1.0`.
2. `failure_after_replacing_a_kept_version_restores_it` : mesmo preparo do teste 1; execute o plano 1.0 com `FailAt(k)` para `k` = índice do passo `LinkCurrent` **e** para o índice do `RecordState` ⇒ `Err`; em ambos: `apps\app\1.0\stale.txt` existe de novo, `current_target` termina em `2.0`, nenhum `.replaced-*` nem `.staging-*` sobra, `state.db` registra `2.0`.
3. `promote_still_refuses_the_folder_current_points_to` : monte à mão um estado em que `current` aponta para `apps\app\1.0` mas o `state.db` registra outra versão (ex.: grave o registro de `app` com versão `0.9` via `s.state.record`), planeje o manifesto 1.0 ⇒ execução `Err` com mensagem contendo `"já existe"` e `apps\app\1.0` intacta.
4. `upgrade_with_keep_old_keeps_v1` (existente) continua verde.

#### Checks beyond the steps

1. Substituição: mutate voltando o `Promote` a recusar toda pasta existente ⇒ PEGA (teste 1).
2. Rollback: mutate removendo o `push(Undo::RestoreAside …)` ⇒ PEGA (teste 2).
3. Limpeza: mutate removendo a remoção do `.replaced-*` após o sucesso ⇒ PEGA (teste 1).
4. Proteção do alvo de `current`: mutate removendo a comparação com `current` ⇒ PEGA (teste 3).
5. Snapshot de rollback geral: os testes de "falha injetada em cada passo" existentes continuam verdes.

**Commit:** `fix(executor): reinstall a version whose folder was kept on disk`

---

### Task 52: Um SHA-256 por blob (cache e download)

#### Where this fits

Em `cached` (`src/install/download.rs:91-100`), para hash `sha256` o nome do blob **é** o hash do manifesto; `BlobStore::get` (`src/cache/mod.rs:164-167`) lê o arquivo inteiro e confere `sha256(conteúdo) == nome`, e logo depois `verify(spec, &bytes)` calcula o **mesmo** SHA-256 de novo. No download (`download.rs:131-137`) é igual: `verify` (SHA-256) e depois `Sha256::digest` para o nome. Desde a Task 42, `apply` passa por `fetch_each` (prefetch) e o executor passa de novo por `fetch_all` no `Step::Download` (`src/install/executor.rs:433-447`), então um blob em cache é lido 2× e hasheado 4× por `install`; baixado, 3 leituras e 4 hashes. A correção abaixo tira a duplicata **dentro** de cada chamada (4 → 2 hashes); a revalidação do executor fica (é a checagem de integridade imediatamente antes de extrair — mudar isso é outra decisão, ver "Itens não incluídos").

Ganho: **não medido**. Ordem de grandeza: o maior blob em cache (tesseract-languages, 335 MB) passa duas vezes a menos pelo SHA-256.

#### What binds this task

- `cached`: quando `spec.algo == HashAlgo::Sha256`, devolve o caminho se `BlobStore::get(&name)` for `Some` **sem** chamar `verify` (o `get` já provou `sha256(bytes) == name == spec.hex` em minúsculas). Para `sha1`/`sha512`/`md5` nada muda: `get` (integridade do blob pelo nome) **e** `verify` (hash do manifesto) — são hashes diferentes.
- `download_one`: quando `spec.algo == HashAlgo::Sha256`, `name = src.hash.hex.to_ascii_lowercase()` depois do `verify` (sem `Sha256::digest`). Outros algoritmos: inalterado.
- A integridade continua verificada **uma** vez por chamada em todos os caminhos. Nenhuma mudança de assinatura, de `index.json` ou de nome de blob.

**Files:**
- Modify: `src/install/download.rs`
- Test: `tests/install_download.rs`

Testes (os de integridade já existem e são os que pegam as mutações perigosas):
1. `tampered_cached_blob_is_redownloaded` (existente, l. 99): continua verde.
2. `index_pointing_at_wrong_blob_is_not_trusted` (existente, l. 123, MD5): continua verde.
3. `uppercase_sha256_in_manifest_lands_in_a_lowercase_blob`: fonte com o SHA-256 de `b"abc"` em **maiúsculas**; `fetch_all` ⇒ o caminho devolvido termina no hex minúsculo, a listagem de `blobs\` (ignorando `index.json`) tem exatamente esse nome em minúsculas; segunda chamada ⇒ 1 requisição no total (cache).
4. `sha512_cached_blob_with_wrong_content_is_redownloaded`: baixa uma fonte `sha512`; sobrescreve o blob **e** renomeia para o SHA-256 do conteúdo novo, atualizando `index.json` para apontar para ele (blob íntegro pelo nome, errado pelo manifesto) ⇒ segunda `fetch_all` baixa de novo (2 requisições) e devolve `b"abc"`.

#### Checks beyond the steps

1. Não pular demais: mutate pulando `verify` para **todos** os algoritmos em `cached` ⇒ PEGA (testes 2 e 4).
2. Não pular a integridade do blob: mutate trocando `BlobStore::get` por leitura sem conferência no ramo sha256 ⇒ PEGA (teste 1).
3. Nome minúsculo: mutate usando `src.hash.hex` sem `to_ascii_lowercase` ⇒ PEGA (teste 3: o NTFS não diferencia caixa, então confira o **nome** do arquivo listado em `blobs\`, não só `exists()`).
4. A redução de hash em si é comportamento-preservante e não tem teste que falhe — **no mutation proof** para o ganho. O orquestrador mede A/B (`bench.ps1 -Baseline`) de um `install` com blob grande em cache e raiz temporária, se quiser número; senão o relatório diz "não medido".

**Commit:** `perf(download): hash a cached or downloaded blob once`

---


## TUI — navegação (Task 53)

### Task 53: Navegação nunca espera os detalhes

#### Where this fits

O usuário relata que na aba Atualizações "só passa para o próximo item depois que os detalhes carregam". O `update` não espera nada (`src/tui/app.rs:322-331`: `move_to` só devolve `Cmd::LoadDetails`). O que trava é o laço em volta (`src/tui/run.rs:101-147`): (1) o ticker do spinner é `tokio::time::interval(TICK)` (`run.rs:98`) com o padrão `MissedTickBehavior::Burst`; o ramo só é consultado enquanto `details_loading` (`run.rs:111`), mas o relógio continua contando, e ao selecionar um item sem detalhes **todos os ticks perdidos disparam em rajada**, cada um com um `terminal.draw` (`run.rs:144-146`), disputando o `select!` com as teclas até os detalhes chegarem; (2) um `draw` por mensagem, sem juntar teclas; (3) `details_loading` (`run.rs:151-154`) fica verdadeiro para sempre depois de `Booted`/`CatalogLoaded`/`InstalledLoaded`/troca de aba/Esc, porque esses caminhos não pedem detalhes (`app.rs:362-366`, `372-396`, `398-402`, `333-342`, `526-537`). Nada disso foi medido; esta tarefa também põe os contadores que permitem medir.

#### What binds this task

- **Estado de detalhe pendente em `App`** (`src/tui/app.rs`): campo novo `pub details_pending: Option<String>` (chave `"{source}/{id}"`). `details_cmd` passa a `&mut self`: quando emite `Cmd::LoadDetails` grava a chave em `details_pending`; quando a seleção não tem chave ou a chave já está em `details`, grava `None`. `Msg::DetailsLoaded { key, .. }` sempre insere no cache, e só limpa `details_pending` se for **a mesma chave**. Método novo `pub fn animating(&self) -> bool { self.refreshing || self.details_pending.is_some() }`. `Msg::Tick` avança `spinner_frame` só se `animating()`.
- **Todo caminho que muda a linha selecionada termina em `details_cmd()`**: `move_to` (já), `set_tab`, `Msg::Booted` com catálogo `Ok`, `Msg::CatalogLoaded`, `Msg::InstalledLoaded`, Esc que limpa a busca, Backspace, digitação, Ctrl+S (os três últimos já). Quando `set_tab` devolve `Cmd::ScanInstalled`, devolve também o `details_cmd()` (que dará vazio se não houver linha).
- **Laço** (`src/tui/run.rs`):
  - `pub fn spinner_ticker() -> tokio::time::Interval`: `interval(TICK)` com `set_missed_tick_behavior(MissedTickBehavior::Skip)`. O laço usa esta função; o ramo do tick fica condicionado a `app.animating()`. Apague `details_loading`.
  - `pub fn drain_ready<S: futures_util::Stream<Item = Msg> + Unpin>(first: Msg, more: &mut S, cap: usize) -> Vec<Msg>`: devolve `first` seguido de tudo que `more` já tem pronto, sem esperar (`futures_util::FutureExt::now_or_never` sobre `more.next()`: `None` = nada pronto ⇒ para; `Some(None)` = fim ⇒ para), no máximo `cap` itens no total. O laço drena **as teclas e o canal `rx`** com `cap = 256` antes de desenhar. Erro do `EventStream` continua encerrando o laço com `Err` como hoje (`run.rs:107`); `Msg::Panicked` continua passando por `jobs::propagate` antes do `update`.
  - `pub fn last_details_only(cmds: Vec<Cmd>) -> Vec<Cmd>`: remove todo `Cmd::LoadDetails` exceto o último; os demais comandos ficam na ordem original.
  - `pub fn needs_draw(batch: &[Msg], animating: bool) -> bool`: verdadeiro se o lote tem qualquer mensagem que não seja `Msg::Tick`, ou se tem `Msg::Tick` e `animating`. Avaliado antes de aplicar o lote. **Um `draw` por lote**, não por mensagem.
  - Os comandos do lote inteiro passam por `last_details_only` antes de executar.
  - `spawn_details` aguarda o trabalho por um `JoinHandle` (regra do CLAUDE.md: pânico nunca engolido): `pub async fn join_details(key: String, h: tokio::task::JoinHandle<Msg>) -> Msg` devolve a mensagem; pânico ⇒ `Msg::Panicked(payload)`; cancelamento ⇒ `Msg::DetailsLoaded { key, result: Err(<texto>) }`. Debounce de 150 ms e descarte por geração (`run.rs:241-255`) ficam como estão.
  - **Contadores** (instrumentação para medir, não otimização): o laço conta `frames`, `keys`, `ticks` e o maior `draw` em µs; ao sair, `tracing::info!(frames, keys, ticks, max_draw_us, secs, "tui stats")`; `tracing::debug!(draw_us, batch, "frame lento")` só quando um `draw` passa de 16 000 µs. Nada em stdout.
- Não mude a aparência (snapshots de `tests/tui_view.rs` iguais), nem a lógica de debounce, nem `jobs.rs`.
- Commits separados: (1) contadores sozinhos, para servir de base de comparação; (2) o resto.

**Files:**
- Modify: `src/tui/run.rs`, `src/tui/app.rs`
- Test: `tests/tui_state.rs`, `tests/tui_view.rs`, `tests/tui_loop.rs` (novo arquivo de teste)

Testes:
1. `selection_moves_while_details_are_pending` (`tests/tui_state.rs`): `on_tab(Tab::Updates, vec![irow("a", "wingetrs", "scoop:main", true), irow("b", "wingetrs", "scoop:main", true), irow("c", "wingetrs", "scoop:main", true)])`; `update(Down)` ⇒ `[Cmd::LoadDetails { source: "scoop:main", id: "b" }]`; **sem entregar** `DetailsLoaded`, `update(Down)` ⇒ `a.selected == 2`, comandos `[Cmd::LoadDetails { id: "c", .. }]`, `a.details_pending == Some("scoop:main/c".into())`. (Passa no código atual também: é a guarda contra introduzir a espera.)
2. `selected_row_moves_on_screen_while_details_load` (`tests/tui_view.rs`): `App::new(entries(), GlyphSet::Ascii)`, duas setas para baixo sem `DetailsLoaded`; `render(&a, 100, 16)` ⇒ a linha do buffer que começa com `"│> "` contém `"powertoys"` (o 3º item; ordem do snapshot `offline`), e o buffer contém `"carregando"`.
3. `every_view_change_requests_details_for_the_new_selection` (`tests/tui_state.rs`): (a) `App::new(vec![], GlyphSet::Nerd)` + `Msg::CatalogLoaded(<as 3 entradas de app()>)` ⇒ comandos contêm `LoadDetails` do primeiro hit; o mesmo com `Msg::Booted { catalog: Ok(<as 3 entradas>), age: None, needs_refresh: false }` ⇒ comandos contêm `ScanInstalled` e `LoadDetails`; (b) `on_tab(Tab::Updates, vec![])` + `Msg::InstalledLoaded(vec![irow("a", "wingetrs", "scoop:main", true)])` ⇒ `LoadDetails { id: "a", .. }`; (c) app com `installed` não vazio na aba Pesquisar, tecla `Tab` ⇒ `LoadDetails` da linha 0 de Instalados; (d) digitar `z` e Esc ⇒ `LoadDetails` do primeiro hit da busca vazia.
4. `spinner_stops_when_the_selected_details_arrive` (`tests/tui_state.rs`): depois de (a) do teste 3, `DetailsLoaded` com a chave pendente ⇒ `!a.animating()`; `Msg::Tick` ⇒ `spinner_frame` inalterado.
5. `stale_details_keep_the_spinner_for_the_current_item` (`tests/tui_state.rs`): Down (pendente b), Down (pendente c), `DetailsLoaded` de b ⇒ `a.animating()` verdadeiro e `a.details` contém b; `DetailsLoaded` de c ⇒ falso.
6. `spinner_ticker_skips_missed_ticks` (`tests/tui_loop.rs`, `#[tokio::test]`): `spinner_ticker().missed_tick_behavior() == MissedTickBehavior::Skip`.
7. `drain_ready_takes_what_is_queued_up_to_the_cap` (`tests/tui_loop.rs`): `let mut s = futures_util::stream::iter((0..4).map(|_| key(KeyCode::Down))).chain(futures_util::stream::pending());` ⇒ `drain_ready(key(KeyCode::Down), &mut s, 256).len() == 5`; com outro stream igual e `cap = 3` ⇒ 3; com `futures_util::stream::pending()` ⇒ 1.
8. `last_details_only_keeps_the_newest_request` (`tests/tui_loop.rs`): `[LoadDetails a, Refresh { force: true }, LoadDetails b, ScanInstalled]` ⇒ `[Refresh { force: true }, LoadDetails b, ScanInstalled]`.
9. `idle_ticks_do_not_draw` (`tests/tui_loop.rs`): `needs_draw(&[Msg::Tick, Msg::Tick], false) == false`; `needs_draw(&[Msg::Tick], true) == true`; `needs_draw(&[key(KeyCode::Down)], false) == true`.
10. `details_worker_panic_reaches_the_loop` (`tests/tui_loop.rs`, `#[tokio::test]`): `join_details("k".into(), tokio::spawn(async { panic!("x") }))` ⇒ `Msg::Panicked(_)`; com uma tarefa abortada (`h.abort()` antes do `await`) ⇒ `Msg::DetailsLoaded { key: "k", result: Err(_) }`.

#### Checks beyond the steps

1. Navegação não espera: mutate inserindo `if self.details_pending.is_some() { return Vec::new(); }` no início de `move_to` ⇒ PEGA (teste 1).
2. Troca de aba pede detalhes: mutate removendo o `details_cmd()` de `set_tab` ⇒ PEGA (teste 3).
3. Resultado velho não encerra o spinner: mutate limpando `details_pending` em todo `DetailsLoaded` ⇒ PEGA (teste 5).
4. Sem rajada: mutate trocando `Skip` por `Burst` ⇒ PEGA (teste 6).
5. Teto do lote: mutate ignorando `cap` em `drain_ready` ⇒ PEGA (teste 7).
6. Último pedido vence: mutate mantendo o primeiro `LoadDetails` ⇒ PEGA (teste 8).
7. Tick ocioso não desenha: mutate fazendo `needs_draw` devolver `true` sempre ⇒ PEGA (teste 9).
8. Pânico não engolido: mutate trocando o ramo de pânico de `join_details` por `DetailsLoaded { result: Err(..) }` ⇒ PEGA (teste 10).
9. Medição (orquestrador, quando o usuário puder abrir a TUI): no commit dos contadores e no final, abrir a TUI, ficar 30 s parado na aba Atualizações e depois segurar ↓ 5 s; comparar a linha `tui stats` do log (`frames`, `ticks`, `max_draw_us`). Esperado (não medido): parado ⇒ `ticks` ≈ 0; segurando ⇒ `frames` bem menor que `keys`. Sem a TUI aberta, registrar "não medido".

**Commit:** `fix(tui): navigation never waits for details; one frame per batch`

---


## Desempenho do `update` (Tasks 54–56)

Origem: spans da Task 45 num `wingetrs update` real (~2 s): `update.search_idx` ~409 ms, `update.catalog_write{source=winget}` ~1,06 s, `update.parse` (winget) ~364 ms. Três investigações (2026-10-11, base `ae7ec20`; os arquivos de `src/` tocados aqui não mudaram até `ddce967`; linhas citadas conferidas nesse HEAD) mediram cada etapa e prototiparam a correção. **Todas as medidas foram feitas com a máquina carregada** (outros agentes compilando; i7-10750H, 12 CPUs lógicas, sem SHA-NI), então só os deltas intercalados contam; os valores absolutos estão inflados ~1,5×.

Regras comuns (repetidas em cada tarefa): **cada tarefa roda sozinha** (nenhum outro agente durante a tarefa e a medição, `scripts/leftovers.ps1` limpo antes); nenhum teste com limiar de relógio (`docs/ARMADILHAS.md`, "Teste com limiar de relógio"); os spans da Task 45 **não mudam** de nome, de lugar nem de pai (o A/B do orquestrador lê esses spans no log); ganho sem medição A/B ⇒ revert.

### Task 54: `search.idx` direto das linhas do SQLite, numa conexão

**Modelo:** sonnet (patch pronto num arquivo; o risco é de equivalência byte a byte, coberto pelo teste 1).

#### Where this fits

Independente das Tasks 55–56; roda primeiro e sozinha. `update.search_idx` (`src/update.rs:276`, `write_search_index` em `src/update.rs:284-289`) chama `index_file::write` (`src/catalog/index_file.rs:37-97`), que lê tudo com `Catalog::load_entries` (`src/catalog/mod.rs:394`: 8 conexões só-leitura, uma faixa de rowid por thread, um `PackageEntry` com `String`s por linha; `tags`/`commands` passam por `split_words` e voltam com `join(" ")`).

Medido (sob carga):
- Modo normal: ~100 ms (leitura 61, serializar 11, `fs::write` 2,5, `rename` 3,7, `drop` dos 19,6 mil `PackageEntry` 22).
- Modo travado (bimodal): só a leitura leva **430–1040 ms**, com os 8 workers parados juntos pelo mesmo tempo (ex.: 8 × 579–582 ms; 1ª linha em < 0,5 ms; sem busy handler; `mmap`, WAL, checkpoint e espera de 2 s não explicam). Os 409 ms do usuário estão nessa faixa. Só acontece dentro do processo do `update`; em processo separado e quieto o paralelo não travou.
- Uma conexão, codificando direto das linhas: 0 travamentos em 34 execuções (máx. 218 ms).
- A/B com binários intercalados (10 + 10, `update` real): mediana **219 → 64,5 ms**; delta pareado mediano **−128 ms**; pior caso **1040 → 190 ms**; `search.idx` **byte a byte idêntico** (3 031 711 bytes, mesmo SHA-256).

Outros chamadores de `index_file::write` (`src/buckets.rs:200`, `bucket add`/`rm`) ganham o mesmo caminho. `load_entries` continua existindo (TUI, `install::target`, testes) e **não muda**.

#### What binds this task

- `src/catalog/index_file.rs` `write(catalog, path) -> Result<usize, IndexFileError>`: mesma assinatura, mesmo retorno (`Ok(n)` = número de registros), mesma gravação atômica (`path.tmp` + `rename` com 5 tentativas a 20 ms; vira `fn persist(path, &[u8])`, comportamento inalterado). Não chama mais `load_entries`.
- Nova `pub fn encode_catalog(catalog: &Catalog) -> Result<(Vec<u8>, usize), IndexFileError>`: **um** `SELECT {ENTRY_COLS} FROM packages ORDER BY rowid` em `catalog.connection()` (`src/catalog/mod.rs:252`; `ENTRY_COLS` em `:199`), cada coluna lida por `get_ref(i)` emprestada da linha (sem `PackageEntry`, sem `String` por campo). **Nenhuma thread, nenhuma conexão nova.** Um statement = uma transação de leitura = um snapshot.
- Obrigatórias (`source`, `kind`, `id`, `name`, `version`, `tags`, `commands`) com `as_str()` (NULL é erro, como `r.get::<String>` hoje); opcionais (`moniker`, `publisher`, `description`) com `as_str_or_null()`. `kind` via `super::kind_from_db` (`src/catalog/mod.rs:194`, mesmo fallback para Winget). Erros de SQLite viram `IndexFileError::Catalog`.
- `tags` e `commands` gravados por `fn put_words`: palavras de `split_whitespace` unidas por **um** espaço — o mesmo texto que `split_words` + `join(" ")` dá hoje (espaço duplo, tab, espaço no começo/fim). O arquivo sai **byte a byte igual** ao do HEAD.
- Cabeçalho e trailer num `fn assemble(n, &offsets, &body) -> Result<Vec<u8>, IndexFileError>` (mesmos `MAGIC`, contagem `u32`, reservado `0u32`, `body_len u64`).
- **O codificador antigo (laço sobre `&[PackageEntry]`) sai de `src/`:** o único usuário era `write`. A equivalência fica provada contra uma cópia congelada dele **no teste** (`reference_bytes`, abaixo), que passa a ser o teste de formato do arquivo. Se você achar outro chamador do codificador antigo em `src/`, pare e relate (não mantenha dois caminhos sem decidir).
- Doc comment de `write` atualizado; `grep -n "load_entries" src/catalog/index_file.rs` ⇒ **0 linhas**.
- Não mexer em `load_entries`, `rowid_ranges`, `open_worker_connection`, `search_on_disk`, nem no paralelismo de `index_file::search` (`PARALLEL_MIN`): fora do `update` o paralelo não travou e é ~2× mais rápido.
- `docs/ARMADILHAS.md`, seção Desempenho, item novo (texto literal): "**Leitura paralela travando dentro do `update`.** Task 54: 8 conexões só-leitura no mesmo processo que acabou de parsear e gravar o catálogo travaram juntas por 0,4–1 s (sem busy handler, 1ª linha em < 0,5 ms; mmap, WAL e espera de 2 s não explicam); uma conexão só: 55–65 ms, nunca travou. Paralelismo que ganha no harness precisa ser medido no processo real."

Código de referência (protótipo medido; nomes podem mudar, comportamento não):

```rust
/// The column's whitespace-separated words joined by one space (= `split_words` + `join(" ")`).
fn put_words(out: &mut Vec<u8>, s: &str) {
    let at = out.len();
    out.extend_from_slice(&[0; 4]);
    for (i, w) in s.split_whitespace().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        out.extend_from_slice(w.as_bytes());
    }
    let n = (out.len() - at - 4) as u32;
    out[at..at + 4].copy_from_slice(&n.to_le_bytes());
}

fn sql(e: rusqlite::Error) -> IndexFileError {
    IndexFileError::Catalog(e.into())
}

/// File bytes for every package, borrowing each column from the row (no `PackageEntry`).
/// One statement on the catalog's own connection: one snapshot, no worker threads (parallel
/// readers stalled 0.4–1 s inside `update`, see docs/ARMADILHAS.md).
pub fn encode_catalog(catalog: &Catalog) -> Result<(Vec<u8>, usize), IndexFileError> {
    let mut st = catalog
        .connection()
        .prepare(&format!(
            "SELECT {} FROM packages ORDER BY rowid",
            super::ENTRY_COLS
        ))
        .map_err(sql)?;
    let mut rows = st.query([]).map_err(sql)?;
    let mut offsets: Vec<u8> = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    let mut n = 0usize;
    while let Some(r) = rows.next().map_err(sql)? {
        let start = u32::try_from(body.len())
            .map_err(|_| IndexFileError::Invalid("corpo grande demais"))?;
        offsets.extend_from_slice(&start.to_le_bytes());
        n += 1;
        let s = |i: usize| -> Result<&str, IndexFileError> {
            r.get_ref(i).map_err(sql)?.as_str().map_err(|e| sql(e.into()))
        };
        let o = |i: usize| -> Result<Option<&str>, IndexFileError> {
            r.get_ref(i).map_err(sql)?.as_str_or_null().map_err(|e| sql(e.into()))
        };
        body.push(match super::kind_from_db(s(1)?) {
            SourceKind::Winget => 0,
            SourceKind::Scoop => 1,
        });
        put_str(&mut body, Some(s(0)?));
        put_str(&mut body, Some(s(2)?));
        put_str(&mut body, Some(s(3)?));
        put_str(&mut body, o(4)?);
        put_str(&mut body, Some(s(5)?));
        put_str(&mut body, o(6)?);
        put_str(&mut body, o(7)?);
        put_words(&mut body, s(8)?);
        put_words(&mut body, s(9)?);
    }
    Ok((assemble(n, &offsets, &body)?, n))
}
```

(`kind_from_db` e `ENTRY_COLS` são privados de `src/catalog/mod.rs`; `index_file` é submódulo e enxerga `super::…`.)

**Files:**
- Modify: `src/catalog/index_file.rs`, `docs/ARMADILHAS.md`
- Test: `tests/index_file.rs`

Testes (literais não compilados por quem escreveu o plano — ajuste tipos/imports se o compilador pedir, **nunca** a asserção; rode clippy nos testes):

1. `streamed_bytes_equal_the_frozen_entry_encoder_including_odd_whitespace` (`tests/index_file.rs`):

```rust
/// Frozen copy of the pre-Task-54 encoder (entries -> file bytes): the file-format pin.
fn reference_bytes(entries: &[wingetrs::core::PackageEntry]) -> Vec<u8> {
    fn put(out: &mut Vec<u8>, s: Option<&str>) {
        match s {
            Some(s) => {
                out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            None => out.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()),
        }
    }
    let mut offsets = Vec::new();
    let mut body = Vec::new();
    for e in entries {
        offsets.extend_from_slice(&(body.len() as u32).to_le_bytes());
        body.push(match e.source {
            SourceKind::Winget => 0,
            SourceKind::Scoop => 1,
        });
        put(&mut body, Some(&e.source_name));
        put(&mut body, Some(&e.id));
        put(&mut body, Some(&e.name));
        put(&mut body, e.moniker.as_deref());
        put(&mut body, Some(&e.version));
        put(&mut body, e.publisher.as_deref());
        put(&mut body, e.description.as_deref());
        put(&mut body, Some(&e.tags.join(" ")));
        put(&mut body, Some(&e.commands.join(" ")));
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(&offsets);
    out.extend_from_slice(&body);
    out.extend_from_slice(MAGIC);
    out
}

#[test]
fn streamed_bytes_equal_the_frozen_entry_encoder_including_odd_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    let mk = |i: usize| NewPackage {
        id: format!("P.{i}"),
        name: format!("Name {i} ✓"),
        moniker: i.is_multiple_of(2).then(|| format!("m{i}")),
        version: format!("1.{i}"),
        publisher: i.is_multiple_of(3).then(|| "Pub".into()),
        description: i.is_multiple_of(5).then(String::new),
        // join(" ") of these stores doubled/leading/tab whitespace in the column;
        // load_entries splits it, so the stored text differs from the file text.
        tags: vec![" lead".into(), "a\tb".into(), String::new(), format!("t{i} ")],
        commands: if i.is_multiple_of(4) { vec![] } else { vec!["c  d".into()] },
        ..Default::default()
    };
    let w: Vec<NewPackage> = (0..3000).map(mk).collect();
    c.replace_source("winget", SourceKind::Winget, "v", 1, &w, &WingetMaps::default())
        .unwrap();
    let s: Vec<NewPackage> = (3000..3400).map(mk).collect();
    c.replace_source("scoop:main", SourceKind::Scoop, "e", 1, &s, &WingetMaps::default())
        .unwrap();
    // Re-import winget so rowids are not contiguous by source (winget lands after scoop).
    c.replace_source("winget", SourceKind::Winget, "v2", 2, &w, &WingetMaps::default())
        .unwrap();
    let (bytes, n) = index_file::encode_catalog(&c).unwrap();
    assert_eq!(n, 3400);
    // Not assert_eq!: a 3400-record diff would flood the log.
    assert!(bytes == reference_bytes(&c.load_entries().unwrap()), "streamed bytes differ");
}
```

2. `write_writes_the_reference_bytes` (`tests/index_file.rs`): com o `catalog(500)` existente, `index_file::write(&cat, &idx)` devolve `666` e `std::fs::read(&idx).unwrap() == reference_bytes(&cat.load_entries().unwrap())` (de novo `assert!(a == b)`), e `idx.with_extension("idx.tmp")` não existe.
3. `round_trip_every_entry` e `index_search_matches_in_memory_and_sqlite_exactly` (equivalência memória × `search.idx` × SQLite, resultado a resultado) continuam verdes **sem mudança**. Os demais testes de `tests/index_file.rs` também.

#### Checks beyond the steps

Mutações (`scripts/mutate.ps1 -Path src/catalog/index_file.rs -Anchor '<âncora>' -Replacement '<troca>' -TestCommand 'cargo test --quiet --test index_file streamed_bytes'`; nunca corte a saída do `mutate.ps1` com `Select-Object -First`: o pipe para antes do restore e a rodada seguinte dá falso "SOBREVIVEU"):
1. Tags normalizadas: `put_words(&mut body, s(8)?);` → `put_str(&mut body, Some(s(8)?));` ⇒ PEGA (teste 1).
2. Commands normalizados: o mesmo com `s(9)` ⇒ PEGA (teste 1).
3. Ordem dos campos: `put_str(&mut body, o(6)?);` → `put_str(&mut body, o(7)?);` ⇒ PEGA (teste 1).
4. NULL ≠ vazio: `put_str(&mut body, o(4)?);` → `put_str(&mut body, Some(o(4)?.unwrap_or_default()));` ⇒ PEGA (teste 1: `moniker` é NULL nos ímpares).
5. Ordem dos registros: `ORDER BY rowid` → `ORDER BY id` ⇒ PEGA (teste 1: winget reimportado fica depois do scoop; `"P.10" < "P.2"`).
6. Sem caminho antigo e sem thread: `grep -n "load_entries\|open_worker_connection\|rowid_ranges" src/catalog/index_file.rs` ⇒ nada; `encode_catalog`/`write`/`persist` sem `thread::` (o revisor confere no diff; o `thread::scope` de `search` já existia).
7. Gate: `pwsh -File scripts/verify.ps1` verde; erros só via `Select-String` em `target/verify.log`.

**Medição do orquestrador (A/B, decide se a tarefa fica).** Rode sozinho, depois do gate, com `scripts/leftovers.ps1` limpo. `$S` = scratchpad da sessão; `$BASE` = SHA registrado antes de despachar a tarefa.

a) Binários (base num worktree temporário; cópias fixas para builds posteriores não trocarem o arquivo):

```powershell
$S = '<scratchpad>'; $BASE = '<sha base>'
git worktree add --detach "$S\wt-base-54" $BASE
$env:CARGO_TARGET_DIR = "$S\tgt-base-54"; cargo build --release --quiet --manifest-path "$S\wt-base-54\Cargo.toml"; Remove-Item Env:CARGO_TARGET_DIR
Copy-Item "$S\tgt-base-54\release\wingetrs.exe" "$S\ab54-base.exe"
git worktree remove --force "$S\wt-base-54"
cargo build --release --quiet; Copy-Item target\release\wingetrs.exe "$S\ab54-new.exe"
```

b) Entrada fixa, sem rede: o **mesmo** `source2.msix` para as Tasks 54–56, servido localmente (`WINGETRS_WINGET_INDEX_URL`, `src/update.rs:24-29`); scoop aponta para porta morta (falha offline e mantém as linhas do scoop). Uma vez: `New-Item -ItemType Directory -Force "$S\ab-msix"; Copy-Item "$S\inv-read\dl\source2.msix" "$S\ab-msix\"` (3 766 341 bytes, baixado em 2026-10-11; se não existir, `curl.exe -L -o "$S\ab-msix\source2.msix" https://cdn.winget.microsoft.com/cache/source2.msix`).

c) Salve como `$S\ab-update.ps1` (reusado nas Tasks 55 e 56). Cada raiz (`LOCALAPPDATA` temporário) parte de uma **cópia** do `catalog.db` real (só leitura do real; nada é escrito em `%LOCALAPPDATA%\wingetrs`); cada execução apaga o `http.json` e o log da raiz e roda `update --force` (reimporta o winget ⇒ `catalog_write`, `parse` e `search_idx` acontecem); ordem base/new sorteada por rodada; 2 rodadas de aquecimento descartadas.

```powershell
param(
    [Parameter(Mandatory)] [string]$Tag,
    [Parameter(Mandatory)] [string]$Base,
    [Parameter(Mandatory)] [string]$New,
    # Regex of the span prefix in the file log, e.g. 'update\.search_idx'.
    [Parameter(Mandatory)] [string]$Span,
    [int]$Runs = 10,
    [int]$Port = 8765
)
$ErrorActionPreference = 'Stop'
$S = '<scratchpad>'
$real = Join-Path $env:LOCALAPPDATA 'wingetrs'
$roots = @{}
foreach ($k in 'base', 'new') {
    $r = "$S\ab$Tag-root-$k"
    if (Test-Path $r) { Remove-Item -Recurse -Force $r }
    $d = New-Item -ItemType Directory -Force "$r\wingetrs"
    foreach ($f in 'catalog.db', 'catalog.db-wal', 'catalog.db-shm', 'config.toml') {
        if (Test-Path "$real\$f") { Copy-Item "$real\$f" $d }
    }
    $roots[$k] = $r
}
function Busy([string]$root) {
    $log = Get-ChildItem "$root\wingetrs\log" -Filter 'wingetrs.log.*' | Sort-Object Name | Select-Object -Last 1
    $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
    if (-not $m) { throw "span ausente em $root" }
    $v = [double]$m.Matches[0].Groups[1].Value
    switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
}
$srv = Start-Process python -ArgumentList '-I', '-m', 'http.server', "$Port", '--bind', '127.0.0.1', '--directory', "$S\ab-msix" -PassThru -WindowStyle Hidden
$orig = $env:LOCALAPPDATA
$env:WINGETRS_WINGET_INDEX_URL = "http://127.0.0.1:$Port/source2.msix"
$env:WINGETRS_SCOOP_BASE_URL = 'http://127.0.0.1:9'
$exes = @{ base = $Base; new = $New }
$ms = @{ base = @(); new = @() }
try {
    Start-Sleep -Milliseconds 500
    for ($i = 0; $i -lt $Runs + 2; $i++) {
        $round = @{}
        foreach ($k in @('base', 'new') | Sort-Object { Get-Random }) {
            $env:LOCALAPPDATA = $roots[$k]
            Remove-Item -Force -ErrorAction SilentlyContinue "$($roots[$k])\wingetrs\http.json"
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$($roots[$k])\wingetrs\log"
            & $exes[$k] update --force *> $null
            $round[$k] = Busy $roots[$k]
        }
        if ($i -ge 2) { $ms.base += $round.base; $ms.new += $round.new }
    }
} finally {
    $env:LOCALAPPDATA = $orig
    Remove-Item Env:WINGETRS_WINGET_INDEX_URL, Env:WINGETRS_SCOOP_BASE_URL -ErrorAction SilentlyContinue
    Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue
}
function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
$d = for ($j = 0; $j -lt $ms.base.Count; $j++) { $ms.new[$j] - $ms.base[$j] }
foreach ($k in 'base', 'new') {
    $x = $ms[$k] | Measure-Object -Minimum -Maximum
    '{0}: n={1} mediana={2:N1} min={3:N1} max={4:N1} ms; >400 ms: {5}' -f $k, $ms[$k].Count, (Med $ms[$k]), $x.Minimum, $x.Maximum, @($ms[$k] | Where-Object { $_ -gt 400 }).Count
}
$x = $d | Measure-Object -Minimum -Maximum
'delta pareado (new - base): mediana={0:N1} min={1:N1} max={2:N1} ms' -f (Med $d), $x.Minimum, $x.Maximum
'brutos base: ' + ($ms.base -join ', ')
'brutos new:  ' + ($ms.new -join ', ')
```

d) Rodada desta tarefa: `pwsh -File "$S\ab-update.ps1" -Tag 54 -Base "$S\ab54-base.exe" -New "$S\ab54-new.exe" -Span 'update\.search_idx' -Runs 10`. Depois, mesmos bytes no real: `(Get-FileHash "$S\ab54-root-base\wingetrs\search.idx").Hash -eq (Get-FileHash "$S\ab54-root-new\wingetrs\search.idx").Hash` ⇒ `True` (as duas raizes tiveram a mesma história: mesma semente, mesmo msix, mesmo número de imports). Confirme com `Get-Process python -ErrorAction SilentlyContinue` que o servidor morreu; `leftovers.ps1` limpo.

e) **Critério (mediana E pior caso):** fica se (1) delta pareado mediano ≤ −30 ms, (2) nenhuma execução `new` > 400 ms (limiar do modo travado) e `max(new) ≤ max(base)`, (3) hashes do `search.idx` iguais. Qualquer item falhando ⇒ `git revert --no-edit <sha da tarefa>` e registro no ledger com os números colados. Observação: sem o scoop no processo o modo travado da base pode nem aparecer; nesse caso (2) e (3) ainda decidem e o ganho esperado cai para o modo normal (~100 → ~60 ms). Cole no ledger as 4 linhas de resumo e os brutos.

**Commit:** `perf(update): encode search.idx straight from catalog rows on one connection`

---

### Task 55: Gravação do catálogo — esvaziar de uma vez os mapas que são todos da fonte

**Modelo:** sonnet (mudança pequena num arquivo; a armadilha é a regra "linha alheia ou órfã ⇒ caminho antigo", coberta pelos testes 1 e 3).

#### Where this fits

Depois da Task 54 (independente no código; em série porque mede tempo). `update.catalog_write{source=winget}` (`src/update.rs:303`, corpo de `apply`) mede ~1,06 s; ~70 % vai para as tabelas `map_*` (146 059 linhas, 9,4 por pacote, 12,5 dos 16,1 MB do `catalog.db`). Com o catálogo **já populado** (caso normal do `update`), o maior item é apagar as linhas antigas dos mapas em `delete_source_rows` (`src/catalog/mod.rs:168-179`: `DELETE FROM {t} WHERE package IN (SELECT rowid FROM packages WHERE source = ?1)` em cada uma das 5 tabelas).

Medido (harness com o `source2.msix` real, modo "replace", sob carga, medianas da mesma bateria):
- Total da gravação: 1296 [925–3691] ms; `DELETE` nos 5 mapas: **420 [323–752] ms**.
- Com `DELETE FROM map_x` (caminho de truncate do SQLite) quando todas as linhas do mapa são da fonte: delete dos mapas **420 → 127 ms** (bateria ab3) e **514 → 146 ms** (ab2); total **−133 ms** (ab3) e **−359 ms** (ab2).
- `update` real com catálogo populado (máquina muito carregada): o `del` foi 635–1766 ms.
- Catálogo vazio (1º import): ganho 0 (nada a apagar).
- Equivalência no protótipo: digest SHA-256 de `packages` com rowid, dos 5 mapas como multiconjunto `(key, package)`, de `meta` e do schema iguais em todas as variantes; `integrity_check` ok.

Esta tarefa faz **só** isso. Ficam para uma tarefa separada, depois desta medida (não implementar aqui): (P2) derrubar os 5 índices `ix_*` dentro da transação e recriá-los no fim, só quando o import tem mapas — mediu −213 a −303 ms a mais, mas tem riscos próprios (DDL transacional precisa de teste de rollback; rebuild num import do scoop custaria 100–300 ms por bucket); (P3) `cache_size` de 64 MiB durante o import — −43 a −304 ms, ruidoso; (P4) mapas `WITHOUT ROWID` — exige migração de schema. `synchronous=NORMAL`, inserts ordenados e `ATTACH` + `INSERT … SELECT` foram medidos sem ganho ou piores.

#### What binds this task

- `delete_source_rows(tx, source)` (`src/catalog/mod.rs:168`): para cada tabela de `MAP_TABLES` (`:49`), se **nenhuma** linha tem `package` fora dos rowids da fonte — `SELECT EXISTS(SELECT 1 FROM {t} WHERE package NOT IN (SELECT rowid FROM packages WHERE source = ?1))` é falso — apaga com `DELETE FROM {t}` **sem `WHERE`**; senão mantém o `DELETE … WHERE package IN (…)` atual, literal.
- Linha **órfã** (`package` sem pacote) conta como alheia ⇒ caminho antigo ⇒ a órfã continua lá, exatamente como hoje. (Por isso a checagem é `NOT IN (rowids da fonte)`, não "existe pacote de outra fonte".)
- `delete_source_rows` passa a devolver `DeletedRows { packages: usize, truncated_maps: usize }` (struct privada do módulo; `truncated_maps` = tabelas esvaziadas pelo `DELETE` sem `WHERE`). `replace_source` (`:301`) ignora o valor; `remove_source` (`:292-299`) devolve `packages` como hoje. Nenhuma assinatura pública muda.
- Vale para `replace_source` e `remove_source` (mesma função). No `update` real só o winget tem mapas ⇒ o import do winget sempre pega o caminho rápido; o import de um bucket do scoop acha linhas do winget ⇒ caminho antigo (a checagem para na primeira linha alheia).
- Nada muda em `SCHEMA`, índices, `lookup`, `ImportStats`, `cache_size`, `synchronous` nem na ordem dos inserts.

**Files:**
- Modify: `src/catalog/mod.rs`
- Test: `#[cfg(test)] mod tests` novo no fim de `src/catalog/mod.rs` (precisa de `delete_source_rows` e `conn`, privados) e `tests/catalog.rs`

Testes (literais não compilados por quem escreveu o plano — ajuste tipos/imports se o compilador pedir, nunca a asserção):

1. `truncating_owned_map_tables_equals_the_per_row_delete` (unitário, `src/catalog/mod.rs`). Oráculo = o algoritmo antigo literal, em catálogos montados de forma idêntica; compara tudo que um leitor observa:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(id: &str) -> NewPackage {
        NewPackage {
            id: id.into(),
            name: id.into(),
            version: "1".into(),
            ..Default::default()
        }
    }

    fn maps(ids: &[&str]) -> WingetMaps {
        let pairs = |p: &str| -> Vec<(String, String)> {
            ids.iter().map(|id| (format!("{p}_{id}"), (*id).to_string())).collect()
        };
        WingetMaps {
            productcodes: pairs("PC"),
            upgradecodes: pairs("UC"),
            pfns: pairs("PFN"),
            norm_names: pairs("nn"),
            norm_publishers: pairs("np"),
        }
    }

    /// Everything a reader can observe: packages with rowid, map rows as a multiset, meta,
    /// and integrity.
    fn content(conn: &Connection) -> Vec<String> {
        let mut out = Vec::new();
        let mut q = |sql: &str| {
            let mut st = conn.prepare(sql).unwrap();
            let rows = st
                .query_map([], |r| {
                    (0..r.as_ref().column_count())
                        .map(|i| r.get_ref(i).map(|v| format!("{v:?}")))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap();
            for row in rows {
                out.push(row.unwrap().join("|"));
            }
        };
        q("SELECT rowid, * FROM packages ORDER BY rowid");
        for t in MAP_TABLES {
            q(&format!("SELECT '{t}', key, package FROM {t} ORDER BY key, package"));
        }
        q("SELECT * FROM meta ORDER BY source");
        q("PRAGMA integrity_check");
        out
    }

    /// Pre-change algorithm, literal: per-row delete in every map table.
    fn delete_per_row(tx: &rusqlite::Transaction<'_>, source: &str) -> usize {
        for t in MAP_TABLES {
            tx.execute(
                &format!(
                    "DELETE FROM {t} WHERE package IN (SELECT rowid FROM packages WHERE source = ?1)"
                ),
                params![source],
            )
            .unwrap();
        }
        tx.execute("DELETE FROM packages WHERE source = ?1", params![source])
            .unwrap()
    }

    fn winget_only(c: &mut Catalog) {
        c.replace_source(
            "winget",
            SourceKind::Winget,
            "v",
            1,
            &[pkg("W.1"), pkg("W.2"), pkg("W.3")],
            &maps(&["W.1", "W.2", "W.3"]),
        )
        .unwrap();
        c.replace_source(
            "scoop:main",
            SourceKind::Scoop,
            "e",
            1,
            &[pkg("S.1"), pkg("S.2")],
            &WingetMaps::default(),
        )
        .unwrap();
    }

    fn two_owners(c: &mut Catalog) {
        winget_only(c);
        c.replace_source("other", SourceKind::Winget, "o", 1, &[pkg("O.1")], &maps(&["O.1"]))
            .unwrap();
    }

    fn orphan_pfn(c: &mut Catalog) {
        winget_only(c);
        c.conn
            .execute("INSERT INTO map_pfn (key, package) VALUES ('orphan', 999999)", [])
            .unwrap();
    }

    fn check(setup: fn(&mut Catalog), source: &str, truncated: usize) {
        let mut a = Catalog::open_in_memory().unwrap();
        let mut b = Catalog::open_in_memory().unwrap();
        setup(&mut a);
        setup(&mut b);
        let tx = a.conn.transaction().unwrap();
        let got = delete_source_rows(&tx, source).unwrap();
        tx.commit().unwrap();
        let tx = b.conn.transaction().unwrap();
        let want = delete_per_row(&tx, source);
        tx.commit().unwrap();
        assert_eq!(got.packages, want, "{source}");
        assert_eq!(got.truncated_maps, truncated, "{source}");
        assert_eq!(content(&a.conn), content(&b.conn), "{source}");
    }

    #[test]
    fn truncating_owned_map_tables_equals_the_per_row_delete() {
        // Only winget has maps: every table is truncated.
        check(winget_only, "winget", 5);
        // A source without maps sees winget rows: nothing truncated.
        check(winget_only, "scoop:main", 0);
        // Two sources with maps: per-row path for both, the other source's rows stay.
        check(two_owners, "winget", 0);
        check(two_owners, "other", 0);
        // An orphan row counts as foreign: map_pfn keeps it, the other four are truncated.
        check(orphan_pfn, "winget", 4);
    }
}
```

2. `replace_keeps_map_rows_of_another_source` (`tests/catalog.rs`, usa o `pkg` existente do arquivo):

```rust
#[test]
fn replace_keeps_map_rows_of_another_source() {
    let pfn = |k: &str, id: &str| WingetMaps {
        pfns: vec![(k.into(), id.into())],
        ..Default::default()
    };
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source("a", SourceKind::Winget, "1", 1, &[pkg("A.A", "Alpha", "1")], &pfn("PFN_A", "A.A"))
        .unwrap();
    c.replace_source("b", SourceKind::Winget, "1", 1, &[pkg("B.B", "Beta", "1")], &pfn("PFN_B", "B.B"))
        .unwrap();
    c.replace_source("a", SourceKind::Winget, "2", 2, &[pkg("A.A", "Alpha", "2")], &pfn("PFN_A2", "A.A"))
        .unwrap();
    assert_eq!(c.lookup(MapTable::Pfn, "pfn_b").unwrap(), ["B.B"]);
    assert!(c.lookup(MapTable::Pfn, "pfn_a").unwrap().is_empty());
    assert_eq!(c.lookup(MapTable::Pfn, "pfn_a2").unwrap(), ["A.A"]);
}
```

3. `remove_source_keeps_other_sources_maps` (`tests/catalog.rs`): as fontes `a` e `b` do teste 2 (dois primeiros `replace_source`); `c.remove_source("a").unwrap() == 1`; `lookup(MapTable::Pfn, "pfn_b") == ["B.B"]`; `lookup(MapTable::Pfn, "pfn_a")` vazio.
4. Os testes atuais de `tests/catalog.rs` (inclusive `replace_source_only_touches_that_source`, `dangling_map_rolls_back_whole_import`, `code_lookup_folds_case_in_both_directions`, `reader_keeps_old_snapshot_while_writer_replaces`) e de `tests/update.rs` continuam verdes sem mudança.

#### Checks beyond the steps

Mutações (`scripts/mutate.ps1 -Path src/catalog/mod.rs -Anchor '<âncora>' -Replacement '<troca>' -TestCommand '<comando>'`; âncora = o texto exato que você escreveu — ajuste ao seu código):
1. Truncate só sem linha alheia: a condição que escolhe o `DELETE` sem `WHERE` vira sempre verdadeira (ex.: `if foreign {` → `if false {`) ⇒ PEGA (teste 1 `two_owners`/`orphan_pfn`; teste 2; teste 3). `-TestCommand 'cargo test --quiet --lib truncating_owned'`.
2. O caminho rápido é usado: a condição vira sempre falsa (sempre o `DELETE … IN`) ⇒ PEGA (teste 1, `truncated_maps == 5`).
3. Órfã é alheia: a checagem trocada por "existe pacote de **outra** fonte" (`SELECT EXISTS(SELECT 1 FROM {t} m JOIN packages p ON p.rowid = m.package WHERE p.source <> ?1)`) ⇒ PEGA (teste 1, `orphan_pfn`: a órfã seria apagada e `truncated_maps` daria 5).
4. Gate: `pwsh -File scripts/verify.ps1` verde.

**Medição do orquestrador (A/B, decide se a tarefa fica).** Sozinho, `leftovers.ps1` limpo, mesmo `$S\ab-msix\source2.msix` e mesmo `$S\ab-update.ps1` da Task 54 (passos a e c de lá, com `-Tag 55`, `wt-base-55`, `tgt-base-55`, `ab55-base.exe`, `ab55-new.exe`; base = HEAD antes desta tarefa):

`pwsh -File "$S\ab-update.ps1" -Tag 55 -Base "$S\ab55-base.exe" -New "$S\ab55-new.exe" -Span 'update\.catalog_write\{source=winget\}' -Runs 10`

Equivalência no catálogo real (as duas raizes tiveram a mesma história). Salve como `$S\catdigest.py` e rode `python -I "$S\catdigest.py" "$S\ab55-root-base\wingetrs\catalog.db" "$S\ab55-root-new\wingetrs\catalog.db"` ⇒ os dois digests iguais e `ok` nos dois (`fetched_at` fica de fora: é o relógio de cada execução):

```python
import hashlib, sqlite3, sys

MAPS = ("map_productcode", "map_upgradecode", "map_pfn", "map_norm_name", "map_norm_publisher")

def digest(path):
    c = sqlite3.connect(path)
    h = hashlib.sha256()
    def feed(sql):
        for row in c.execute(sql):
            h.update(repr(row).encode())
        h.update(b"|")
    feed("SELECT rowid, * FROM packages ORDER BY rowid")
    for t in MAPS:  # multiset: the map row's own rowid is not observable (lookup orders by p.rowid)
        feed(f"SELECT key, package FROM {t} ORDER BY key, package")
    feed("SELECT source, kind, source_version FROM meta ORDER BY source")
    ok = c.execute("PRAGMA integrity_check").fetchone()[0]
    c.close()
    return h.hexdigest(), ok

for p in sys.argv[1:]:
    print(p, *digest(p))
```

**Critério (mediana E pior caso):** fica se (1) delta pareado mediano ≤ −100 ms (o protótipo mediu −133 a −359 ms), (2) `max(new) ≤ max(base)` — se só (2) falhar, repita uma rodada de 10; falhou de novo ⇒ revert —, (3) digests iguais e `integrity_check` = `ok` nas duas raizes. Senão `git revert --no-edit <sha da tarefa>` e ledger com os números. Os spans `catalog_write` do scoop não aparecem nesse A/B (scoop offline): o custo novo para eles é um `EXISTS` que para na 1ª linha alheia — **não medido**, registre assim.

**Commit:** `perf(catalog): empty a map table at once when every row belongs to the replaced source`

---

### Task 56: Validação do msix do winget em pipeline (inflate + hashes)

**Modelo:** opus (concorrência com threads e precedência de erros; o oráculo serial é o que prova).

#### Where this fits

Depois da Task 55 (independente no código; em série porque mede tempo). `update.parse` do winget (`src/sources/winget/mod.rs:59`, em volta de `validate_msix` + gravação do `.db` temporário + `read_index`) mediu 364 ms no `update` real. `validate_msix` (`src/sources/winget/index.rs:61-103`) faz em série: inflate de `Public/index.db` (8,7 MB, 133 blocos de 64 KiB), SHA-256 de cada bloco contra o `AppxBlockMap.xml` e SHA-256 do arquivo inteiro (`FileHash`) — os mesmos 8,7 MB hasheados duas vezes, em software (sem SHA-NI).

Medido (harness, catálogo real, 9 rodadas, sob carga, ~1,5× acima do run de 364 ms): inflate **67 ms** (11 %), hash por bloco **85,5 ms** (14 %), `FileHash` **83,9 ms** (14 %); leitura do SQLite + `String` ~55 %. Pipeline só dos hashes (inflate numa thread, as duas somas em duas outras): delta pareado **−170 ms** (11 rodadas) e **−162 ms** (15 rodadas), resultado idêntico ao do código atual em todas as rodadas. O ganho ponta a ponta só foi medido para o protótipo completo (pipeline + leitura paralela: `update.parse` 591 → 317 ms, −252 ms pareado); **só o pipeline, ponta a ponta: não medido** — é o que a medição desta tarefa decide. Projeção (não medida) na carga do run citado: ~364 → ~250 ms.

Fora desta tarefa, de propósito: a leitura do `index.db` com conexões SQLite extras em paralelo (−191 a −205 ms no harness). **Não abra conexões novas**: dentro do `update`, conexões paralelas no mesmo processo travaram juntas por 430–1040 ms na investigação do `search.idx` (`docs/ARMADILHAS.md`, Desempenho: conexões paralelas competem no `prepare`; e o item da Task 54). Também fora: `spawn_blocking` do parse (não medido) e menos alocações no formato de import.

#### What binds this task

- `validate_msix(msix: &[u8]) -> Result<Vec<u8>, IndexError>`: mesma assinatura, **mesmos bytes** e **mesmo erro** que hoje para qualquer entrada, inclusive quando há mais de um defeito. Precedência atual (a ser preservada): (1) `ZipArchive::new` falha ⇒ `Zip`; (2) `Public/index.db` ausente ⇒ `MissingEntry("Public/index.db")`; (3) erro ao inflar `index.db` (inclusive CRC) ⇒ `Zip`; (4) `AppxBlockMap.xml` ausente ⇒ `MissingEntry("AppxBlockMap.xml")`; XML inválido ou sem `<File Name="Public\index.db">` ⇒ `BlockMap`; (5) contagem de blocos ≠ `db.len().div_ceil(65536)` ⇒ `BlockMap("esperados {n} blocos, encontrados {m}")`; (6) primeiro bloco cujo `base64(sha256)` difere ⇒ `Integrity { block }`; (7) `FileHash` presente e diferente ⇒ `FileHash`.
- Como o pipeline precisa dos hashes **antes** de inflar, o blockmap é lido primeiro; para manter (3) antes de (4): se a leitura do blockmap falhar, infle `index.db` inteiro com o `read_entry` atual (propagando o erro dele) e só então devolva o erro do blockmap. Custo zero no caminho feliz.
- Pipeline: `std::thread::scope`; a thread chamadora infla em blocos de 64 KiB (`read_block` preenche até 64 KiB ou fim) e envia cada bloco como `Arc<Vec<u8>>` por dois `std::sync::mpsc::sync_channel(32)`; uma thread devolve o índice do **primeiro** bloco ruim (blocos além de `hashes.len()` não são comparados), a outra devolve se o `FileHash` (quando existe) confere. Os dois `Sender` são soltos, **as duas threads são juntadas (pânico ⇒ `std::panic::resume_unwind`) antes de qualquer `?`**, e só então vêm, nesta ordem: erro de leitura do produtor (`Zip`), contagem, `Integrity`, `FileHash`. Receptor fechado (worker em pânico) ⇒ o produtor para de mandar e o `join` relança o pânico.
- Buffer inicial = `min(tamanho declarado no zip, 64 MiB)`: o tamanho declarado é dado externo e não pode virar uma alocação gigante.
- **Nenhuma conexão SQLite nova; `read_index` e `fetch_index` não mudam.** Sem crate nem feature nova; sem `unwrap`/`expect` fora de testes; nenhuma thread fora do `scope`. `FileHash` continua conferido (no pipeline ele não está no caminho crítico).
- Spans da Task 45 intactos (`update.parse` continua em volta das mesmas três etapas).

Código de referência (protótipo medido + a regra de precedência acima; nomes podem mudar, comportamento não):

```rust
/// Fills `buf` unless the reader ends first; returns the bytes read.
fn read_block<R: Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Block hashes and the optional whole-file hash of Public\index.db, from AppxBlockMap.xml.
fn read_blockmap<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Result<(Vec<String>, Option<String>), IndexError> {
    let map = read_entry(zip, BLOCKMAP_ENTRY)?;
    let xml = std::str::from_utf8(&map).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let doc = roxmltree::Document::parse(xml).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let file = doc
        .descendants()
        .find(|n| n.has_tag_name("File") && n.attribute("Name") == Some(INDEX_BLOCKMAP_NAME))
        .ok_or_else(|| IndexError::BlockMap("entrada Public\\index.db ausente".into()))?;
    let hashes = file
        .children()
        .filter(|n| n.has_tag_name("Block"))
        .filter_map(|n| n.attribute("Hash"))
        .map(str::to_owned)
        .collect();
    // b4:FileHash is a child element of <File> carrying the base64 sha256 in `Hash`.
    let file_hash = file
        .children()
        .find(|n| n.has_tag_name("FileHash"))
        .and_then(|n| n.attribute("Hash"))
        .map(str::to_owned);
    Ok((hashes, file_hash))
}

/// Cap for the up-front buffer: the declared size comes from the download.
const PREALLOC_MAX: usize = 64 << 20;

/// Returns the validated bytes of Public/index.db.
/// Inflate, per-block hashes and the whole-file hash run as a pipeline (this thread plus two
/// scoped workers); serially they cost ~235 ms under load on a CPU without SHA-NI.
pub fn validate_msix(msix: &[u8]) -> Result<Vec<u8>, IndexError> {
    use std::sync::{Arc, mpsc};
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(msix))
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    if zip.index_for_name(INDEX_ENTRY).is_none() {
        return Err(IndexError::MissingEntry(INDEX_ENTRY));
    }
    let (hashes, file_hash) = match read_blockmap(&mut zip) {
        Ok(m) => m,
        Err(e) => {
            // index.db used to be inflated first, so its read error wins.
            read_entry(&mut zip, INDEX_ENTRY)?;
            return Err(e);
        }
    };
    let mut entry = zip
        .by_name(INDEX_ENTRY)
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    let cap = usize::try_from(entry.size()).map_or(PREALLOC_MAX, |n| n.min(PREALLOC_MAX));
    let (hashes, file_hash) = (&hashes, file_hash.as_deref());
    std::thread::scope(|s| {
        let (block_tx, block_rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(32);
        let (file_tx, file_rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(32);
        let blocks = s.spawn(move || {
            let mut first_bad = None;
            for (i, block) in block_rx.into_iter().enumerate() {
                if first_bad.is_none()
                    && hashes
                        .get(i)
                        .is_some_and(|h| STANDARD.encode(Sha256::digest(&block[..])) != *h)
                {
                    first_bad = Some(i);
                }
            }
            first_bad
        });
        let whole = s.spawn(move || {
            let mut h = Sha256::new();
            for block in file_rx {
                h.update(&block[..]);
            }
            file_hash.is_none_or(|expected| STANDARD.encode(h.finalize()) == expected)
        });
        let mut db = Vec::with_capacity(cap);
        let produced = (|| -> Result<(), IndexError> {
            loop {
                let mut buf = vec![0u8; BLOCK_SIZE];
                let n = read_block(&mut entry, &mut buf)
                    .map_err(|e| IndexError::Zip(e.to_string()))?;
                if n == 0 {
                    return Ok(());
                }
                buf.truncate(n);
                db.extend_from_slice(&buf);
                let buf = Arc::new(buf);
                // A closed receiver means that worker panicked; the join below re-raises it.
                if block_tx.send(Arc::clone(&buf)).is_err() || file_tx.send(buf).is_err() {
                    return Ok(());
                }
            }
        })();
        // Closing both channels ends the workers' loops; join both before any `?`.
        drop(block_tx);
        drop(file_tx);
        let first_bad = blocks.join().unwrap_or_else(|p| std::panic::resume_unwind(p));
        let file_ok = whole.join().unwrap_or_else(|p| std::panic::resume_unwind(p));
        produced?;
        if hashes.len() != db.len().div_ceil(BLOCK_SIZE) {
            return Err(IndexError::BlockMap(format!(
                "esperados {} blocos, encontrados {}",
                db.len().div_ceil(BLOCK_SIZE),
                hashes.len()
            )));
        }
        if let Some(block) = first_bad {
            return Err(IndexError::Integrity { block });
        }
        if !file_ok {
            return Err(IndexError::FileHash);
        }
        Ok(db)
    })
}
```

**Files:**
- Modify: `src/sources/winget/index.rs`
- Test: `tests/winget_index.rs`

Testes (os existentes — `valid_msix_yields_exact_index_bytes`, `corrupt_block_is_rejected`, `missing_block_is_rejected`, `bad_file_hash_is_rejected`, `not_a_zip_is_an_error`, `reads_packages_and_all_maps`, `unsupported_major_version_is_refused` e os de `fetch_index` — continuam verdes sem mudança). Literais não compilados por quem escreveu o plano — ajuste tipos/imports (`zip::write::SimpleFileOptions`, `zip::CompressionMethod`) se o compilador pedir, nunca a asserção:

1. `pipelined_validate_msix_equals_serial_reference` (`tests/winget_index.rs`): oráculo = `validate_msix` do HEAD **copiado literalmente** para o teste (com o `read_entry` dele); cada entrada é passada aos dois e o resultado precisa ser igual (bytes, ou o `Debug` do erro: variante + mensagem/bloco). As entradas cobrem cada degrau da precedência, mais de um defeito ao mesmo tempo e mais blocos que a capacidade do canal (41 > 32):

```rust
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

/// Pre-Task-56 validate_msix, literal (serial: inflate, block hashes, file hash).
fn validate_msix_serial(msix: &[u8]) -> Result<Vec<u8>, IndexError> {
    fn read_entry<R: std::io::Read + std::io::Seek>(
        zip: &mut zip::ZipArchive<R>,
        name: &'static str,
    ) -> Result<Vec<u8>, IndexError> {
        let mut f = zip.by_name(name).map_err(|e| match e {
            zip::result::ZipError::FileNotFound => IndexError::MissingEntry(name),
            other => IndexError::Zip(other.to_string()),
        })?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).map_err(|e| IndexError::Zip(e.to_string()))?;
        Ok(buf)
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(msix))
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    let db = read_entry(&mut zip, "Public/index.db")?;
    let map = read_entry(&mut zip, "AppxBlockMap.xml")?;
    let xml = std::str::from_utf8(&map).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let doc = roxmltree::Document::parse(xml).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let file = doc
        .descendants()
        .find(|n| n.has_tag_name("File") && n.attribute("Name") == Some("Public\\index.db"))
        .ok_or_else(|| IndexError::BlockMap("entrada Public\\index.db ausente".into()))?;
    let hashes: Vec<&str> = file
        .children()
        .filter(|n| n.has_tag_name("Block"))
        .filter_map(|n| n.attribute("Hash"))
        .collect();
    if hashes.len() != db.len().div_ceil(65536) {
        return Err(IndexError::BlockMap(format!(
            "esperados {} blocos, encontrados {}",
            db.len().div_ceil(65536),
            hashes.len()
        )));
    }
    for (i, (chunk, expected)) in db.chunks(65536).zip(&hashes).enumerate() {
        if STANDARD.encode(Sha256::digest(chunk)) != *expected {
            return Err(IndexError::Integrity { block: i });
        }
    }
    let file_hash = file
        .children()
        .find(|n| n.has_tag_name("FileHash"))
        .and_then(|n| n.attribute("Hash"));
    if let Some(expected) = file_hash
        && STANDARD.encode(Sha256::digest(&db)) != expected
    {
        return Err(IndexError::FileHash);
    }
    Ok(db)
}

/// (name, bytes, stored) entries in order; stored = no compression (lets a test corrupt them).
fn zip_of(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, bytes, stored) in files {
            let o = zip::write::SimpleFileOptions::default();
            let o = if *stored { o.compression_method(zip::CompressionMethod::Stored) } else { o };
            z.start_file(*name, o).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn fixture_blockmap() -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(MSIX)).unwrap();
    let mut v = Vec::new();
    std::io::Read::read_to_end(&mut z.by_name("AppxBlockMap.xml").unwrap(), &mut v).unwrap();
    v
}

/// 41 blocks (> the channel capacity of 32) with a hand-written blockmap.
fn synthetic(bad_block: Option<usize>, bad_file_hash: bool) -> Vec<u8> {
    let data: Vec<u8> = (0..40 * 65536 + 123).map(|i: usize| (i % 251) as u8).collect();
    let mut xml = String::from(r#"<BlockMap><File Name="Public\index.db">"#);
    for c in data.chunks(65536) {
        xml.push_str(&format!(r#"<Block Hash="{}"/>"#, STANDARD.encode(Sha256::digest(c))));
    }
    let fh = if bad_file_hash { Sha256::digest(b"x") } else { Sha256::digest(&data) };
    xml.push_str(&format!(r#"<FileHash Hash="{}"/></File></BlockMap>"#, STANDARD.encode(fh)));
    let mut stored = data;
    if let Some(b) = bad_block {
        stored[b * 65536 + 7] ^= 0xFF;
    }
    zip_of(&[("Public/index.db", &stored, false), ("AppxBlockMap.xml", xml.as_bytes(), false)])
}

#[test]
fn pipelined_validate_msix_equals_serial_reference() {
    let bm = fixture_blockmap();
    let flip = |at: &[usize]| {
        let mut d = DB.to_vec();
        for &i in at {
            d[i] ^= 0xFF;
        }
        d
    };
    let last_block = flip(&[DB.len() - 1]);
    let two_bad = flip(&[65536 + 5, 3 * 65536 + 5]);
    let mut bad_and_long = flip(&[5]);
    bad_and_long.extend_from_slice(&[0u8; 65536]);
    // Stored index.db with one byte flipped inside the zip: the CRC check fails on read.
    let crc_broken = |map: &[u8]| {
        let mut z = zip_of(&[("Public/index.db", DB, true), ("AppxBlockMap.xml", map, false)]);
        let at = z.windows(16).position(|w| w == &DB[..16]).unwrap();
        z[at + 70_000] ^= 0xFF;
        z
    };
    let cases: Vec<Vec<u8>> = vec![
        MSIX.to_vec(),
        CORRUPT.to_vec(),
        MISSING_BLOCK.to_vec(),
        BAD_FILEHASH.to_vec(),
        b"hello".to_vec(),
        zip_of(&[("AppxManifest.xml", b"<x/>", false)]),
        zip_of(&[("Public/index.db", DB, false)]),
        zip_of(&[("Public/index.db", DB, false), ("AppxBlockMap.xml", b"<not xml", false)]),
        zip_of(&[("Public/index.db", DB, false), ("AppxBlockMap.xml", b"<BlockMap/>", false)]),
        zip_of(&[("Public/index.db", &last_block, false), ("AppxBlockMap.xml", &bm, false)]),
        zip_of(&[("Public/index.db", &two_bad, false), ("AppxBlockMap.xml", &bm, false)]),
        zip_of(&[("Public/index.db", &bad_and_long, false), ("AppxBlockMap.xml", &bm, false)]),
        crc_broken(&bm),
        crc_broken(b"<not xml"),
        synthetic(None, false),
        synthetic(Some(37), false),
        synthetic(None, true),
    ];
    let outcome = |r: Result<Vec<u8>, IndexError>| r.map_err(|e| format!("{e:?}"));
    let mut seen = Vec::new();
    for (i, msix) in cases.iter().enumerate() {
        let want = outcome(validate_msix_serial(msix));
        let got = outcome(validate_msix(msix));
        assert!(got == want, "case {i}: got {:?}, want {:?}", got.as_ref().err(), want.as_ref().err());
        seen.push(want.err().unwrap_or_else(|| "Ok".into()));
    }
    // Guard against degenerate inputs: every rung of the precedence is exercised.
    for kind in ["Ok", "Zip(", "MissingEntry(\"Public", "MissingEntry(\"Appx", "BlockMap(", "Integrity", "FileHash"] {
        assert!(seen.iter().any(|s| s.starts_with(kind)), "no case yields {kind}: {seen:?}");
    }
    // Two defects at once: the serial order decides.
    assert!(seen[11].starts_with("BlockMap("), "count before Integrity: {}", seen[11]);
    assert!(seen[12].starts_with("Zip("), "read error before Integrity: {}", seen[12]);
    assert!(seen[13].starts_with("Zip("), "read error before BlockMap: {}", seen[13]);
    assert_eq!(seen[10], "Integrity { block: 1 }");
    assert_eq!(seen[15], "Integrity { block: 37 }");
}
```

(Se o `Debug` de algum erro `Zip` diferir só no texto da mensagem entre os dois caminhos, as duas mensagens vêm do mesmo `io::Error` do `zip`; faça o caminho novo produzir o mesmo texto — não afrouxe a comparação.)

#### Checks beyond the steps

Mutações (`scripts/mutate.ps1 -Path src/sources/winget/index.rs -Anchor '<âncora>' -Replacement '<troca>' -TestCommand 'cargo test --quiet --test winget_index'`; âncoras = texto exato do seu código):
1. Hash por bloco: a thread de blocos devolve sempre `None` (`first_bad = Some(i);` → `first_bad = None;`) ⇒ PEGA (`corrupt_block_is_rejected`, teste 1).
2. `FileHash`: a thread do arquivo devolve sempre `true` ⇒ PEGA (`bad_file_hash_is_rejected`, teste 1 caso `synthetic(None, true)`).
3. `MissingEntry` do índice primeiro: remova o `if zip.index_for_name(INDEX_ENTRY).is_none() { … }` ⇒ PEGA (teste 1: o zip só com `AppxManifest.xml` daria `MissingEntry("AppxBlockMap.xml")`).
4. Erro de leitura antes do blockmap: no `Err(e)` de `read_blockmap`, tire o `read_entry(&mut zip, INDEX_ENTRY)?;` ⇒ PEGA (teste 1, caso `crc_broken(b"<not xml")`).
5. Erro de leitura antes de `Integrity`: mova o `produced?;` para depois do `if let Some(block) = first_bad` ⇒ PEGA (teste 1, caso `crc_broken(&bm)`).
6. Contagem antes de `Integrity`: troque a ordem dos dois `if` ⇒ PEGA (teste 1, caso `bad_and_long`).
7. **Não** rode a mutação "tirar `drop(block_tx)`": ela trava o teste para sempre (os workers nunca terminam) e o `mutate.ps1` não volta. O revisor confere por leitura que os dois `Sender` são soltos antes dos `join` e que os dois `join` vêm antes de qualquer `?`.
8. Regras: `pwsh -File scripts/check_rules.ps1` (join que engole pânico); `grep -n "open_with_flags\|Connection::open" src/sources/winget/index.rs` ⇒ só a linha que já existia em `read_index`.
9. Gate: `pwsh -File scripts/verify.ps1` verde.

**Medição do orquestrador (A/B, decide se a tarefa fica).** Sozinho, `leftovers.ps1` limpo, mesmo `$S\ab-msix\source2.msix` e mesmo `$S\ab-update.ps1` da Task 54 (passos a e c de lá, com `-Tag 56`, `wt-base-56`, `tgt-base-56`, `ab56-base.exe`, `ab56-new.exe`; base = HEAD antes desta tarefa):

`pwsh -File "$S\ab-update.ps1" -Tag 56 -Base "$S\ab56-base.exe" -New "$S\ab56-new.exe" -Span 'update\.fetch\{source=winget\}:update\.parse' -Runs 10`

Equivalência no real: `search.idx` com o mesmo hash nas duas raizes (comando do passo d da Task 54 com `ab56-root-*`) e `python -I "$S\catdigest.py"` (Task 55) nas duas `catalog.db` ⇒ digests iguais, `ok`.

**Critério (mediana E pior caso):** fica se (1) delta pareado mediano ≤ −80 ms (metade dos −162/−170 ms do harness), (2) `max(new) ≤ max(base)` — se só (2) falhar, repita uma rodada de 10; falhou de novo ⇒ revert —, (3) `search.idx` e digests iguais. Senão `git revert --no-edit <sha da tarefa>` e ledger com os números colados. Se o pipeline mostrar o mesmo tipo de travamento bimodal da Task 54 (execuções `new` muito acima da base sem padrão), registre em `docs/ARMADILHAS.md` mesmo revertendo.

**Commit:** `perf(winget): validate the index msix as an inflate and hash pipeline`


## TUI — redesenho no estilo Bubble Tea (Tasks 57–69)

Decisões do usuário (2026-10-11, recomendações aceitas): D1 = só a TUI (sem U13), D2 = três modos de confirmação com padrão "sempre", D3 = ajuste de resíduos com padrão "perguntar", D4 = paleta Charm com ajuda clareada para `#6C6C6C`, D5 = itens em duas linhas, D6 = barra de ajuda só com altura ≥ 20, D7 = busca fora da UI só se a medição pedir, D8 = emendas aprovadas (fase 1 §7.1/§7.2/§7.3, fase 2a §8/§10, já no texto das specs), D9 = toast de erro fixo até `Esc`, D10 = sem linha entre itens, D11 = toast de sucesso da atualização do catálogo só com Ctrl+R. Pedido extra do usuário: `←`/`→` trocam de aba (Task 57); por isso, na aba Ajustes (Task 68), o valor muda com `Enter`/Espaço/`l` (e `h` volta), não com as setas laterais.

Ordem e paralelismo: 57, 58, 61 e 67 tocam arquivos disjuntos e rodam em paralelo; 59 (mede frame) roda sozinha; 60 (condicional, mede busca) sozinha; 62 → 63 → 64 → 65 → 66 em série (todas em `src/tui/view.rs`); 68 depois de 66 e 67; 69 depois de 68. Números de linha citados nos textos copiados do rascunho podem ter deslocado com as Tasks 53–56: localize pelo símbolo citado junto.

### Task 57: `←` e `→` trocam de aba

#### Where this fits

Pedido do usuário (2026-10-11), emenda da spec da fase 1 §7.2. Hoje só `Tab`/`Shift+Tab` trocam de aba (`src/tui/app.rs`, `on_key`, ramos `KeyCode::Tab` e `KeyCode::BackTab`, cada um com seu `match` de abas); `KeyCode::Left`/`Right` não fazem nada. A Task 68 vai acrescentar a aba Ajustes ao ciclo: esta tarefa deixa o ciclo num lugar só para isso.

#### What binds this task

- `src/tui/app.rs`: duas funções públicas, `pub fn next_tab(tab: Tab) -> Tab` e `pub fn prev_tab(tab: Tab) -> Tab`, circulares, mesma ordem de hoje (Pesquisar → Instalados → Atualizações → Pesquisar). Os ramos `Tab` e `Right` chamam `app.set_tab(next_tab(app.tab))`; `BackTab` e `Left` chamam `app.set_tab(prev_tab(app.tab))`. Os dois `match` inline atuais deixam de existir.
- Vale com foco na lista **ou** nos detalhes, com a busca vazia ou não (o campo de busca não tem cursor; seta lateral nunca altera `query`).
- Diálogo aberto (`app.modal.is_some()`): as setas continuam indo para `on_modal_key` (a checagem do modal vem antes; não mude a ordem). Ajuda aberta (`show_help`): ignoradas, como as outras teclas hoje.
- Textos (português): rodapé `"Tab aba"` passa a `"←→ aba"`; no diálogo de ajuda, a linha `key("Tab", "próxima aba")` passa a `key("←→ / Tab", "trocar aba")`. Atualize os `.snap` afetados com `INSTA_UPDATE=always cargo test --quiet --test tui_view` e confira no diff que só o rodapé/ajuda mudaram.

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/view.rs`
- Test: `tests/tui_state.rs`, `.snap` de `tests/snapshots/tui_view__*.snap`

Testes (`tests/tui_state.rs`, helpers de tecla e de app que o arquivo já tem):
1. `right_and_left_arrows_cycle_tabs`: a partir de Pesquisar, `Right` ⇒ Instalados, `Right` ⇒ Atualizações, `Right` ⇒ Pesquisar; `Left` ⇒ Atualizações, `Left` ⇒ Instalados.
2. `arrows_match_tab_and_backtab`: para cada aba de partida (as três), o estado de aba depois de `Right` é igual ao depois de `Tab`, e depois de `Left` igual ao depois de `BackTab` (dois `App` iguais, um por tecla).
3. `arrows_switch_tabs_with_text_and_from_details`: digitar `po` ⇒ `Right` ⇒ aba Instalados e `query` sem caractere novo; noutro app, `Enter` (foco nos detalhes) ⇒ `Right` ⇒ aba Instalados.
4. `arrows_belong_to_an_open_dialog`: com um diálogo de confirmação aberto (mesmo jeito que os testes existentes abrem `Modal::Confirm`, p.ex. `Msg::Planned(Ok(Planned::Ready { .. }))`), `Right` e `Left` ⇒ aba inalterada e `modal` continua `Confirm`.

#### Checks beyond the steps

1. Direção: mutate fazendo o ramo `Right` chamar `prev_tab` ⇒ PEGA (teste 1).
2. Ciclo único: mutate trocando em `next_tab` o destino de Atualizações para Instalados ⇒ PEGA (testes 1 e 2).
3. Diálogo primeiro: mutate tratando `Left`/`Right` antes da checagem de `app.modal` ⇒ PEGA (teste 4).

**Commit:** `feat(tui): left and right arrows switch tabs`

---

### Task 58: Progresso da TUI somado entre downloads paralelos

#### Where this fits

Pendência cosmética registrada na Task 42 (`.claude/tasks/progress.md`, linha da Task 42: "ProgressThrottle da TUI único para todas as URLs (rótulo alterna)"). Com `fetch_each` baixando até `MAX_PARALLEL = 4` (`src/install/download.rs:46`), o callback é chamado por URL (`download.rs:129`); a TUI encaminha cada chamada admitida como `Msg::InstallProgress { label: url, done, total }` (`src/tui/run.rs:410-420`), e o modal guarda só a última (`src/tui/app.rs:383-386`): o rótulo e a barra pulam entre arquivos. Além disso o `ProgressThrottle` (`src/tui/jobs.rs:15-41`) deixa passar "o final" por URL (`total == Some(done)`), mas uma URL pode dominar os 20/s. A CLI já soma por URL (`LineProgress`, `src/cli/install_cmd.rs:93-126`). **Tarefa pequena**: só `src/tui/jobs.rs` e 3 linhas de `src/tui/run.rs`.

#### What binds this task

- `ProgressThrottle` passa a somar por rótulo: guarda `Mutex<BTreeMap<String, (u64, Option<u64>)>>` (lock com `unwrap_or_else(|p| p.into_inner())`, como `LineProgress`) e troca `admit` por
  `pub fn admit(&self, now: Instant, label: &str, done: u64, total: Option<u64>) -> Option<(String, u64, Option<u64>)>`
  que atualiza o mapa e, se admitido, devolve o agregado:
  - rótulo: o próprio `label` quando o mapa tem 1 entrada; senão `format!("{n} arquivos")`;
  - `done` = soma dos `done`; `total` = soma dos `total` só se **todos** forem `Some`, senão `None`;
  - admitido se é o primeiro, se passaram `PROGRESS_GAP` desde o último admitido, **ou** se o agregado está completo (`total == Some(done)`). O final de **uma** URL com outras em curso não fura o intervalo.
- `src/tui/run.rs` `spawn_execute`: a closure chama `throttle.admit(Instant::now(), label, done, total)` e envia o agregado. Nada mais em `run.rs`.
- `Msg::InstallProgress` e o modal não mudam.

**Files:**
- Modify: `src/tui/jobs.rs`, `src/tui/run.rs`
- Test: `tests/tui_jobs.rs`

Testes:
1. `progress_is_coalesced_to_about_twenty_per_second` (existente, l. 270): adapte para a nova assinatura com um rótulo só (`"u1"`); mesmas expectativas e rótulo devolvido `"u1"`.
2. `parallel_downloads_are_summed_not_alternated`: `t.admit(at(0), "u1", 10, Some(100))` ⇒ `Some(("u1", 10, Some(100)))`; `at(60), "u2", 5, Some(50)` ⇒ `Some(("2 arquivos", 15, Some(150)))`; `at(120), "u1", 40, Some(100)` ⇒ `Some(("2 arquivos", 45, Some(150)))` (o `done` de `u2` foi mantido).
3. `unknown_total_makes_the_sum_unknown`: `"u1", 10, Some(100)` em 0 ms e `"u2", 5, None` em 60 ms ⇒ segundo devolve `("2 arquivos", 15, None)`.
4. `one_finished_file_does_not_bypass_the_gap_while_others_run`: `"u1", 10, Some(100)` em 0 ms (admitido); `"u2", 50, Some(50)` em 10 ms ⇒ `None` (u2 terminou, o lote não); `"u1", 100, Some(100)` em 20 ms ⇒ `Some(("2 arquivos", 150, Some(150)))` (lote completo passa).

#### Checks beyond the steps

1. Soma: mutate devolvendo `(label, done, total)` da chamada em vez do agregado ⇒ PEGA (teste 2).
2. Total desconhecido: mutate somando só os `Some` ⇒ PEGA (teste 3).
3. Final do lote, não do arquivo: mutate usando `total == Some(done)` da chamada individual ⇒ PEGA (teste 4).
4. A TUI usa o agregado: sem teste automático do `run.rs` (exige TTY, como registrado na Task 39); o revisor confere o diff de 3 linhas — **no mutation proof** para essa ligação.

**Commit:** `fix(tui): sum download progress across parallel files`

---

### Task 59: Frame barato, medido

#### Where this fits

Depende de Task 53. Cada frame monta até 1000 linhas para mostrar ~30 (`src/tui/view.rs:238-296`, recorte só em `view.rs:341`); nas abas Instalados/Atualizações, `basics` procura a entrada do catálogo com busca linear nas ~19,5 mil entradas **a cada frame** (`view.rs:407-410`); `installed_view()` (`src/tui/app.rs:237-254`) refaz o filtro com `to_lowercase` por linha a cada chamada (várias por evento). Regra do projeto: medir antes, medir delta; otimização sem ganho medido é revertida.

#### What binds this task

- **Primeiro commit: o benchmark**, `examples/tui_frame.rs` (novo): abre o catálogo real (`DataDir::from_env()`, `Catalog::open(&data.catalog_path())?.load_entries()?`), monta `App::new(entries, GlyphSet::Nerd)` e 400 `ListRow` sintéticos casados (`Correlation::Matched`) com as 400 primeiras entradas do catálogo, um a cada quatro com `UpdateState::Available`. Para cada cenário — Pesquisar sem busca, Pesquisar com busca `code`, Instalados, Atualizações — renderiza 200 frames em `TestBackend::new(120, 40)` com `Theme::new(true, ColorDepth::TrueColor)`, descendo a seleção um item por frame, e imprime mediana e p95 em µs por cenário (o exemplo pode usar `println!`; a regra só vale para `src/`). Rode `cargo run --release --example tui_frame` 3 vezes e cole os números como **antes**.
- **Janela visível**: função pública nova em `src/tui/view.rs`, `pub fn list_window(selected: usize, scroll: usize, len: usize, height: usize) -> std::ops::Range<usize>`, com exatamente a lógica de `view.rs:331-340` (início = `selected` se `selected < scroll`; `selected + 1 - height` se `selected >= scroll + height`; senão `scroll`; limitado a `len - 1`) e fim `min(início + height, len)`; `len == 0` ⇒ `0..0`. `list_lines` monta **só** essas linhas; o destaque da seleção usa o índice relativo.
- **Índice do catálogo**: `App` ganha `catalog_pos: HashMap<(String, String), usize>` (`(source_name, id)` ⇒ posição em `search.entries()`), refeito em `App::new` e em `Msg::CatalogLoaded`; `basics` usa o índice em vez de `iter().find`.
- **Filtro dos instalados em cache**: `App` ganha `view_rows: Vec<usize>` (índices em `installed`), refeito em todo lugar que muda `installed`, `query`, `filter` ou `tab` (`refresh_view`, `set_tab`, `Msg::InstalledLoaded`, Ctrl+S, digitação, Backspace, Esc). `pub fn installed_view(&self) -> Vec<&ListRow>` mantém a assinatura (testes usam) e passa a mapear `view_rows`.
- Cada uma das três mudanças em commit próprio, com o benchmark rodado depois de cada uma; mudança sem ganho na sua aba é revertida (registre no relatório).
- Aparência inalterada: nenhum `.snap` muda.

**Files:**
- Create (novo): `examples/tui_frame.rs`
- Modify: `src/tui/view.rs`, `src/tui/app.rs`
- Test: `tests/tui_view.rs`, `tests/tui_state.rs`

Testes:
1. `list_window_keeps_selection_visible` (`tests/tui_view.rs`): `(0, 0, 1500, 10)` ⇒ `0..10`; `(700, 0, 1500, 10)` ⇒ `691..701`; `(1499, 1490, 1500, 10)` ⇒ `1490..1500`; `(5, 3, 1500, 10)` ⇒ `3..13`; `(0, 0, 3, 10)` ⇒ `0..3`; `(0, 0, 0, 10)` ⇒ `0..0`.
2. `long_list_end_is_rendered` (`tests/tui_view.rs`): 1500 entradas winget `pkg0000`…`pkg1499` (nome e id iguais), `App::new(.., GlyphSet::Ascii)`, tecla `End`, `render(&a, 100, 20)` ⇒ a linha que começa com `"│> "` contém `pkg1499`; o buffer não contém `pkg0000`.
3. `installed_details_show_the_catalog_publisher` (`tests/tui_view.rs`): `App::new(vec![], GlyphSet::Ascii)` e depois `Msg::CatalogLoaded` com uma entrada winget `Acme.Tool` de `publisher: Some("Acme Corp".into())`; `installed` com uma linha `Matched { source: "winget", package_id: "Acme.Tool", .. }`; aba Instalados; o render contém `"Acme Corp"`.
4. `cached_installed_view_matches_a_fresh_filter` (`tests/tui_state.rs`): 6 linhas variadas (winget/scoop, com e sem atualização, nomes `Alpha`, `beta`, `Gamma`, `alfa`, `Zed`, `zebra`); para cada combinação de aba (Instalados, Atualizações) × filtro (3 toques de Ctrl+S) × busca (`""`, `"a"`, `"ze"`), chegando ao estado **só por teclas**, os nomes de `a.installed_view()` são iguais ao filtro de referência escrito no teste (mesmo predicado de `app.rs:237-254`).

#### Checks beyond the steps

1. Janela: mutate trocando `selected + 1 - height` por `selected - height` ⇒ PEGA (teste 1).
2. Só linhas visíveis sem perder a seleção: mutate desenhando a janela a partir de `scroll` sem o ajuste da seleção ⇒ PEGA (teste 2).
3. Índice refeito na carga: mutate removendo a reconstrução de `catalog_pos` em `CatalogLoaded` ⇒ PEGA (teste 3).
4. Cache invalidado: mutate removendo a reconstrução de `view_rows` no ramo Ctrl+S ⇒ PEGA (teste 4).
5. Ganho medido: relatório com a tabela antes/depois do `tui_frame` por cenário (mediana e p95). Sem número colado, a otimização não conta.

**Commit:** `perf(tui): build only visible rows and index catalog lookups`

---

### Task 60: Busca fora da thread de UI (condicional à medição)

#### Where this fits

Depende de Task 59. Hoje cada tecla de texto roda `SearchIndex::search` inteira na thread de UI (`src/tui/app.rs:284-300`, `research`/`refresh_view`). **Esta tarefa só é implementada se a medição justificar.**

#### What binds this task

- **Passo 0, medir**: estenda `examples/tui_frame.rs` (criado na Task 59) com o cenário "busca": `app.search.search(q, None, 1000)` para `q` em `c`, `co`, `cod`, `code`, `vscode`, 50 repetições cada, mediana e p95 em µs. Cole os números. **Se o p95 de todas as consultas for ≤ 8 000 µs, pare**: relatório `DONE` com "não necessário" e os números; nenhum código além do cenário do benchmark.
- Se passar de 8 ms:
  - `App.search` vira `std::sync::Arc<SearchIndex>`; `Cmd::Search { generation: u64, query: String, kind: Option<SourceKind> }` emitido por `refresh_view` na aba Pesquisar no lugar da busca síncrona; `Msg::SearchDone { generation, hits }`. `App` guarda `search_generation`; resultado de geração diferente da atual é descartado. Até chegar, a lista mostra os hits anteriores; ao chegar, `selected = 0`, `scroll = 0` e `details_cmd()`.
  - `src/tui/run.rs` executa a busca em `spawn_blocking` com o handle aguardado (regra do CLAUDE.md; pânico ⇒ `Msg::Panicked`), pela função pública única `pub fn run_search(index: &SearchIndex, query: &str, kind: Option<SourceKind>) -> Vec<SearchHit>` (= `index.search(query, kind, MAX_HITS)`), que também é a usada em `App::new`/`CatalogLoaded`.
  - Regra do projeto: dois caminhos para o mesmo resultado ⇒ teste de equivalência.

**Files:**
- Modify: `examples/tui_frame.rs`, `src/tui/app.rs`, `src/tui/run.rs`
- Test: `tests/tui_state.rs`, `tests/tui_loop.rs`

Testes (só se implementar):
1. `async_search_equals_sync_search` (`tests/tui_state.rs`): com as entradas de `app()`, para cada consulta em `["", "p", "po", "zeal", "xyz"]` e filtro `None`/`Some(Winget)`: digitar até a consulta, executar o `Cmd::Search` emitido com `run_search` e entregar `Msg::SearchDone` ⇒ `a.hits` igual a `a.search.search(q, kind, 1000)` (índice e `name_indices`).
2. `stale_search_results_are_dropped` (`tests/tui_state.rs`): digitar `p` (geração 1) e `o` (geração 2); entregar `SearchDone` da 2 e depois da 1 ⇒ `a.hits` é o da 2.
3. `typing_does_not_search_on_the_ui_thread` (`tests/tui_state.rs`): digitar `p` ⇒ comandos contêm `Cmd::Search { query: "p", .. }` e `a.hits` ainda é o da consulta anterior.

#### Checks beyond the steps

1. Equivalência: mutate fazendo `run_search` usar limite 999 ⇒ PEGA (teste 1).
2. Geração: mutate aplicando qualquer `SearchDone` ⇒ PEGA (teste 2).
3. Fora da UI: mutate voltando a chamar `research()` síncrono em `refresh_view` ⇒ PEGA (teste 3).
4. Se parar no passo 0: "no mutation proof (tarefa encerrada pela medição)", com os números colados.

**Commit:** `perf(tui): search off the UI thread with generations`

---

### Task 61: Paleta Bubble Tea nos tokens

#### Where this fits

Independente (paralelo a Task 59/Task 67). Troca os valores dos tokens de `src/tui/theme.rs` pela paleta do Charm (Lip Gloss/Bubbles) e acrescenta os tokens que Task 63–Task 68 vão usar. Regra do projeto: cores só via tokens de `src/tui/theme.rs` (o gate barra `Color::` fora dele).

#### What binds this task

- Valores (truecolor; índice 256 entre parênteses), escuro / claro:
  - `accent`: `#7D56F4` (99) / `#874BFD` (99). `border_focus` = `accent`.
  - `sel_bar` e `sel_desc` (novos): `#AD58B4` (133) / `#F793FF` (219).
  - `sel_title` (novo): `#EE6FF8` (213) nos dois.
  - `fg`: `#DDDDDD` (253) / `#1A1A1A` (234). `fg_dim`: `#777777` (243) / `#A49FA5` (247).
  - `border`: `#383838` (237) / `#D9DCCF` (253).
  - `ok`: `#73F59F` (85) / `#43BF6D` (71).
  - `help_key`, `help_desc`, `help_sep` (novos): `#626262` (241), `#6C6C6C` (242) (D4: clareado do `#4A4A4A` original), `#3C3C3C` (237) / `#909090` (246), `#B2B2B2` (249), `#DADADA` (253).
  - `badge` (novo, `Style` com fg e bg): fg índice 230 / `#FFFFD7`, bg índice 62 / `#5F5FD7`, `BOLD`, nos dois temas.
  - `pill_msg` (novo): fg `#FFFDF5` (230), bg `#FF5F87` (204). `pill_a`: fg `#FFFDF5` (230), bg `#A550DF` (134). `pill_b`: fg `#FFFDF5` (230), bg `#6124DF` (56). Iguais nos dois temas.
  - `spinner` (novo): `#747373` (243) / `#8E8E8E` (245).
  - `warn`, `muted`, `alert`, `err`, `src_winget`, `src_scoop`, `bg_selected`: valores atuais.
- `NoColor`: nenhum token tem cor; modificadores mantidos; `badge`, `pill_msg`, `pill_a`, `pill_b` ganham `Modifier::REVERSED` (para continuarem visíveis sem cor).
- Spinner dos glifos `Nerd` e `Unicode` passa ao `Dot` do Bubbles: `["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"]`; `Ascii` inalterado.
- Glifos novos de aba em `Glyphs` (usados na Task 63/Task 68): `tab_search`, `tab_installed`, `tab_updates`, `tab_settings` = Nerd `"\u{f002}"`, `"\u{f0a0}"`, `"\u{f06b0}"`, `"\u{f013}"`; Unicode `"⌕"`, `"▤"`, `"↑"`, `"⚙"`; Ascii `""` (sem glifo). E `bar`: Nerd/Unicode `"┃"`, Ascii `"|"`.
- Nenhuma view muda nesta tarefa além do que a troca de valores implica; os `.snap` (NoColor + Ascii) não mudam.

**Files:**
- Modify: `src/tui/theme.rs`
- Test: `tests/tui_theme.rs` (novo arquivo de teste)

Testes:
1. `dark_truecolor_uses_the_charm_palette`: `Theme::new(true, ColorDepth::TrueColor)` ⇒ `accent().fg == Some(Color::Rgb(0x7D, 0x56, 0xF4))`, `sel_title().fg == Some(Color::Rgb(0xEE, 0x6F, 0xF8))`, `help_key().fg == Some(Color::Rgb(0x62, 0x62, 0x62))`, `badge().bg == Some(Color::Rgb(0x5F, 0x5F, 0xD7))`.
2. `light_truecolor_accent_is_darker_purple`: `Theme::new(false, TrueColor).accent().fg == Some(Color::Rgb(0x87, 0x4B, 0xFD))`.
3. `ansi256_uses_the_listed_indexes`: `Theme::new(true, ColorDepth::Ansi256)` ⇒ `accent` 99, `sel_title` 213, `pill_msg().bg` 204, `ok` 85.
4. `no_color_keeps_pills_visible`: `Theme::new(true, ColorDepth::NoColor)` ⇒ `pill_msg().bg == None` e `pill_msg().add_modifier` contém `REVERSED`; `badge()` contém `REVERSED` e `BOLD`.
5. `nerd_and_unicode_spin_with_dot_frames`: `Glyphs::for_set(GlyphSet::Nerd).spinner.len() == 8` e primeiro quadro `"⣾"`; Ascii continua `["|", "/", "-", "\\"]`.

#### Checks beyond the steps

1. Paleta: mutate trocando o valor escuro de `accent` por `(122, 162, 247)` ⇒ PEGA (teste 1).
2. Claro ≠ escuro: mutate usando o par escuro de `accent` no tema claro ⇒ PEGA (teste 2).
3. Sem cor visível: mutate removendo o `REVERSED` do `pill_msg` em `NoColor` ⇒ PEGA (teste 4).
4. `scripts/check_rules.ps1` continua verde (nenhum `Color::` fora de `src/tui/theme.rs`).

**Commit:** `feat(tui): Charm palette tokens and Dot spinner`

---

### Task 62: Escala de espaçamento num lugar só

#### Where this fits

Depende de Task 59 (as duas mexem em `src/tui/view.rs`) e de Task 61. O usuário pediu padding entre os widgets. Hoje o texto encosta na borda (snapshot `tests/snapshots/tui_view__search_screen_snapshot.snap`: `"│> W PowerToys"`) e os espaços são números soltos espalhados por `src/tui/view.rs` (`" "` e `"  "` em `list_lines`, `centered(area, 90, h)` etc.). Esta tarefa cria a escala (`tui-design.md` §2.8) e a aplica **às caixas que existem hoje**; Task 63–Task 66 a usam no layout novo.

#### What binds this task

- Módulo novo `src/tui/layout.rs` (registrar em `src/tui/mod.rs`), só com constantes `u16` documentadas e duas funções puras:
  - `PAD_X = 1` (entre borda e texto em toda caixa com borda), `FILTER_GAP = 1` (linhas entre a linha da busca e a lista), `GUTTER = 1` (colunas de cada lado do divisor lista/detalhes), `ITEM_GAP = 0` (linhas entre itens; D10), `DIALOG_PAD_X = 2`, `DIALOG_PAD_Y = 1`, `TOAST_PAD_X = 1`, `TOAST_MARGIN_X = 2`, `TOAST_MARGIN_Y = 1`, `TAB_PAD_X = 1`, `BAR_PAD_X = 1`.
  - `pub fn inset(area: Rect, x: u16, y: u16) -> Rect`: tira `x` colunas de cada lado e `y` linhas em cima e embaixo, com subtração saturada (área menor que o recuo ⇒ largura/altura 0, nunca pânico).
  - `pub fn split_with_gutter(area: Rect, left_percent: u16) -> (Rect, u16, Rect)`: divide na horizontal em `(esquerda, coluna_do_divisor, direita)` deixando `GUTTER` colunas em branco de cada lado do divisor; a esquerda tem `area.width * left_percent / 100` colunas (antes dos vãos); áreas estreitas demais saturam em 0.
- Aplicação nesta tarefa (layout atual, três caixas): conteúdo da caixa de busca, da lista e dos detalhes desenhado em `inset(blk.inner(area), PAD_X, 0)`; diálogos (Confirmar, seletor, resíduos, Em andamento, ajuda) em `inset(blk.inner(rect), DIALOG_PAD_X, DIALOG_PAD_Y)`, com a altura/largura do diálogo crescendo `2*DIALOG_PAD_Y` / `2*DIALOG_PAD_X` para caber o mesmo texto; barra de status começando em `BAR_PAD_X` (hoje é um `" "` literal no `format!`; troque pela constante).
- **Nenhum número de espaçamento solto** fora de `src/tui/layout.rs` nas funções de desenho alteradas (os `" "` entre glifo e nome dentro de um item continuam; são parte do item, não do layout).
- Os testes de Task 53 (`selected_row_moves_on_screen_while_details_load`) e Task 59 (`long_list_end_is_rendered`) procuram a linha que começa com `"│> "`; com o padding ela passa a começar com `"│ > "`. Ajuste o prefixo procurado, sem enfraquecer a asserção (continua: a linha do item selecionado contém o nome esperado).
- `CHROME_ROWS` não muda nesta tarefa (o padding é horizontal; nos diálogos não afeta a lista).
- Atualize os `.snap` com `INSTA_UPDATE=always cargo test --quiet --test tui_view` e cole o novo `search_screen_snapshot` no relatório.

**Files:**
- Create (novo): `src/tui/layout.rs`
- Modify: `src/tui/mod.rs`, `src/tui/view.rs`
- Test: `tests/tui_view.rs` e os `.snap` de `tests/snapshots/tui_view__*.snap`

Testes (`render` do arquivo, `GlyphSet::Ascii`, `ColorDepth::NoColor`; "coluna x da linha y" = símbolo da célula):
1. `inset_saturates_on_tiny_rects`: `inset(Rect::new(0, 0, 1, 1), 1, 1)` tem largura e altura 0; `inset(Rect::new(2, 3, 10, 5), 1, 1) == Rect::new(3, 4, 8, 3)`.
2. `split_with_gutter_leaves_blank_columns`: `split_with_gutter(Rect::new(0, 0, 100, 10), 55)` ⇒ esquerda `Rect::new(0, 0, 55, 10)`, divisor na coluna 56, direita começa na coluna 58 e termina na 99 (largura 42).
3. `list_and_details_text_start_one_column_after_the_border`: `typed("powert")`, 100×16 ⇒ nas linhas 4 a 6, a coluna 1 (logo depois da borda da lista) é `" "` e a coluna 2 é `">"` ou `" "` seguida do glifo da fonte na coluna 4; no painel de detalhes (borda esquerda na coluna 55), a coluna 56 da linha 4 é `" "` e a 57 é o `P` de `PowerToys`.
4. `confirm_dialog_has_inner_padding`: `with_confirm()` (helper existente), 100×24 ⇒ dentro do diálogo, a primeira linha interna é toda espaço e as duas primeiras colunas internas das linhas de texto são espaço.
5. `layout_constants_are_the_documented_scale`: as constantes têm os valores listados acima (trava contra mudança acidental; a escala é decisão do usuário).

#### Checks beyond the steps

1. Padding da lista: mutate trocando `PAD_X` por `0` em `src/tui/layout.rs` ⇒ PEGA (teste 3, e o 5).
2. Padding do diálogo: mutate desenhando o diálogo em `blk.inner(rect)` sem `inset` ⇒ PEGA (teste 4).
3. Vão do divisor: mutate removendo o `GUTTER` da direita em `split_with_gutter` ⇒ PEGA (teste 2).
4. Saturação: mutate trocando `saturating_sub` por `-` em `inset` ⇒ PEGA (teste 1, com pânico de overflow em debug).

**Commit:** `feat(tui): one spacing scale for every box and dialog`

---

### Task 63: Abas encaixadas no topo

#### Where this fits

Depende de Task 62 (escala de espaçamento; Task 62 já depende de Task 59 e Task 61). Substitui as abas-texto do título da caixa de busca (`src/tui/view.rs:81-107`, `148-166`) pelas abas do exemplo `tabs` do Bubble Tea: caixas arredondadas, a ativa aberta para a janela de conteúdo. Layout de referência: `tui-design.md` §2.1–2.3.

#### What binds this task

- Módulo novo `src/tui/tabs.rs` (registrar em `src/tui/mod.rs`): `pub fn draw_tabs(frame: &mut Frame, area: Rect, labels: &[String], active: usize, theme: &Theme)`, desenhando 3 linhas:
  - Cada aba tem interior `" " + rótulo + " "` (largura pelo `Span::width`, que conta unicode).
  - Linha 0: por aba `╭` + `─`×interior + `╮`; o resto, espaços.
  - Linha 1: por aba `│` + interior + `│`; o resto, espaços, com o badge ` wingetrs ` (token `badge`) alinhado à direita e um espaço de margem, só se couber.
  - Linha 2: aba ativa `┘` + espaços×interior + `└`; inativa `┴` + `─`×interior + `┴`; **exceção da primeira aba**: canto esquerdo `│` se ativa, `├` se inativa; depois da última aba, `─` até a penúltima coluna e `╮` na última.
  - Bordas no token `accent`; rótulo ativo `accent` + `BOLD`; inativos `fg_dim`; aba Atualizações com contagem > 0 e inativa usa `warn` (como hoje).
  - **Estreito**: se a soma das larguras das abas + 1 > `area.width`, modo compacto: linha 0 vazia; linha 1 rótulos separados por `" · "` (ativo `accent` + `BOLD`); linha 2 `╭` + `─`×(largura−2) + `╮`. Largura < 2: não desenha nada (sem pânico).
- Rótulos: `format!("{glyph} {nome}")`, ou só o nome quando o glifo é `""` (Ascii): `Pesquisar`, `Instalados`, `Atualizações` / `Atualizações N`.
- Tela (`draw_screen`): linhas = abas (3) + janela (resto) + status (1). A janela é um `Block` com `Borders::LEFT | RIGHT | BOTTOM`, arredondado, borda `accent`; dentro, tudo em `inset(janela_interna, PAD_X, 0)` (`src/tui/layout.rs`): linha da busca (prompt + consulta + à direita o `corner_line` atual), `FILTER_GAP` linha(s) em branco, e lista | divisor `│` (token `border`) | detalhes via `split_with_gutter(.., 55)` (`GUTTER` em branco de cada lado do divisor). Rótulo de aba com `TAB_PAD_X` de cada lado (é o `" " + rótulo + " "` abaixo). A caixa de busca separada e as duas caixas da lista/detalhes deixam de existir. Abaixo de 60 colunas os detalhes somem (como hoje); abaixo de 8 linhas só a lista (como hoje).
- `CHROME_ROWS` (`src/tui/app.rs:206`) passa a `3 + 1 + FILTER_GAP + 1 + 1` = 7 (abas + busca + vão + borda de baixo + status), escrito com as constantes.
- Atualize os `.snap` de `tests/snapshots/` com `INSTA_UPDATE=always cargo test --quiet --test tui_view` e cole o novo `tui_view__search_screen_snapshot.snap` no relatório. Ajuste o teste `selected_row_moves_on_screen_while_details_load` (Task 53) e `long_list_end_is_rendered` (Task 59) se o prefixo da linha selecionada mudar (a janela agora começa em `│` + espaço?); a asserção continua sendo "a linha do item selecionado contém o nome".

**Files:**
- Create (novo): `src/tui/tabs.rs`
- Modify: `src/tui/mod.rs`, `src/tui/view.rs`, `src/tui/app.rs` (usa `src/tui/layout.rs` da Task 62)
- Test: `tests/tui_view.rs` e os `.snap` de `tests/snapshots/tui_view__*.snap`

Testes (rótulos `["Pesquisar", "Instalados", "Atualizações 2"]`, `TestBackend::new(60, 3)`, `Theme::new(true, ColorDepth::NoColor)` salvo o teste 4; "linha" = conteúdo da linha do buffer):
1. `active_first_tab_opens_into_the_window`: ativa 0 ⇒ linha 0 `format!("╭{}╮╭{}╮╭{}╮{}", "─".repeat(11), "─".repeat(12), "─".repeat(16), " ".repeat(15))`; linha 1 `format!("│ Pesquisar ││ Instalados ││ Atualizações 2 │{} wingetrs  ", " ".repeat(4))`; linha 2 `format!("│{}└┴{}┴┴{}┴{}╮", " ".repeat(11), "─".repeat(12), "─".repeat(16), "─".repeat(14))`.
2. `active_middle_tab_tees_the_first_corner`: ativa 1 ⇒ linha 2 `format!("├{}┴┘{}└┴{}┴{}╮", "─".repeat(11), " ".repeat(12), "─".repeat(16), "─".repeat(14))`.
3. `tabs_fall_back_to_one_line_when_narrow`: largura 30 ⇒ linha 0 só espaços; linha 1 contém `Pesquisar · Instalados`; linha 2 `format!("╭{}╮", "─".repeat(28))`.
4. `active_label_is_bold_accent_and_others_dim`: `Theme::new(true, ColorDepth::TrueColor)`, ativa 0 ⇒ célula `(2, 1)` (`P`) com `fg == theme.accent().fg` e `BOLD`; célula `(15, 1)` (`I`) com `fg == theme.fg_dim().fg`.
5. `tabs_never_panic_on_tiny_widths`: larguras 0..=12, alturas 0..=3.
6. `search_screen_snapshot` (existente) atualizado: as três primeiras linhas do snapshot são as abas e a busca fica dentro da janela.
7. `window_spacing_matches_the_scale` (tela inteira, `typed("powert")`, Ascii, NoColor, 100×16): coluna 1 em branco nas linhas 3 a 13 (`PAD_X`; a 14 é a borda de baixo); a linha 4 (entre a busca e a lista) é só borda + espaços + borda (`FILTER_GAP`); com o divisor na coluna `d` (procure o `│` interno da linha 5), as colunas `d-1` e `d+1` são espaço em todas as linhas da lista (`GUTTER`).

#### Checks beyond the steps

1. Fundo aberto da ativa: mutate trocando `┘`/`└` por `┴`/`┴` na ativa ⇒ PEGA (teste 1).
2. Canto da primeira aba: mutate usando sempre `│` na primeira aba ⇒ PEGA (teste 2).
3. Modo compacto: mutate trocando o `+ 1` do limiar por `+ 100` ⇒ PEGA (teste 3).
4. Estilo: mutate usando `fg_dim` no rótulo ativo ⇒ PEGA (teste 4).
5. Vão entre busca e lista: mutate desenhando a lista logo abaixo da busca (sem `FILTER_GAP`) ⇒ PEGA (teste 7).
6. Gutter: mutate trocando `split_with_gutter` por um `Layout` 55/45 sem vãos ⇒ PEGA (teste 7).

**Commit:** `feat(tui): lipgloss-style tabs joined to the content window`

---

### Task 64: Itens de lista em duas linhas com barra de seleção

#### Where this fits

Depende de Task 63. Itens no estilo do `list.DefaultDelegate` do Bubbles: título + linha de descrição; selecionado com barra vertical à esquerda e cor rosa (D5 = duas linhas).

#### What binds this task

- Constante pública `pub const ITEM_ROWS: usize = 2;` em `src/tui/app.rs`. `clamp` calcula a página em **itens**: `(altura - CHROME_ROWS) / (ITEM_ROWS + ITEM_GAP)` (`ITEM_GAP` de `src/tui/layout.rs`, 0 por D10), mínimo 1; entre dois itens desenhe `ITEM_GAP` linhas em branco; `PageUp/PageDown` continuam 10 itens. `list_window` (Task 59) recebe a altura em itens.
- Linha 1 do item: barra (selecionado: `glyphs.bar` + espaço no token `sel_bar`; senão dois espaços) + glifo da fonte + espaço + nome (letras casadas sublinhadas, `matched`) + à direita a versão como hoje (Pesquisar: versão `fg_dim`; Instalados: versão + glifo de estado; Atualizações: `antiga → nova`, antiga `dim`, nova `ok` + `BOLD`). Nome: selecionado `sel_title` + `BOLD`; senão `name()`.
- Linha 2: barra igual + `"  "` + texto: Pesquisar `"{id} · {descrição}"` (descrição cortada para caber; sem descrição, `"{id} · {source_name}"`); Instalados/Atualizações `"{package_id} · {fonte}"` quando casado, `"sem correspondência no catálogo"` ou `"correspondência ambígua"` senão. Estilo: selecionado `sel_desc`; senão `fg_dim`.
- Sem fundo na linha selecionada (o `bg_selected` fica só para o seletor modal).
- O glifo `selected` antigo deixa de ser usado na lista.
- Atualize os `.snap` (como na Task 63) e as asserções de "linha selecionada" dos testes de Task 53/Task 59 para procurar a barra (`|` no Ascii).

**Files:**
- Modify: `src/tui/view.rs`, `src/tui/app.rs`
- Test: `tests/tui_view.rs`, `tests/tui_state.rs` e os `.snap` de `tests/snapshots/tui_view__*.snap`

Testes:
1. `selected_item_has_bar_and_pink_title` (`tests/tui_view.rs`): `Theme::new(true, ColorDepth::TrueColor)`, Nerd, 100×20, primeiro item selecionado ⇒ a célula da barra tem símbolo `"┃"` e `fg == theme.sel_bar().fg`; a primeira célula do nome tem `fg == theme.sel_title().fg` e `BOLD`; a primeira célula do nome do 2º item tem `fg == theme.fg().fg`.
2. `second_line_shows_id_and_description` (`tests/tui_view.rs`): com `entries()` do arquivo, a linha logo abaixo de `PowerToys` contém `Microsoft.PowerToys · Utilitários para power users`.
3. `end_of_list_is_visible_with_two_row_items` (`tests/tui_state.rs` + render): 50 entradas `pkg00`…`pkg49`, tamanho `(100, 20)`, `Msg::Resize(100, 20)`, `End` ⇒ o render contém `pkg49` e a linha dele começa (depois da borda) com a barra.
4. `page_down_moves_ten_items` (`tests/tui_state.rs`): 50 entradas, `PageDown` ⇒ `selected == 10`.

#### Checks beyond the steps

1. Página em itens: mutate trocando `/ (ITEM_ROWS + ITEM_GAP)` por `/ 1` em `clamp` ⇒ PEGA (teste 3).
2. Estilo da seleção: mutate trocando `sel_title` por `name()` no selecionado ⇒ PEGA (teste 1).
3. Segunda linha: mutate omitindo a descrição ⇒ PEGA (teste 2).

**Commit:** `feat(tui): two-row list items with a selection bar`

---

### Task 65: Barra de status em pepitas e ajuda contextual

#### Where this fits

Depende de Task 64. Troca a frase fixa do rodapé (`src/tui/view.rs:533-553`) por uma barra de status no estilo do exemplo `layout` do Lip Gloss e uma barra de ajuda no estilo do `help` do Bubbles, que muda por aba. A ajuda completa (`?`, `view.rs:555-592`) fica em colunas.

#### What binds this task

- Módulo novo `src/tui/help.rs` (registrar em `src/tui/mod.rs`):
  - `pub fn hints(app: &App) -> Vec<(&'static str, &'static str)>`: com foco nos detalhes começa por `("esc", "voltar")`; depois `("↑/↓", "mover")`, `("←/→", "aba")`, `("enter", "detalhes")`; Pesquisar acrescenta `("i", "instalar")`; Instalados `("u", "remover")`; Atualizações `("U", "atualizar")`; as três abas de lista `("ctrl+s", "fonte")`; sempre por último `("?", "ajuda")`.
  - `pub fn help_line(hints: &[(&str, &str)], width: usize, theme: &Theme) -> Line<'static>`: `"tecla desc"` separados por `" • "`; tecla `help_key`, descrição `help_desc`, separador `help_sep`; põe dicas em ordem enquanto couberem deixando espaço para `" …"`; se alguma ficou de fora, termina com `" …"` (`help_sep`). Nunca corta uma dica no meio.
- Barras de status e ajuda começam em `BAR_PAD_X` (`src/tui/layout.rs`) e terminam `BAR_PAD_X` antes da borda direita.
- Barra de status (linha acima da ajuda): à esquerda, se `app.status` existe, pepita `pill_msg` `" {status} "`; senão `"{sel+1} de {visíveis}"` em `fg_dim` (Pesquisar acrescenta `" · {total} no catálogo"`). À direita: pepita `pill_a` `" {spinner} atualizando "` enquanto `refreshing`; `" offline "` em `alert` + `BOLD` quando offline; pepita `pill_b` `" {glyph refresh} índice {idade} "` quando há idade. O `corner_line` sai da linha da busca.
- Altura ≥ 20: status + ajuda (2 linhas); abaixo de 20: só status (D6). `CHROME_ROWS` passa a 8 e `clamp` usa 7 quando a altura < 20 (função `pub fn chrome_rows(height: u16) -> usize` em `src/tui/app.rs`, usada por `clamp` e pela view).
- Ajuda completa: diálogo arredondado com borda `accent`, conteúdo em `inset(.., DIALOG_PAD_X, DIALOG_PAD_Y)` (já aplicado na Task 62); cada linha `tecla` (`accent` + `BOLD`, coluna de 12) + descrição (`fg`); mesmas teclas de hoje.

**Files:**
- Create (novo): `src/tui/help.rs`
- Modify: `src/tui/mod.rs`, `src/tui/view.rs`, `src/tui/app.rs`
- Test: `tests/tui_view.rs` e os `.snap` de `tests/snapshots/tui_view__*.snap`

Testes:
1. `hints_change_with_the_tab`: aba Pesquisar ⇒ chaves `["↑/↓", "←/→", "enter", "i", "ctrl+s", "?"]`; Instalados ⇒ `["↑/↓", "←/→", "enter", "u", "ctrl+s", "?"]`; Atualizações ⇒ `[…, "U", "ctrl+s", "?"]`; foco nos detalhes ⇒ primeira chave `"esc"`.
2. `help_line_truncates_whole_hints_with_ellipsis`: dicas da Pesquisar, largura 40 ⇒ o texto termina com `" …"`, tem no máximo 40 colunas e cada dica presente aparece inteira; largura 200 ⇒ sem `"…"` e contém `"? ajuda"`.
3. `status_message_is_a_pink_pill`: TrueColor; `app.status = Some("Instalado: rg".into())`, 100×24 ⇒ a célula do `I` de `Instalado` tem `bg == theme.pill_msg().bg`.
4. `short_terminal_hides_the_help_line`: 100×18 ⇒ o render não contém `"? ajuda"`; 100×24 contém.
5. `help_line_colors_keys_and_descriptions`: TrueColor ⇒ a célula da tecla `i` tem `fg == theme.help_key().fg` e a primeira letra de `instalar` tem `fg == theme.help_desc().fg`.

#### Checks beyond the steps

1. Dicas por aba: mutate mostrando `("i", "instalar")` em todas as abas ⇒ PEGA (teste 1).
2. Corte inteiro: mutate cortando no limite exato de colunas (`take(width)`) ⇒ PEGA (teste 2).
3. Pepita: mutate desenhando a mensagem com `warn` sem fundo ⇒ PEGA (teste 3).
4. Altura: mutate trocando o limiar 20 por 0 ⇒ PEGA (teste 4).

**Commit:** `feat(tui): status pills and contextual help bar`

---

### Task 66: Toasts para eventos, com relógio injetado

#### Where this fits

Depende de Task 65 (barra de status em pepitas, que vira o destino do toast em terminal pequeno) e de Task 62 (margens). Hoje todo aviso é uma string em `app.status` que fica até a próxima mensagem (`src/tui/app.rs`, por exemplo `:369` `falha ao ler o catálogo`, `:687` resumo da operação, `:703` falha). Proposta e tabela "o que vira toast" em `tui-design.md` §2.7. Regra: **toast = evento que aconteceu; barra de status = estado verdadeiro agora.**

#### What binds this task

- Tipos em `src/tui/app.rs`: `pub enum ToastKind { Success, Info, Warning, Error }`; `pub struct Toast { pub kind: ToastKind, pub text: String, pub expires: Option<std::time::Instant>, pub count: u32 }`. Durações: `Success` 3 s, `Info` 4 s, `Warning` 6 s, `Error` `None` (fixo até `Esc`; D9). Constante `pub const MAX_TOASTS: usize = 3`.
- `App` ganha `pub now: std::time::Instant` (`App::new` usa `Instant::now()`; **o laço atualiza `app.now = Instant::now()` antes de aplicar cada lote**; os testes atribuem à mão — nada de `sleep` nos testes) e `pub toasts: Vec<Toast>` (índice 0 = mais novo).
- `pub fn push_toast(&mut self, kind: ToastKind, text: impl Into<String>)`: se o toast mais novo com o mesmo `kind` e `text` existe em qualquer posição, ele sobe para a posição 0, `count += 1` e o prazo recomeça de `now`; senão insere em 0 com `count = 1`. Se passar de `MAX_TOASTS`, remove o **mais velho com prazo**; se todos forem `Error`, remove o mais velho.
- `pub fn prune_toasts(&mut self)`: remove os que têm `expires <= now`. Chamado no começo de `update` para **toda** mensagem (assim um toast vencido some mesmo que outra mensagem chegue antes do despertar).
- `pub fn next_toast_deadline(&self) -> Option<Instant>`: o menor `expires` entre os toasts; `None` se não há toast com prazo.
- `Msg::Clock` (nova): só provoca o `prune_toasts` e um redesenho.
- `Esc` (fora de diálogo): se existe toast `Error`, remove o mais novo deles e **para** (não volta foco, não limpa busca, não sai). Toasts com prazo não reagem ao `Esc`. Dentro de diálogo, o `Esc` continua do diálogo.
- **Conversões** (cada `app.status = Some(..)` listado vira `push_toast`; o resto continua em `status`):
  - `Success`: resumo de `on_finished` `Ok`; `Resíduos removidos`; atualização do catálogo terminada sem falha **quando pedida com Ctrl+R** (`App` ganha `refresh_forced: bool`, ligado no Ctrl+R, desligado em `RefreshFinished`/`RefreshBusy`), texto `"catálogo atualizado"`.
  - `Info`: `resíduos mantidos`; `Aguardando: outra atualização em andamento`; `aguarde: operação em andamento`.
  - `Warning`: `outra instalação em andamento`; `instalação na fase 2d`; `NOT_OURS`; `Falha ao atualizar (…)` quando **todas** as falhas são offline.
  - `Error`: `falha ao ler o catálogo: …` (`Msg::CatalogFailed` e `Msg::Booted` com `Err`); `falha ao ler instalados: …`; `Falha ao limpar resíduos: …`; erro de `on_planned`; `InstallFail::Other`; `Falha ao atualizar (…)` com alguma falha não offline.
  - Continua em `status`: `planejando…` e qualquer texto de progresso/andamento. `Msg::Booted` com `Ok` continua limpando `status`.
- **Laço** (`src/tui/run.rs`): `app.now = Instant::now()` antes do `update` de cada lote; ramo novo no `select!`: `_ = tokio::time::sleep_until(tokio::time::Instant::from_std(d)), if let Some(d) = wake => Msg::Clock`, com `let wake = app.next_toast_deadline();` calculado antes do `select!` (se o `select!` desta versão do tokio não aceitar `if let` na pré-condição, use `wake.is_some()` e `wake.unwrap_or(app.now)` — `unwrap_or`, não `unwrap`). **Nenhum `Interval` novo**: o despertar é um prazo único recalculado a cada volta, então não existe tick perdido para acumular. `needs_draw` (Task 53) trata `Msg::Clock` como mensagem que desenha.
- **Desenho** (`src/tui/toast.rs`, novo; registrar em `src/tui/mod.rs`): `pub fn draw_toasts(frame: &mut Frame, window: Rect, toasts: &[Toast], theme: &Theme, glyphs: &Glyphs)` chamado por último em `view` (por cima de diálogos). Cada toast: largura `min(44, window.width - 2*TOAST_MARGIN_X)`, borda arredondada na cor do tipo (`ok`/`accent`/`warn`/`err`), `Clear` embaixo, texto em `inset(.., TOAST_PAD_X, 0)`; linha 1 = glifo + título (`Sucesso`/`Info`/`Aviso`/`Erro`, cor do tipo + `BOLD`) + `" ×N"` quando `count > 1`; depois o texto em `fg`, quebrado em até 3 linhas (o resto vira `…`); `Error` acrescenta a linha `"esc fecha"` em `help_desc`. Posição: borda direita do toast a `TOAST_MARGIN_X` colunas da borda direita da janela; o primeiro toast começa `TOAST_MARGIN_Y` linha abaixo do topo da janela (linha 3 + 1 = 4 com as abas de Task 63); os seguintes logo abaixo do anterior; o que não couber na altura não é desenhado.
- Glifos novos em `Glyphs` (`src/tui/theme.rs`): `toast_ok`, `toast_info`, `toast_warn`, `toast_err` = Nerd `"\u{f00c}"`, `"\u{f05a}"`, `"\u{f071}"`, `"\u{f057}"`; Unicode `"✓"`, `"ℹ"`, `"⚠"`, `"✗"`; Ascii `"+"`, `"i"`, `"!"`, `"x"`.
- **Terminal pequeno** (largura < 40 ou altura < 12): nenhuma caixa; a pepita de mensagem da barra de status (Task 65) mostra o texto do toast mais novo quando `status` está vazio.
- Dica de ajuda (`src/tui/help.rs`): com algum toast `Error`, acrescenta `("esc", "fecha erro")` antes de `("?", "ajuda")`.

**Files:**
- Create (novo): `src/tui/toast.rs`
- Modify: `src/tui/mod.rs`, `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/view.rs`, `src/tui/theme.rs`, `src/tui/help.rs`
- Test: `tests/tui_state.rs`, `tests/tui_view.rs`, `tests/tui_loop.rs`; testes existentes que conferem `a.status` dessas mensagens (ex.: `installed_failure_shows_status`, `busy_shows_status_and_rescans`, `finished_ok_rescans_and_reports`, `planning_error_becomes_status`, `refresh_busy_is_not_offline_and_says_in_progress` em `tests/tui_state.rs`) passam a conferir o toast — mantenha o nome do teste e registre a troca no relatório.

Testes (`let t0 = a.now;`, avanço com `a.now = t0 + Duration::from_millis(..)`):
1. `success_toast_expires_after_three_seconds` (`tests/tui_state.rs`): `Msg::InstallFinished(Ok(Done { summary: "Instalado: rg".into(), residues: None }))` ⇒ `a.toasts[0].kind == ToastKind::Success` e texto `"Instalado: rg"`; `a.now = t0 + 2999 ms`, `Msg::Clock` ⇒ ainda 1; `t0 + 3001 ms`, `Msg::Clock` ⇒ vazio.
2. `catalog_error_toast_stays_until_esc` (`tests/tui_state.rs`): `Msg::CatalogFailed("database is locked".into())` ⇒ `Error`, `expires == None`; `a.now = t0 + 600 s`, `Msg::Clock` ⇒ continua; `Esc` ⇒ some, comandos vazios (não saiu).
3. `esc_ignores_timed_toasts` (`tests/tui_state.rs`): toast de sucesso + digitar `ab`; `Esc` ⇒ `query` vazio e o toast continua.
4. `stack_keeps_three_and_drops_the_oldest_timed` (`tests/tui_state.rs`): `push_toast(Error, "E")`, `(Success, "S1")`, `(Success, "S2")`, `(Success, "S3")` ⇒ textos `["S3", "S2", "E"]`.
5. `repeated_toast_counts_and_restarts_its_timer` (`tests/tui_state.rs`): `(Success, "x")` em `t0`, de novo em `t0 + 2 s` ⇒ um toast, `count == 2`, `expires == t0 + 5 s`.
6. `next_deadline_is_the_earliest_timed_toast` (`tests/tui_state.rs`): `(Warning, "w")` em `t0`, `(Success, "s")` em `t0` ⇒ `Some(t0 + 3 s)`; só um `Error` ⇒ `None`.
7. `forced_refresh_success_toasts_and_automatic_is_silent` (`tests/tui_state.rs`): `Ctrl+R`, `RefreshStarted`, `RefreshFinished(vec![])` ⇒ toast `"catálogo atualizado"`; sem `Ctrl+R`: `RefreshStarted`, `RefreshFinished(vec![])` ⇒ nenhum toast.
8. `offline_refresh_failure_is_a_warning` (`tests/tui_state.rs`): `RefreshFinished(failed_report(true))` ⇒ `Warning`; `failed_report(false)` ⇒ `Error` (helper existente).
9. `planning_stays_in_the_status_bar` (`tests/tui_state.rs`): `i` num pacote scoop ⇒ `status == Some("planejando…")` e nenhum toast.
10. `expired_toast_disappears_on_any_message` (`tests/tui_state.rs`): toast de sucesso em `t0`; `a.now = t0 + 4 s`; `update(Msg::Resize(100, 24))` ⇒ sem toasts.
11. `toast_box_sits_top_right_with_margins` (`tests/tui_view.rs`): Ascii, NoColor, 100×24, um toast `Success` `"Instalado: rg"` ⇒ na linha 4, a coluna 97 é `╮` (borda direita do toast), a coluna 98 é `" "` e a 99 é a borda da janela; a linha 5 contém `"+ Sucesso"` e a linha 6 contém `"Instalado: rg"` precedido de um espaço depois da borda (`TOAST_PAD_X`).
12. `small_terminal_puts_the_toast_in_the_status_bar` (`tests/tui_view.rs`): 50×10, mesmo toast ⇒ nenhum `╮` nas colunas 40–49 das linhas 3–6; a última linha contém `"Instalado: rg"`.
13. `clock_message_draws` (`tests/tui_loop.rs`): `needs_draw(&[Msg::Clock], false) == true`.

#### Checks beyond the steps

1. Prazo: mutate trocando a duração de `Success` de 3 s para 30 s ⇒ PEGA (teste 1).
2. Erro fixo: mutate dando prazo de 4 s ao `Error` ⇒ PEGA (teste 2).
3. Esc não engole: mutate fazendo o `Esc` remover qualquer toast ⇒ PEGA (teste 3).
4. Pilha: mutate removendo o mais velho de qualquer tipo ⇒ PEGA (teste 4).
5. Despertar único: mutate fazendo `next_toast_deadline` devolver o **maior** prazo ⇒ PEGA (teste 6).
6. Silêncio do automático: mutate dando toast em todo `RefreshFinished` sem falha ⇒ PEGA (teste 7).
7. Poda em toda mensagem: mutate podando só em `Msg::Clock` ⇒ PEGA (teste 10).
8. Margem: mutate trocando `TOAST_MARGIN_X` por 0 no cálculo da posição ⇒ PEGA (teste 11).

**Commit:** `feat(tui): event toasts with a single wake-up deadline`

---

### Task 67: `[behavior]` no `config.toml` com gravação ler-modificar-gravar

#### Where this fits

Independente (paralelo a Task 59). Prepara a aba Ajustes (Task 68) e o uso dos ajustes (Task 69). Hoje `src/config.rs` tem `scoop_buckets`, `buckets`, `glyphs`, `theme` e `save` (temporário + `rename`, `config.rs:63-77`).

#### What binds this task

- Em `src/config.rs`:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)] #[serde(rename_all = "lowercase")] pub enum ConfirmMode { #[default] Always, Removals, Never }`
  - `pub enum ResidueMode { #[default] Ask, High, Keep }` com os mesmos derives e `rename_all`.
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)] #[serde(default)] pub struct Behavior { pub confirm: ConfirmMode, pub purge: bool, pub keep_old: bool, pub residues: ResidueMode }`
  - `Config` ganha `pub behavior: Behavior` (`Default` = `Behavior::default()`); no arquivo, a seção `[behavior]`.
  - `pub fn update_file(path: &Path, change: impl FnOnce(&mut Config)) -> Result<Config, ConfigError>`: **relê** o arquivo (`Config::load`, arquivo ausente ⇒ padrão), aplica `change`, grava com `save` e devolve o que gravou. Motivo: a TUI fica aberta enquanto a CLI pode ter feito `bucket add`; gravar uma cópia antiga em memória apagaria o bucket.
- Arquivo antigo sem `[behavior]` carrega com os padrões. Valor inválido (`confirm = "as vezes"`) ⇒ `ConfigError::Parse` com o caminho (comportamento atual de arquivo inválido).
- Nada usa os campos ainda (Task 68/Task 69). Nenhuma mudança na CLI.

**Files:**
- Modify: `src/config.rs`
- Test: `tests/config.rs`

Testes:
1. `missing_behavior_section_gives_defaults`: arquivo `scoop_buckets = ["main"]\n` ⇒ `c.behavior == Behavior::default()`, `confirm == ConfirmMode::Always`, `residues == ResidueMode::Ask`, `!purge`, `!keep_old`.
2. `behavior_values_are_read`: arquivo `"[behavior]\nconfirm = \"removals\"\npurge = true\nkeep_old = true\nresidues = \"high\"\n"` ⇒ campos correspondentes.
3. `behavior_round_trips_through_save`: `Behavior { confirm: Never, purge: true, keep_old: false, residues: Keep }` ⇒ `save` + `load` igual.
4. `invalid_confirm_value_mentions_the_path`: `"[behavior]\nconfirm = \"as vezes\"\n"` ⇒ erro cujo texto contém `config.toml`.
5. `update_file_keeps_changes_made_meanwhile`: grava config com `buckets = [{name = "x", repo = "a/x"}]`; carrega uma cópia; outra gravação adiciona o bucket `y`; `update_file(&p, |c| c.behavior.purge = true)` ⇒ arquivo final tem `x`, `y` e `purge == true`.

#### Checks beyond the steps

1. Releitura: mutate trocando o `Config::load(path)` de `update_file` por `Config::default()` ⇒ PEGA (teste 5).
2. Padrão: mutate trocando o `#[default]` de `ConfirmMode` para `Never` ⇒ PEGA (teste 1).
3. Nomes no arquivo: mutate removendo `rename_all = "lowercase"` de `ResidueMode` ⇒ PEGA (teste 2).

**Commit:** `feat(config): behavior section with read-modify-write updates`

---

### Task 68: Aba Ajustes

#### Where this fits

Depende de Task 66 (toasts; Task 66 já vem depois de Task 65) e Task 67 (`Behavior`, `update_file`). Quarta aba, desenho em `tui-design.md` §2.4. Esta tarefa mostra, muda e grava os ajustes; **usá-los nas operações é a Task 69**.

#### What binds this task

- Módulo novo `src/tui/settings.rs` (registrar em `src/tui/mod.rs`):
  - `pub enum SettingId { Confirm, Purge, KeepOld, Residues, Glyphs, Theme }` e `pub const ORDER: [SettingId; 6]` nessa ordem.
  - `pub struct SettingsState { pub selected: usize, pub behavior: Behavior, pub glyphs: GlyphSet, pub dark: bool, pub glyphs_forced: bool, pub saved: (Behavior, GlyphSet, bool) }` (o `saved` guarda o último valor gravado com sucesso).
  - Rótulos e opções (UI em português): Confirmar antes de aplicar = `sempre`/`só remoções`/`nunca`; Ao remover, apagar dados (purge) = `não`/`sim`; Ao atualizar, manter versão antiga = `não`/`sim`; Resíduos após remover = `perguntar`/`só alta confiança`/`manter`; Glifos = `nerd`/`unicode`/`ascii`; Tema = `escuro`/`claro`. Descrição de uma linha por ajuste (ex.: Confirmar: "Mostra o plano e espera Enter antes de instalar, atualizar ou remover.").
  - `pub fn cycle(&mut self, forward: bool) -> bool`: muda o valor do ajuste selecionado (circular); devolve `false` sem mudar quando o ajuste é Glifos e `glyphs_forced`.
- `src/tui/app.rs`: `Tab::Settings` depois de `Updates` no ciclo de `next_tab`/`prev_tab` (Task 57; vale para `Tab`, `BackTab`, `←` e `→`); `App` ganha `pub settings: SettingsState` (em `App::new`, padrão de `Config::default()` com os glifos recebidos) e `pub buckets_summary: String`. Na aba Ajustes: `↑/↓/j/k` mudam `settings.selected` (0..=5); `Enter`/Espaço/`l` chamam `cycle(true)` e `h` chama `cycle(false)`; `←`/`→` trocam de aba como nas outras abas (Task 57); se mudou, aplica na hora (`app.glyphs`, `app.dark`) e devolve `Cmd::SaveSettings { behavior, glyphs: "nerd"|"unicode"|"ascii", theme: "dark"|"light" }`, status `"salvando…"`. Letras não vão para a busca nesta aba; `q`, `?`, `Esc`, `Ctrl+C`, `Tab` funcionam como nas outras. `Msg::SettingsSaved(Result<(), String>)`: `Ok` ⇒ `saved` = atual, `status = None` e `push_toast(Success, "ajustes salvos")` (repetições somam `×N`, Task 66); `Err(e)` ⇒ volta ao `saved` (e `app.glyphs`/`app.dark`), `status = None` e `push_toast(Error, format!("falha ao salvar: {e}"))`.
- `App` ganha `pub dark: bool`; `src/tui/run.rs` guarda o `Theme` numa variável do laço e o recria com `Theme::new(app.dark, depth)` quando `app.dark` muda.
- `run` passa a receber se os glifos vieram da flag: `pub fn run(glyphs: GlyphSet, glyphs_forced: bool)`; `src/cli/mod.rs:908` passa `cli.glyphs.is_some()`. `run` preenche `app.settings` a partir do `config.toml` lido e `app.buckets_summary` (nomes de `scoop_buckets` + `buckets`, separados por `", "`).
- Execução de `Cmd::SaveSettings` em `src/tui/run.rs`: `spawn_job` existente (handle aguardado, pânico ⇒ `Msg::Panicked`) chamando `pub fn save_settings(path: &Path, behavior: Behavior, glyphs: &str, theme: &str) -> Result<(), String>` em `src/tui/jobs.rs`, que usa `Config::update_file`.
- Desenho: seções "Operações", "Aparência", "Fontes (somente leitura)"; ajuste selecionado com a barra de seleção (`sel_bar`) e rótulo `sel_title`; opção atual `‹ valor ›` em `accent` + `BOLD`, as outras em `fg_dim`; Glifos forçado mostra `"(substituído por --glyphs nesta sessão)"`; última linha a descrição do ajuste selecionado (`fg_dim` + itálico). Dicas da ajuda (`src/tui/help.rs`) na aba: `("↑/↓", "escolher")`, `("enter", "mudar")`, `("←/→", "aba")`, `("?", "ajuda")`. A aba ganha rótulo `Ajustes` com `glyphs.tab_settings`.

**Files:**
- Create (novo): `src/tui/settings.rs`
- Modify: `src/tui/mod.rs`, `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/help.rs`, `src/tui/run.rs`, `src/tui/jobs.rs`, `src/cli/mod.rs`
- Test: `tests/tui_state.rs`, `tests/tui_view.rs`, `tests/tui_jobs.rs`, snapshot novo `tests/snapshots/tui_view__settings_screen_snapshot.snap` (gerado)

Testes:
1. `settings_tab_is_in_the_tab_cycle` (`tests/tui_state.rs`): `Tab` ×4 a partir de Pesquisar ⇒ Instalados, Atualizações, Ajustes, Pesquisar; `BackTab` de Pesquisar ⇒ Ajustes; `Right` a partir de Atualizações ⇒ Ajustes; `Left` a partir de Pesquisar ⇒ Ajustes.
2. `enter_changes_confirm_and_requests_save` (`tests/tui_state.rs`): na aba Ajustes, `Enter` ⇒ `settings.behavior.confirm == ConfirmMode::Removals` e comandos `[Cmd::SaveSettings { behavior.confirm: Removals, glyphs: "nerd", theme: "dark" }]`.
3. `failed_save_restores_the_last_saved_value` (`tests/tui_state.rs`): depois do 2, `Msg::SettingsSaved(Err("disco cheio".into()))` ⇒ `confirm == Always`, `a.toasts[0].kind == ToastKind::Error` e texto contém `"disco cheio"`.
4. `forced_glyphs_cannot_be_changed` (`tests/tui_state.rs`): `settings.glyphs_forced = true`, selecionar Glifos (4× `Down`), `Enter` ⇒ nenhum comando, `app.glyphs` inalterado.
5. `letters_do_not_type_in_settings` (`tests/tui_state.rs`): `x` ⇒ `query` vazio.
6. `theme_setting_flips_dark` (`tests/tui_state.rs`): selecionar Tema (5× `Down`), `Enter` ⇒ `!app.dark`.
7. `h_cycles_backwards_and_arrows_leave_settings` (`tests/tui_state.rs`): na aba Ajustes com Confirmar selecionado, `h` ⇒ `confirm == ConfirmMode::Never`; `Right` ⇒ aba Pesquisar e `confirm` inalterado.
8. `settings_screen_snapshot` (`tests/tui_view.rs`): Ascii, NoColor, 100×24, `buckets_summary = "main, extras"`.
9. `save_settings_keeps_buckets` (`tests/tui_jobs.rs`): tempdir com `config.toml` contendo um bucket `x`; `save_settings(&p, Behavior { purge: true, ..Default::default() }, "ascii", "light")` ⇒ arquivo tem o bucket `x`, `purge = true`, `glyphs = "ascii"`, `theme = "light"`.

#### Checks beyond the steps

1. Gravação pedida: mutate removendo o `Cmd::SaveSettings` do ramo de `Enter` ⇒ PEGA (teste 2).
2. Reversão: mutate ignorando `Err` em `SettingsSaved` ⇒ PEGA (teste 3).
3. Flag manda: mutate ignorando `glyphs_forced` em `cycle` ⇒ PEGA (teste 4).
4. Arquivo preservado: mutate fazendo `save_settings` gravar `Config::default()` modificado em vez de `update_file` ⇒ PEGA (teste 9).
5. Setas laterais: mutate fazendo `Right` chamar `cycle` na aba Ajustes ⇒ PEGA (teste 7).

**Commit:** `feat(tui): settings tab persisted to config.toml`

---

### Task 69: Ajustes aplicados às operações da TUI

#### Where this fits

Depende de Task 68. Hoje a TUI sempre confirma (`src/tui/app.rs:661-681`), remove sem purge (`src/tui/jobs.rs:100`, `plan_uninstall(c, app, false)`), atualiza sem manter a versão antiga (`jobs.rs:87` e `jobs.rs:133`, `plan_for(c, &cand, false)`) e sempre abre a lista de resíduos (`app.rs:683-707`). Esta tarefa liga os ajustes de `app.settings.behavior` a esses pontos. **Só na TUI** (D1 = A); a CLI não muda.

#### What binds this task

- Comandos levam o ajuste do momento da tecla: `Cmd::PlanRemove { app, purge: bool }`, `Cmd::PlanInstall { source_name, id, keep: bool }`, `Cmd::PlanUpgradeAll { keep: bool }`. Instalar um pacote novo (`i`) usa `keep: false`.
- `src/tui/jobs.rs`: `plan_remove(c, app, purge)` passa `purge` para `ops::plan_uninstall`; a última linha do texto do plano é `"Os dados persistentes do app são mantidos."` sem purge e `"Os dados persistentes do app serão APAGADOS (purge)."` com purge. `plan_install(c, source_name, id, keep)` e `plan_upgrade_all(c, keep)` passam `keep` para `ops::plan_for`. Atualize as chamadas existentes de `jobs::plan_remove` em `tests/tui_jobs.rs` para a nova assinatura.
- Confirmação (`on_planned`, `Planned::Ready`): `pub fn needs_confirm(mode: ConfirmMode, action: &Action) -> bool` = `Always` ⇒ `true`; `Removals` ⇒ só `Action::Remove`; `Never` ⇒ `false`. Sem confirmação: mesmo caminho do Enter no diálogo (`Modal::Running` com `action.label()` + `Cmd::Execute(action)`). `Planned::Choose` sempre abre o seletor (nunca se escolhe pacote pelo usuário).
- Resíduos (`on_finished`, remoção com resíduos): `Ask` ⇒ seletor como hoje; `High` ⇒ os `residues` com `preselected == true`: se houver algum, `Modal::Running` com `"Limpando resíduos de {app}"` + `Cmd::CleanResidues { app, residues: <só os pré-marcados> }`; se nenhum, `push_toast(Info, format!("resíduos mantidos: {n} itens"))`; `Keep` ⇒ o mesmo toast `Info`, sem seletor. `Cmd::ScanInstalled` continua saindo sempre.

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/jobs.rs`, `src/tui/run.rs`
- Test: `tests/tui_state.rs`, `tests/tui_jobs.rs`

Testes:
1. `never_confirm_executes_directly` (`tests/tui_state.rs`): `settings.behavior.confirm = ConfirmMode::Never`; `update(ready("a"))` ⇒ `modal` é `Running` e comandos `[Cmd::Execute(apply_action("a"))]`.
2. `removals_mode_confirms_only_removals` (`tests/tui_state.rs`): `Removals`; `ready("a")` (Apply) ⇒ `Execute`; `Msg::Planned(Ok(Planned::Ready { text, action: Action::Remove(<UninstallPlan mínimo de src/install/plan.rs, app "a">) }))` ⇒ `Modal::Confirm`.
3. `ambiguous_target_still_opens_the_picker` (`tests/tui_state.rs`): `Never`; `Planned::Choose(vec![candidate("main"), candidate("extras")])` ⇒ `Modal::Picking`.
4. `remove_carries_the_purge_setting` (`tests/tui_state.rs`): `on_tab(Tab::Installed, vec![irow("a", "wingetrs", "scoop:main", false)])`, `purge = true`, tecla `u` ⇒ `[Cmd::PlanRemove { app: "a", purge: true }]`.
5. `upgrade_carries_the_keep_setting` (`tests/tui_state.rs`): `on_tab(Tab::Updates, vec![irow("a", "wingetrs", "scoop:main", true)])`, `keep_old = true`, tecla `U` ⇒ `[Cmd::PlanInstall { keep: true, .. }]`; com seleção vazia (`vec![]`) ⇒ `[Cmd::PlanUpgradeAll { keep: true }]`.
6. `high_residue_mode_cleans_only_preselected` (`tests/tui_state.rs`): `ResidueMode::High`; `Msg::InstallFinished(Ok(Done { summary, residues: Some(("a".into(), vec![residue("p1", true), residue("p2", false)])) }))` ⇒ comandos contêm `ScanInstalled` e `CleanResidues { app: "a", residues: [p1] }`; `modal` é `Running`.
7. `keep_residue_mode_opens_nothing` (`tests/tui_state.rs`): `ResidueMode::Keep`, mesmos resíduos ⇒ `modal` é `None`, `a.toasts[0].kind == ToastKind::Info` e texto contém `"2 itens"`.
8. `purge_plan_says_data_will_be_deleted` (`tests/tui_jobs.rs`, `World` existente): app instalado; `jobs::plan_remove(&ctx, "leaf", true)` ⇒ `Planned::Ready` com texto contendo `"APAGADOS"` e `Action::Remove(p)` com `p.purge == true`; com `false` ⇒ texto contém `"mantidos"` e `p.purge == false`.

#### Checks beyond the steps

1. Confirmação: mutate fazendo `needs_confirm` devolver `true` sempre ⇒ PEGA (teste 1).
2. Só remoções: mutate trocando `Action::Remove` por `Action::Apply` no ramo `Removals` ⇒ PEGA (teste 2).
3. Seletor nunca pulado: mutate executando o primeiro candidato em `Never` ⇒ PEGA (teste 3).
4. Purge: mutate passando `false` fixo para `ops::plan_uninstall` em `plan_remove` ⇒ PEGA (teste 8).
5. Keep: mutate emitindo `keep: false` fixo no `U` ⇒ PEGA (teste 5).
6. Resíduos: mutate limpando todos os resíduos no modo `High` (sem filtrar `preselected`) ⇒ PEGA (teste 6).

**Commit:** `feat(tui): apply confirm, purge, keep and residue settings`

---

## Desempenho e robustez (Tasks 70–78)

Preparado em 2026-10-12 sobre HEAD `922a995`. Fontes: rascunho `tasks-perf.md` (P1–P8, escrito em `97d1341`), ledger das Tasks 44–69, relatório `inv-catalog-write.md`. Cada premissa foi reconferida no código de HEAD (arquivo:linha abaixo). **Nada foi medido nesta preparação**: todo ganho citado é do relatório de origem ou hipótese.

Para colar no plano `docs/superpowers/plans/2026-10-10-wingetrs-fase2a.md` sob um `## ` novo (o orquestrador escolhe o título). Cada seção `### Task N` é autocontida: o `scripts/brief.ps1` copia só a seção da tarefa e o `## Global Constraints`, então o protocolo de medição está **repetido dentro de cada tarefa** (de propósito). Nenhuma seção usa cabeçalho `## ` (cortaria o brief).

**Resumo do lote**

| Task | Título | Origem | Depende de | Tier | Arquivos |
|---|---|---|---|---|---|
| 70 | `show` winget: versionData em cache pelo hash do catálogo | P1 | 45 | **design ⇒ opus** (sonda decide se há código) | `src/sources/winget/details.rs`, `src/cli/mod.rs` (spans), `tests/winget_details.rs` |
| 71 | Catálogo: `prepare_cached` em `get`/`find_by_id`/`lookup` | P2 | 45 | transcrição ⇒ sonnet | `src/catalog/mod.rs`, `tests/catalog.rs` |
| 72 | `Http` preguiçoso e runtime de thread única nos comandos locais | P3 | 45 | transcrição ⇒ sonnet (com portão) | `src/http.rs`, `src/cli/mod.rs` |
| 73 | `find_by_id` com índice de expressão `lower(id)` | P4 | 71 (mesmo arquivo) | transcrição ⇒ sonnet | `src/catalog/mod.rs`, `tests/catalog.rs` |
| 74 | `list`: threads de varredura logo após o log (com portão) | P5 | 72 (portão medido depois dela) | transcrição ⇒ sonnet | `src/cli/mod.rs` |
| 75 | Fonte Scoop sem varrer os mapas; `bucket rm` sem regravar índice à toa | P6 | 73 (mesmo arquivo) | transcrição ⇒ sonnet | `src/catalog/mod.rs`, `src/buckets.rs`, `tests/catalog.rs`, `tests/buckets.rs` |
| 76 | `update`: catálogo aberto e `http.json` gravado só quando algo mudou | P7 | 73 (nota do índice) | transcrição ⇒ sonnet | `src/update.rs`, `tests/update.rs` |
| 77 | Gravação do catálogo: índices dos mapas reconstruídos uma vez por import do winget (+ `cache_size` medido à parte) | `inv-catalog-write.md` (P2/P3 de lá) | 75 (mesma função), 55 (feita) | transcrição ⇒ sonnet (patch pronto; revisor opus: DDL transacional) | `src/catalog/mod.rs`, `tests/catalog.rs` |
| 78 | Investigação do "Acesso negado" (os error 5) intermitente nos testes sob carga | ledger (pendência OPEN) | — | **design ⇒ opus** | diagnóstico: worktree descartável; correção (condicional à evidência): `src/fs_retry.rs` (novo), `src/lib.rs`, `src/install/extract.rs`, `src/install/executor.rs`, `src/cache/mod.rs`, `src/install/download.rs`, `src/install/state.rs`, `src/catalog/index_file.rs`, `tests/install_extract.rs`, `tests/index_file.rs` |

**Conflitos de arquivo** (todas as tarefas de desempenho rodam sozinhas, então conflito só impõe ordem; a coluna "Depende de" já serializa):
- `src/catalog/mod.rs` e `tests/catalog.rs`: 71, 73, 75, 77 (77 mexe em `delete_source_rows`/`replace_source`/`SCHEMA` que 73 e 75 também mudam — rode 77 por último dos quatro e localize pelos símbolos).
- `src/cli/mod.rs`: 70 (só spans), 72, 74.
- `src/buckets.rs`, `tests/buckets.rs`: só 75. `src/update.rs`, `tests/update.rs`: só 76. `src/http.rs`: só 72. `src/sources/winget/details.rs`: só 70.
- 78 não divide arquivo com 70–77. **Mas** ela gera carga de propósito (reprodução): nunca em paralelo com uma tarefa que mede tempo. Sugestão de ordem: 78 primeiro (deixa o gate confiável para o resto do lote) ou por último; nunca no meio de uma medição.

**Ordem sugerida das de desempenho:** 77 tem o maior ganho esperado (−200 a −400 ms no `update`, medido no protótipo) mas depende de 75 por arquivo; se o orquestrador preferir o ganho antes, rode 77 antes de 73/75 e ajuste os testes de 73/75 ao `delete_source_rows` resultante (as tarefas citam símbolos, não só linhas).

**Descartada:**
- **P8** (`bucket rm` filtrar o `search.idx` em vez de reler o catálogo): a premissa era "regrava o índice relendo o catálogo inteiro (`load_entries`, várias conexões; `update.search_idx` 409 ms)". A Task 54 trocou isso por um único `SELECT` na conexão do catálogo (`src/catalog/index_file.rs:72-116`, `encode_catalog`), medido 68 → 31 ms no `update`. Sobra ≤ ~30 ms num comando raro, ao preço de um segundo caminho para o mesmo resultado (teste de equivalência obrigatório). Reabrir só se a Task 75 medir `bucket.rm.index_write` ≥ 20 ms **e** ≥ 40 % do delta do `bucket rm`.

**Achado fora do escopo (não virou tarefa):** os detalhes da TUI ainda abrem o catálogo como escritor a cada seleção — `Catalog::open` (WAL + DDL do `SCHEMA`) em `src/tui/run.rs:529` (`load_details`), e também em `:153`, `:488`, `:601`; para leitura o projeto usa `Catalog::open_read_only` (ARMADILHAS: "`Catalog::open` roda DDL"). Era o item 3 da lista "Para o redesign da TUI" do rascunho; as Tasks 57–69 não tocaram nisso.

---

### Task 70: `show` winget — versionData em cache pelo hash do catálogo

**Modelo:** opus (a sonda decide se existe código; o ramo de hash divergente precisa de desenho).

#### Where this fits

Depende só da Task 45 (feita). `show <id>` de pacote winget e o painel de detalhes da TUI (`src/tui/run.rs:523-560`, `load_details`, que chama a mesma função) fazem **sempre** um GET de `versionData.mszyml`: `src/sources/winget/details.rs:17-19` — o comentário diz "versionData has no known content hash, so it is always downloaded". Mas a URL é `packages/<id>/<hex8>/versionData.mszyml` com `hex8` = 4 primeiros bytes do `winget_hash` do catálogo (`src/sources/winget/version_data.rs:64-68`, `version_data_path`), e a fixture mostra que o hash inteiro é o SHA-256 do arquivo: `HASH_HEX` (`tests/winget_details.rs:10`) = SHA-256 de `tests/fixtures/winget/powertoys.versionData.mszyml`. Se isso valer no catálogo real, o versionData é endereçado por conteúdo e pode ir para o `BlobStore` verificado (`src/cache/mod.rs:169-188`: `get` relê e confere o SHA-256 do conteúdo contra o nome; `put_verified` recusa bytes que não conferem). Ganho esperado: um round-trip HTTPS (com TLS quando a conexão é nova) por `show` e por seleção na TUI — **não medido**.

Linhas conferidas em HEAD `922a995`; tarefas anteriores do lote podem deslocá-las — localize pelo símbolo citado junto.

#### What binds this task

- **(a) Spans (commit 1, item 5 do protocolo).** Em `src/cli/mod.rs`, braço `Command::Show` (`:683-734`): `show.find_by_id` em volta de `catalog.find_by_id(&id)` (`:690`). Em `details` (`details.rs`): `show.version_data` (GET + `version_data::decode_and_parse`) e `show.manifest` (blob ou GET + `manifest::parse`), com `.instrument(..)` nos trechos que têm `.await`. O nome tem prefixo `show.` mesmo quando a TUI chama (é a função).
- **(b) Sonda — portão antes de qualquer código da otimização.** Na raiz isolada (protocolo, item 3), salve `<S>\vd-probe.py` e rode `python -I "<S>\vd-probe.py" "<S>\la\wingetrs\catalog.db" "<S>\vd"`:
  ```python
  import hashlib, pathlib, sqlite3, sys, urllib.request
  db, out = sys.argv[1], pathlib.Path(sys.argv[2])
  out.mkdir(exist_ok=True)
  c = sqlite3.connect(pathlib.Path(db).as_uri() + "?mode=ro", uri=True)
  rows = c.execute(
      "SELECT id, lower(hex(winget_hash)) FROM packages "
      "WHERE source = 'winget' AND winget_hash IS NOT NULL LIMIT 5 OFFSET 1000"
  ).fetchall()
  for i, (pid, h) in enumerate(rows):
      url = f"https://cdn.winget.microsoft.com/cache/packages/{pid}/{h[:8]}/versionData.mszyml"
      body = urllib.request.urlopen(url, timeout=30).read()
      (out / str(i)).write_bytes(body)  # só para hashear; nunca executado nem importado
      got = hashlib.sha256(body).hexdigest()
      print(pid, h, got, "OK" if got == h else "DIVERGE")
  ```
  Cole as 5 linhas. **Qualquer `DIVERGE` ⇒ pare: `BLOCKED: versionData não é endereçado pelo winget_hash`, com a tabela; só o commit de spans fica.** Apague `<S>\vd` no fim.
- **(c) Mudança (commit 2), em `details`:**
  - Chave do blob = `hex::encode(hash)` (hash inteiro, minúsculo). `blobs.get(&chave)` devolveu bytes ⇒ usa (o `get` já confere o SHA-256). Senão: GET e `blobs.put_verified(&chave, &bytes)`.
  - `put_verified` com `Err(CacheError::HashMismatch { .. })` ⇒ **não grava**, `tracing::warn!` e segue com os bytes baixados (comportamento de hoje: o `show` não falha por isso). Outro erro de cache ⇒ `WingetError::Cache(e)`, como no manifesto (`details.rs:34-37`).
  - Reescreva o comentário da linha 17 (agora há hash). Nada muda no manifesto nem na lista de versões.
- **(d) Medição** com o protocolo abaixo (base = commit 1, nova = commit 2), mesma raiz para os dois lados (as 2 rodadas de aquecimento do `ab-cmd.ps1` deixam o manifesto em cache nos dois e o versionData em cache na nova): `-CmdArgs 'show Microsoft.PowerToys --source winget' -Span 'show\.version_data'`. Antes, cole a mediana dos três spans do commit 1 (item 5).
- **Reversão.** Métrica = `wall`; **L = 50 ms** (a mudança troca rede por disco; abaixo disso é ruído de rede). Esperado também `span` da nova ≤ 5 ms na mediana (cole; não decide). Equivalência: testes 1–3 verdes.

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf70-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-70" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-70\Cargo.toml" --target-dir "<S>\tgt-base-70"
   Copy-Item "<S>\tgt-base-70\release\wingetrs.exe" "<S>\perf70-base.exe"
   git worktree remove --force "<S>\wt-base-70"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf70-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf70-base.exe" -New "<S>\perf70-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-70`, `<S>\perf70-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-70`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Files:**
- Modify: `src/sources/winget/details.rs`, `src/cli/mod.rs` (só spans)
- Test: `tests/winget_details.rs`

Testes (use `HASH_HEX`, `VD`, `MANIFEST` do arquivo; cada teste com um `MockServer` próprio e `.expect(n)` no mock do versionData — o wiremock confere no `drop`; mesma base de URL dos testes atuais):
1. `version_data_is_cached_by_catalog_hash`: mock do versionData com `.expect(1)`; `details(..)` duas vezes com o mesmo `BlobStore` (tempdir) ⇒ os dois `PackageDetails` iguais (`assert_eq!`) e `blobs.get(HASH_HEX).is_some()`.
2. `tampered_cached_version_data_is_refetched`: antes, grave lixo em `<dir do BlobStore>/<HASH_HEX>`; versionData `.expect(1)`; uma chamada ⇒ versão `0.101.2362.0` e 115 versões; depois `blobs.get(HASH_HEX).is_some()` (o blob passou a conferir).
3. `version_data_with_another_hash_is_shown_but_not_cached`: hash = bytes de `HASH_HEX` com o **último** byte trocado (mesmos 4 primeiros bytes ⇒ mesma URL); versionData `.expect(2)`; duas chamadas ⇒ ambas `Ok` com versão `0.101.2362.0`; `blobs.get(<hex do hash trocado>)` é `None`.

#### Checks beyond the steps

Mutações (`pwsh -File scripts/mutate.ps1 -Path <arquivo> -Anchor '<texto exato>' -Replacement '<texto>' -TestCommand 'cargo test --quiet --test winget_details <nome>'`; com a TUI do usuário aberta, use `$env:CARGO_TARGET_DIR` separado e confira na saída o **nome do teste** que falhou — ARMADILHAS: "Mutação PEGA por falha de build"):
1. Cache usado: a consulta `blobs.get(&<chave>)` do versionData trocada por `None::<Vec<u8>>` ⇒ PEGA (teste 1, `expect(1)`).
2. Chave certa: `hex::encode(hash)` → `hex::encode(&hash[..4])` ⇒ PEGA (teste 1: o `put_verified` com chave curta falha e a 2ª chamada baixa de novo).
3. Verificação na leitura: em `src/cache/mod.rs` `BlobStore::get`, `.then_some(bytes)` → `.then_some(bytes).or(Some(Vec::new()))` ⇒ PEGA (teste 2). (Mutação temporária em código existente; o `mutate.ps1` restaura.)
4. Hash divergente não derruba o `show`: o ramo `HashMismatch` passando a devolver erro (`return Err(WingetError::ManifestIntegrity)`) ⇒ PEGA (teste 3).
5. Orquestrador: a tabela da sonda colada no relatório; sem ela a tarefa não é aceita.

**Commit:** `perf(show): cache winget versionData by its catalog hash` (precedido de `perf(show): spans for find_by_id, version_data and manifest`)

---

### Task 71: Catálogo — `prepare_cached` em `get`, `find_by_id` e `lookup`

**Modelo:** sonnet (mudança mecânica; a prova é o A/B).

#### Where this fits

Depende só da Task 45. `list` correlaciona cada app instalado com várias consultas ao catálogo, e cada uma compila o SQL de novo: `Catalog::get` (`src/catalog/mod.rs:494-503`, `query_row(&format!(..))`), `find_by_id` (`:505-512`, `prepare`) e `lookup` (`:514-527`, `prepare`). Em `src/installed/correlate.rs` um app ARP passa por `by_key` (`:108-119`, `lookup` em `:117`), `by_name_and_publisher` (até 3 `lookup`: `:136-144`) e `get` (`:163`, `:243`, `:270`); o install também chama `get`/`find_by_id` (`src/install/ops.rs:327`, `:340`, `:396`, `:884`). O span `step{step="correlate"}` (`crate::installed::timed`, `src/installed/mod.rs:107-114`) mediu 39 ms no log real de 2026-10-10 (79–96 ms em 2026-10-07). `prepare_cached` usa o cache de instruções da conexão (capacidade padrão 16; são 7 SQL distintos: `get`, `find_by_id` e um `lookup` por tabela de mapa). Ganho **não medido**.

#### What binds this task

- **(a) Medir antes:** o span já existe (sem commit de spans). Base = HEAD antes da tarefa.
- **(b) Mudança:** em `get`, `find_by_id` e `lookup`, `self.conn.prepare_cached(&sql)?` no lugar de `prepare`/`query_row(&sql, ..)`; para `get`: `self.conn.prepare_cached(&sql)?.query_row(params![source, id], stored_from_row).optional()?`. Texto do SQL idêntico; colunas lidas idênticas; nada mais muda.
- **(c) Medição:** protocolo abaixo, `-CmdArgs 'list' -Span 'step\{step=.correlate.\}'` (os pontos casam as aspas da linha do log).
- **Reversão.** Métrica = `span` (`correlate`); **L = max(3 ms, 10 % da mediana do span na base)**. Não-regressão: `wall` com delta pareado mediano ≤ +5 ms.

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf71-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-71" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-71\Cargo.toml" --target-dir "<S>\tgt-base-71"
   Copy-Item "<S>\tgt-base-71\release\wingetrs.exe" "<S>\perf71-base.exe"
   git worktree remove --force "<S>\wt-base-71"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf71-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf71-base.exe" -New "<S>\perf71-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-71`, `<S>\perf71-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-71`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Files:**
- Modify: `src/catalog/mod.rs`
- Test: `tests/catalog.rs`

Testes:
1. `cached_statements_return_fresh_rows_for_each_key`: `Catalog::open_in_memory()`; `replace_source("winget", SourceKind::Winget, "v", 1, &[pkg("A.App","A","1"), pkg("B.App","B","1")], &WingetMaps { productcodes: vec![("{PC-A}".into(),"A.App".into()), ("{PC-B}".into(),"B.App".into())], norm_names: vec![("aapp".into(),"A.App".into())], ..Default::default() })` (use o `pkg` do arquivo). Em sequência intercalada: `lookup(ProductCode, "{pc-a}") == ["A.App"]`; `lookup(ProductCode, "{PC-B}") == ["B.App"]`; `get("winget","B.App")` com `entry.id == "B.App"`; `lookup(NormName, "aapp") == ["A.App"]`; `lookup(ProductCode, "{PC-A}") == ["A.App"]` de novo; `find_by_id("a.app")` com 1 item de id `A.App`; `get("winget","A.App")` com `entry.id == "A.App"`.
2. `cached_statements_see_a_replaced_source`: o cenário do teste 1 (com as mesmas consultas já feitas, para os statements estarem no cache), depois `replace_source("winget", ..)` com o mapa `productcodes = [("{PC-A}", "B.App")]` ⇒ `lookup(ProductCode, "{PC-A}") == ["B.App"]` e `get("winget","A.App")` ainda `Some` (nenhum resultado guardado entre chamadas).
3. Regressão: `tests/catalog.rs`, `tests/correlate.rs` e `tests/install_ops.rs` verdes sem mudança.

#### Checks beyond the steps

1. Teste 1 não degenerado (consultas a tabelas diferentes não se confundem): em `lookup`, `table.table()` → `MapTable::ProductCode.table()` ⇒ PEGA (teste 1, `NormName`). `pwsh -File scripts/mutate.ps1 -Path src/catalog/mod.rs -Anchor 'table.table()' -Replacement 'MapTable::ProductCode.table()' -TestCommand 'cargo test --quiet --test catalog cached_statements'`.
2. A otimização em si não muda resultado: "sem mutação aplicável; a prova de desempenho é o A/B colado". Sem o A/B colado, a mudança é revertida.

**Commit:** `perf(catalog): reuse prepared statements for per-app lookups`

---

### Task 72: `Http` preguiçoso e runtime de thread única nos comandos locais

**Modelo:** sonnet (com portão de medição no início).

#### Where this fits

Depende só da Task 45. Todo comando de CLI monta **dois** `reqwest::Client` com rustls em `Http::with_timeouts` (`src/http.rs:50-67`), chamado em `run_async` (`src/cli/mod.rs:623`, antes do `match`) — inclusive `bucket`, que nem recebe `http` (`run_bucket(action, &data, cfg)`, `:815`) — e um runtime tokio multi-thread (`tokio::runtime::Runtime::new()`, `:910`; cria uma thread por núcleo e as junta no fim). Sem rede no caminho comum: `bucket add|list|rm`; `search` com índice válido (`:647-682`); `list`; `show` de pacote Scoop; `uninstall` (rede só via `ensure_catalog_ran`, `:465`, quando o catálogo está vazio). Download (`dl_client`) só em install/upgrade. A TUI também chama `Http::new()` (`src/tui/run.rs:170`) e ganha a preguiça de graça. `spawn_blocking` (`src/update.rs:194`, `src/install/ops.rs:277`) funciona nos dois sabores de runtime; não há `block_in_place` no código (conferido com grep). Ganho **não medido**.

#### What binds this task

- **(a) Spans e portão (commit 1).** `startup.runtime` em volta da construção do runtime (`:910`), `startup.runtime_drop` em volta de um `drop(rt)` explícito depois do `block_on` (guarde o resultado do `block_on` antes), `startup.http` em volta de `Http::new()` (`:623`). Meça (item 5 do protocolo) com 15 execuções de `bucket list`:
  ```powershell
  . "<S>\perf-env.ps1"; Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$W\log"
  1..15 | ForEach-Object { & "<S>\perf72-new.exe" bucket list *> $null }
  Select-String -Path "$W\log\wingetrs.log.*" -Pattern 'startup\.(runtime|runtime_drop|http).*close time\.busy' | ForEach-Object Line
  ```
  Cole as linhas e a mediana de cada um. **Se a soma das três medianas for < 2 ms, pare: relatório "não vale", só o commit de spans fica.**
- **(b) Mudança 1 — `Http` preguiçoso (commit 2, `src/http.rs`).** `Http::new()`/`with_timeouts` só guardam `connect`, `total` e `chunk_timeout`; `client` e `dl_client` viram `Arc<std::sync::OnceLock<reqwest::Client>>`, construídos no primeiro uso (`get_conditional` e `get` usam `client`; `get_to_file` usa `dl_client`). Erro de construção no primeiro uso ⇒ `HttpError::Network(e.to_string())` (o mesmo de hoje). Clones compartilham o `Arc` (um clone que inicializa serve a todos). Assinaturas públicas não mudam (`new()` continua devolvendo `Result`). Use `OnceLock::get_or_init` com o builder dentro de uma função que devolve `Result` guardado (ex.: `OnceLock<Result<Client, String>>`), sem `unwrap`/`expect`.
- **(c) Mudança 2 — runtime por comando (commit 3, `src/cli/mod.rs` `run`).** Função pura `fn wants_multi_thread(cmd: &Command) -> bool`: `false` para `Search`, `List`, `Bucket`; `true` para o resto. `false` ⇒ `tokio::runtime::Builder::new_current_thread().enable_all().build()?`. Motivo de manter multi-thread em update/install/upgrade/show/uninstall/repair: as tarefas de conexão do hyper rodam nos workers enquanto o futuro principal faz trabalho síncrono (decode MSZIP, import SQLite, extração, hash); em thread única elas param. Consequência aceita: `search`/`list` com catálogo **vazio** (1ª execução) fazem o `update` bloqueante em thread única — correto, mais lento; registre no relatório.
- **(d) Medição** (protocolo abaixo), duas decisões independentes: mudança 1 com base = commit 1 e nova = commit 2; mudança 2 com base = commit 2 e nova = commit 3. Para cada uma: métrica em `-CmdArgs 'bucket list'` (só custo fixo) e não-regressão em `-CmdArgs 'search powertoys'` e `-CmdArgs 'list'`.
- **Reversão.** Métrica = `wall` de `bucket list`; **L = max(5 ms, 10 % do "sobre o piso" da base)**. Não-regressão: `search powertoys` e `list` com delta pareado mediano ≤ +L. Reverta só o commit (2 ou 3) que falhar.

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf72-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-72" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-72\Cargo.toml" --target-dir "<S>\tgt-base-72"
   Copy-Item "<S>\tgt-base-72\release\wingetrs.exe" "<S>\perf72-base.exe"
   git worktree remove --force "<S>\wt-base-72"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf72-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf72-base.exe" -New "<S>\perf72-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-72`, `<S>\perf72-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-72`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Files:**
- Modify: `src/http.rs`, `src/cli/mod.rs`
- Test: `#[cfg(test)] mod tests` em `src/http.rs`; `mod early_tests` em `src/cli/mod.rs` (já existe, `:914-915`)

Testes:
1. `http_builds_no_client_until_used` (`src/http.rs`, `#[tokio::test]`, wiremock): helper `#[cfg(test)] fn built(&self) -> (bool, bool)` (= `client` e `dl_client` inicializados). `Http::new()` ⇒ `(false, false)`; após `get(<mock 200>)` ⇒ `(true, false)`; após `get_to_file(<mock 200>, <tempfile>, |_, _| {})` ⇒ `(true, true)`.
2. `http_clones_share_built_clients`: `let a = Http::new().unwrap(); let b = a.clone(); b.get(<mock>).await.unwrap();` ⇒ `a.built().0`.
3. `runtime_flavor_by_command` (`src/cli/mod.rs`): `Cli::try_parse_from` de `["wingetrs","search","x"]`, `["wingetrs","list"]`, `["wingetrs","bucket","list"]` ⇒ `wants_multi_thread(..) == false`; `["wingetrs","update"]`, `["wingetrs","install","x"]`, `["wingetrs","show","x"]`, `["wingetrs","upgrade","--all"]`, `["wingetrs","uninstall","x"]` ⇒ `true`.
4. Regressão: `tests/cli_offline.rs`, `tests/cli_install.rs`, `tests/http.rs`, `tests/update.rs` verdes (a rede via wiremock passa pelo cliente preguiçoso).

#### Checks beyond the steps

Mutações (`scripts/mutate.ps1`, `-TestCommand 'cargo test --quiet --lib <nome>'`; confira o nome do teste que falhou):
1. Preguiça: construir o `client` dentro de `with_timeouts` (chamar o `get_or_init` do `client` no construtor) ⇒ PEGA (teste 1).
2. Compartilhamento: `Arc<OnceLock<..>>` → `OnceLock<..>` sem `Arc` com `Clone` que copia o estado antes do init ⇒ PEGA (teste 2).
3. Sabor: incluir `Command::Update { .. }` no ramo `false` ⇒ PEGA (teste 3).

**Commit:** `perf(cli): build http clients on first use` e `perf(cli): single-thread runtime for local commands` (precedidos de `perf(cli): spans for runtime and http setup`)

---

### Task 73: `find_by_id` com índice de expressão `lower(id)`

**Modelo:** sonnet.

#### Where this fits

Depois da Task 71 (mesmo arquivo; rodam em série). `find_by_id` (`src/catalog/mod.rs:505-512`) faz `WHERE lower(id) = lower(?1)` sem índice: varre toda a tabela `packages`, cujas linhas carregam `raw` (manifesto JSON do Scoop) — páginas inteiras lidas para achar um id. Usado por `show` (`src/cli/mod.rs:690`) e pela resolução de dependências do install (`src/install/ops.rs:396`). O SQLite usa um índice de expressão quando o `WHERE` repete a mesma expressão, então **o SQL da consulta não muda**. Ganho **não medido**.

#### What binds this task

- **(a) Medir antes:** span `show.find_by_id` (criado na Task 70; se a 70 parou na sonda ou foi revertida, o commit de spans dela continua lá; se não existir, crie-o aqui num commit de spans, item 5 do protocolo). `EXPLAIN QUERY PLAN` na cópia isolada, colado: `python -I -c "import pathlib,sqlite3,sys; c=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); print(*c.execute('EXPLAIN QUERY PLAN SELECT rowid FROM packages WHERE lower(id) = lower(?1)', ('git',)), sep=chr(10))" "<S>\la\wingetrs\catalog.db"` — esperado `SCAN packages`.
- **(b) Mudança:** `SCHEMA` (`:10-45`) ganha `CREATE INDEX IF NOT EXISTS ix_pkg_id_lower ON packages(lower(id));`. O `WHERE` de `find_by_id` sai para `const FIND_BY_ID_WHERE: &str = "lower(id) = lower(?1)";` usado no `format!` do SQL (texto final idêntico ao de hoje), para o teste de plano exercitar o texto real. Nada mais muda. Catálogos existentes ganham o índice no próximo `Catalog::open` (escritores: `update`, `bucket rm`, partes da TUI); `open_read_only` continua sem DDL e uma consulta sem índice continua correta.
- **(c) Medição com duas raízes** (o binário base usaria o índice se ele existisse no arquivo): `perf-root.ps1 -Name la` (base) e `-Name la2` (nova); `update` com o binário base em `la` e com o novo em `la2` (cria o índice). Confirme com o mesmo `EXPLAIN` em `la2`: `SEARCH packages USING INDEX ix_pkg_id_lower (<expr>=?)`. Séries, nesta ordem:
  1. Métrica: `-RootBase "<S>\la" -RootNew "<S>\la2" -CmdArgs 'show git --source scoop' -Span 'show\.find_by_id'`.
  2. Não-regressão: `-RootBase "<S>\la" -RootNew "<S>\la2" -CmdArgs 'install ripgrep --dry-run'`.
  3. Custo de escrita (com o servidor local do msix abaixo; reimporta o winget a cada execução): `-RootBase "<S>\la" -RootNew "<S>\la2" -CmdArgs 'update --force' -Drop http.json -Span 'update\.catalog_write\{source=winget\}'`.
- **Reversão.** Métrica = `wall` da série 1; **L = max(5 ms, 10 % do "sobre o piso" da base)**. Não-regressão: série 2 com delta pareado mediano ≤ +L; série 3 com delta pareado mediano do `span` ≤ +10 % da mediana da base (senão reverte mesmo com ganho na série 1).

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf73-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-73" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-73\Cargo.toml" --target-dir "<S>\tgt-base-73"
   Copy-Item "<S>\tgt-base-73\release\wingetrs.exe" "<S>\perf73-base.exe"
   git worktree remove --force "<S>\wt-base-73"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf73-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf73-base.exe" -New "<S>\perf73-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-73`, `<S>\perf73-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-73`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Servidor local do msix** (entrada fixa, sem rede no laço; o servidor vive só dentro da chamada de shell que mede e morre no `finally`):
   ```powershell
   New-Item -ItemType Directory -Force "<S>\ab-msix" | Out-Null
   # Uma vez: o MESMO arquivo serve os dois lados (baixado da CDN; só é lido pelo wingetrs).
   if (-not (Test-Path "<S>\ab-msix\source2.msix")) { curl.exe -sSL -o "<S>\ab-msix\source2.msix" https://cdn.winget.microsoft.com/cache/source2.msix }
   $srv = Start-Process python -ArgumentList '-I', '-m', 'http.server', '8765', '--bind', '127.0.0.1', '--directory', "<S>\ab-msix" -PassThru -WindowStyle Hidden
   try {
       . "<S>\perf-env.ps1"
       $env:WINGETRS_WINGET_INDEX_URL = 'http://127.0.0.1:8765/source2.msix'
       $env:WINGETRS_SCOOP_BASE_URL = 'http://127.0.0.1:9'   # porta morta: scoop falha offline e mantém suas linhas
       Start-Sleep -Milliseconds 700
       # <comandos desta tarefa aqui>
   } finally { Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue }
   ```
   (`WINGETRS_WINGET_INDEX_URL`/`WINGETRS_SCOOP_BASE_URL`: `src/update.rs:24-37`; scoop vira `<base>/<bucket>.tgz`.)

**Files:**
- Modify: `src/catalog/mod.rs`
- Test: `#[cfg(test)] mod tests` em `src/catalog/mod.rs` (já existe, `:538`; precisa da `const` privada) e `tests/catalog.rs`

Testes:
1. `find_by_id_where_uses_the_lower_id_index` (unitário, `src/catalog/mod.rs`): `Catalog::open_in_memory()`; `EXPLAIN QUERY PLAN SELECT rowid FROM packages WHERE {FIND_BY_ID_WHERE}` com o parâmetro `"x"` na própria `conn` ⇒ alguma linha (coluna `detail`) contém `ix_pkg_id_lower`.
2. `find_by_id_is_the_same_with_and_without_the_index` (`tests/catalog.rs`): catálogo em arquivo (tempdir) com fontes `winget` (`Git.Git`) e `scoop:main` (`git`, `Ünïcode`); para cada consulta em `["GIT.GIT", "git", "Git", "ünïcode", "ÜNÏCODE", "Ünïcode"]` guarde os `(source_name, id)` de `find_by_id`; feche, `DROP INDEX ix_pkg_id_lower` por uma `rusqlite::Connection` à parte, reabra com `Catalog::open_read_only` e repita ⇒ listas idênticas, e, nos dois casos, `find_by_id("GIT.GIT") == [("winget", "Git.Git")]` e `find_by_id("Git") == [("scoop:main", "git")]` (`lower("Git.Git")` é `git.git`). Fixa também que o `lower()` do SQLite é só ASCII: `"ünïcode"` não acha `Ünïcode` nos dois casos.

#### Checks beyond the steps

1. Índice existe e é usado: remover a linha `CREATE INDEX IF NOT EXISTS ix_pkg_id_lower ON packages(lower(id));` ⇒ PEGA (teste 1). `-TestCommand 'cargo test --quiet --lib find_by_id_where'`.
2. Consulta continua usando a expressão: `FIND_BY_ID_WHERE` → `"id = ?1 COLLATE NOCASE"` ⇒ PEGA (teste 1: o plano deixa de citar o índice).
3. Equivalência com e sem índice: `FIND_BY_ID_WHERE` → `"lower(id) = ?1"` ⇒ PEGA (teste 2: `"GIT.GIT"` deixa de achar).

**Commit:** `perf(catalog): index lower(id) for case-insensitive id lookups`

---

### Task 74: `list` — threads de varredura logo após o log

**Modelo:** sonnet (portão de medição no início).

#### Where this fits

Depois da Task 72 (o portão é medido com ela integrada). `list` dispara varredura, rastreamento do winget e normalizador (`src/cli/mod.rs:740-756`, braço `Command::List` em `:735-799`) só dentro de `run_async`, depois de `data.ensure()`, `init_logging` e `Config::load` (`:893-895`), da construção do runtime (`:910`) e de `Http::new()` (`:623`). O `search` já antecipa a busca no topo de `run()` (`EarlySearch`, `:559-584`; criação em `:877-892`). As threads precisam começar **depois** de `init_logging` (antes dele os spans `scan_*` das threads se perdem: o subscriber global ainda não existe), então o ganho máximo é `Config::load` + runtime + `Http::new` — que a Task 72 encolhe. Por isso há portão.

#### What binds this task

- **(a) Medida e portão (commit 1).** Guarde um `Instant` logo depois de `init_logging` em `run()` e, no braço `List`, imediatamente antes do primeiro `spawn`, emita `tracing::debug!(ms = <decorrido>, "list.setup")` (um evento, não span: atravessaria o `block_on`). 15 execuções de `list` na raiz isolada e `Select-String -Path "$W\log\wingetrs.log.*" -Pattern 'list\.setup' | ForEach-Object Line`. **Mediana < 3 ms ⇒ pare: "não vale", só o commit 1 fica.**
- **(b) Mudança (commit 2).** Guard `EarlyList` no molde exato de `EarlySearch` (`:559-584`): três `Option<JoinHandle<_>>`, `join_all(&mut self)` que devolve os três resultados propagando pânico com `std::panic::resume_unwind`, e `Drop` que junta todos (com o ramo `std::thread::panicking()` de `EarlySearch`, `:575-582`). Em `run()`, logo após `init_logging`, se o comando é `List` cria o guard; `run_async` recebe `Option<EarlyList>` e o braço `List` usa os handles em vez de criar threads. Os joins continuam **antes** de propagar o erro do catálogo (`:757-762`). O evento `list.setup` continua (agora medindo quase nada).
- **(c) Medição:** protocolo abaixo, base = commit 1, nova = commit 2, `-CmdArgs 'list'`.
- **Reversão.** Métrica = `wall`; **L = max(5 ms, 10 % do "sobre o piso" da base)**.

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf74-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-74" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-74\Cargo.toml" --target-dir "<S>\tgt-base-74"
   Copy-Item "<S>\tgt-base-74\release\wingetrs.exe" "<S>\perf74-base.exe"
   git worktree remove --force "<S>\wt-base-74"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf74-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf74-base.exe" -New "<S>\perf74-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-74`, `<S>\perf74-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-74`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Files:**
- Modify: `src/cli/mod.rs`
- Test: `mod early_tests` em `src/cli/mod.rs`

Testes:
1. `early_list_propagates_worker_panic`: como `early_search_propagates_worker_panic` (`:918-925`), com um dos três handles em pânico ⇒ `catch_unwind` de `join_all` é `Err`.
2. `early_list_joins_every_thread_on_drop`: três threads que esperam uma `std::sync::Barrier` compartilhada com o teste e depois marcam um `Arc<AtomicBool>` cada; o teste cria o guard, passa a barreira e faz `drop(guard)` ⇒ os três `true` (sem `sleep`: ARMADILHAS, "Teste com limiar de relógio").
3. Regressão: `list_without_catalog_and_without_network_fails_cleanly_and_fast` (`tests/cli_offline.rs`) verde.

#### Checks beyond the steps

1. `Drop` junta: corpo do `Drop` de `EarlyList` esvaziado ⇒ PEGA (teste 2).
2. Pânico propagado: `std::panic::resume_unwind(p)` → `Default::default()`/`None` no `join_all` ⇒ PEGA (teste 1) — e o `check_rules.ps1` do gate barra `.ok()`/`unwrap_or_default` em `join`.

**Commit:** `perf(list): start the installed scan right after logging setup` (precedido de `perf(list): measure setup before the scan threads`)

---

### Task 75: Fonte Scoop sem varrer os mapas do winget; `bucket rm` sem regravar índice à toa

**Modelo:** sonnet.

#### Where this fits

Depois da Task 73 (mesmo arquivo). Desde a Task 55, `delete_source_rows` (`src/catalog/mod.rs:174-208`) faz, para cada tabela de mapa, `SELECT EXISTS(SELECT 1 FROM {t} WHERE package NOT IN (SELECT rowid FROM packages WHERE source = ?1))` e então esvazia a tabela inteira (todas as linhas são da fonte) ou cai no `DELETE FROM {t} WHERE package IN (SELECT rowid FROM packages WHERE source = ?1)`. Os índices dos mapas são só em `key` (`:40-44`), então esse `DELETE` varre a tabela de mapa inteira. Para uma fonte **Scoop** o `EXISTS` acha linha alheia (do winget) na 1ª linha ⇒ sempre o caminho por linha ⇒ 5 varreduras das 146 mil linhas de mapa do winget para apagar **nada** (o Scoop nunca grava mapa: `src/update.rs:260` passa `&WingetMaps::default()`). Acontece em todo import de bucket (`replace_source`, `:332`) e no `bucket rm` (`remove_source`, `:321-330`). Além disso `bucket rm` (`src/buckets.rs:182-216`, `remove_and_cleanup`) descarta o número de linhas removidas (`:196-198`) e **sempre** regrava o `search.idx` (`:199-203`; desde a Task 54, um `SELECT` de todo o `packages` em `index_file::encode_catalog`), mesmo quando o bucket foi adicionado e nunca baixado (0 linhas). Ganho **não medido**.

#### What binds this task

- **(a) Spans (commit 1):** `catalog.delete_source_rows` em volta do corpo de `delete_source_rows` e `bucket.rm.index_write` em volta do `index_file::write` de `remove_and_cleanup`.
- **(b) Mudança 1 — mapas (commit 2, `src/catalog/mod.rs`).** `delete_source_rows(tx, source, kind: SourceKind)` pula os 5 `DELETE`/`EXISTS` de mapa quando `kind == SourceKind::Scoop` (`truncated_maps` = 0). `replace_source` passa o `kind` que recebe; `remove_source` lê `SELECT kind FROM meta WHERE source = ?1` dentro da transação e, **sem linha em `meta`, trata como winget** (apaga mapas: caminho seguro). Para a premissa não apodrecer: `replace_source` com `kind == Scoop` e algum mapa não vazio ⇒ `Err(CatalogError::MapsForScoop(source.to_string()))` (variante nova, mensagem `"fonte scoop não aceita mapeamentos: {0}"`) **antes** de abrir a transação. O teste de equivalência existente (`check`, `:653-680`, contra o `delete_per_row` literal) ganha o parâmetro `kind` e continua comparando conteúdo: `check(winget_only, "winget", Winget, 5)`, `check(winget_only, "scoop:main", Scoop, 0)`, `check(two_owners, "winget", Winget, 0)`, `check(two_owners, "other", Winget, 0)`, `check(orphan_pfn, "winget", Winget, 4)`.
- **(c) Mudança 2 — `bucket rm` (commit 3, `src/buckets.rs`).** `let n = catalog.remove_source(..)?;` e regrava o `search.idx` só se `n > 0 || !idx.exists()`.
- **(d) Medição** (protocolo abaixo). `bucket rm` é destrutivo: restauração antes de cada execução com `-Pristine`. Preparação, numa chamada:
  ```powershell
  . "<S>\perf-env.ps1"
  & "<S>\perf75-new.exe" bucket list       # confira que extras está configurado
  python -I -c "import pathlib,sqlite3,sys; c=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); print(c.execute('SELECT count(*) FROM packages WHERE source = ?', (sys.argv[2],)).fetchone()[0])" "$W\catalog.db" scoop:extras
  New-Item -ItemType Directory -Force "<S>\pristine75a" | Out-Null
  Copy-Item "$W\catalog.db", "$W\search.idx", "$W\http.json" "<S>\pristine75a\"; if (Test-Path "$W\config.toml") { Copy-Item "$W\config.toml" "<S>\pristine75a\" }
  & "<S>\perf75-new.exe" bucket add perfx ScoopInstaller/Versions --yes   # bucket configurado e nunca baixado
  New-Item -ItemType Directory -Force "<S>\pristine75b" | Out-Null
  Copy-Item "$W\catalog.db", "$W\search.idx", "$W\http.json", "$W\config.toml" "<S>\pristine75b\"
  ```
  (Se o `print` acima der 0, use outro bucket com linhas que `bucket list` mostre e registre qual.) Séries:
  - Cenário (i), bucket com linhas: `-CmdArgs 'bucket rm extras' -Pristine "<S>\pristine75a" -Span 'catalog\.delete_source_rows'`.
  - Cenário (ii), bucket nunca baixado: `-CmdArgs 'bucket rm perfx' -Pristine "<S>\pristine75b"`.
- **Reversão** (por mudança; reverta só o commit que falhar). Mudança 1 (base = commit 1, nova = commit 2): métrica = `span` no cenário (i); **L = max(5 ms, 10 % da mediana do span na base)**; não-regressão: `wall` do cenário (i) com delta pareado mediano ≤ +5 ms. Mudança 2 (base = commit 2, nova = commit 3): métrica = `wall` no cenário (ii); **L = max(5 ms, 10 % do "sobre o piso" da base)**. Registre também a mediana de `bucket.rm.index_write` no cenário (i) (é o número que reabriria a P8 descartada).

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf75-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-75" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-75\Cargo.toml" --target-dir "<S>\tgt-base-75"
   Copy-Item "<S>\tgt-base-75\release\wingetrs.exe" "<S>\perf75-base.exe"
   git worktree remove --force "<S>\wt-base-75"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf75-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf75-base.exe" -New "<S>\perf75-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-75`, `<S>\perf75-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-75`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Files:**
- Modify: `src/catalog/mod.rs`, `src/buckets.rs`
- Test: `tests/catalog.rs`, `tests/buckets.rs`, `mod tests` de `src/catalog/mod.rs`

Testes:
1. `scoop_source_with_maps_is_refused` (`tests/catalog.rs`): `replace_source("scoop:main", SourceKind::Scoop, "v", 1, &[pkg("x","x","1")], &WingetMaps { productcodes: vec![("{X}".into(), "x".into())], ..Default::default() })` ⇒ `Err(CatalogError::MapsForScoop(_))`; `sources()` não lista `scoop:main`.
2. `removing_a_scoop_source_keeps_winget_maps` (`tests/catalog.rs`): fonte `winget` com `productcodes = [("{PC-A}", "A.App")]` e fonte `scoop:main` com 2 pacotes; `remove_source("scoop:main") == 2` e `lookup(ProductCode, "{PC-A}") == ["A.App"]`; idem depois de um novo `replace_source` da `scoop:main`.
3. `removing_a_winget_source_still_drops_its_maps` (`tests/catalog.rs`): mesmo cenário em arquivo temporário; `remove_source("winget")` ⇒ `lookup(ProductCode, "{PC-A}")` vazio e, por uma `rusqlite::Connection` à parte, `SELECT count(*) FROM map_productcode` == 0.
4. `truncating_owned_map_tables_equals_the_per_row_delete` (unitário existente) com o parâmetro `kind`, nas 5 chamadas do item (b).
5. `rm_of_a_never_updated_bucket_keeps_search_idx` (`tests/buckets.rs`): raiz temporária (`DataDir::new`), catálogo com uma fonte `scoop:main` de 1 pacote, `search.idx` gravado por `index_file::write`, mtime do `.idx` ajustado para `UNIX_EPOCH + 1_000_000 s` (`std::fs::File::options().write(true).open(..)?.set_modified(..)`); `add(&mut cfg, "perfx", "me/scoop-perfx", true)` sem linhas no catálogo; `remove_and_cleanup(&data, &mut cfg, "perfx", &db)` ⇒ mtime inalterado e bytes iguais.
6. Regressão: `remove_and_cleanup_purges_catalog_index_and_http_state` e `remove_source_clears_map_rows` (`tests/buckets.rs`) verdes.

#### Checks beyond the steps

1. Winget ainda limpa mapas: `delete_source_rows` pulando os mapas para qualquer `kind` (`if kind == SourceKind::Scoop` → `if true`) ⇒ PEGA (teste 3).
2. Premissa protegida: remover a recusa `MapsForScoop` ⇒ PEGA (teste 1).
3. Índice só quando mudou: `n > 0 ||` → `true ||` ⇒ PEGA (teste 5).
4. Sem `meta` ⇒ trata como winget: não há caminho público que crie pacotes sem `meta`; o revisor confere no diff (registre "sem mutação aplicável").

**Commit:** `perf(catalog): skip winget map scans for scoop sources` e `perf(buckets): keep search.idx when rm removed no rows` (precedidos de `perf(catalog): spans for delete_source_rows and bucket rm index write`)

---

### Task 76: `update` — catálogo aberto e `http.json` gravado só quando algo mudou

**Modelo:** sonnet.

#### Where this fits

Depois da Task 73 (nota abaixo). `run_update` (`src/update.rs:184-280`) sempre faz `Catalog::open` (`:231`; escritor: `journal_mode=WAL` + DDL do `SCHEMA`) e sempre grava `http.json` com `sync_all` (`:269-270`; `HttpState::save`, `src/cache/mod.rs:139-149`). Com todas as fontes `Fresh` (`update` dentro da janela de 15 min) nada mudou e as duas coisas são trabalho morto; com tudo `304` o `http.json` precisa ser gravado (o `checked_at` muda, `:236-241`) mas o catálogo não precisa ser aberto, salvo se o `search.idx` faltar (`:271-278`). Ganho esperado pequeno (ARMADILHAS: abrir SQLite + 1ª consulta custa 11–15 ms) — **não medido**.

Nota de interação com a Task 73: depois desta tarefa, um catálogo antigo só ganha o índice `ix_pkg_id_lower` num `update` que importe algo (ou num `bucket rm`). Aceitável: a consulta continua correta sem índice.

#### What binds this task

- **(a) Span (commit 1):** `update.catalog_open` em volta do `Catalog::open` (`:231`). Os spans da Task 45 (`update.state_save`, `update.search_idx`, `update.catalog_write`) **não mudam** de nome, lugar nem pai.
- **(b) Mudança (commit 2, `src/update.rs`):**
  - Função pura `fn needs_catalog(results: &[Option<Result<Fetched, (String, bool)>>], index_exists: bool) -> bool` = algum `Some(Ok(Fetched::Winget(..) | Fetched::Scoop(..)))` **ou** `!index_exists`. O catálogo é aberto uma vez, só se ela der `true`; os relatórios saem idênticos. Sem `unwrap`/`expect`: se um ramo de import ficar sem catálogo por construção, ele vira `Outcome::Failed` com mensagem interna e é registrado no relatório como inalcançável (mutação equivalente — ARMADILHAS).
  - `state.save` só quando algum relatório é `Outcome::Unchanged` ou `Outcome::Updated { .. }` (são os únicos que mudam um `HttpEntry`); tudo `Fresh`/`Failed` ⇒ não grava.
- **(c) Medição** (protocolo abaixo; base = commit 1, nova = commit 2), duas séries:
  1. Tudo `Fresh` (sem rede), raiz `la` logo depois do `update` do item 4 do protocolo: `-CmdArgs 'update'` (sem `-Span`: na nova o `update.state_save` não acontece; a métrica é `wall`).
  2. Tudo `304` com servidor local, numa raiz **vazia** própria (sem copiar o catálogo real):
     ```powershell
     $srv304 = "<S>\srv304"; New-Item -ItemType Directory -Force $srv304 | Out-Null
     Copy-Item tests\fixtures\winget\source2-mini.msix "$srv304\source2.msix"
     Copy-Item tests\fixtures\scoop\Main-mini.tar.gz "$srv304\main.tgz"; Copy-Item tests\fixtures\scoop\Main-mini.tar.gz "$srv304\extras.tgz"
     $srv = Start-Process python -ArgumentList '-I', '-m', 'http.server', '8766', '--bind', '127.0.0.1', '--directory', $srv304 -PassThru -WindowStyle Hidden
     try {
         $env:WINGETRS_WINGET_INDEX_URL = 'http://127.0.0.1:8766/source2.msix'
         $env:WINGETRS_SCOOP_BASE_URL = 'http://127.0.0.1:8766'
         New-Item -ItemType Directory -Force "<S>\la304\wingetrs" | Out-Null
         Start-Sleep -Milliseconds 700
         $env:LOCALAPPDATA = "<S>\la304"; & "<S>\perf76-new.exe" update --force; $env:LOCALAPPDATA = "<S>\la"
         pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf76-base.exe" -New "<S>\perf76-new.exe" -CmdArgs 'update --force' -RootBase "<S>\la304" -Runs 15
     } finally { Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue }
     ```
     (O `http.server` do Python responde 304 a `If-Modified-Since`, e o wingetrs manda esse cabeçalho a partir do `Last-Modified` gravado.) Confirme numa execução avulsa com `-v` que os três resultados saem `Unchanged`. No fim, apague `<S>\srv304` e `<S>\la304`.
- **Reversão.** Métrica = `wall`; **L = max(5 ms, 10 % do "sobre o piso" da base)**. Fica se pelo menos uma das duas séries passar no critério e a outra tiver delta pareado mediano ≤ +L; senão reverte o commit 2.

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório). Rode tudo **da raiz do repositório**. O shell da ferramenta **não guarda variáveis entre chamadas**: toda chamada que mede começa com `. "<S>\perf-env.ps1"`.

1. **Sozinho.** Nenhum outro agente, build ou teste rodando durante a medição (o piso de processo nesta máquina varia de 38 a 300 ms com a carga — ARMADILHAS, "Piso de processo"). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre (o modo automático bloqueia). Meça só com o gate verde.
2. **Binários fixos.** `cargo build --release --quiet; cargo build --release --example noop --quiet`, depois `Copy-Item target\release\wingetrs.exe "<S>\perf76-new.exe"` (builds seguintes não trocam o arquivo medido). Base = o commit imediatamente **antes** do commit da otimização (se a tarefa tem commit de spans, a base é ele: os dois lados têm o mesmo instrumento):
   ```powershell
   git worktree add --detach "<S>\wt-base-76" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-76\Cargo.toml" --target-dir "<S>\tgt-base-76"
   Copy-Item "<S>\tgt-base-76\release\wingetrs.exe" "<S>\perf76-base.exe"
   git worktree remove --force "<S>\wt-base-76"
   ```
3. **Raiz isolada** (a raiz real só é lida; nada é escrito em `%LOCALAPPDATA%\wingetrs`). Salve `<S>\perf-root.ps1` e rode `pwsh -File "<S>\perf-root.ps1"` (padrão `-Name la`; segunda raiz: `-Name la2`):
   ```powershell
   param([string]$Name = 'la')
   $ErrorActionPreference = 'Stop'
   $S = '<S>'
   $real = [Environment]::GetFolderPath('LocalApplicationData')   # pasta real, mesmo com $env:LOCALAPPDATA redirecionado
   $R = "$real\wingetrs"; $LA = "$S\$Name"; $W = "$LA\wingetrs"
   if (Test-Path $LA) { Remove-Item -Recurse -Force $LA }
   New-Item -ItemType Directory -Force $W | Out-Null
   # Cópia consistente do catálogo (arquivo do próprio wingetrs), lido em modo só-leitura.
   python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$R\catalog.db" "$W\catalog.db"
   if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
   foreach ($f in 'search.idx', 'http.json', 'config.toml') { if (Test-Path "$R\$f") { Copy-Item "$R\$f" $W } }
   # Para `list`: cópia do rastreamento do winget; o original nunca é aberto pelo binário medido.
   $T = 'Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe'
   if (Test-Path "$real\$T\installed.db") { New-Item -ItemType Directory -Force "$LA\$T" | Out-Null; Copy-Item "$real\$T\installed.db" "$LA\$T\" }
   "raiz pronta: $W"
   ```
   e `<S>\perf-env.ps1` (dot-source em toda chamada que mede):
   ```powershell
   $S = '<S>'; $LA = "$S\la"; $W = "$LA\wingetrs"
   $env:LOCALAPPDATA = $LA
   $env:WINGETRS_TEST_ENV_KEY = 'Software\wingetrs-test\perf'
   $env:WINGETRS_TEST_SHORTCUT_DIR = "$S\sc"
   ```
4. **Catálogo fresco.** Antes de cada série: `. "<S>\perf-env.ps1"; & "<S>\perf76-new.exe" update` (renova `checked_at`; `MAX_AGE_SECS` = 900 em `src/update.rs:13`, então nenhum `update --background` nos 15 min seguintes; com duas raízes, rode também com `$env:LOCALAPPDATA = "<S>\la2"`). Série que passar de 10 min: repita o `update` antes da próxima. O `ab-cmd.ps1` avisa se sobrou `wingetrs` rodando no fim (refresh em 2º plano ⇒ série inválida, repita).
5. **Spans.** Span que a tarefa manda criar vai num **primeiro commit separado** (`perf(<mod>): spans for <etapa>`), no formato da Task 45: `tracing::debug_span!("<comando>.<etapa>")`, `.in_scope(..)` em código síncrono, `.instrument(..)` em assíncrono (nunca `span.enter()` atravessando `.await`), sempre criado depois de `init_logging` (antes dele o span se perde). Prova de que mede a etapa: as linhas do log coladas, com `time.busy` > 0 e coerente com a etapa — `Select-String -Path "$W\log\wingetrs.log.*" -Pattern '<regex do span>.*close time\.busy' | Select-Object -Last 15` (ARMADILHAS: "Span criado não é span medido").
6. **A/B intercalado.** Salve `<S>\ab-cmd.ps1` (cada rodada sorteia a ordem base/nova/noop; 2 rodadas de aquecimento descartadas; `-Pristine` restaura arquivos antes de cada execução, fora do cronômetro; `-Drop` apaga arquivos da raiz antes de cada execução):
   ```powershell
   param(
       [Parameter(Mandatory)] [string]$Base,
       [Parameter(Mandatory)] [string]$New,
       [Parameter(Mandatory)] [string]$CmdArgs,
       [string]$Span = '',
       [string]$RootBase = '',
       [string]$RootNew = '',
       [string]$Pristine = '',
       [string[]]$Drop = @(),
       [int]$Runs = 15
   )
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   if (-not $RootBase) { $RootBase = "$S\la" }
   if (-not $RootNew) { $RootNew = $RootBase }
   $roots = @{ base = $RootBase; new = $RootNew }
   $exes = @{ base = $Base; new = $New }
   $noop = (Resolve-Path 'target\release\examples\noop.exe').Path
   $argv = -split $CmdArgs
   function Busy([string]$w) {
       $log = Get-ChildItem "$w\log" -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
       if (-not $log) { throw "sem log em $w" }
       $m = Select-String -Path $log.FullName -Pattern ($Span + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span '$Span' ausente em $w" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $orig = $env:LOCALAPPDATA
   $ms = @{ base = @(); new = @(); noop = @(); sbase = @(); snew = @() }
   try {
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           $r = @{}
           foreach ($k in @('base', 'new', 'noop') | Sort-Object { Get-Random }) {
               if ($k -eq 'noop') {
                   $sw = [Diagnostics.Stopwatch]::StartNew(); & $noop *> $null; $r.noop = $sw.Elapsed.TotalMilliseconds
                   continue
               }
               $w = Join-Path $roots[$k] 'wingetrs'
               if ($Pristine) {
                   Remove-Item -Force -ErrorAction SilentlyContinue "$w\catalog.db-wal", "$w\catalog.db-shm"
                   Copy-Item "$Pristine\*" $w -Force
               }
               foreach ($f in $Drop) { Remove-Item -Force -ErrorAction SilentlyContinue "$w\$f" }
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$w\log"
               $exe = $exes[$k]
               $env:LOCALAPPDATA = $roots[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe @argv *> $null
               $r[$k] = $sw.Elapsed.TotalMilliseconds
               $code = $LASTEXITCODE
               $env:LOCALAPPDATA = $orig
               if ($code -ne 0) { throw "$k '$CmdArgs' saiu com $code" }
               if ($Span) { $r["s$k"] = Busy $w }
           }
           if ($i -ge 2) { foreach ($key in @($r.Keys)) { $ms[$key] += $r[$key] } }
       }
   } finally { $env:LOCALAPPDATA = $orig }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   function Row([string]$name, $b, $n) {
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-4} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado (nova-base) med={5,8:N1} ms' -f `
           $name, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
   }
   function Raw($a) { ($a | ForEach-Object { '{0:N1}' -f $_ }) -join ' ' }
   Row 'wall' $ms.base $ms.new
   'piso noop med={0:N1} ms | sobre o piso: base {1:N1}, nova {2:N1} ms' -f (Med $ms.noop), ((Med $ms.base) - (Med $ms.noop)), ((Med $ms.new) - (Med $ms.noop))
   if ($Span) { Row 'span' $ms.sbase $ms.snew }
   'brutos wall base: ' + (Raw $ms.base); 'brutos wall nova: ' + (Raw $ms.new)
   if ($Span) { 'brutos span base: ' + (Raw $ms.sbase); 'brutos span nova: ' + (Raw $ms.snew) }
   if (Get-Process wingetrs -ErrorAction SilentlyContinue) { 'AVISO: wingetrs ainda rodando (refresh em 2º plano?): série inválida, repita' }
   ```
   Rode `. "<S>\perf-env.ps1"; pwsh -File "<S>\ab-cmd.ps1" -Base "<S>\perf76-base.exe" -New "<S>\perf76-new.exe" -CmdArgs '<args>' [-Span '<regex>'] -Runs 15` e cole a saída **inteira** (resumo e brutos).
7. **Decisão**, com a métrica e o limiar `L` do item **Reversão** desta tarefa. Fica se **todas**: (1) mediana do delta pareado (nova − base) da métrica ≤ −L; (2) pior caso: `max(nova) ≤ max(base)` na métrica — se só (2) falhar, rode mais uma série; falhou de novo ⇒ reverte; (3) as não-regressões e equivalências da tarefa. Senão: `git revert --no-edit <sha da otimização>` (spans e testes que continuam válidos ficam) e o relatório diz "sem ganho medido" com os números colados. Número que você não mediu é escrito "não medido".
8. **Limpeza.** Apague `<S>\la*`, `<S>\pristine*`, `<S>\tgt-base-76`, `<S>\perf76-*.exe`, `<S>\sc`; `git worktree list` sem `wt-base-76`; `reg query HKCU\Software\wingetrs-test\perf` ⇒ chave inexistente (nenhum comando medido escreve no ambiente); todo `python` que você iniciou, encerrado pelo PID que você guardou; `pwsh -File scripts/leftovers.ps1` limpo.

**Files:**
- Modify: `src/update.rs`
- Test: `tests/update.rs` e `#[cfg(test)] mod tests` novo em `src/update.rs` (a função e o `Fetched` são privados)

Testes:
1. `needs_catalog_only_for_imports_or_missing_index` (`src/update.rs`): `[None, Some(Ok(Fetched::NotModified)), Some(Err(("x".into(), true)))]` com índice ⇒ `false`; o mesmo sem índice ⇒ `true`; `[Some(Ok(Fetched::Scoop(BucketImport { packages: vec![], skipped: vec![] }, HttpEntry::default())))]` com índice ⇒ `true`; idem com `Fetched::Winget(WingetImport { packages: vec![], maps: WingetMaps::default() }, HttpEntry::default())`.
2. `fresh_update_touches_neither_http_json_nor_the_catalog` (`tests/update.rs`): wiremock com os mocks `.expect(0)`; `http.json` com todas as URLs dos `Endpoints` com `checked_at = now_unix()` (via `HttpState::set` + `save`); `search.idx` presente (`index_file::write(&Catalog::open_in_memory().unwrap(), &data.search_index_path())`); mtime do `http.json` ajustado para `UNIX_EPOCH + 1_000_000 s`; **sem** `catalog.db`; `run_update(.., force = false, ..)` ⇒ todos `Outcome::Fresh`, mtime inalterado e `!data.catalog_path().exists()`.
3. `not_modified_update_still_refreshes_checked_at` (`tests/update.rs`): wiremock respondendo 304 para tudo e `http.json` com `checked_at = 1` ⇒ todos `Outcome::Unchanged` e o `checked_at` de cada URL, lido de volta com `HttpState::load`, > 1. (Se já existir teste equivalente, cite-o em vez de duplicar.)

#### Checks beyond the steps

1. Abertura condicional: `needs_catalog` devolvendo sempre `true` ⇒ PEGA (testes 1 e 2).
2. Índice ausente ainda força abrir: `|| !index_exists` removido ⇒ PEGA (teste 1).
3. Sem gravação à toa: `state.save` incondicional de volta ⇒ PEGA (teste 2).
4. 304 ainda grava: condição de gravação só com `Outcome::Updated { .. }` ⇒ PEGA (teste 3).

**Commit:** `perf(update): open the catalog and save http state only when something changed` (precedido de `perf(update): span for catalog open`)

---

### Task 77: Gravação do catálogo — índices dos mapas reconstruídos uma vez por import do winget

**Modelo:** sonnet (patch pronto no relatório de investigação; revisor opus: DDL dentro da transação).

#### Where this fits

Depois da Task 75 (mesma função `delete_source_rows`/`replace_source`) e da Task 55 (feita: esvaziar de uma vez os mapas que são todos da fonte). `update.catalog_write{source=winget}` (`src/update.rs:303`, corpo de `apply`) mede ~1,06–1,2 s no `update` real (log de 2026-10-12: 1,20 s); ~70 % vai para as tabelas `map_*` (146 059 linhas, 5 índices em `key`, `src/catalog/mod.rs:40-44`). Investigação de 2026-10-11 (harness com o `source2.msix` real, A/B intercalado sob carga, deltas da mesma bateria):
- Inserir as 146 mil linhas mantendo os 5 índices linha a linha custa ~500 ms; derrubar os índices dentro da transação e recriá-los no fim (build ordenado) tirou **−213 ms** a mais sobre truncate+cache (bateria ab3, modo "replace") e **−303 ms** sozinho (ab1); no catálogo vazio, −239/−241 ms.
- `cache_size` de 64 MiB durante o import: −43 ms (ab3), −304 ms (ab1, ruidoso). Fica num **commit separado**, medido à parte.
- Medido e descartado (não implementar): `synchronous=NORMAL`, inserts ordenados por chave, `ATTACH` + `INSERT … SELECT`. Fora do escopo: mapas `WITHOUT ROWID` (exige migração de schema; não há `user_version`).
- Equivalência no protótipo: digest SHA-256 de `packages` com rowid, dos 5 mapas como multiconjunto `(key, package)`, de `meta` e do schema iguais em todas as variantes; `integrity_check` ok.

Riscos: DDL é transacional no SQLite — erro no meio (ex.: `DanglingMap`) faz rollback e os índices voltam (precisa de teste). Leitores WAL abertos (TUI) ficam no snapshot antigo; depois do commit o schema cookie muda e o `sqlite3_prepare_v2` reprepara sozinho (inclusive statements em cache da Task 71; precisa de teste). Rebuild num import do **scoop** (sem mapas) custaria 100–300 ms de `CREATE INDEX` por bucket ⇒ só reconstruir quando o import **tem** mapas.

#### What binds this task

- **Commit 1 (índices):**
  - Os 5 `CREATE INDEX` saem de `SCHEMA` para `const MAP_INDEXES: &str` (mesmo texto, mesmos nomes), executada logo depois de `SCHEMA` em `Catalog::open` (`:258-268`) e `open_in_memory` (`:315-319`). `const DROP_MAP_INDEXES: &str = "DROP INDEX IF EXISTS ix_pc; DROP INDEX IF EXISTS ix_uc; DROP INDEX IF EXISTS ix_pfn; DROP INDEX IF EXISTS ix_nn; DROP INDEX IF EXISTS ix_np;"`. Schema final igual ao de hoje.
  - Função pura `pub(crate) fn rebuilds_map_indexes(maps: &WingetMaps) -> bool`: `true` se qualquer um dos 5 vetores não estiver vazio.
  - `replace_source` delega para `fn replace_source_impl(&mut self, source, kind, source_version, fetched_at, packages, maps, rebuild: bool)`; o público passa `rebuild = rebuilds_map_indexes(maps)`. Com `rebuild`: `tx.execute_batch(DROP_MAP_INDEXES)` **dentro da transação, antes** de `delete_source_rows`, e `tx.execute_batch(MAP_INDEXES)` **dentro da transação, depois** do upsert em `meta` e antes do `commit`. Qualquer erro antes do commit desfaz tudo, inclusive o `DROP` (o drop de `tx` já faz rollback). Sem `rebuild`, nada muda em relação a hoje.
  - `remove_source`, `lookup`, `ImportStats` e o formato do arquivo não mudam.
- **Commit 2 (`cache_size`, decidido à parte):** em `replace_source_impl`, antes da transação, `let prev: i64 = self.conn.query_row("PRAGMA cache_size", [], |r| r.get(0))?;` e `self.conn.pragma_update(None, "cache_size", -65536i64)?;`; depois do commit **ou do erro**, restaurar `prev` (um guard com `Drop` ou o resultado guardado antes do `?`; o erro original tem precedência sobre um erro da restauração). Memória: até 64 MiB a mais durante o import (registre).
- **Commit 3 (índice `lower(id)`, decisão do usuário 2026-10-12, opção A):** a Task 73 (commit `2348ee9`, revertido em `8634bf2`; relatório `.claude/tasks/task-73-report.md`) criou `CREATE INDEX IF NOT EXISTS ix_pkg_id_lower ON packages(lower(id));` mantido linha a linha: span `show.find_by_id` 14,6 → 0,2 ms, wall do `show` −15,8 ms, mas `update.catalog_write{source=winget}` +104 ms (+13,9 %) ⇒ revertido. Agora: o índice entra no mesmo conjunto reconstruído do commit 1 — acrescente-o a `MAP_INDEXES` (renomeie a constante para `REBUILT_INDEXES` e `DROP_MAP_INDEXES` para `DROP_REBUILT_INDEXES`, incluindo `DROP INDEX IF EXISTS ix_pkg_id_lower;`), de modo que no import do winget ele seja derrubado antes e construído uma vez no fim, dentro da mesma transação. Import sem mapas (scoop) não reconstrói: o índice é mantido linha a linha (~4 mil linhas por bucket). Recupere de `2348ee9` a consulta que usa o índice (`git show 2348ee9 -- src/catalog/mod.rs`) e os 3 testes (`git show 2348ee9 -- tests`), adaptando ao código atual (Task 71 usa `prepare_cached`).
- **Medição** (protocolo abaixo; a tarefa usa o seu próprio A/B de `update`). Três A/B: commit 1 contra a base (HEAD antes da tarefa), commit 2 contra o commit 1, commit 3 contra o último commit mantido (1 ou 2). Para o commit 3, além do A/B de `update`, um A/B de `show Microsoft.PowerToys --source winget` com o span `show.find_by_id` e o wall (reaproveite o script do relatório da Task 73 ou o `ab-cmd.ps1` da Task 70, mesmas regras: raízes copiadas, base/nova intercaladas, 15 execuções).
- **Reversão.** Commit 1: métrica = span `update.catalog_write{source=winget}`; fica se (1) delta pareado mediano ≤ **−100 ms**, (2) `max(nova) ≤ max(base)` no span — se só (2) falhar, mais uma série de 10; falhou de novo ⇒ reverte —, (3) digests do catálogo iguais e `integrity_check` = `ok` nas duas raízes, schema igual, `search.idx` com o mesmo hash, (4) série com scoop servido localmente: delta pareado mediano do span `update.catalog_write{source=scoop:main}` ≤ +10 ms (o rebuild não pode rodar para o scoop). Commit 2: mesmas regras com **−40 ms** em (1); falhou ⇒ `git revert` só do commit 2. Commit 3 fica se **todas**: (a) `show.find_by_id` delta pareado mediano ≤ −5 ms e wall do `show` ≤ −5 ms; (b) `update.catalog_write{source=winget}` delta pareado mediano ≤ **+20 ms** contra o último commit mantido; (c) `update.catalog_write{source=scoop:main}` (série `-ScoopLocal`) ≤ +10 ms; (d) digests de conteúdo iguais e `integrity_check` = `ok` (o digest de schema muda por causa do índice novo: confira que a única diferença é a linha `ix_pkg_id_lower`). Falhou ⇒ `git revert` só do commit 3.

#### Protocolo de medição (completo; não depende de nenhuma outra seção do plano)

`<S>` = caminho absoluto do scratchpad do agente (nunca dentro do repositório); rode da raiz do repositório; o shell da ferramenta não guarda variáveis entre chamadas (cada chamada abaixo é autossuficiente).

1. **Sozinho.** Nenhum outro agente, build ou teste rodando (o piso de processo varia 38–300 ms com a carga). `pwsh -File scripts/leftovers.ps1` antes e depois; processo de outro projeto: anote, não encerre. Só com o gate verde.
2. **Binários fixos** (base num worktree temporário; cópias fixas):
   ```powershell
   git worktree add --detach "<S>\wt-base-77" <sha da base>
   cargo build --release --quiet --manifest-path "<S>\wt-base-77\Cargo.toml" --target-dir "<S>\tgt-base-77"
   Copy-Item "<S>\tgt-base-77\release\wingetrs.exe" "<S>\ab77-base.exe"
   git worktree remove --force "<S>\wt-base-77"
   cargo build --release --quiet; Copy-Item target\release\wingetrs.exe "<S>\ab77-new.exe"
   ```
   Para o commit 2: `ab77-c1.exe` (build do commit 1, pelo mesmo worktree) e `ab77-c2.exe` (cabeça).
3. **Entrada fixa:** o mesmo `source2.msix` para todas as séries: `New-Item -ItemType Directory -Force "<S>\ab-msix" | Out-Null; if (-not (Test-Path "<S>\ab-msix\source2.msix")) { curl.exe -sSL -o "<S>\ab-msix\source2.msix" https://cdn.winget.microsoft.com/cache/source2.msix }`. Para a série com scoop: `Copy-Item tests\fixtures\scoop\Main-mini.tar.gz "<S>\ab-msix\main.tgz"; Copy-Item tests\fixtures\scoop\Main-mini.tar.gz "<S>\ab-msix\extras.tgz"`.
4. **A/B intercalado.** Salve `<S>\ab-update77.ps1`. Cada raiz (`LOCALAPPDATA` temporário) parte de uma **cópia** do catálogo real (o real só é lido); cada execução apaga `http.json` e o log da raiz e roda `update --force` (reimporta ⇒ `catalog_write`, `parse` e `search_idx` acontecem); ordem base/nova sorteada por rodada; 2 rodadas de aquecimento descartadas; servidor local do msix; scoop em porta morta (falha offline e mantém suas linhas) ou, com `-ScoopLocal`, servido pelo mesmo servidor:
   ```powershell
   param([string]$Base, [string]$New, [int]$Runs = 10, [int]$Port = 8765, [switch]$ScoopLocal)
   $ErrorActionPreference = 'Stop'
   [Threading.Thread]::CurrentThread.CurrentCulture = 'en-US'
   $S = '<S>'
   $spans = [ordered]@{
       catalog_write = 'update\.catalog_write\{source=winget\}'
       search_idx    = 'update\.search_idx'
       parse         = 'update\.fetch\{source=winget\}:update\.parse'
   }
   if ($ScoopLocal) { $spans['scoop_write'] = 'update\.catalog_write\{source=scoop:main\}' }
   $real = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'wingetrs'
   $roots = @{}
   foreach ($k in 'base', 'new') {
       $r = "$S\ab77-root-$k"
       if (Test-Path $r) { Remove-Item -Recurse -Force $r }
       $d = New-Item -ItemType Directory -Force "$r\wingetrs"
       python -I -c "import pathlib,sqlite3,sys; s=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro', uri=True); d=sqlite3.connect(sys.argv[2]); s.backup(d); d.close(); s.close()" "$real\catalog.db" "$d\catalog.db"
       if ($LASTEXITCODE -ne 0) { throw 'cópia do catálogo falhou' }
       if (Test-Path "$real\config.toml") { Copy-Item "$real\config.toml" $d }
       $roots[$k] = $r
   }
   function Busy([string]$root, [string]$pat) {
       $log = Get-ChildItem "$root\wingetrs\log" -Filter 'wingetrs.log.*' | Sort-Object Name | Select-Object -Last 1
       $m = Select-String -Path $log.FullName -Pattern ($pat + '.*close time\.busy=([\d.]+)(ms|µs|s)\b') | Select-Object -Last 1
       if (-not $m) { throw "span $pat ausente em $root" }
       $v = [double]$m.Matches[0].Groups[1].Value
       switch -CaseSensitive ($m.Matches[0].Groups[2].Value) { 'µs' { $v / 1000 } 's' { $v * 1000 } default { $v } }
   }
   $srv = Start-Process python -ArgumentList '-I', '-m', 'http.server', "$Port", '--bind', '127.0.0.1', '--directory', "$S\ab-msix" -PassThru -WindowStyle Hidden
   $orig = $env:LOCALAPPDATA
   $env:WINGETRS_WINGET_INDEX_URL = "http://127.0.0.1:$Port/source2.msix"
   $env:WINGETRS_SCOOP_BASE_URL = if ($ScoopLocal) { "http://127.0.0.1:$Port" } else { 'http://127.0.0.1:9' }
   $exes = @{ base = $Base; new = $New }
   $ms = @{}
   foreach ($k in 'base', 'new') { $ms[$k] = @{}; foreach ($s in @($spans.Keys) + 'wall') { $ms[$k][$s] = @() } }
   try {
       Start-Sleep -Milliseconds 700
       for ($i = 0; $i -lt $Runs + 2; $i++) {
           foreach ($k in @('base', 'new') | Sort-Object { Get-Random }) {
               $env:LOCALAPPDATA = $roots[$k]
               Remove-Item -Force -ErrorAction SilentlyContinue "$($roots[$k])\wingetrs\http.json"
               Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$($roots[$k])\wingetrs\log"
               $exe = $exes[$k]
               $sw = [Diagnostics.Stopwatch]::StartNew()
               & $exe update --force *> $null
               $sw.Stop()
               $env:LOCALAPPDATA = $orig
               if ($i -ge 2) {
                   foreach ($s in $spans.Keys) { $ms[$k][$s] += Busy $roots[$k] $spans[$s] }
                   $ms[$k]['wall'] += $sw.Elapsed.TotalMilliseconds
               }
           }
       }
   } finally {
       $env:LOCALAPPDATA = $orig
       Remove-Item Env:WINGETRS_WINGET_INDEX_URL, Env:WINGETRS_SCOOP_BASE_URL -ErrorAction SilentlyContinue
       Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue
   }
   function Med($a) { $s = @($a | Sort-Object); $s[[int][math]::Floor($s.Count / 2)] }
   foreach ($s in @($spans.Keys) + 'wall') {
       $b = $ms.base[$s]; $n = $ms.new[$s]
       $d = for ($j = 0; $j -lt $b.Count; $j++) { $n[$j] - $b[$j] }
       '{0,-14} base med={1,8:N1} max={2,8:N1} | nova med={3,8:N1} max={4,8:N1} | delta pareado med={5,8:N1} ms' -f $s, (Med $b), ($b | Measure-Object -Maximum).Maximum, (Med $n), ($n | Measure-Object -Maximum).Maximum, (Med $d)
       "  brutos base: $($b -join ' ')"; "  brutos nova: $($n -join ' ')"
   }
   'search.idx igual: ' + ((Get-FileHash "$($roots.base)\wingetrs\search.idx").Hash -eq (Get-FileHash "$($roots.new)\wingetrs\search.idx").Hash)
   ```
   Rodadas: `pwsh -File "<S>\ab-update77.ps1" -Base "<S>\ab77-base.exe" -New "<S>\ab77-new.exe" -Runs 10` (decide o commit 1) e `pwsh -File "<S>\ab-update77.ps1" -Base "<S>\ab77-base.exe" -New "<S>\ab77-new.exe" -Runs 10 -ScoopLocal` (só o item (4) da reversão: `scoop_write`). Para o commit 2: as mesmas duas com `ab77-c1.exe`/`ab77-c2.exe`. Cole a saída inteira de cada rodada.
5. **Equivalência do conteúdo** (as duas raízes tiveram a mesma história: mesmo msix, mesmo número de imports). Salve `<S>\catdigest.py` e rode `python -I "<S>\catdigest.py" "<S>\ab77-root-base\wingetrs\catalog.db" "<S>\ab77-root-new\wingetrs\catalog.db"` depois de **cada** rodada ⇒ as duas linhas com o mesmo digest de conteúdo, o mesmo digest de schema e `ok` (`fetched_at` fica de fora: é o relógio de cada execução):
   ```python
   import hashlib, pathlib, sqlite3, sys

   MAPS = ("map_productcode", "map_upgradecode", "map_pfn", "map_norm_name", "map_norm_publisher")

   def digest(path):
       c = sqlite3.connect(pathlib.Path(path).as_uri() + "?mode=ro", uri=True)
       h, sh = hashlib.sha256(), hashlib.sha256()
       def feed(hh, sql):
           for row in c.execute(sql):
               hh.update(repr(row).encode())
           hh.update(b"|")
       feed(h, "SELECT rowid, * FROM packages ORDER BY rowid")
       for t in MAPS:  # multiset: the map row's own rowid is not observable (lookup orders by p.rowid)
           feed(h, f"SELECT key, package FROM {t} ORDER BY key, package")
       feed(h, "SELECT source, kind, source_version FROM meta ORDER BY source")
       feed(sh, "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
       ok = c.execute("PRAGMA integrity_check").fetchone()[0]
       c.close()
       return h.hexdigest(), sh.hexdigest()[:16], ok

   for p in sys.argv[1:]:
       print(p, *digest(p))
   ```
6. **Decisão:** regras do item **Reversão** acima. Reverter = `git revert --no-edit <sha do commit>`; ledger com os números colados.
7. **Limpeza:** apagar `<S>\ab77-root-*`, `<S>\tgt-base-77`, `<S>\ab77-*.exe` (o `<S>\ab-msix` pode ficar para outras tarefas); `git worktree list` sem `wt-base-77`; `Get-Process python -ErrorAction SilentlyContinue` sem o servidor que você iniciou; `leftovers.ps1` limpo.

**Files:**
- Modify: `src/catalog/mod.rs`
- Test: `tests/catalog.rs` e `mod tests` de `src/catalog/mod.rs`

Testes:
1. `rebuilds_map_indexes_only_for_imports_with_maps` (unitário): `WingetMaps::default()` ⇒ `false`; só `norm_publishers` com um par ⇒ `true`; só `productcodes` com um par ⇒ `true`.
2. `rebuilt_import_equals_the_import_that_keeps_indexes` (unitário; equivalência obrigatória — dois caminhos para o mesmo resultado): dois `Catalog::open_in_memory()`; nos dois, o mesmo histórico: `winget_only` (helper existente) e depois `replace_source_impl("winget", Winget, "v2", 2, &[pkg("W.2"), pkg("W.4")], &maps(&["W.2", "W.4"]), rebuild)` com `rebuild = true` num e `false` no outro ⇒ `content(&a.conn) == content(&b.conn)` (helper existente: pacotes com rowid, mapas como multiconjunto, `meta`, `integrity_check`) e `SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name` igual nos dois. Repita a partir de catálogo vazio (1º import).
3. `replace_with_maps_keeps_the_schema_of_a_fresh_catalog` (`tests/catalog.rs`): catálogo em disco (tempdir); `SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name` logo depois de `Catalog::open` e de novo depois de um `replace_source` com mapas nas 5 tabelas (por uma `rusqlite::Connection` à parte) ⇒ iguais, e `EXPLAIN QUERY PLAN SELECT package FROM map_productcode WHERE key = 'x'` cita `ix_pc`.
4. `dangling_map_rollback_keeps_the_map_indexes` (`tests/catalog.rs`): catálogo em disco com um import válido com mapas; segundo import com mapa apontando para id inexistente ⇒ `Err(CatalogError::DanglingMap(_))`; os 5 `ix_*` continuam em `sqlite_master` e `lookup` do import válido continua respondendo.
5. `reader_sees_new_rows_after_an_index_rebuild` (`tests/catalog.rs`): catálogo em disco; leitor `Catalog::open_read_only` faz `lookup(Pfn, "pfn_a") == ["A.A"]`; escritor (`Catalog::open`) faz `replace_source` com `pfns = [("PFN_B", "A.A")]`; o mesmo leitor ⇒ `lookup(Pfn, "pfn_b") == ["A.A"]` e `lookup(Pfn, "pfn_a")` vazio (o statement do leitor sobrevive à troca de schema).
6. Commit 2: `import_restores_cache_size` (unitário): `PRAGMA cache_size` lido antes == lido depois de um `replace_source` com sucesso **e** depois de um que falha com `DanglingMap`.
7. Commit 3: os 3 testes de `2348ee9` adaptados, mais `rebuilt_import_keeps_the_lower_id_index` (`tests/catalog.rs`): depois de um `replace_source` com mapas (caminho de rebuild) e depois de um sem mapas (scoop), `sqlite_master` tem `ix_pkg_id_lower` e `EXPLAIN QUERY PLAN` da consulta de `find_by_id` cita `ix_pkg_id_lower`. O teste 2 (equivalência) e o teste 4 (rollback mantém os índices) passam a incluir `ix_pkg_id_lower`.
8. Regressão: `replace_keeps_map_rows_of_another_source`, `remove_source_keeps_other_sources_maps`, `dangling_map_rolls_back_whole_import`, `code_lookup_folds_case_in_both_directions`, `truncating_owned_map_tables_equals_the_per_row_delete` verdes sem mudança.

#### Checks beyond the steps

1. Índices recriados: remover o `tx.execute_batch(MAP_INDEXES)` do fim de `replace_source_impl` ⇒ PEGA (teste 3).
2. Scoop não reconstrói: `rebuilds_map_indexes` devolvendo sempre `true` ⇒ PEGA (teste 1).
3. Equivalência: no ramo `rebuild`, recriar só 4 dos 5 índices (tirar `ix_np` da string) ⇒ PEGA (teste 2, schema).
4. `DROP` dentro da transação: mover o `DROP_MAP_INDEXES` para antes de `transaction_with_behavior` (fora da transação) ⇒ PEGA (teste 4).
5. Commit 2: remover a restauração do `cache_size` ⇒ PEGA (teste 6).
6. Commit 3: tirar `ix_pkg_id_lower` de `REBUILT_INDEXES` (só o `DROP` continua) ⇒ PEGA (teste 7); trocar a consulta de `find_by_id` de volta para a forma sem `lower(id)` da base ⇒ PEGA (teste de `EXPLAIN QUERY PLAN`).

**Commit:** `perf(catalog): rebuild map indexes once per winget import`, (se ficar) `perf(catalog): larger page cache during imports` e (se ficar) `perf(catalog): lower(id) index rebuilt with the map indexes`

---

### Task 78: Investigação do "Acesso negado" (os error 5) intermitente sob carga

**Modelo:** opus (diagnóstico; a correção depende da evidência).

#### Where this fits

Independente das Tasks 70–77, mas **nunca em paralelo com tarefa que mede tempo** (a reprodução gera carga de propósito). Pendência `OPEN` no ledger: 4+ falhas sob carga que passam isoladas. Evidência já registrada nos relatórios:
- `task-50-report.md:24`: `install_executor`, passo `extrair [1]` ⇒ `E/S: Acesso negado. (os error 5)`.
- `task-66-report.md:18`: `install_ops::apply_skips_a_dependency_installed_meanwhile` (`tests/install_ops.rs:862-871`) ⇒ `E/S: Acesso negado. (os error 5)` na extração.
- `task-69-report.md:43`: `install_executor::failure_after_replacing_a_kept_version_restores_it` (`tests/install_executor.rs:1028-1070`) ⇒ "Acesso negado (os error 5)" ao extrair.
- `task-54-report.md:14`, `task-67-report.md:12`: `install_executor` "Acesso negado" e "index_file lock test" (nome exato não registrado).
- Ledger `idx-flaky` (8de8685): 1/200 "Acesso negado" real durante o `rename` do `search.idx` ⇒ leitura com 5 tentativas (`src/catalog/index_file.rs:392-411`, `read_retrying`) e `persist` com 5 tentativas (`:118-145`).

Código no caminho das falhas (HEAD `922a995`):
- Extração: `extract_all_with` (`src/install/extract.rs:164-204`) extrai para `.wingetrs-extract-<pid>-<seq>` ao lado do destino (`:192-197`) e chama `merge_into` (`:352-375`), que **renomeia a pasta recém-escrita** (`fs::rename(src, dest)`, `:354`) ou, com destino existente, `remove_file(&to)` + `rename` por arquivo (`:364-370`). Todo `io::Error` vira `ExtractError::Io(e.to_string())` por `From` (`:80-84`): **a mensagem perde a operação e o caminho**.
- Executor: `Step::Promote` renomeia a versão antiga para o lado e o staging para a versão (`src/install/executor.rs:595-614`); desfazer renomeia de volta (`:261`); `remove_any` (`:115-130`); `persist` (`:818`).
- Nomes temporários: extração e "aside" usam pid + contador atômico (`extract.rs:192-197`, `executor.rs:489-504`); downloads `dl-<pid>-<seq>.part` (`src/install/download.rs:125-129`); cada teste usa o próprio `tempfile::tempdir()` (`tests/install_executor.rs:202`, `tests/install_ops.rs:124`, `tests/index_file.rs:8`).
- `tests/index_file.rs:163-191` (`search_retries_while_the_file_is_briefly_locked`) depende de relógio: o leitor tem ~50 ms de tentativas e a thread que segura o arquivo dorme 5 ms antes de soltar — sob carga, a thread pode ficar fora da CPU por mais que isso.

Hipóteses a separar **com evidência** (nenhuma está provada):
- **H1 — scanner externo** (Windows Defender em tempo real, indexador) abre o arquivo recém-escrito; enquanto o handle existe, o `rename` da pasta que o contém (ou do próprio arquivo) falha com 5 (ou 32, sharing violation).
- **H1b — "delete pending"**: `remove_file` seguido de criar/renomear para o mesmo nome enquanto um handle externo mantém o arquivo antigo pendente de exclusão ⇒ 5.
- **H2 — handle nosso ainda aberto** na hora do `rename`/`remove` (um `File`, `ZipArchive` ou `GzDecoder` vivo).
- **H3 — temporários compartilhados** entre testes paralelos (mesmo caminho usado por dois testes).
- **H4 — só no `index_file`**: o teste de trava acima falha por relógio (ARMADILHAS: "Teste com limiar de relógio"), não por código de produção.

#### What binds this task

- **Passo 1 — instrumento (commit 1, permanente: também ajuda o usuário).** Erro de E/S da extração diz operação e caminho: helper `fn io_at(op: &'static str, path: &Path) -> impl FnOnce(io::Error) -> ExtractError` que produz `ExtractError::Io(format!("{op} {}: {e}", path.display()))` com `op` em português (`"renomear"`, `"apagar"`, `"criar pasta"`, `"criar arquivo"`), aplicado a toda chamada de `fs::*` em `merge_into`, `write_entry` e `extract_all_with`. O `From<io::Error>` fica para as leituras do arquivo compactado. Teste `extract_io_error_names_operation_and_path` (abaixo).
- **Passo 2 — sonda do mecanismo (sem commit; cole a saída).** Programa descartável em `<S>\probe78\` (crate própria, `cargo new`, só `std`; nunca no repositório) que, numa pasta temporária, cria `d\f.txt` e tenta `rename(d, d2)` com um handle aberto em `f.txt` em cada modo de compartilhamento do Windows (`share_mode` 0, `1|2` = leitura+escrita, `1|2|4` = +exclusão, via `std::os::windows::fs::OpenOptionsExt`), e também `remove_file(f)` + `File::create(f)` com handle `1|2|4` aberto. Tabela: modo ⇒ `Ok` ou `raw_os_error`. Isso diz quais erros um handle externo **pode** causar em cada operação do caminho (H1/H1b plausíveis ou não).
- **Passo 3 — reprodução (sem commit).** Binários de teste num target separado (`$env:CARGO_TARGET_DIR = "<S>\t78"; cargo test --no-run --quiet --test install_executor --test install_ops --test index_file`). Salve `<S>\eacces-loop.ps1` e rode-o duas vezes — carregado (`-Parallel 4`) e controle quieto (`-Parallel 1 -Threads 1`) — com o **mesmo** `-Rounds` (comece com 40; zero falhas carregado ⇒ suba para 120 e `-Parallel 6`, uma vez):
  ```powershell
  param([int]$Rounds = 40, [int]$Parallel = 4, [int]$Threads = 0, [string]$Tag = 'load')
  $ErrorActionPreference = 'Stop'
  $S = '<S>'; $repo = (Get-Location).Path
  $bins = foreach ($n in 'install_executor', 'install_ops', 'index_file') {
      Get-ChildItem "$S\t78\debug\deps\$n-*.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
  }
  $out = "$S\eacces-$Tag"; if (Test-Path $out) { Remove-Item -Recurse -Force $out }; New-Item -ItemType Directory $out | Out-Null
  $runs = 0; $fails = 0
  for ($r = 0; $r -lt $Rounds; $r++) {
      $procs = foreach ($b in $bins) {
          foreach ($p in 1..$Parallel) {
              $a = @('--quiet'); if ($Threads -gt 0) { $a += "--test-threads=$Threads" }
              Start-Process $b.FullName -ArgumentList $a -WorkingDirectory $repo -PassThru -WindowStyle Hidden `
                  -RedirectStandardOutput "$out\r$r-$($b.BaseName)-$p.out" -RedirectStandardError "$out\r$r-$($b.BaseName)-$p.err"
          }
      }
      $procs | Wait-Process
      foreach ($p in $procs) { $runs++; if ($p.ExitCode -ne 0) { $fails++ } }
  }
  "execuções=$runs falhas=$fails"
  Select-String -Path "$out\*.out" -Pattern '^---- .* stdout ----|os error (5|32)|Acesso negado|negado' | ForEach-Object { "$($_.Filename): $($_.Line.Trim())" } | Sort-Object -Unique
  ```
  Cole o resumo dos dois modos e as linhas de falha (nome do teste + mensagem com operação e caminho, graças ao passo 1). Antes e depois: `Get-MpComputerStatus | Select-Object RealTimeProtectionEnabled, AMRunningMode` (só leitura; se negar, registre). **Não** mude configuração do Defender nem crie exclusões.
- **Passo 4 — discriminador H1 × H2 (sem commit).** Worktree descartável `<S>\wt-78-diag` (de HEAD + commit 1), target `<S>\t78d`. Nele, cada `fs::rename`/`fs::remove_file`/`fs::remove_dir_all` dos arquivos citados acima passa por `diag(op, path, || ..)`: no erro 5 ou 32, repete a **mesma** operação a cada 5 ms por até 5 s, **sem o código avançar** (a thread fica presa ali), e anexa uma linha `op;caminho;código;ms_até_liberar_ou_TIMEOUT` em `%TEMP%\wingetrs-diag-<pid>.txt`. Rode o laço do passo 3 com esses binários. Leitura: liberou sozinho em `t` ms sem o nosso código andar ⇒ o dono do handle é **outro** (H1/H1b, ou H3 se o caminho é de outro teste); `TIMEOUT` ⇒ o handle é **nosso** e está aberto naquele ponto (H2): ache-o pelo caminho. Cole a distribuição (mín/mediana/máx de `t`, contagem por operação). H3: para cada caminho que falhou, mostre que ele está dentro do tempdir **do próprio teste** e liste (`grep -n`) toda construção de nome temporário de `src/install` e `src/cache` — nome sem pid+seq dentro de pasta compartilhada é H3.
- **Passo 5 — correção conforme a evidência** (commit 2; nada de correção sem os passos 3–4 colados):
  - **H1/H1b** (externo): módulo novo `src/fs_retry.rs` (declarado em `src/lib.rs`): `pub fn is_transient(e: &io::Error) -> bool` (= `PermissionDenied` ou `raw_os_error() == Some(32)`, a mesma regra de `read_retrying`, que passa a usar esta função); `pub fn retry_with<T>(delays: &[Duration], f: impl FnMut() -> io::Result<T>, sleep: impl FnMut(Duration)) -> io::Result<T>` (núcleo testável: tenta, e a cada erro transitório chama `sleep(delays[i])` e tenta de novo; erro não transitório ou atraso esgotado ⇒ devolve o último erro); `pub fn retry<T>(op: &'static str, path: &Path, f: impl FnMut() -> io::Result<T>) -> io::Result<T>` = `retry_with(DELAYS, f, std::thread::sleep)` com `tracing::debug!(op, path = %path.display(), attempt, "transient fs error, retrying")` antes de cada espera e `tracing::warn!` ao desistir. `DELAYS`: escada dobrando a partir de 10 ms cujo total seja ≥ 2× o maior `t` medido no passo 4 e ≤ 2 s (justifique o número com a distribuição colada). Aplique a: `merge_into` (rename e remove), `TempGuard`/limpeza do temporário da extração, `Step::Promote` e o desfazer `RestoreAside`, `remove_any`, `persist` do executor, `BlobStore::put_verified` e `HttpState::save` (rename em `src/cache/mod.rs:148`, `:187`), os dois `rename` de `src/install/download.rs` (`:84`, `:149`) e o de `src/install/state.rs:240`. Os laços próprios de `index_file` (`persist`, `read_retrying`) ficam como estão (têm testes próprios), só trocando o predicado. Erro genuíno de permissão também é `PermissionDenied`: custa o total da escada e falha com a mesma mensagem — aceito e registrado.
  - **H2**: feche o handle antes da operação (drop explícito no ponto achado) e teste que, logo depois da função, cada arquivo do destino abre com `share_mode(0)` (exclusivo) — sem retry.
  - **H3**: nome único (pid + contador) no ponto achado e teste que dois usos simultâneos não colidem (barreira, sem relógio).
  - **H4**: reescreva `search_retries_while_the_file_is_briefly_locked` sem relógio: núcleo `fn read_retrying_with(path, sleep: impl FnMut(Duration))` em `index_file`, e um teste unitário em que o `sleep` injetado solta o handle exclusivo na 1ª chamada (determinístico); o teste de integração por relógio sai.
  - Hipótese sem evidência ⇒ **sem** correção para ela.
- **Passo 6 — prova depois da correção:** o mesmo laço carregado do passo 3, com o mesmo `-Rounds` e `-Parallel`, sobre os binários da cabeça. Critério: o modo carregado da base teve **≥ 5 falhas** e a cabeça **0** no mesmo N (se a base teve < 5, aumente N até ter, ou registre "taxa baixa demais para concluir" e `DONE_WITH_CONCERNS`). E o gate (`scripts/verify.ps1`) 3× seguidas verde.
- **Sem reprodução** (0 falhas carregado depois de subir para 120 rodadas): **nenhuma correção de produção**; fica só o commit 1 (instrumento), relatório `DONE_WITH_CONCERNS` com os números, a tabela da sonda do passo 2 e a recomendação; a pendência continua `OPEN` no ledger (o orquestrador atualiza).
- **Limpeza:** apagar `<S>\t78`, `<S>\t78d`, `<S>\probe78`, `<S>\eacces-*`, `%TEMP%\wingetrs-diag-*.txt`; `git worktree remove --force "<S>\wt-78-diag"`; nenhum binário de teste vivo (`Get-Process install_executor-*, install_ops-*, index_file-* -ErrorAction SilentlyContinue` vazio); `leftovers.ps1` limpo.

**Files:**
- Modify (commit 1): `src/install/extract.rs`; Test: `tests/install_extract.rs`
- Create/Modify (commit 2, só o que a evidência pedir): `src/fs_retry.rs` (novo), `src/lib.rs`, `src/install/extract.rs`, `src/install/executor.rs`, `src/cache/mod.rs`, `src/install/download.rs`, `src/install/state.rs`, `src/catalog/index_file.rs`; Test: `#[cfg(test)] mod tests` em `src/fs_retry.rs`, `tests/install_extract.rs`, `tests/index_file.rs`
- Nunca no repositório: sonda, laço, worktree de diagnóstico.

Testes:
1. `extract_io_error_names_operation_and_path` (`tests/install_extract.rs`, `#[cfg(windows)]`): destino `out` já existe com `x.txt` aberto com `share_mode(0)` durante toda a chamada (o teste nunca solta o handle); `extract` de um zip com `x.txt` ⇒ `Err(ExtractError::Io(m))` com `m` contendo `"apagar"` e `"x.txt"`. Depois do commit 2 o erro só sai ao fim da escada de tentativas — continua determinístico.
2. `retry_with_succeeds_after_transient_errors` (`src/fs_retry.rs`): `f` falha 2× com `io::ErrorKind::PermissionDenied` e depois `Ok(7)`; `sleep` grava os atrasos ⇒ `Ok(7)` e atrasos `== [DELAYS[0], DELAYS[1]]`.
3. `retry_with_gives_up_after_the_last_delay`: `f` sempre `PermissionDenied` ⇒ `Err` com esse `kind` e atrasos `== DELAYS` (todos, nenhum a mais).
4. `retry_with_does_not_retry_other_errors`: `f` devolve `NotFound` ⇒ `Err(NotFound)` e nenhum atraso; `raw_os_error 32` (`io::Error::from_raw_os_error(32)`) ⇒ conta como transitório.
5. `rename_of_a_dir_with_an_open_child_waits_for_release` (`src/fs_retry.rs`, `#[cfg(windows)]`, sistema de arquivos real): `tmp/a/f.txt` aberto no modo que o passo 2 mostrou bloquear o `rename` da pasta; `retry_with(DELAYS, || fs::rename(a, b), |_| { held.take(); })` ⇒ `Ok` e `b/f.txt` existe; com `sleep` que não solta ⇒ `Err` com `raw_os_error() == Some(<código do passo 2>)`. Determinístico: quem solta é o próprio `sleep` injetado.
6. `extraction_retries_a_held_file_until_it_is_released` (`tests/install_extract.rs`, `#[cfg(windows)]`): como o teste 1, mas o handle fica num `Arc<Mutex<Option<File>>>` e uma camada `tracing` de teste (subscriber global instalado uma vez + `thread_local!` por teste, como `tests/spans/mod.rs` `Closed::install`) solta o handle no `on_event` cuja mensagem é `"transient fs error, retrying"` ⇒ `extract` `Ok`, `x.txt` com o conteúdo novo e o `Option` vazio (o caminho de retry rodou). Sem relógio.
7. (Só se H4) `read_retrying_survives_a_lock_released_by_the_retry` (unitário em `src/catalog/index_file.rs`), no molde do teste 5.

#### Checks beyond the steps

1. Instrumento: em `merge_into`, a chamada com `io_at("apagar", ..)` trocada por `?` puro ⇒ PEGA (teste 1).
2. Retry ligado na produção: em `merge_into`, `fs_retry::retry("apagar", &to, || fs::remove_file(&to))` → `fs::remove_file(&to)` ⇒ PEGA (teste 6).
3. Escada inteira: em `retry_with`, `for d in delays` → `for d in delays.iter().take(1)` ⇒ PEGA (teste 3).
4. Só transitórios: `is_transient` devolvendo `true` para tudo ⇒ PEGA (teste 4).
5. Prova estatística (passo 6) colada: base ≥ 5 falhas, cabeça 0 no mesmo N; sem ela, a correção não é aceita.
6. Mutação "PEGA" só conta com o nome do teste na saída (target separado: `$env:CARGO_TARGET_DIR`), nunca por falha de build (ARMADILHAS).

**Commit:** `fix(install): extraction io errors name the operation and path` e (conforme a evidência) `fix(install): retry renames and removals blocked by a transient external handle`

## Manutenção (Tasks 80–81)

Pedido do usuário (2026-10-13): limpar `blobs/` e fazer a TUI abrir o catálogo só para leitura. As duas tarefas tocam arquivos disjuntos e rodam em paralelo; nenhuma mede tempo.
- **Task 80:** `src/tui/run.rs`, `src/tui/jobs.rs` e os testes da TUI.
- **Task 81:** `src/cache/mod.rs`, `src/install/download.rs`, `src/update.rs` e os testes de cache, download e update.

### Task 80: A TUI lê o catálogo só para leitura

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

A TUI chama `Catalog::open` em cinco lugares que só leem. Essa função abre o banco para leitura e escrita e roda o DDL de `SCHEMA` a cada chamada. Os cinco lugares:
- `src/tui/run.rs` `index_age` (~:153): idade do catálogo na abertura.
- `src/tui/run.rs`, carga dos instalados (~:488, `correlate`).
- `src/tui/run.rs` `load_details` (~:529): roda **a cada seleção** de item.
- `src/tui/jobs.rs` `read_catalog` (~:88): carga da lista de pacotes.
- `src/tui/run.rs` `with_ops` (~:601): planejamento e execução de operações. `OpsCtx` só lê o catálogo.

As únicas escritas no catálogo estão em `update.rs` `apply`, `buckets.rs` `remove_source` e no import de `cli/mod.rs`. Confira com `grep -rn "replace_source\|remove_source" src`.

O DDL pega o lock de escrita do SQLite e compete com um `update` em andamento, ficando ocupado até 5 s. `Catalog::open_read_only` (`src/catalog/mod.rs:291`) não roda DDL. Ela usa `open_worker_connection`: `SQLITE_OPEN_READ_ONLY | NO_MUTEX`, `busy_timeout` de 5 s e `mmap_size`. A CLI já usa `open_read_only` no caminho barato (`src/cli/mod.rs` `ensure_catalog_ran`, ~:470). Armadilha em `docs/ARMADILHAS.md`: "`Catalog::open` roda DDL do schema; para leitura use `open_read_only`".

#### What binds this task

- Os cinco lugares acima passam a abrir o catálogo só para leitura, por um helper único em `src/tui/jobs.rs`:
  ```rust
  /// Opens the catalog for reading without DDL or a write lock; a missing file reads as empty.
  pub fn open_catalog_for_reading(path: &Path) -> Result<Catalog, CatalogError> {
      if path.exists() { Catalog::open_read_only(path) } else { Catalog::open_in_memory() }
  }
  ```
  `run.rs` importa esse helper de `jobs.rs`.
- **Catálogo ausente** (primeira execução, ou arquivo apagado): `Catalog::open` criava o arquivo vazio, e `open_read_only` falharia. O helper lê um catálogo vazio em memória e nunca cria o arquivo. O comportamento visível não muda:
  - `index_age` devolve `None`;
  - `read_catalog` devolve uma lista vazia;
  - a carga dos instalados correlaciona contra um catálogo vazio;
  - `load_details` devolve o mesmo erro de "pacote não encontrado" de hoje.
- **Arquivo existente e ilegível** continua sendo erro, nunca lista vazia (o comentário de `read_catalog` já diz isso).
- Nada mais muda: mesma ordem de chamadas, mesmos `spawn_blocking`, mesmos erros.

**Files:**
- Modify: `src/tui/run.rs`, `src/tui/jobs.rs`
- Test: no arquivo onde já houver testes de `read_catalog` (procure com `grep -rn "read_catalog" tests src`). Se não houver, use `tests/tui_jobs.rs` (novo).

Testes (catálogo em disco num `tempdir`):
1. `reading_the_catalog_never_writes_it`:
   - Crie o catálogo com `Catalog::open` e um `replace_source` de 2 pacotes, e feche.
   - Marque `catalog.db` como só leitura: `let mut p = fs::metadata(&db)?.permissions(); p.set_readonly(true); fs::set_permissions(&db, p)?`.
   - `read_catalog(&db)` devolve os 2 pacotes, e `open_catalog_for_reading(&db)?.get(source, id)` devolve o pacote.
   - Restaure a permissão no fim, antes de qualquer `assert`, com um guard `Drop`, para o tempdir ser apagado.
   - O caminho antigo (`Catalog::open`) falha com o arquivo só leitura; é isso que a mutação 1 prova.
2. `reading_a_missing_catalog_is_empty`: caminho inexistente. `read_catalog` devolve `Ok(vec![])`, e o arquivo **continua** inexistente (hoje seria criado).
3. `reading_a_corrupt_catalog_is_an_error`: `catalog.db` com 4096 bytes de lixo. `read_catalog` devolve `Err(_)`.

#### Checks beyond the steps

1. Helper voltando a `Catalog::open(path)` no ramo de arquivo existente ⇒ PEGA (teste 1).
2. Helper sem o ramo `!path.exists()`, ou seja, `Catalog::open` para arquivo ausente ⇒ PEGA (teste 2: o arquivo passa a existir).
3. `read_catalog` devolvendo vazio em qualquer erro ⇒ PEGA (teste 3).
4. Um dos cinco lugares ainda com `Catalog::open`: `grep -n "Catalog::open(" src/tui` vazio. Cole a saída no relatório.

**Commit:** `fix(tui): open the catalog read-only for reads`

---

### Task 81: Limpeza de `blobs/` por idade de uso

**Modelo:** sonnet (revisor opus: apaga arquivos e concorre com a instalação).

#### Where this fits

`%LOCALAPPDATA%\wingetrs\blobs\` (`DataDir::blobs_dir`, `src/cache/mod.rs:80`) guarda arquivos endereçados pelo SHA-256 do conteúdo: 64 caracteres hex minúsculos, sem extensão. São três tipos:
- **Manifestos e `versionData` do winget.** Gravados por `BlobStore::put_verified` (`src/cache/mod.rs:176`) e usados por `src/sources/winget/details.rs` (Task 70).
- **Downloads de instalação do Scoop** (`src/install/download.rs` `download_one`, ~:117).
  - Com hash SHA-256, o nome do arquivo é esse hash.
  - Com outro algoritmo, o nome é o SHA-256 do conteúdo, e `blobs/index.json` mapeia `"<algo>:<hex>"` para o nome (`read_index`/`write_index`, ~:68-89).
  - `cached` (~:92) lê via `BlobStore::get`.
- **Temporários que sobram quando o processo morre:** `<hash>.tmp` (de `put_verified`) e `index.json.<pid>-<seq>.tmp` (de `write_index`).

Nada apaga esses arquivos. Hoje são 44 arquivos e 10,4 MB:
- 3 downloads de 1,8 a 5,5 MB;
- o resto são manifestos de 5 a 19 KB.

A pasta cresce cerca de 5 KB por pacote consultado a cada versão do catálogo, mais um arquivo por download.

Decisões do orquestrador (2026-10-13; o usuário pode mudar o número):
- **Política: idade desde o último uso, 30 dias.** Em `src/cache/mod.rs`: `pub const BLOB_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);`.
  - O último uso é o `mtime`. Uma gravação nova já tem `mtime` atual, e um `BlobStore::get` com acerto (hash conferido) atualiza o `mtime` para agora.
  - Consequência aceita: reinstalar offline um app cujo arquivo não é usado há 30 dias exige rede.
- **Quando:** no fim de `run_update` (`src/update.rs`), depois do `search_idx` e antes de `saved?`, em toda execução. A CLI e a TUI usam a mesma função. A chamada fica dentro de `tracing::debug_span!("update.blob_prune")`. O custo é um `read_dir` de dezenas de arquivos.
- **Concorrência:**
  - A limpeza só roda segurando o `install.lock` (`Layout::install_lock()`, `src/install/layout.rs:46`), obtido com `UpdateLock::try_acquire`, sem esperar.
  - Se estiver ocupado, pula com log `debug`; nunca espera nem faz o `update` falhar.
  - Imediatamente antes de apagar cada arquivo, releia o `mtime`. Se ficou novo, porque alguém usou o arquivo entre a listagem e a remoção, não apague.
- **A limpeza nunca faz o `update` falhar.** Qualquer erro dela vira `tracing::warn!`, e o `update` segue com o mesmo resultado.

#### What binds this task

- **`BlobStore::get`** (`src/cache/mod.rs:171`), no acerto: abre o arquivo com `OpenOptions::new().write(true)` e chama `File::set_modified(SystemTime::now())`.
  - Erro nesse toque é ignorado (log `debug`) e **não** muda o retorno.
  - Sem acerto (arquivo ausente ou hash errado), não toca.
- **Função nova em `src/cache/mod.rs`:**
  ```rust
  #[derive(Debug, Default, PartialEq, Eq)]
  pub struct PruneStats { pub removed: usize, pub bytes: u64, pub kept: usize }
  impl BlobStore {
      /// Removes content blobs unused for `max_age` and leftover temp files older than a day.
      pub fn prune(&self, now: SystemTime, max_age: Duration) -> Result<PruneStats, CacheError>
  }
  ```
  Regras:
  - Considera só entradas diretamente em `dir`, sem recursão.
  - Usa `DirEntry::file_type()`, que não segue symlink nem reparse point. Symlink ou diretório: ignora e conta em `kept`.
  - Blob: nome com exatamente 64 caracteres `[0-9a-f]`. Apaga se `now - mtime > max_age`.
  - Temporário: nome terminando em `.tmp`. Apaga se `now - mtime > 1 dia`.
  - `index.json` e qualquer outro nome nunca são apagados; contam em `kept`.
  - `mtime` no futuro (relógio adiantado): mantém.
  - Remoção que falha: mantém e segue, com log `debug`.
  - `dir` inexistente: `Ok(PruneStats::default())`.
- **Função nova em `src/install/download.rs`:** `pub fn prune_index(data: &DataDir) -> Result<usize, DownloadError>`.
  - Lê `index.json` com `read_index` e remove as entradas cujo arquivo `blobs/<nome>` não existe.
  - Regrava com `write_index` **só se** removeu alguma, e devolve quantas removeu.
  - `index.json` ausente ou corrompido: `Ok(0)`, sem gravar. Teste o corrompido lendo os bytes direto, sem o `unwrap_or_default` de `read_index`.
- **Em `src/update.rs`:** `fn prune_blobs(data: &DataDir)`, síncrona e sem propagar erro com `?`.
  - Chama `UpdateLock::try_acquire(&data.install_layout().install_lock())`. Se der `Ok(None)` ou `Err`, sai com log `debug`.
  - Senão chama `BlobStore::new(data.blobs_dir()).prune(SystemTime::now(), BLOB_MAX_AGE)` e depois `prune_index(data)`.
  - Registra `tracing::info!` com `removed`, `bytes`, `kept` e o número de entradas removidas do índice.
  - É chamada no fim de `run_update`, conforme as decisões acima.
- **Nada fora de `blobs_dir()` é tocado** (regra de escrita do CLAUDE.md). Os testes usam `tempdir` e `File::set_modified` para envelhecer arquivos, nunca o `%LOCALAPPDATA%` real.

**Files:**
- Modify: `src/cache/mod.rs`, `src/install/download.rs`, `src/update.rs`
- Test: o arquivo de testes de cache existente (`grep -rln "BlobStore" tests`), o de download (`grep -rln "index.json\|download_all" tests`) e `tests/update.rs`

Testes (helper `age(path, days)`, que abre com `write(true)` e faz `set_modified(now - days)`; `days` negativo = futuro):
1. `get_hit_refreshes_mtime`:
   - Blob válido envelhecido 40 dias: `get` devolve `Some`, e o `mtime` passa a ser ≥ o instante anterior à chamada.
   - Blob com conteúdo trocado: `get` devolve `None`, e o `mtime` fica inalterado.
2. `prune_removes_only_old_blobs_and_old_temps`. Em `dir`:
   - sai: blob válido de 40 dias;
   - fica: blob de 10 dias;
   - sai: `<hash>.tmp` de 2 dias;
   - fica: `<hash>.tmp` de 1 hora;
   - ficam: `index.json`, `notes.txt`, um nome de 63 hex e um subdiretório, todos de 400 dias.

   `prune(now, 30 dias)` deve dar `removed == 2`, `bytes` igual à soma dos dois e exatamente esse conjunto de nomes restantes.
3. `prune_keeps_future_mtime_and_missing_dir`:
   - um blob com `mtime` 1 dia no futuro fica;
   - `dir` inexistente dá `Ok(default)`, e o diretório continua inexistente.
4. `prune_index_drops_entries_without_blob`: `index.json` com duas entradas, uma com arquivo presente e outra ausente.
   - `prune_index` dá `Ok(1)`, e o índice é regravado só com a entrada presente.
   - Uma segunda chamada dá `Ok(0)`, e o conteúdo e o `mtime` do `index.json` não mudam. Antes da 2ª chamada, envelheça o `index.json` 1 dia para o `mtime` ser comparável.
5. `pruned_download_is_fetched_again`, com wiremock e o mesmo padrão dos testes de download existentes:
   - baixar um arquivo faz 1 GET;
   - envelhecer o blob 40 dias e rodar `prune` + `prune_index`;
   - pedir o mesmo arquivo de novo faz um 2º GET (`expect(2)`), e o arquivo volta para o lugar;
   - no controle sem envelhecer, o 2º pedido não faz GET.
6. `update_prunes_old_blobs` (`tests/update.rs`, mesmo arranjo dos testes de `run_update` com wiremock e raiz temporária): com um blob de 40 dias e outro de 1 dia em `blobs/`, depois do `run_update` só o de 1 dia existe.
7. `update_skips_prune_while_install_lock_is_held`: o mesmo, mas segurando `UpdateLock::try_acquire(install_lock)` durante o `run_update`. Os dois blobs continuam, e o `run_update` devolve `Ok` com os mesmos outcomes do teste 6.
8. `prune_failure_never_fails_update`: com `blobs` sendo um **arquivo**, não um diretório, `run_update` devolve `Ok` com os mesmos outcomes.

#### Checks beyond the steps

1. Idade: inverter a comparação de idade dos blobs ⇒ PEGA (teste 2).
2. Só 64 hex: aceitar como blob qualquer nome sem `.` ⇒ PEGA (teste 2).
3. Toque: remover o toque do acerto em `get` ⇒ PEGA (teste 1).
4. Lock: chamar a limpeza sem o `try_acquire` ⇒ PEGA (teste 7).
5. Índice: `prune_index` regravando sempre ⇒ PEGA (teste 4).
6. Releitura do `mtime` antes de apagar: não há teste determinístico simples. Cite no relatório a linha do código que faz a releitura; o revisor confere.

**Commit:** `feat(cache): prune blobs unused for 30 days after update`

## Ícones dos pacotes (Task 82: sonda)

Pedido do usuário (2026-10-13): mostrar o logotipo dos programas, como o winget faz com sixel, no painel de detalhes, em outros lugares adequados e, se der, como ícone na lista. O usuário aprovou a fase 1: uma sonda, sem tocar no código do produto. As fases 2 (ícone nos detalhes), 3 (ícone na lista, só do cache) e 4 (ícones locais dos apps instalados) serão decididas com os números da sonda e exigem emenda na spec da fase 1 (§7).

Fatos conhecidos:
- Manifestos do winget (esquema ≥ 1.4, manifesto de locale) podem ter `Icons:`, uma lista de `IconUrl`, `IconFileType` (png, ico, jpeg, svg), `IconResolution`, `IconTheme` e `IconSha256`. No cache do usuário, 15 de 41 blobs pequenos têm `IconUrl`; um deles é `Notepad++.Notepad++` (ico, IconSha256 `7f00c181…`).
- O projeto usa `ratatui = "0.30.2"` e `crossterm = "0.29.0"`. O terminal do usuário é o Windows Terminal 1.25.2733.0, que tem sixel desde a 1.22.
- Os manifestos do Scoop não têm ícone.

### Task 82: Sonda de ícones (sixel com ratatui-image)

**Modelo:** opus (decisões de viabilidade; revisor: o orquestrador lê o relatório).

#### Where this fits

Antes de adicionar dependências ao `Cargo.toml`, a regra do projeto exige provar o conjunto num crate de sonda: as versões das dependências foram verificadas juntas. Esta tarefa responde, com números, a cinco perguntas:
- (a) se `ratatui-image` compila com o ratatui 0.30.2 e o crossterm 0.29;
- (b) se a detecção de protocolo acha sixel no terminal do usuário;
- (c) quanto custa desenhar um ícone, por quadro e no primeiro desenho;
- (d) que fração dos pacotes do winget tem ícone, e de que tipos;
- (e) se `image` decodifica os tipos reais encontrados: ico, png, jpeg; svg provavelmente não.

#### What binds this task

- **Nada no repositório muda**, exceto o relatório `.claude/tasks/task-82-report.md`. Não toque em `Cargo.toml`, `Cargo.lock` nem `src/`.
- **Crate de sonda** em `<S>\icon-probe` (`<S>` = o scratchpad do agente, nunca dentro do repositório). Ele é binário e depende de:
  - `ratatui = "=0.30.2"` e `crossterm = "=0.29.0"`, as mesmas versões do projeto;
  - a versão mais recente de `ratatui-image` que declare compatibilidade com ratatui 0.30; se nenhuma declarar, registre isso e pare em BLOCKED com as versões vistas;
  - `image`, com só as features de png, ico, jpeg e gif.

  Registre as versões resolvidas e o tamanho do binário release com e sem as dependências de imagem (um binário mínimo com ratatui+crossterm como base).
- **Amostra de ícones (d):**
  - Copie o catálogo real para uma raiz temporária, com o mesmo método de cópia `python -I … backup` do protocolo da Task 77.
  - Escolha 200 ids do winget com semente fixa: os 50 primeiros por ordem alfabética entre os ids instalados (`wingetrs list` na cópia) e 150 sorteados com `random.seed(82)`.
  - Rode `target/release/wingetrs.exe show <id> --source winget` com `LOCALAPPDATA` apontando para a raiz temporária. Os manifestos ficam no `blobs/` dela.
  - Conte quantos têm `Icons:`, a distribuição de `IconFileType` e `IconResolution`, e quantos têm mais de um ícone (temas claro e escuro, resoluções).
  - Baixe até 30 ícones, confira o `IconSha256` e registre tamanho em bytes, dimensões e se `image` decodificou. Falha de decodificação: registre o tipo e o motivo.
  - Coloque um intervalo de ≥ 200 ms entre requisições, para não martelar o CDN nem os sites dos fabricantes.
- **Binário de demonstração (b, c)** no crate de sonda, para **o usuário** rodar no terminal dele, porque o agente não tem terminal interativo:
  - Detecta o protocolo com o `Picker` do `ratatui-image` (consulta ao terminal; fallback por variável de ambiente) e mostra o protocolo escolhido e o tamanho de fonte em pixels.
  - Desenha numa tela ratatui:
    - o ícone do Notepad++, que vem embutido no binário com `include_bytes!` de um ícone baixado e verificado;
    - à direita, o mesmo ícone em 2 linhas de altura, como seria na lista;
    - 10 linhas de lista fictícias com ícone de 2 linhas, rolando com ↑/↓.
  - Mede com `Instant`: primeiro desenho; mediana e máximo do tempo de `terminal.draw` em 200 quadros de rolagem automática; tempo de codificação do protocolo, se a API expuser.
  - Ao sair (`q`), imprime os números em texto. Também os grava em `<S>\icon-probe\last-run.txt` para o agente ler depois.
  - Sem sixel detectado: imprime "protocolo: <nome>" e desenha o fallback (meio-bloco), para comparação visual.
- **Entrega ao usuário:** o caminho do exe e um comando pronto para colar (`! & "<S>\icon-probe\target\release\icon-probe.exe"`). O agente **não** espera o usuário; o orquestrador cuida disso.
- **Relatório:**
  - versões e compatibilidade (a);
  - números da amostra (d, e);
  - tamanho do binário;
  - riscos vistos: sixel dentro de uma lista que rola (ratatui-image redesenha? pisca?), imagem sobre um diálogo, redimensionamento, troca de tema;
  - uma recomendação concreta para as fases 2 e 3, com a API do `ratatui-image` que seria usada: `StatefulImage`, `Resize` e onde guardar o protocolo codificado.
- **Limpeza:**
  - apague a raiz temporária e o catálogo copiado;
  - o crate de sonda e o exe **ficam** no scratchpad para o usuário rodar;
  - `leftovers.ps1` limpo.

**Files:**
- Create: `.claude/tasks/task-82-report.md`, que é o único arquivo do repositório tocado.

#### Checks beyond the steps

1. Nenhum arquivo do repositório além do relatório mudou: cole a saída de `git status --short`.
2. Cada número do relatório tem a saída de origem colada; número não medido é escrito "não medido".
3. Os `IconSha256` dos ícones baixados foram conferidos: cole quantos conferiram e quantos divergiram.

**Commit:** `chore(tasks): icon probe report for task 82` (só o relatório)

## Ícones dos pacotes (Tasks 83–86)

Decisões do usuário (2026-10-13): protocolo iTerm2 por padrão no WezTerm, que gastou menos recursos na sonda, com opção de troca nos Ajustes; dependências novas aprovadas; ícone nos detalhes e na lista (esta só a partir do cache); **nos detalhes, espaço entre o ícone e o texto**. A emenda está na spec da fase 1, §3.2 e §7.3 (2026-10-13), que é a fonte das regras. Sonda: relatório `.claude/tasks/task-82-report.md`; o código de exemplo da API do `ratatui-image` 11.1 está em `C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\icon-probe\src\main.rs` (somente leitura: copie o que servir, não dependa do arquivo).

Números da sonda no terminal do usuário (WezTerm, fonte 10×23 px, ícone .ico 256×256 de 186 KB):

| Medida | iTerm2 | sixel |
|---|---|---|
| Decodificar o ícone | 0,93 ms | 1,36 ms |
| Codificar um ícone pequeno | 0,2–0,3 ms | 0,5–0,8 ms |
| Primeiro desenho do ícone grande | 12 ms | 20 ms |
| Mediana por quadro, rolando 10 ícones | 3,6–3,9 ms | 5,4–6,2 ms |
| Bytes enviados por quadro | cerca de 44 KB | cerca de 56 KB |

Ordem: 83 → 84 → 85 → 86, em série. A 84, a 85 e a 86 mexem em `src/tui/app.rs`/`view.rs`. A 85 mede o tempo de quadro e roda sozinha.

### Task 83: Ícone no modelo de detalhes, baixado e guardado em cache

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

O manifesto mesclado do winget (`src/sources/winget/manifest.rs`, `MergedManifest`) ainda não lê `Icons`. Exemplo real (Notepad++): `Icons:` é uma lista de mapas com `IconUrl`, `IconFileType` (`ico`), `IconResolution` (`custom`), `IconTheme` (`default`) e `IconSha256` (64 hex). `details()` (`src/sources/winget/details.rs`) devolve `PackageDetails` (`src/core/mod.rs:67`). A TUI recebe os detalhes em `Msg::DetailsLoaded { key, result }` (`src/tui/app.rs`), e `load_details` fica em `src/tui/run.rs`. O `BlobStore` (`src/cache/mod.rs`) confere o SHA-256 na leitura e na gravação, e a Task 81 o limpa por idade.

#### What binds this task

- Em `manifest.rs`: `#[serde(default)] pub icons: Vec<Icon>` em `MergedManifest`, e `struct Icon { icon_url: String, icon_file_type: Option<String>, icon_resolution: Option<String>, icon_theme: Option<String>, icon_sha256: Option<String> }`, com `rename_all = "PascalCase"`.
- Em `src/core/mod.rs`: `pub struct IconRef { pub url: String, pub sha256: String }` e o campo `pub icon: Option<IconRef>` em `PackageDetails`. Os pacotes do Scoop ficam com `None`. Todo construtor de `PackageDetails` (procure com `grep -rn "PackageDetails {" src tests`) ganha `icon: None`, exceto o do winget.
- Função pura `pub fn pick_icon(icons: &[Icon]) -> Option<IconRef>`, em `manifest.rs`:
  - ignora ícone sem `icon_sha256`, ou cujo `icon_sha256` não tenha exatamente 64 caracteres hex (devolve o hash em minúsculas);
  - ignora URL que não comece com `https://`;
  - ordem de preferência: tema `default` (sem diferenciar maiúsculas; tema ausente conta como `default`), depois `dark`, depois qualquer outro;
  - empate: o primeiro na ordem do manifesto.
- `into_details` preenche `icon` com `pick_icon(&self.icons)`.
- Novo `src/icons.rs`, declarado em `lib.rs`:
  - `pub const MAX_ICON_BYTES: usize = 1024 * 1024;`
  - `pub async fn fetch(http: &Http, blobs: &BlobStore, icon: &IconRef) -> Result<Vec<u8>, IconError>`:
    - acerto em `blobs.get(&icon.sha256)` ⇒ devolve sem rede;
    - senão `http.get(&icon.url)`;
    - mais de `MAX_ICON_BYTES` ⇒ `IconError::TooLarge`;
    - grava com `blobs.put_verified(&icon.sha256, &b)`; `HashMismatch` ⇒ `IconError::Integrity`, sem gravar.
  - Mapa `src/icons.rs` ⇒ arquivo `icons.json` em `DataDir::root()`, com o formato `{"<source>/<id>": "<sha256>"}`:
    - `pub fn remember(data: &DataDir, source: &str, id: &str, sha: &str)`: lê, insere e grava com temporário único + `fs_retry` (mesmo padrão de `write_index` em `src/install/download.rs`). Erro vira `tracing::debug!`; a função não devolve erro.
    - `pub fn cached_icon(data: &DataDir, map: &BTreeMap<String,String>, source: &str, id: &str) -> Option<Vec<u8>>`: `BlobStore::get` do sha mapeado.
    - `pub fn load_map(data: &DataDir) -> BTreeMap<String,String>`: ausente ou corrompido devolve vazio.
  - `IconError` com `thiserror`: `Http`, `TooLarge`, `Integrity`, `Cache`.
- TUI (`src/tui/run.rs`, `load_details`): depois de obter os detalhes com `icon: Some(r)`, chama `icons::fetch`, junto e no mesmo `spawn`/tarefa dos detalhes, depois deles.
  - Sucesso ⇒ `icons::remember(..)` e o resultado vai junto no `Msg::DetailsLoaded`, num campo novo `icon: Option<Vec<u8>>` com os bytes brutos.
  - Falha ⇒ `None` e `tracing::debug!`. A falha nunca vira erro dos detalhes.
- `App` guarda os bytes em `pub icon_bytes: HashMap<String, Vec<u8>>`, chaveado como `details`, para a Task 84 desenhar. Nada é desenhado nesta tarefa.
- A CLI (`show`) não muda, nem o texto nem os snapshots.

**Files:**
- Create: `src/icons.rs`, `tests/icons.rs` (novos)
- Modify: `src/sources/winget/manifest.rs`, `src/core/mod.rs`, `src/lib.rs`, `src/tui/run.rs`, `src/tui/app.rs`, construtores de `PackageDetails` em `src/` e `tests/`
- Test: `tests/icons.rs` (novo), `mod tests` de `manifest.rs`

Testes:
1. `pick_icon_prefers_default_then_dark`: com [dark, light, default], escolhe o default; com [light, dark], o dark; com [light], o light; tema ausente conta como default.
2. `pick_icon_requires_https_and_a_valid_sha`: sem `IconSha256`, com sha de 63 hex, com sha não hex ou com `http://` ⇒ `None`; sha em maiúsculas ⇒ minúsculas.
3. `manifest_with_icons_parses`: YAML mesclado mínimo com o bloco `Icons` do Notepad++ (5 campos) ⇒ `details.icon == Some(IconRef { url, sha })`; manifesto sem `Icons` ⇒ `None`.
4. `fetch_caches_by_sha_and_rejects_mismatch`, com wiremock:
   - 1º `fetch` faz 1 GET e grava em `blobs/<sha>`; o 2º `fetch` não faz GET (`expect(1)`);
   - bytes com outro hash ⇒ `IconError::Integrity` e nada gravado;
   - corpo de `MAX_ICON_BYTES + 1` ⇒ `TooLarge` e nada gravado.
5. `icon_map_round_trips_and_survives_corruption`: `remember` + `load_map` + `cached_icon` devolve os bytes; `icons.json` com lixo ⇒ `load_map` vazio, e um `remember` posterior regrava válido; blob ausente ⇒ `cached_icon` = `None`.
6. `details_icon_failure_never_fails_details`: no teste de estado da TUI (`tests/tui_state.rs`), `Msg::DetailsLoaded` com `result: Ok(..)` e `icon: None` ⇒ detalhes presentes e `icon_bytes` sem a chave.

#### Checks beyond the steps

1. Preferência: trocar a ordem `default`/`dark` em `pick_icon` ⇒ PEGA (teste 1).
2. Integridade: gravar mesmo com `HashMismatch` ⇒ PEGA (teste 4).
3. Limite: tirar a checagem de `MAX_ICON_BYTES` ⇒ PEGA (teste 4).
4. Cache: buscar sempre na rede ⇒ PEGA (teste 4, `expect(1)`).

**Commit:** `feat(icons): pick, fetch and cache winget package icons`

---

### Task 84: Desenhar o ícone nos detalhes (ratatui-image, protocolo pelo ajuste)

**Modelo:** opus (revisor opus: dependência nova, thread/tarefa de codificação, sobreposição com diálogos).

#### Where this fits

Depois da Task 83: `App.icon_bytes` já tem os bytes brutos verificados. Os detalhes são desenhados por `draw_details` (`src/tui/view.rs`, ~:762), que hoje é um `Paragraph` de `details_lines` (~:678). Os espaçamentos vivem em `src/tui/layout.rs` ("no loose spacing numbers elsewhere"). A configuração fica em `src/config.rs`, `Config`, com `glyphs: String` como modelo de campo texto. A sonda achou: `Picker::from_query_stdio` cria thread que nunca é juntada e trava com stdin em EOF, por isso **é proibido usá-lo**.

#### What binds this task

- `Cargo.toml`: `ratatui-image = { version = "=11.1.0", default-features = false, features = [<as necessárias para crossterm, iterm2 e sixel; confira no Cargo.toml do crate 11.1.0>] }` e `image = { version = "<a que o ratatui-image 11.1.0 resolve>", default-features = false, features = ["png", "ico", "jpeg", "gif"] }`.
  - Rode `cargo build` e confira no `Cargo.lock` que o ratatui continua `0.30.2` e o crossterm `0.29.0`, sem `cargo update`.
  - Cole o diff do `Cargo.lock` resumido: crates novos e versões.
- `src/config.rs`: `pub icons: String`, com padrão `"auto"` e valores válidos `auto`, `iterm2`, `sixel`, `off`. Valor desconhecido ⇒ `auto` e `tracing::warn!`.
- Novo `src/tui/icons.rs`:
  - `pub enum IconProtocol { Iterm2, Sixel, Off }`.
  - `pub fn choose(setting: &str, term_program: Option<&str>, wt_session: bool) -> IconProtocol` (pura):
    - `iterm2`/`sixel`/`off` explícitos valem como estão;
    - `auto` ⇒ WezTerm (`TERM_PROGRAM` igual a `WezTerm`, sem diferenciar maiúsculas) dá `Iterm2`; `WT_SESSION` presente dá `Sixel`; qualquer outro caso dá `Off`.
  - `pub fn font_size() -> (u16, u16)`: `crossterm::terminal::window_size()` com `width`/`height` em pixels > 0 ⇒ `(px_w / cols, px_h / rows)`; senão `(10, 20)`.
  - Construção do `Picker` com `Picker::from_fontsize(font_size())` e o tipo de protocolo fixado (`set_protocol_type`), nunca por consulta.
- Codificação **fora da thread da UI**:
  - Os bytes são decodificados (`image::load_from_memory`) e convertidos no protocolo em `spawn_blocking`, com o handle aguardado (regra do projeto) e o pânico repassado com `resume_unwind`.
  - O resultado volta por `Msg::IconReady { key, size: (u16,u16), proto }`.
  - Falha de decodificação ⇒ `tracing::debug!` e a chave marcada como "sem ícone", sem tentar de novo nesta sessão.
  - Cache do protocolo codificado em `App`, chaveado por `(key, cols, rows)`, com no máximo 64 entradas. Ao passar disso, sai a mais antiga.
  - `Off` ⇒ nada é decodificado nem desenhado.
- `src/tui/layout.rs`:
  - `pub const DETAIL_ICON_ROWS: u16 = 4; pub const DETAIL_ICON_COLS: u16 = 8; pub const DETAIL_ICON_GAP: u16 = 2;` (pedido do usuário: espaço entre ícone e texto).
  - Uma função `pub fn split_icon(area: Rect, rows: u16, cols: u16, gap: u16) -> (Rect, Rect)` que devolve (área do ícone, área do texto à direita), com a área do texto começando `gap` colunas depois do ícone.
- `draw_details`, quando há ícone codificado para o pacote selecionado:
  - as primeiras `DETAIL_ICON_ROWS` linhas são divididas por `split_icon`;
  - à esquerda vai o ícone; à direita vão as primeiras linhas de `details_lines` (nome, versão, publisher; o que couber em 4 linhas);
  - as linhas restantes vão abaixo, na largura toda.
  - Sem ícone (pacote sem ícone, `Off`, ainda codificando ou área com menos de `DETAIL_ICON_COLS + DETAIL_ICON_GAP + 10` colunas): o layout fica exatamente como hoje, e os snapshots existentes não mudam.
- Sobreposição: com `app.modal.is_some()` ou `app.show_help`, nenhum ícone é desenhado. Um ícone cuja área cruza a área de um toast visível também não é desenhado.
- Desenho com o widget de imagem do `ratatui-image` (o `Image` com o protocolo já codificado, não o `StatefulImage` que codifica no desenho).
- Os testes de view usam `TestBackend`: o ícone não sai no buffer de texto, mas a área reservada sai. Os testes verificam por células, nunca por bytes de protocolo.

**Files:**
- Create: `src/tui/icons.rs`
- Modify: `Cargo.toml`, `Cargo.lock`, `src/config.rs`, `src/tui/mod.rs`, `src/tui/layout.rs`, `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/view.rs`
- Test: `mod tests` de `src/tui/icons.rs`, `tests/tui_view.rs`, `tests/tui_state.rs`, `tests/config.rs` (ou onde estão os testes de config: `grep -rln "glyphs" tests`)

Testes:
1. `choose_protocol`: tabela com `auto`+WezTerm ⇒ Iterm2; `auto`+`wezterm` ⇒ Iterm2; `auto`+WT_SESSION ⇒ Sixel; `auto`+ambos ⇒ Iterm2 (WezTerm vence); `auto`+nada ⇒ Off; `sixel` no WezTerm ⇒ Sixel; `off` no WT ⇒ Off; `iterm2` sem nada ⇒ Iterm2.
2. `config_icons_default_and_unknown`: config sem `icons` ⇒ `"auto"`; `icons = "bogus"` ⇒ protocolo de `auto` (via `choose`).
3. `split_icon_leaves_a_gap`: `split_icon(Rect(0,0,40,10), 4, 8, 2)` ⇒ ícone `(0,0,8,4)`, texto `(10,0,30,4)`; área de 15 colunas ⇒ texto com largura ≥ 0 (sem pânico).
4. `details_text_starts_after_the_icon_gap` (`tests/tui_view.rs`, `TestBackend`):
   - App com detalhes de um pacote e um protocolo de teste no cache (o que o ratatui-image oferecer para teste; se não houver, um `IconSlot` interno que só reserva a área);
   - as colunas `x..x+8` das 4 primeiras linhas do painel ficam sem texto;
   - as 2 colunas seguintes ficam em branco;
   - o nome do pacote começa exatamente na coluna `x+10`.
5. `details_without_icon_keep_the_old_layout`: snapshot existente dos detalhes inalterado com `icon_bytes` vazio e com `Off`.
6. `no_icon_over_a_dialog`: com `Modal::Confirm` aberto, a área do ícone não é reservada nem desenhada (verifique pela função que decide, por exemplo `fn icon_visible(app) -> bool`).
7. `undecodable_icon_is_remembered_as_none`: bytes lixo ⇒ `Msg::IconReady` com falha ⇒ a chave fica "sem ícone", e um 2º pedido não é enviado (contagem de `Cmd`).

#### Checks beyond the steps

1. Espaço: `DETAIL_ICON_GAP` ignorado em `split_icon` (texto começando em `x+8`) ⇒ PEGA (testes 3 e 4).
2. Protocolo: em `choose`, `auto` sempre `Sixel` ⇒ PEGA (teste 1).
3. Diálogo: desenhar ícone com modal aberto ⇒ PEGA (teste 6).
4. Codificação fora da UI: `grep -n "load_from_memory\|new_protocol" src/tui/view.rs` vazio. Cole a saída.
5. `grep -rn "from_query_stdio" src` vazio. Cole a saída.

**Commit:** `feat(tui): package icon in the details pane`

---

### Task 85: Ícones na lista, só do cache

**Modelo:** sonnet (revisor sonnet). **Roda sozinha (mede quadro).**

#### Where this fits

Depois da Task 84: protocolo, cache de codificação e `Msg::IconReady` já existem. Lista: `draw_list` (`src/tui/view.rs`, ~:546) e a montagem dos itens visíveis (~:426-476), com `ITEM_ROWS = 2`. O tempo de quadro é medido por `examples/tui_frame.rs` (Task 59: Pesquisar ~0,27–0,40 ms). O mapa `icons.json` (Task 83, `icons::load_map`, `icons::cached_icon`) diz quais pacotes têm ícone no cache.

#### What binds this task

- `src/tui/layout.rs`: `pub const LIST_ICON_COLS: u16 = 4; pub const LIST_ICON_GAP: u16 = 1;`. A altura é `ITEM_ROWS`.
- Ícones ligados (protocolo ≠ `Off`): todo item da lista recua `LIST_ICON_COLS + LIST_ICON_GAP` colunas, tenha ícone ou não, para o texto ficar alinhado. Desligados: a lista fica exatamente como hoje (snapshots existentes inalterados).
- O mapa é carregado uma vez ao abrir a TUI (`load_map` em `spawn_blocking`, handle aguardado) e atualizado em memória quando a Task 83 grava um ícone novo (`remember`).
- A cada quadro, só os itens **visíveis** (a janela `list_window`) que têm entrada no mapa e ainda não têm protocolo codificado em `(key, LIST_ICON_COLS, ITEM_ROWS)` geram pedidos de codificação.
  - Os bytes vêm de `cached_icon`, lido em `spawn_blocking`, nunca na UI.
  - No máximo 8 pedidos em andamento ao mesmo tempo; os demais esperam o próximo quadro.
  - **Nada de rede** para a lista.
- Mesmas regras de sobreposição da Task 84: com modal ou ajuda abertos, não desenha; ícone que cruza um toast também não.
- Medição (protocolo da Task 59):
  - `examples/tui_frame.rs` com a aba Pesquisar, ícones desligados e ligados, usando um protocolo de teste que só reserva a área (o terminal real não é medido aqui);
  - 3 séries de 200 quadros: mediana e p95;
  - a regra é que, com ícones desligados, o quadro não piore mais de +0,05 ms contra a base;
  - cole os números no relatório.
  - Medida do terminal real: escreva no relatório o comando para o usuário rodar a TUI (`cargo run --release`) e fica "não medido".

**Files:**
- Modify: `src/tui/layout.rs`, `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/view.rs`, `examples/tui_frame.rs`
- Test: `tests/tui_view.rs`, `tests/tui_state.rs`

Testes:
1. `list_indents_every_item_when_icons_are_on` (`TestBackend`): 3 itens, um com ícone no cache; com ícones ligados, o texto dos 3 começa na mesma coluna (`x + 5`), e as 4 colunas do ícone ficam sem texto; com `Off`, o snapshot de antes não muda.
2. `only_visible_cached_icons_are_requested`: lista de 50 itens, 10 visíveis, 3 dos visíveis e 5 dos invisíveis no mapa ⇒ exatamente 3 pedidos `Cmd` de codificação; nenhum `Cmd` de rede.
3. `icon_requests_are_capped`: 20 visíveis no mapa ⇒ no máximo 8 pedidos por quadro; depois de 8 `Msg::IconReady`, os próximos são pedidos.
4. `list_never_fetches_icons`: nenhum `Cmd` de download de ícone sai da lista. A rede só ocorre em `load_details`.

#### Checks beyond the steps

1. Alinhamento: recuar só os itens com ícone ⇒ PEGA (teste 1).
2. Só visíveis: pedir para todos os itens do mapa ⇒ PEGA (teste 2).
3. Limite: tirar o teto de 8 ⇒ PEGA (teste 3).

**Commit:** `feat(tui): cached package icons in the list`

---

### Task 86: Ajuste "Ícones" na aba Ajustes

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

A aba Ajustes (`src/tui/settings.rs`: `SettingId`, `ORDER`, `label`, `values`, `help`) salva via `Cmd::SaveSettings` (`src/tui/app.rs`, ~:205 e ~:905; `run.rs` ~:251), com `save_in_flight` e coalescência (Task 68). `Glifos` é o modelo: um valor texto da config, que avança com `Enter`/Espaço/`l` e volta com `h`. Depois das Tasks 84 e 85, `Config.icons` e `IconProtocol` já existem.

#### What binds this task

- `SettingId::Icons`, depois de `Glyphs` em `ORDER`.
  - Rótulo: `"Ícones"`.
  - Valores mostrados: `Automático`, `iTerm2`, `Sixel` e `Desligado` (config: `auto`, `iterm2`, `sixel`, `off`).
  - Ajuda: `"Imagens dos pacotes: automático usa iTerm2 no WezTerm e sixel no Windows Terminal."`.
- Mudar o valor:
  - aplica na hora: novo `IconProtocol` via `choose` e cache de protocolos codificados esvaziado (os ícones são recodificados no próximo quadro);
  - salva em `config.toml` pelo mesmo `Cmd::SaveSettings` (campo novo `icons: Option<String>`, enviado só se o usuário mexeu, como `glyphs_touched`).
- `Desligado` ⇒ nenhum ícone nem recuo na lista (Task 85) e layout antigo nos detalhes (Task 84), no mesmo quadro.

**Files:**
- Modify: `src/tui/settings.rs`, `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/view.rs` (se a linha do ajuste precisar), `src/config.rs` (só se faltar algo)
- Test: `tests/tui_state.rs`, `tests/tui_view.rs` (snapshot da aba Ajustes)

Testes:
1. `icons_setting_cycles_and_saves`: `Enter` na linha Ícones ⇒ Automático → iTerm2 → Sixel → Desligado → Automático; o `Cmd::SaveSettings` carrega `icons: Some("iterm2")` depois do 1º `Enter`; mexer em outro ajuste não envia `icons`.
2. `turning_icons_off_clears_the_cache_and_the_indent`: com protocolo codificado no cache, mudar para Desligado ⇒ cache vazio e lista sem recuo (`TestBackend`).
3. Snapshot da aba Ajustes atualizado: só a linha nova muda (confira no diff).

#### Checks beyond the steps

1. Salvar sem mexer: enviar sempre `icons` ⇒ PEGA (teste 1).
2. Aplicar na hora: não esvaziar o cache ⇒ PEGA (teste 2).

**Commit:** `feat(tui): icons setting`
