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

