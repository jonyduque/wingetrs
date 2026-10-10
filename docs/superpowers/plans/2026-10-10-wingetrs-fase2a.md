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
