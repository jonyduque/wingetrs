# wingetrs — itens do BACKLOG planejados (2026-10-22)

> Rascunhos de quatro planejadores (opus), juntados pelo orquestrador. Código literal ainda não compilado: o implementador ajusta nomes ao código atual e diz no relatório o que mudou. Decisões do usuário no fim de cada bloco; a seção de cada task traz a opção recomendada até o usuário decidir.

| Task | Item | Depende de |
|---|---|---|
| 223 | A — `list` mais rápido (Fase 1 mede; Fase 2 só com ≥ 50 ms) | — (tarefa que mede: roda sozinha) |
| 224 | B — quadros lentos na TUI (Fase 1 mede) | — (roda sozinha) |
| 225 | C — aplicabilidade no `list --upgradable` | 205, 207; nunca junto de 210/211 |
| 226 | D — remover várias linhas marcadas na TUI | 207 |
| 227 | E — portão de scripts com o AST do PowerShell | decisão d-E |
| 228 | H — winget sem elevação a partir de terminal admin | 233; sondas S1–S3 do usuário |
| 229 | G — análise de portabilidade de MSI + sonda | OK do usuário para a sonda (~3 GB) |
| 230 | I — extratores informam bytes | — |
| 231 | G — `install --portable` (MSI sem UAC) | 229, 205, decisões D-G |
| 232 | I — barra de progresso na extração (CLI e TUI) | 230, 205, 207 |
| 233 | `uninstall_by_winget` sem PATH (segurança) | — |
| 234 | `reg.exe`/`msiexec.exe` por KnownFolder (segurança, revisão da 233) | — |

**Decisões do usuário (2026-10-22):**
- d-E = B: um pwsh reaproveitado por execução, cache por SHA, e o AST nunca deixa o veredito mais brando.
- d-H = A: token do Explorer + `CreateProcessWithTokenW`, depois das sondas S1–S3 do usuário.
- G: sonda autorizada (~3 GB, sem executar nada). O modo cobre só MSI (D-G1), `msiexec /a` é recusado (D-G3), só a flag sem config (D-G4) e arquivos para pastas do Windows são bloqueados (D-G5). A D-G2 fica para depois da sonda.
- Demais decisões aceitas como recomendadas: C1 (avisos fora do `--json`), D1 (Esc pula só o item), D2 (cada item confirma), D-A1 (cache dos nomes MSIX em disco se a Fase 1 der ≥ 50 ms), D-B1 (o usuário roda o `tui_frame console`), D-B2 (Fase 2 sem nova consulta para o buffer; consultar para o Sixel), D-I1 (tar sem total), D-I2 (230 e 232 separadas).
- O exemplo da 227 que lê scripts reais do catálogo ainda precisa de pedido explícito.
- Depois da sonda da 229 (2026-10-22):
  - Regra "espalhado": a pasta principal é um `Directory` cujo id não é pasta padrão do Windows Installer, e 1 componente basta.
  - D-G2 como o revisor propôs:
    - ação de código adiada bloqueia;
    - viram perda avisada: só rollback (`Type & 0x100`) e `Wix4CreateInternetShortcuts_{X86,X64,A64}`;
    - os tipos 7, 23 e 39 bloqueiam sempre;
    - as ações imediatas viram perda avisada.
  - Na 231, `Shortcut.Name` passa por `install::safepath` antes de virar `.lnk`.

## Bloco 233

### Task 233: Ponte do winget sem PATH quando o processo é elevado

**Modelo:** sonnet (revisor opus: segurança). Origem: achado do planejador do item H (2026-10-22). `uninstall_by_winget` (`src/winget_install/uninstall.rs`) chama `(ctx.bridge)("winget", …)`, que no CLI e na TUI é `run_tool` ⇒ `bridge::spawn_win::run(program, …)`, e o `Command::new("winget")` procura pelo PATH. Num terminal de administrador, a remoção de pacote HKLM roda isso com o processo elevado: um `winget.exe` plantado numa pasta do PATH do usuário (gravável sem admin) vira admin. É a mesma classe da revisão da 137 (regra "Código que roda elevado" do `CLAUDE.md`: nunca procurar executável pelo PATH nem por alias). O `detect_tools` da TUI (`src/tui/run.rs`, `run_tool("winget", ["--version"])`) e todo outro chamador de `spawn_win::run` com nome solto têm o mesmo problema quando o processo está elevado.

**Files:** `src/bridge/spawn_win.rs`, `src/bridge/elevate_win.rs` (só reaproveitar `winget_exe()`), `tests/it/bridge_spawn.rs` (novo, ou o arquivo de testes da ponte que já existir; confira com `grep -rln "spawn_win" tests`).

**Contrato:**
- Uma função pura, testável em qualquer plataforma: `resolve_program(program: &str, elevated: bool, winget_exe: impl FnOnce() -> io::Result<PathBuf>) -> io::Result<PathBuf>`.
  - `elevated == false`: devolve `PathBuf::from(program)` (comportamento atual, PATH do usuário, processo do usuário).
  - `elevated == true` e `program` é caminho absoluto: devolve como está.
  - `elevated == true` e `program == "winget"` (sem caixa, com ou sem `.exe`): devolve `winget_exe()?`, que já resolve só dentro de `Program Files\WindowsApps` (pasta só de TrustedInstaller).
  - `elevated == true` e qualquer outro nome solto (ex.: `scoop`, `pwsh`): erro `PermissionDenied` com a mensagem "o wingetrs não procura programas pelo PATH com o processo elevado; abra um terminal sem administrador e repita".
- `spawn_win::run` chama `resolve_program(program, launch_win::is_elevated()?, elevate_win::winget_exe)` antes do `Command::new`. Nenhum outro chamador muda.

**Passos TDD:**

1. Testes (falham antes):

```rust
#[test]
fn unelevated_keeps_the_bare_name() {
    let p = resolve_program("winget", false, || panic!("não deve resolver")).unwrap();
    assert_eq!(p, PathBuf::from("winget"));
}

#[test]
fn elevated_winget_comes_from_the_admin_folder() {
    let fixed = PathBuf::from(r"C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1_x64__8wekyb3d8bbwe\winget.exe");
    let p = resolve_program("WinGet.exe", true, || Ok(fixed.clone())).unwrap();
    assert_eq!(p, fixed);
}

#[test]
fn elevated_other_bare_names_are_refused() {
    let e = resolve_program("scoop", true, || panic!("não deve resolver")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("não procura programas pelo PATH"));
}

#[test]
fn elevated_absolute_path_is_kept() {
    let abs = PathBuf::from(r"C:\Windows\System32\cmd.exe");
    let p = resolve_program(abs.to_str().unwrap(), true, || panic!("não deve resolver")).unwrap();
    assert_eq!(p, abs);
}

#[test]
fn elevated_winget_missing_is_an_error_not_the_path() {
    let e = resolve_program("winget", true, || {
        Err(std::io::Error::new(std::io::ErrorKind::NotFound, "sem winget"))
    })
    .unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
}
```

2. Implemente `resolve_program` (neutro de plataforma, fora do `*_win.rs` se o arquivo for só Windows; siga a regra ⚙ do `CLAUDE.md`) e ligue no `spawn_win::run`.
3. `rustfmt <arquivo>` em cada arquivo tocado; `cargo test --quiet --test it <módulo>`; gate.

#### Verificação (cole cada `[OK] mutação PEGA`)
- trocar `if elevated` por `if false` na resolução ⇒ `elevated_other_bare_names_are_refused` falha;
- trocar `winget_exe()?` por `PathBuf::from(program)` ⇒ `elevated_winget_comes_from_the_admin_folder` falha.

**Fora do escopo:** rodar o winget sem elevação a partir de terminal admin (Task 228); mudar o `bridge-worker` (já usa `winget_exe`).

**Relatório:** `.claude/tasks/task-233-report.md` com status, SHAs, gate, mutações e a lista de chamadores de `spawn_win::run` conferidos (`grep -rn "spawn_win::run\|run_tool(" src`).

---

## Bloco AB

### Task 223: `list` mais rápido: cache dos nomes MSIX (`scan_msix_props`)

**Modelo:** sonnet (revisor sonnet).

**Depois de:** nenhuma tarefa aberta mexe em `src/installed/`. `Cargo.toml` só ganha uma feature do crate `windows` já usado (linha `windows = { ... features = [...] }`); se outra tarefa aberta estiver editando o `Cargo.toml`, faça a edição por último e confira o diff.

> Origem: `docs/superpowers/plans/BACKLOG.md`, item A. Medido em 2026-10-17 (release): `list` 293 ms (delta 262 ms sobre o processo vazio); `scan_all` 195 ms, dos quais `scan_msix_props` 155–168 ms; `normalizer_new` 64 ms em paralelo; `correlate` 30 ms. Ganho estimado ~150 ms.

**A tarefa tem duas fases com uma parada no meio.** Fase 1 só mede e não muda código (a não ser um experimento descartado). Fase 2 só começa se a Fase 1 der o número pedido no passo 1.4. Medição só quando o orquestrador liberar a máquina (nenhum outro agente compilando ou testando); ARMADILHAS "Piso de processo varia de 38 a 300 ms com a carga da máquina" e "Processos órfãos de agentes distorcem medições".

#### Where this fits

- `src/installed/appx_win.rs` (só Windows): `Candidate { pkg, family, id_name, version }` (`:12-17`), `identify` (`:19`, leitura barata: família, nome do id, versão), `read_one` (`:47`, as leituras caras: `Package::DisplayName()` e `Package::PublisherDisplayName()`, resolução de recursos do pacote), `read_appx` (`:77`): `scan_msix_enumerate` (`PackageManager::FindPackagesByUserSecurityId`) ⇒ `scan_msix_identify` (paralelo) ⇒ maior versão por família ⇒ `scan_msix_props` (`:109`, `read_one` em paralelo com `WORKERS = 8` pelo `super::par::parallel`, que preserva a ordem).
- `src/installed/mod.rs`: `timed(step, f)` (`:111`) loga `INFO ... span step="<nome>" ms=<n>` (aparece no stderr com `-v`); `collect_msix()` (`:139`, Windows; `:146` stub) chama `appx_win::read_appx()` sem pasta de dados; `collectors(data)` (`:162`) monta `("msix", Box::new(collect_msix))`; `scan_all_in(data)` (`:177`), `scan_all()` (`:182`, `DataDir::from_env`).
- Quem chama `scan_all`/`scan_all_in`: `list` do CLI (`src/cli/mod.rs:914-916`, numa thread em paralelo com `winget_tracking` e `normalizer_new`, `:917-928`), a TUI (`src/tui/run.rs:870`), `src/cli/run_win.rs:113`, `src/winget_install/prod_win.rs:17`; testes `live` em `tests/it/installed.rs:163,177`, `tests/it/correlate.rs:265`, `tests/it/winget_applicability.rs:521`. Nenhum muda de assinatura.
- `src/cache/mod.rs`: `DataDir` com um método por arquivo (`catalog_path` `:67`, `http_state_path` `:73`, `config_path` `:89`); `HttpState::load`/`save` (`:128-153`) é o modelo de "arquivo ausente ou corrompido ⇒ vazio com `warn`" e de gravação atômica (`<path>.tmp` + `crate::fs_retry::retry("renomear", ...)`).
- Histórico que importa: a Task 17 tentou "menos chamadas `DisplayName`" e **não teve ganho medido** (revertida, ARMADILHAS "Palpite de gargalo falha"); a Task 20 paralelizou com fila atômica (props 260 ⇒ ~190 ms sob disputa). Ou seja: reduzir chamadas não bastou e paralelizar já foi feito; o que sobra é **não fazer** as leituras quando o pacote não mudou. A Fase 1 prova antes se isso vale.

#### Fase 1: medir (sem mudar código)

1.1. Preparação (uma vez): `pwsh -File scripts/leftovers.ps1`; `target\release\wingetrs.exe update` (catálogo em dia, para o `list` não baixar nada durante o bench); `cargo build --release --quiet` e `cargo build --release --example noop --quiet`; `Copy-Item target\release\wingetrs.exe target\release\wingetrs-base.exe`.

1.2. Quebra por etapa, 10 execuções, mediana por etapa (cole a tabela no relatório):

```powershell
$rows = 1..10 | ForEach-Object {
    & target\release\wingetrs-base.exe list -v 2>&1 | Select-String 'step="?(\w+)"? ms=([\d.]+)' | ForEach-Object {
        [pscustomobject]@{ step = $_.Matches[0].Groups[1].Value; ms = [double]$_.Matches[0].Groups[2].Value }
    }
}
$rows | Group-Object step | ForEach-Object {
    $s = @($_.Group.ms | Sort-Object)
    [pscustomobject]@{ step = $_.Name; n = $s.Count; mediana = $s[[int][math]::Floor($s.Count / 2)]; max = $s[-1] }
} | Sort-Object mediana -Descending | Format-Table
```

Se o padrão não casar (formato do log diferente), ajuste a expressão e diga no relatório como ficou. Anote também quantos pacotes passam por `scan_msix_props` (número de linhas MSIX do `list --json`: `(& target\release\wingetrs-base.exe list --json | ConvertFrom-Json | Where-Object origin -eq 'msix').Count`; se o campo tiver outro nome, use o que o JSON tiver e diga qual).

1.3. **Teto do ganho (experimento descartado).** Edite `read_one` (`src/installed/appx_win.rs:47`) trocando as duas leituras por `None` (`let display: Option<String> = None;` e `let publisher: Option<String> = None;`), `cargo build --release --quiet`, `Copy-Item target\release\wingetrs.exe target\release\wingetrs-noprops.exe`, **desfaça a edição** (`git diff --quiet -- src/installed/appx_win.rs` tem de sair 0) e `cargo build --release --quiet` de novo. Esse binário mostra o máximo que qualquer cache pode ganhar. Bench A/B, duas rodadas intercaladas:

```powershell
foreach ($round in 1..2) {
    foreach ($exe in 'wingetrs-base', 'wingetrs-noprops') {
        "== rodada $round $exe"
        pwsh -File scripts/bench.ps1 -Exe "target/release/$exe.exe" -Args 'list' -Runs 15 -Baseline target/release/examples/noop.exe
    }
}
```

Cole as quatro saídas. Ganho = delta mediano de `wingetrs-base` menos o de `wingetrs-noprops`, em cada rodada.

1.4. **Parada.** Grave o relatório da Fase 1 (`.claude/tasks/task-223-report.md`, seção "Fase 1") e:
- ganho do passo 1.3 **≥ 50 ms nas duas rodadas** ⇒ siga para a Fase 2 sem esperar;
- menor ⇒ `Status: DONE_WITH_CONCERNS`, sem Fase 2, sem commit de código; o relatório diz qual etapa do passo 1.2 domina agora (é o insumo para um novo item do BACKLOG). Apague `wingetrs-noprops.exe`.

#### What binds this task (Fase 2)

1. **Cache por nome completo do pacote.** As leituras de `DisplayName`/`PublisherDisplayName` de um pacote acontecem só se o `PackageFullName` (`Package::Id()?.FullName()`, que inclui nome, versão, arquitetura, recurso e hash do publisher) não estiver no cache. Pacote atualizado ⇒ nome completo novo ⇒ leitura nova. A chave **não** é a família (família não muda com a versão).
2. **Idioma no cache.** Os nomes vêm localizados. O arquivo guarda o idioma da interface (`GetUserDefaultUILanguage`, `windows::Win32::Globalization`; adicione a feature `"Win32_Globalization"` à lista do crate `windows` no `Cargo.toml`, sem crate novo e sem `cargo update`). Idioma diferente ⇒ cache vazio.
3. **Arquivo:** `DataDir::msix_names_path()` = `<data>\msix-names.json`. Ausente, corrompido ou de outro idioma ⇒ cache vazio (corrompido loga `warn`), nunca erro nem pânico. Gravação atômica (`.tmp` + rename com `crate::fs_retry::retry`), só quando o conteúdo mudou; falha ao gravar ⇒ `tracing::warn!` e o `list` segue.
4. **O cache guarda só os pacotes desta leitura** (pacote removido some do arquivo), então não cresce sem limite.
5. **Sem pasta de dados (`scan_all_in(None)`), nada de cache:** lê tudo, como hoje.
6. **Saída idêntica** com cache frio, cache quente e sem cache (teste de equivalência; regra do projeto "Mais de um caminho para o mesmo resultado").
7. `msix_display_name` continua aplicado depois (a regra de `ms-resource:`/vazio não muda); o cache guarda o valor cru lido do pacote.
8. Um `tracing::info!(hits, misses, "msix names")` por leitura, para a prova.

#### Fase 2: passos TDD

**Files:** `src/installed/msix_names.rs` (novo), `src/installed/mod.rs`, `src/installed/appx_win.rs`, `src/cache/mod.rs` (só `msix_names_path`), `Cargo.toml` (só a feature), `tests/it/installed.rs`.

**Passo 1 — testes que falham.** Em `tests/it/installed.rs`, no fim do arquivo:

```rust
mod msix_names {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wingetrs::installed::msix_names::{NameCache, Names, resolve};

    fn names(d: &str) -> Names {
        Names {
            display: Some(d.to_string()),
            publisher: Some(format!("{d} Inc")),
        }
    }

    fn keys(k: &[&str]) -> Vec<String> {
        k.iter().map(|s| s.to_string()).collect()
    }

    fn empty(lang: u16) -> NameCache {
        NameCache {
            lang,
            ..Default::default()
        }
    }

    #[test]
    fn msix_names_cold_and_warm_are_equal() {
        let ks = keys(&["B_2.0.0.0_x64__h", "A_1.0.0.0_x64__h", "C_3.0.0.0_x64__h"]);
        let calls = AtomicUsize::new(0);
        let read = |i: usize| {
            calls.fetch_add(1, Ordering::Relaxed);
            names(&ks[i][..1])
        };
        let cold = resolve(&ks, &empty(1046), 4, read);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(cold.hits, 0);
        assert!(cold.changed);
        assert_eq!(cold.cache.lang, 1046);
        assert_eq!(cold.names, vec![names("B"), names("A"), names("C")]);
        let warm = resolve(&ks, &cold.cache, 4, read);
        assert_eq!(calls.load(Ordering::Relaxed), 3, "a warm run reads nothing");
        assert_eq!(warm.hits, 3);
        assert!(!warm.changed);
        assert_eq!(warm.names, cold.names);
    }

    #[test]
    fn msix_names_reads_only_misses_and_keeps_order() {
        let ks = keys(&["B_2", "A_1", "C_3"]);
        let mut cache = empty(1046);
        cache.names.insert("A_1".to_string(), names("cached A"));
        let seen = Mutex::new(Vec::new());
        let r = resolve(&ks, &cache, 4, |i| {
            seen.lock().unwrap().push(i);
            names(&format!("fresh {}", &ks[i][..1]))
        });
        let mut seen = seen.into_inner().unwrap();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 2]);
        assert_eq!(
            r.names,
            vec![names("fresh B"), names("cached A"), names("fresh C")]
        );
        assert_eq!(r.hits, 1);
    }

    #[test]
    fn msix_names_cache_holds_only_current_packages() {
        let mut cache = empty(1046);
        cache.names.insert("A_1".to_string(), names("A"));
        cache.names.insert("Z_9".to_string(), names("Z"));
        let r = resolve(&keys(&["A_1"]), &cache, 4, |_| names("never"));
        assert_eq!(r.names, vec![names("A")]);
        assert_eq!(
            r.cache.names.keys().cloned().collect::<Vec<_>>(),
            vec!["A_1".to_string()]
        );
        assert!(r.changed, "dropping Z_9 is a change");
        let same = resolve(&keys(&["A_1"]), &r.cache, 4, |_| names("never"));
        assert!(!same.changed);
    }

    #[test]
    fn msix_name_cache_load_rejects_other_language_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msix-names.json");
        assert_eq!(NameCache::load(&path, 1046), empty(1046), "missing file");
        let mut c = empty(1046);
        c.names.insert("A_1".to_string(), names("A"));
        c.save(&path).unwrap();
        assert!(!dir.path().join("msix-names.json.tmp").exists());
        assert_eq!(NameCache::load(&path, 1046), c);
        assert_eq!(NameCache::load(&path, 1033), empty(1033), "other language");
        std::fs::write(&path, b"{").unwrap();
        assert_eq!(NameCache::load(&path, 1046), empty(1046), "corrupt file");
    }
}

#[test]
#[ignore = "live: reads this machine's MSIX packages"]
fn live_msix_name_cache_is_equivalent() {
    let dir = tempfile::tempdir().unwrap();
    let data = wingetrs::cache::DataDir::new(dir.path());
    let msix = |scan: wingetrs::installed::InstalledScan| {
        let mut v: Vec<_> = scan
            .apps
            .into_iter()
            .filter(|a| a.source == InstallSource::Msix)
            .collect();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        v
    };
    let uncached = msix(wingetrs::installed::scan_all_in(None));
    let cold = msix(wingetrs::installed::scan_all_in(Some(&data)));
    assert!(data.msix_names_path().exists(), "the cold run writes the cache");
    let warm = msix(wingetrs::installed::scan_all_in(Some(&data)));
    assert!(uncached.len() > 5, "{}", uncached.len());
    assert_eq!(cold, uncached);
    assert_eq!(warm, uncached);
}
```

Os valores de idioma (1046, 1033) não são o padrão `0`, de propósito (ARMADILHAS "Caso degenerado esconde a regra"). As chaves fora de ordem (`B`, `A`, `C`) e o acerto no meio provam a ordem do resultado. `cargo test --quiet --test it installed::msix_names` ⇒ não compila (módulo ausente).

**Passo 2 — `src/installed/msix_names.rs`** (sem nada de Windows; `pub mod msix_names;` em `src/installed/mod.rs`, ao lado de `pub mod par;`):

```rust
//! Cache of MSIX display names: the resource lookups behind `Package::DisplayName` and
//! `PublisherDisplayName` were most of `scan_msix_props` (Task 223).
use std::collections::BTreeMap;
use std::path::Path;

/// Display name and publisher as read from the package, before `msix_display_name`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Names {
    pub display: Option<String>,
    pub publisher: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NameCache {
    /// UI language the names were resolved in (`GetUserDefaultUILanguage`).
    pub lang: u16,
    /// Package full name (name, version, architecture, resource id, publisher hash) -> names.
    pub names: BTreeMap<String, Names>,
}

impl NameCache {
    /// A missing, corrupt (logged) or other-language file yields an empty cache for `lang`.
    pub fn load(path: &Path, lang: u16) -> NameCache {
        let empty = NameCache {
            lang,
            names: BTreeMap::new(),
        };
        let Ok(bytes) = std::fs::read(path) else {
            return empty;
        };
        match serde_json::from_slice::<NameCache>(&bytes) {
            Ok(c) if c.lang == lang => c,
            Ok(_) => empty,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "msix name cache corrupt; starting empty");
                empty
            }
        }
    }

    /// Atomic write: `<path>.tmp`, then rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, &json)?;
        crate::fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, path))
    }
}

pub struct Resolved {
    /// Names in `keys` order.
    pub names: Vec<Names>,
    /// Exactly the entries of `keys`: packages no longer installed are dropped.
    pub cache: NameCache,
    pub hits: usize,
    /// Whether `cache` differs from the one passed in (only then is it worth saving).
    pub changed: bool,
}

/// Names for `keys` (package full names): cached ones as stored, the others from `read(i)`,
/// run on up to `workers` threads.
pub fn resolve(
    keys: &[String],
    cache: &NameCache,
    workers: usize,
    read: impl Fn(usize) -> Names + Sync,
) -> Resolved {
    let misses: Vec<usize> = (0..keys.len())
        .filter(|&i| !cache.names.contains_key(&keys[i]))
        .collect();
    let fresh = super::par::parallel(&misses, workers, |&i| Some(read(i)));
    let mut names: Vec<Names> = keys
        .iter()
        .map(|k| cache.names.get(k).cloned().unwrap_or_default())
        .collect();
    for (&i, n) in misses.iter().zip(fresh) {
        names[i] = n;
    }
    let kept: BTreeMap<String, Names> = keys.iter().cloned().zip(names.iter().cloned()).collect();
    let changed = kept != cache.names;
    Resolved {
        hits: keys.len() - misses.len(),
        names,
        cache: NameCache {
            lang: cache.lang,
            names: kept,
        },
        changed,
    }
}
```

`cargo test --quiet --test it installed::msix_names` ⇒ os 4 passam.

**Passo 3 — `DataDir::msix_names_path()`** em `src/cache/mod.rs`, ao lado de `http_state_path`: `self.root.join("msix-names.json")`, com doc de uma linha.

**Passo 4 — ligar no MSIX.**
- `Cargo.toml`: `"Win32_Globalization"` na lista de features do `windows` (só isso).
- `src/installed/mod.rs`: `collect_msix(data: Option<&crate::cache::DataDir>)` nas duas versões (Windows e stub); a Windows chama `appx_win::read_appx(data.map(|d| d.msix_names_path()).as_deref())`; em `collectors`, `("msix", Box::new(move || collect_msix(data)))`.
- `src/installed/appx_win.rs`:
  - `Candidate` ganha `full_name: String`, lido em `identify` com `id.FullName().ok()?.to_string()` (sem nome completo o pacote é pulado, como já acontece sem família).
  - `read_one(c)` vira `read_names(c: &Candidate) -> super::msix_names::Names` (as duas leituras de hoje, mesmas regras: publisher vazio ⇒ `None`) e `to_app(c: &Candidate, n: Names) -> InstalledApp` (o resto do `read_one` de hoje, com `super::msix_display_name(n.display.as_deref(), &c.id_name)`).
  - `pub fn read_appx(cache_path: Option<&std::path::Path>)`: depois de escolher `winners`,

    ```rust
    // SAFETY: no arguments; returns the current user's UI language id.
    let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    let cache = match cache_path {
        Some(p) => super::msix_names::NameCache::load(p, lang),
        None => super::msix_names::NameCache {
            lang,
            ..Default::default()
        },
    };
    let keys: Vec<String> = winners.iter().map(|c| c.full_name.clone()).collect();
    let r = super::timed("scan_msix_props", || {
        super::msix_names::resolve(&keys, &cache, WORKERS, |i| read_names(winners[i]))
    });
    tracing::info!(hits = r.hits, misses = keys.len() - r.hits, "msix names");
    if r.changed
        && let Some(p) = cache_path
        && let Err(e) = r.cache.save(p)
    {
        tracing::warn!(path = %p.display(), error = %e, "msix name cache not saved");
    }
    Ok(winners.iter().zip(r.names).map(|(c, n)| to_app(c, n)).collect())
    ```

    Se o compilador acusar `unused_unsafe` (a função gerada não é `unsafe` nesta versão do crate), tire o bloco e o comentário `SAFETY` e diga no relatório. O span continua se chamando `scan_msix_props` (a comparação antes/depois usa o mesmo nome).
- `cargo test --quiet --test it installed::` e `cargo test --quiet --release --test it -- --ignored live_msix_name_cache_is_equivalent` (este lê a máquina real, só leitura; grava só no tempdir) ⇒ verdes. Cole a saída do teste `live`.

**Commit:** `perf(installed): cache MSIX display names by package full name`

#### Fase 2: prova de ganho (regra "otimização sem ganho medido é revertida")

1. Com a máquina liberada: `cargo build --release --quiet`; `Copy-Item target\release\wingetrs.exe target\release\wingetrs-new.exe`. Quente (o aquecimento do `bench.ps1` grava o cache), duas rodadas intercaladas como no passo 1.3 com `wingetrs-base` e `wingetrs-new`. Cole as quatro saídas.
2. Frio (cache apagado antes de cada execução; o `bench.ps1` não faz isso):

    ```powershell
    $cache = Join-Path $env:LOCALAPPDATA 'wingetrs\msix-names.json'
    foreach ($exe in 'wingetrs-base', 'wingetrs-new', 'wingetrs-base', 'wingetrs-new') {
        $ms = 1..10 | ForEach-Object {
            Remove-Item $cache -ErrorAction SilentlyContinue
            (Measure-Command { & "target\release\$exe.exe" list *> $null }).TotalMilliseconds
        } | Sort-Object
        "$exe frio: mediana $($ms[5]) ms"
    }
    ```

3. Passo 1.2 de novo com `wingetrs-new` (quente): cole a tabela; `scan_msix_props` e a linha `msix names hits=… misses=0` na 2ª execução.
4. Saída idêntica: `list --json` de `wingetrs-base` e de `wingetrs-new` (frio e quente), comparados com `Compare-Object (Get-Content a.json) (Get-Content b.json)` ⇒ vazio. Gere os arquivos no scratchpad, não no repositório.
5. **Critério:** quente com ganho ≥ 50 ms de delta mediano nas duas rodadas **e** frio no máximo 15 ms pior que a base. Fora disso: reverta o commit de código (`git revert`), `Status: DONE_WITH_CONCERNS` com os números. Apague `wingetrs-base.exe`, `wingetrs-new.exe` e `wingetrs-noprops.exe` no fim.

#### Checks beyond the steps

1. Mutação: `resolve` ignora o cache (`.filter(|&i| !cache.names.contains_key(&keys[i]))` ⇒ `.filter(|_| true)`) ⇒ `msix_names_reads_only_misses_and_keeps_order` **PEGA**.
2. Mutação: `for (&i, n) in misses.iter().zip(fresh) {` ⇒ `for (i, n) in fresh.into_iter().enumerate() {` ⇒ o mesmo teste **PEGA** (ordem).
3. Mutação: `Ok(c) if c.lang == lang => c,` ⇒ `Ok(c) => c,` ⇒ `msix_name_cache_load_rejects_other_language_and_garbage` **PEGA**.
4. Mutação: `let changed = kept != cache.names;` ⇒ `let changed = true;` ⇒ `msix_names_cache_holds_only_current_packages` **PEGA**.
5. Comando de cada mutação: `pwsh -File scripts/mutate.ps1 -Path src/installed/msix_names.rs -Anchor '<âncora>' -Replacement '<troca>' -TestCommand 'cargo test --quiet --test it installed::msix_names'`; confira na saída o nome do teste que falhou (ARMADILHAS "Mutação PEGA por falha de build").
6. Sem mutação possível (só Windows real), o revisor confere no diff: chave = `FullName`, não `FamilyName`; `scan_all_in(None)` não grava nada.
7. Gate: `pwsh -File scripts/verify.ps1`.
8. Relatório: Fase 1 (tabela por etapa, número de pacotes MSIX, as quatro saídas do teto) e Fase 2 (quatro saídas quente, quatro medianas frio, tabela por etapa nova, comparação do JSON, saída do teste `live`).

**Commit (relatório):** `chore(ledger): Task 223 measurements`

### Task 224: Quadros lentos na TUI — medir onde o tempo vai (Fase 1); corrigir só com número (Fase 2)

**Modelo:** sonnet (revisor sonnet).

**Depois de:** nenhuma tarefa aberta em `src/tui/run.rs` (se houver, espere; o `mutate.ps1` restaura o arquivo inteiro, ARMADILHAS "Mutação em arquivo que outro agente edita").

> Origem: `docs/superpowers/plans/BACKLOG.md`, item B ("Quadros de até 190 ms vistos na medição de 2026-10-17"). Dado mais recente, do log real da TUI do usuário (`%LOCALAPPDATA%\wingetrs\log\wingetrs.log.2026-10-*`, lido em 2026-10-22): 301 linhas `frame lento` (limiar 16 ms), 102 acima de 50 ms, 37 acima de 100 ms; `tui stats ... max_draw_us` de até 380650 (380 ms). Contra isso, o `examples/tui_frame.rs` (Task 59, `TestBackend` 120×40) mede 0,3–0,6 ms de mediana por quadro. **A diferença de duas ordens de grandeza diz que o harness não mede o que é lento no terminal real** — provavelmente a escrita no terminal (diff + sequências de escape + `WriteConsole`), não o `view`. É hipótese: esta tarefa mede antes de qualquer correção.

#### Where this fits

- `src/tui/run.rs`: `SLOW_DRAW_US = 16_000` (`:43`); `struct LoopStats` (`:46-97`, privado): `draw(batch, f)` cronometra o `terminal.draw` inteiro e loga `debug!(draw_us, batch, "frame lento")`; o `Drop` loga `info!(frames, keys, ticks, max_draw_us, secs, "tui stats")`. Chamada única no laço: `stats.draw(batch_len, || terminal.draw(|f| view(&app, &theme, f)).map(|_| ()))` (`:560-562`). O terminal é `ratatui::init()` (`:241`), tipo `ratatui::DefaultTerminal` = `Terminal<CrosstermBackend<Stdout>>` (o `Stdout` do Rust tem buffer de linha pequeno). O primeiro quadro (`:257`) fica fora da contagem.
- `App` (`src/tui/app.rs:755`): `tab: Tab` (`Search`, `Installed`, `Updates`, `Settings`), `modal: Option<Modal>`, `icon_protocol: IconProtocol` (`Iterm2`, `Sixel`, `Off`; `src/tui/icons.rs:11`, `Debug`), `size: (u16, u16)`.
- `examples/tui_frame.rs`: `scenario(entries, tabs, query, icons)` (`:75`) monta o `App`, desenha `FRAMES = 200` quadros num `TestBackend::new(120, 40)` apertando Down entre eles e devolve mediana/p95 em µs. `reserve_icon()` (`:56`) usa `Picker::halfblocks()`. `main` (`:231`): modo padrão (4 cenários + busca + rajada) e modo `frames` (Task 85). Precisa do catálogo real em disco. O `TestBackend` não gera bytes: mede só `view` + diff de buffer.
- `wingetrs::tui::icons::picker(IconProtocol::Sixel)` (`src/tui/icons.rs:62`) dá um `Picker` Sixel sem consultar o terminal; `ratatui::TerminalOptions`, `ratatui::Viewport::Fixed(Rect)` e `ratatui::backend::CrosstermBackend` existem no ratatui 0.30.2; `CrosstermBackend::writer()` existe no `ratatui-crossterm` 0.1.2.
- Captura de log nos testes: `crate::spans::Log` (`tests/it/spans/mod.rs:185`, DEBUG e acima da thread que instalou). Testes do laço: `tests/it/tui_loop.rs`.

#### What binds this task

1. **Fase 1 não otimiza nada.** Só instrumenta, mede e para (`Status: DONE` com os números; o orquestrador escreve a Fase 2 com o número na mão).
2. **O quadro lento diz onde o tempo foi:** `view_us` (montar o buffer, dentro do `view`), `flush_us` (o resto do `terminal.draw`: diff, codificação e escrita no terminal), e o contexto (`batch`, `tab`, `modal`, `icons`, `w`, `h`). O resumo `tui stats` ganha `slow_frames`, `max_view_us`, `max_flush_us` (máximos sobre **todos** os quadros).
3. **O harness passa a medir bytes:** modo `bytes` do `tui_frame` desenha pelo `CrosstermBackend` num escritor que conta bytes e chamadas de `write`, por cenário, incluindo ícones Sixel de verdade.
4. **Medida no console real sem abrir a TUI:** modo `console` do `tui_frame` desenha os mesmos cenários no terminal de quem roda (tela alternativa), intercalando `Stdout` puro e `BufWriter` de 64 KiB, e imprime a tabela depois de restaurar a tela. É medição (A/B de escritor), não muda o app. Só o usuário roda esse modo (precisa de um console de verdade).
5. A saída do modo padrão e do modo `frames` do `tui_frame` não muda (números comparáveis com as Tasks 59/60/85).

#### Fase 1: passos TDD

**Files:** `src/tui/run.rs` (só `LoopStats`, `FrameInfo` e a chamada em `:560-562`), `examples/tui_frame.rs`, `tests/it/tui_loop.rs`.

**Passo 1 — testes que falham** (no fim de `tests/it/tui_loop.rs`; acrescente `LoopStats` e `FrameInfo` ao `use wingetrs::tui::run::{...}` do topo):

```rust
fn frame_info() -> FrameInfo {
    FrameInfo {
        batch: 2,
        tab: wingetrs::tui::app::Tab::Installed,
        modal: true,
        icons: wingetrs::tui::icons::IconProtocol::Sixel,
        size: (120, 40),
    }
}

/// Whole whitespace-separated token, so `view_us=4000` does not match `view_us=40000`.
fn has_token(text: &str, tok: &str) -> bool {
    text.split_whitespace().any(|t| t == tok)
}

#[test]
fn slow_frame_log_splits_view_and_flush() {
    let log = crate::spans::Log::default();
    let _g = log.install();
    let mut stats = LoopStats::new();
    stats.record(&frame_info(), 30_000, 4_000);
    stats.record(&frame_info(), 9_000, 8_000);
    let text = log.text();
    assert_eq!(text.matches("frame lento").count(), 1, "{text}");
    for tok in [
        "draw_us=30000",
        "view_us=4000",
        "flush_us=26000",
        "batch=2",
        "tab=Installed",
        "modal=true",
        "icons=Sixel",
        "w=120",
        "h=40",
    ] {
        assert!(has_token(&text, tok), "{tok} missing in {text}");
    }
    drop(stats);
    let text = log.text();
    for tok in [
        "frames=2",
        "slow_frames=1",
        "max_draw_us=30000",
        "max_view_us=8000",
        "max_flush_us=26000",
    ] {
        assert!(has_token(&text, tok), "{tok} missing in {text}");
    }
}

#[test]
fn draw_reads_view_time_after_the_frame() {
    let log = crate::spans::Log::default();
    let _g = log.install();
    let mut stats = LoopStats::new();
    let view_us = std::cell::Cell::new(0);
    stats
        .draw(&frame_info(), &view_us, || {
            view_us.set(123_456_789);
            Ok(())
        })
        .unwrap();
    drop(stats);
    assert!(has_token(&log.text(), "max_view_us=123456789"), "{}", log.text());
}
```

O 2º quadro do teste 1 é rápido e tem o maior `view_us` (8000 > 4000): um máximo calculado só nos quadros lentos dá 4000 e falha (ARMADILHAS "Caso degenerado esconde a regra"). Se o formato do `tracing` puser aspas ou outro separador nos campos, ajuste os tokens ao formato real e diga no relatório (mantendo a comparação de token inteiro). `cargo test --quiet --test it tui_loop::` ⇒ não compila.

**Passo 2 — `LoopStats` público com a divisão** (`src/tui/run.rs`, no lugar do atual `:44-97`; importe `Tab` e `IconProtocol` se ainda não estiverem no `use`):

```rust
/// What a frame showed, for the slow-frame log.
#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    pub batch: usize,
    pub tab: Tab,
    pub modal: bool,
    pub icons: crate::tui::icons::IconProtocol,
    pub size: (u16, u16),
}

/// Loop counters for measuring responsiveness; logged once when the loop ends, on every path.
pub struct LoopStats {
    start: Instant,
    frames: u64,
    keys: u64,
    ticks: u64,
    slow_frames: u64,
    max_draw_us: u64,
    max_view_us: u64,
    max_flush_us: u64,
}

impl Default for LoopStats {
    fn default() -> LoopStats {
        LoopStats::new()
    }
}

impl LoopStats {
    pub fn new() -> LoopStats {
        LoopStats {
            start: Instant::now(),
            frames: 0,
            keys: 0,
            ticks: 0,
            slow_frames: 0,
            max_draw_us: 0,
            max_view_us: 0,
            max_flush_us: 0,
        }
    }

    pub fn count(&mut self, msg: &Msg) {
        match msg {
            Msg::Key(_) => self.keys += 1,
            Msg::Tick => self.ticks += 1,
            _ => {}
        }
    }

    /// Times one `draw`; `f` stores the microseconds spent inside `view` in `view_us`.
    pub fn draw(
        &mut self,
        info: &FrameInfo,
        view_us: &std::cell::Cell<u64>,
        f: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let t = Instant::now();
        let r = f();
        let draw_us = u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.record(info, draw_us, view_us.get());
        r
    }

    /// Accounts one frame: `draw_us` is the whole `Terminal::draw`, `view_us` the part in
    /// `view`; the rest (`flush_us`) is the buffer diff and the write to the terminal.
    pub fn record(&mut self, info: &FrameInfo, draw_us: u64, view_us: u64) {
        let flush_us = draw_us.saturating_sub(view_us);
        self.frames += 1;
        self.max_draw_us = self.max_draw_us.max(draw_us);
        self.max_view_us = self.max_view_us.max(view_us);
        self.max_flush_us = self.max_flush_us.max(flush_us);
        if draw_us > SLOW_DRAW_US {
            self.slow_frames += 1;
            tracing::debug!(
                draw_us,
                view_us,
                flush_us,
                batch = info.batch,
                tab = ?info.tab,
                modal = info.modal,
                icons = ?info.icons,
                w = info.size.0,
                h = info.size.1,
                "frame lento"
            );
        }
    }
}

impl Drop for LoopStats {
    fn drop(&mut self) {
        tracing::info!(
            frames = self.frames,
            keys = self.keys,
            ticks = self.ticks,
            slow_frames = self.slow_frames,
            max_draw_us = self.max_draw_us,
            max_view_us = self.max_view_us,
            max_flush_us = self.max_flush_us,
            secs = self.start.elapsed().as_secs_f64(),
            "tui stats"
        );
    }
}
```

A chamada no laço (`:560-562`) passa a ser:

```rust
let info = FrameInfo {
    batch: batch_len,
    tab: app.tab,
    modal: app.modal.is_some(),
    icons: app.icon_protocol,
    size: app.size,
};
let view_us = std::cell::Cell::new(0);
stats.draw(&info, &view_us, || {
    terminal
        .draw(|f| {
            let t = Instant::now();
            view(&app, &theme, f);
            view_us.set(u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX));
        })
        .map(|_| ())
})?;
```

`cargo test --quiet --test it tui_loop::` ⇒ verde. **Commit:** `perf(tui): slow-frame log splits view time from terminal write`

**Passo 3 — `tui_frame` com bytes e console** (`examples/tui_frame.rs`; nada muda nos modos existentes):
- Separe `scenario` em `setup(entries, tabs, query, icons: Icons) -> App` (o que hoje vem antes do `Terminal::new`) e `frames<B: ratatui::backend::Backend>(app: &mut App, term: &mut Terminal<B>, each: impl FnMut(u128))` (o laço de `FRAMES` quadros com Down, chamando `each` com os µs de cada `term.draw`). `scenario` passa a usar os dois com o mesmo `TestBackend::new(120, 40)` e devolve o mesmo `(mediana, p95)`. `enum Icons { Off, Halfblocks, Sixel }`: `Halfblocks` é o `reserve_icon()` de hoje (o `true` atual); `Sixel` usa o mesmo corpo com `wingetrs::tui::icons::picker(IconProtocol::Sixel).expect("sixel picker")` no lugar de `Picker::halfblocks()` (renomeie para `reserve_icon(picker: &Picker)`).
- Cenários dos dois modos novos: `("pesquisar", 0, "", Off)`, `("pesquisar 'code'", 0, "code", Off)`, `("instalados", 1, "", Off)`, `("atualizacoes", 2, "", Off)`, `("pesquisar icones halfblocks", 0, "", Halfblocks)`, `("pesquisar icones sixel", 0, "", Sixel)`.
- Contador:

    ```rust
    /// Counts what the backend writes; the bytes go nowhere.
    #[derive(Default)]
    struct Counter {
        bytes: usize,
        writes: usize,
    }

    impl std::io::Write for Counter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.bytes += b.len();
            self.writes += 1;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    ```

- Modo `bytes`: para cada cenário, 3 séries; `Terminal::with_options(CrosstermBackend::new(Counter::default()), TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, 120, 40)) })` (viewport fixo: não consulta o tamanho de um console que o agente não tem); depois de cada quadro, a diferença de `term.backend().writer().bytes`/`.writes` desde o quadro anterior. Linha por série: `{nome:<28} draw median {} us p95 {} us   bytes median {} max {}   writes median {}`. Note no relatório: `writes` são chamadas ao escritor, não chamadas ao sistema.
- Modo `console`: `crossterm::terminal::enable_raw_mode()` + `EnterAlternateScreen` com um guard cujo `Drop` faz `LeaveAlternateScreen` + `disable_raw_mode` (inclusive em pânico); para cada cenário, 3 rodadas intercaladas de `Terminal::new(CrosstermBackend::new(std::io::stdout()))` e `Terminal::new(CrosstermBackend::new(std::io::BufWriter::with_capacity(1 << 16, std::io::stdout())))` (tamanho real do console); guarde os resultados e só imprima depois do guard cair: `{nome:<28} {stdout|bufwriter:<9} draw median {} us p95 {} us max {} us`, mais a linha `console {w}x{h}`.
- `cargo build --release --example tui_frame --quiet` e `cargo clippy --all-targets --quiet` limpos.

**Passo 4 — medir no harness** (com a máquina liberada; `pwsh -File scripts/leftovers.ps1` antes): `target\release\examples\tui_frame.exe bytes` duas vezes; cole as duas saídas. **Não** rode o modo `console` (o processo do agente não tem console de verdade); escreva no relatório o comando exato para o usuário.

**Passo 5 — parada.** Relatório com: as duas saídas do modo `bytes`, a tabela "cenário × bytes por quadro" e, para o orquestrador, o roteiro do usuário abaixo. `Status: DONE`.

#### Roteiro do usuário (o orquestrador conduz; diz antes o que observar)

1. Num terminal normal (o que o usuário usa para a TUI), na pasta do repositório: `target\release\examples\tui_frame.exe console`. A tela pisca por ~1 min com listas rolando sozinhas; ao voltar, imprime a tabela. Observar: nada para digitar; se a tela não voltar ao normal, `reset`/fechar a aba. Colar a tabela.
2. Opcional, se a TUI abrir: `target\release\wingetrs.exe` por ~2 min, nesta ordem: Pesquisar rolando ~30 linhas com seta para baixo; aba Instalados idem; Atualizações idem; Enter num pacote (detalhes); sair com `q`. O orquestrador lê `frame lento` e `tui stats` do log do dia (`Select-String 'frame lento|tui stats' $env:LOCALAPPDATA\wingetrs\log\wingetrs.log.<data>`), agrupa por `tab`/`icons` e calcula mediana de `view_us` e `flush_us` dos quadros lentos.

#### Fase 2 (só com o número; o orquestrador escreve o brief a partir do relatório)

Escolha pela medida, nunca pelo palpite. Candidatos já previstos e o número que autoriza cada um:

| Se a medida mostrar | Candidato | Prova de ganho |
|---|---|---|
| `flush_us` domina os quadros lentos **e** no modo `console` o `bufwriter` tem mediana/p95 ≥ 30% menores que `stdout` nas 3 rodadas | Terminal da TUI com `CrosstermBackend<BufWriter<Stdout>>` de 64 KiB (troca o `ratatui::init()` por `ratatui::try_init_with_options` ou montagem própria, mantendo o gancho de pânico que restaura o terminal) | modo `console` antes/depois e roteiro 2: p95 de `draw_us` dos quadros lentos ≥ 30% menor |
| `flush_us` domina e os bytes do cenário com ícone Sixel são ≥ 10× os do mesmo cenário sem ícone | Não reenviar a imagem a cada quadro em que a linha só muda de posição (investigar o `ratatui-image` 11: como o protocolo marca as células) — sonda antes do brief | modo `bytes`: bytes medianos do cenário Sixel; roteiro 2 com ícones ligados |
| `view_us` domina num `tab` específico | Spans `tracing` dentro do `view` daquela aba primeiro (Task 59 já cortou o óbvio); só então mexer | `tui_frame` padrão A/B, 3 rodadas intercaladas |
| Nenhum padrão (lentos esparsos, `batch` alto, sem relação com aba) | Nada a otimizar no desenho; anotar no BACKLOG como carga da máquina | — |

Ganho abaixo do critério ⇒ reverte (regra do projeto) e o relatório diz o teto medido.

#### Checks beyond the steps

1. Mutação: `let flush_us = draw_us.saturating_sub(view_us);` ⇒ `let flush_us = draw_us;` ⇒ `slow_frame_log_splits_view_and_flush` **PEGA**.
2. Mutação: `self.max_view_us = self.max_view_us.max(view_us);` movido para dentro do `if draw_us > SLOW_DRAW_US {` (âncora de várias linhas, `-AnchorFile`/`-ReplacementFile`) ⇒ o mesmo teste **PEGA** (`max_view_us=4000`).
3. Mutação: em `draw`, ler o `view_us` antes de `f()` (`let r = f();` e `self.record(info, draw_us, view_us.get());` trocados por `let v = view_us.get(); let r = f();` … `self.record(info, draw_us, v);`) ⇒ `draw_reads_view_time_after_the_frame` **PEGA**.
4. Comando: `pwsh -File scripts/mutate.ps1 -Path src/tui/run.rs -Anchor '<âncora>' -Replacement '<troca>' -TestCommand 'cargo test --quiet --test it tui_loop::'`; confira o nome do teste que falhou.
5. Saída do modo padrão do `tui_frame` antes e depois do passo 3 com as mesmas linhas (os números variam; os nomes e o formato não).
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit (example):** `perf(tui): tui_frame measures bytes per frame and the real console`

---

## Decisões abertas (só o usuário decide)

**D-A1. Cache de nomes MSIX em disco (Task 223, Fase 2).** Só vale se a Fase 1 der ≥ 50 ms de teto.
- (a) Cache `msix-names.json` na pasta de dados, chave = nome completo do pacote + idioma da interface. Vantagens: some o maior custo do `list` e da carga da TUI (~150 ms medidos em 2026-10-17) a partir da 2ª execução; invalida sozinho quando o pacote atualiza ou o idioma muda. Desvantagens: um arquivo a mais; a 1ª execução depois de cada atualização de pacote paga a leitura daquele pacote; um app que trocasse o nome de exibição sem mudar de versão ficaria com o nome velho (não conheço caso).
- (b) Sem cache, aceitar os ~150 ms. Vantagens: nada novo para manter. Desvantagens: `list` continua ~290 ms; o item A sai do BACKLOG sem ganho.
- **Recomendo (a)**, condicionada ao número da Fase 1.

**D-B1. Quem mede no terminal real (Task 224).** O processo do agente não tem console; o log real mostra quadros de até 380 ms que o harness não reproduz.
- (a) Você roda `tui_frame.exe console` (~1 min, sem digitar nada) e, se a TUI abrir, o roteiro de 2 min. Vantagens: é a única medida do custo real de escrever no terminal; A/B de escritor sem mudar o app. Desvantagens: precisa de você na máquina e o resultado depende do terminal que você usa.
- (b) Só o harness (`bytes`). Vantagens: sem você. Desvantagens: mede bytes, não o tempo do terminal para consumi-los; a Fase 2 sairia no palpite, que a regra do projeto proíbe.
- **Recomendo (a)**; o passo 1 sozinho já basta para decidir o candidato do `BufWriter`.

**D-B2. Fase 2 da Task 224 sem nova consulta.**
- (a) O orquestrador escolhe o candidato pela tabela da Fase 2 e despacha direto, desde que o critério numérico da linha seja cumprido. Vantagens: sem ida e volta. Desvantagens: a troca do `ratatui::init()` mexe na restauração do terminal em pânico (risco de deixar o terminal em modo cru se errar).
- (b) Volta para você com os números antes da Fase 2. Vantagens: você vê o achado. Desvantagens: mais uma rodada de espera.
- **Recomendo (a)** para `BufWriter` e para "nada a otimizar"; (b) para o candidato dos ícones Sixel, que exige uma sonda no `ratatui-image` antes.

---

## Bloco CD

Para colar em `docs/superpowers/plans/2026-10-19-wingetrs-pos-2d.md` (depois da Task 220). Nomes e linhas conferidos no `main` em `b241bef` (2026-10-22); as linhas mudam depois da 205 e da 207, então o brief cita símbolos e o implementador confirma as linhas com `rust-analyzer symbols` e `grep`.

## Ordem e conflitos

| Tarefa | Arquivos | Conflita com | Pode rodar |
|---|---|---|---|
| 225 | `src/cli/upgrade_notes.rs` (novo), `src/cli/mod.rs` (`mod` + braço `Command::List`), `src/cli/output.rs` (função nova + 3 `match` de `Phase`), `src/install/progress.rs`, `src/bridge/mod.rs` (só a constante), `src/tui/run.rs` (só a constante), `src/tui/jobs.rs` (um braço do `match` de `progress_msg`), testes | 205 (`bridge/mod.rs`, `tui/jobs.rs`, `cli/output.rs`); 207 (`tui/run.rs`, `tui/jobs.rs`); 210 e 211 (`cli/mod.rs`, `cli/output.rs`; 211 também `tui/jobs.rs`) | depois de 205 e 207; **não** em paralelo com 210/211 (em série com elas, antes ou depois) |
| 226 | `src/tui/app.rs`, `tests/it/wi_tui.rs`, `tests/it/tui_state.rs` | 207 (`tui/app.rs`); 214/218 já feitas | depois de 207; em paralelo com 225, 210 e 211 (nenhum arquivo em comum) |

Ordem sugerida: 205 → 207 → (225 ‖ 226), com 210/211 antes ou depois da 225, nunca junto.

---

### Task 225: Nota de aplicabilidade do winget no `list --upgradable` do CLI

**Modelo:** sonnet (revisor sonnet).

**Depois de:** 205 (`src/bridge/mod.rs`, `src/tui/jobs.rs`, `src/cli/output.rs`) e 207 (`src/tui/run.rs`, `src/tui/jobs.rs`). Não rodar junto com 210/211 (`src/cli/mod.rs`, `src/cli/output.rs`).

> Origem: `docs/superpowers/plans/BACKLOG.md`, item C. A TUI já diz quando o winget recusaria a atualização (Task 135: Android Studio com `UpgradeBehavior: deny`, Anki com escopo trocado); o CLI lista essas linhas como atualizáveis sem aviso.

#### Where this fits

- `bridge::winget_applicability(manifest, scope, arch) -> WingetFit` (`src/bridge/mod.rs`): `WingetFit { applicability: Applicability, explicit_only: bool }`; `Applicability::{Ok, Denied, ScopeMismatch { installed, offered }, Unknown}`. `bridge::applicability_note(Applicability) -> Option<String>` dá o texto de `Denied` e `ScopeMismatch` (`None` para `Ok`/`Unknown`). `bridge::host_arch()`. `bridge::bridge_target(row, pins) -> Result<BridgeJob, &str>`: `Ok` com `job.tool == Tool::Winget` é a linha que a ponte do winget atualizaria (`job.target` = id do pacote, `job.available` = versão do catálogo).
- A TUI escolhe as linhas em `applicability_cmd` (`src/tui/app.rs`, privado): `bridge_target(r, &app.pins)` com `Tool::Winget` e `available` presente ⇒ `ApplicabilityQuery { app_key, package_id, latest, scope, installed }` dentro de `Cmd::FetchApplicability { rows }`, emitido ao receber `Msg::InstalledLoaded`. Busca em `spawn_applicability`/`applicability` (`src/tui/run.rs`): no máximo `MAX_APPLICABILITY_FETCH` (= 4, `const` privado no topo de `src/tui/run.rs`) manifestos ao mesmo tempo; `catalog.get("winget", id)` ⇒ `pkg.winget_hash` ⇒ `sources::winget::details::merged_manifest(&http, &blobs, sources::winget::CDN_BASE, &pkg.entry.id, hash, Some(&latest))` ⇒ `winget_applicability(&manifest, scope, host_arch())`. `BlobStore::new(data.blobs_dir())` (`crate::cache::BlobStore`) guarda versionData e manifesto pelo SHA: a segunda vez não vai à rede.
- `explicit_only` (`RequireExplicitUpgrade`) é lido pela TUI no `U` (fica fora do "atualizar tudo"; teste `explicit_only_left_out_of_upgrade_all` em `tests/it/winget_applicability.rs`). O CLI não o mostra em lugar nenhum.
- CLI: braço `Command::List { upgradable, source, json }` de `run_async` (`src/cli/mod.rs`): `line = install_cmd::line_progress(&cfg)`, fases `Phase::ReadingInstalled` e `Phase::Correlating` (Task 216), `correlate(...)` ⇒ `rows`, filtros `source` e `upgradable`, `line.finish()`, depois `emit::json(&rows)` ou `emit::out_block(&output::list_text(&rows, upgradable))`. `catalog`, `http` e `data` estão no escopo. `crate::pins::load(&data) -> BTreeSet<String>`. `run` usa `rt.block_on` (o futuro não precisa ser `Send`).
- `Phase` (`src/install/progress.rs`): título em `phase_title`, `phase_status` com `match` exaustivo (fase cancelável ⇒ só `text`). Outros `match` exaustivos que precisam do braço novo: três em `src/cli/output.rs` (procure `| Phase::Correlating`) e um em `progress_msg` de `src/tui/jobs.rs`. Teste de títulos: `new_phase_titles` em `tests/it/install_progress.rs`.
- Saída: `src/cli/output.rs` (`paint`, `HEADER`, `DIM`, `NAME`, `bold_count`); testes sem cor com `wingetrs::cli::style::with_enabled(false, || ...)`.
- Testes existentes reaproveitáveis em `tests/it/winget_applicability.rs`: `brow(name)` (linha ARP casada com `Pub.<name>`, `Available { latest: "2" }`, escopo máquina, `app_key = "arp:<name>"`), `manifest(root, installers)`, `installed_rows_ask_for_applicability`.

#### What binds this task

1. **Mesmas linhas e mesmo limite que a TUI.** `pub const MAX_APPLICABILITY_FETCH: usize = 4;` passa para `src/bridge/mod.rs` (doc: "Manifests fetched at a time to judge upgrades, in the TUI and in `list --upgradable`."); `src/tui/run.rs` apaga o seu `const` e usa o de `bridge`. As linhas conferidas pelo CLI são as que a TUI pede em `Cmd::FetchApplicability` (teste de equivalência 1).
2. **Módulo novo `src/cli/upgrade_notes.rs`** (`pub mod upgrade_notes;` em `src/cli/mod.rs`), sem rede própria (a busca é injetada):

```rust
//! winget's verdict on each upgrade of `list --upgradable` (the TUI's `applicability`).

use std::collections::BTreeSet;
use std::future::Future;

use futures_util::StreamExt;

use crate::bridge::{self, MAX_APPLICABILITY_FETCH, Tool, WingetFit};
use crate::installed::correlate::{InstallScope, ListRow};
use crate::sources::winget::manifest::MergedManifest;

/// The note of an upgrade winget takes only when asked by name (`RequireExplicitUpgrade`).
pub const EXPLICIT_ONLY_NOTE: &str =
    "o editor pede atualização explícita: fica fora de \"atualizar tudo\"";

/// One row whose winget upgrade is checked.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteQuery {
    pub app_key: String,
    pub name: String,
    pub package_id: String,
    /// Catalog version whose manifest decides.
    pub latest: String,
    pub scope: Option<InstallScope>,
}

/// One row winget would refuse or leave out, with the reason.
#[derive(Debug, Clone, PartialEq)]
pub struct UpgradeNote {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpgradeNotes {
    pub notes: Vec<UpgradeNote>,
    /// Rows whose manifest could not be read (offline, not in the catalog): no verdict.
    pub unchecked: Vec<String>,
}

/// The rows the TUI checks (`tui::app::applicability_cmd`): upgraded through the winget bridge,
/// with a catalog version, not pinned.
pub fn queries(rows: &[ListRow], pins: &BTreeSet<String>) -> Vec<NoteQuery> { todo!() }

/// The refusal note, else the explicit-only one; `None` when winget takes it in a batch too.
pub fn note_of(fit: WingetFit) -> Option<String> { todo!() }

/// Judges every query, at most `MAX_APPLICABILITY_FETCH` manifests at a time, notes in the
/// order of `queries`. A failed fetch goes to `unchecked` (and `tracing::warn!`), never fails
/// the listing.
pub async fn collect<F, Fut>(queries: Vec<NoteQuery>, arch: &str, fetch: F) -> UpgradeNotes
where
    F: Fn(NoteQuery) -> Fut,
    Fut: Future<Output = anyhow::Result<MergedManifest>>,
{
    todo!()
}
```

   - `queries`: `bridge::bridge_target(r, pins)` `Ok` com `job.tool == Tool::Winget` e `job.available` `Some` (exatamente o filtro de `applicability_cmd`); `name = r.name`, `package_id = job.target`, `latest = job.available`, `scope = r.scope`.
   - `note_of`: `bridge::applicability_note(fit.applicability)` se `Some`; senão `EXPLICIT_ONLY_NOTE` se `fit.explicit_only`; senão `None` (`Unknown` sem `explicit_only` ⇒ `None`).
   - `collect`: `futures_util::stream::iter(queries).map(|q| { let fut = fetch(q.clone()); async move { (q, fut.await) } }).buffered(MAX_APPLICABILITY_FETCH).collect::<Vec<_>>().await` (ordem preservada); `Ok(m)` ⇒ `note_of(bridge::winget_applicability(&m, q.scope, arch))`; `Err(e)` ⇒ `unchecked.push(q.name)` e `tracing::warn!(package = %q.package_id, error = %format!("{e:#}"), "upgrade check failed")`.
3. **Fase nova** `Phase::CheckingUpgrades { count: usize }` (doc: "`list --upgradable`: reading the manifests of the `count` winget upgrades to see whether winget takes them."). Título: `count == 1` ⇒ `"Conferindo 1 atualização com o winget"`, senão `format!("Conferindo {count} atualizações com o winget")`. Cancelável (em `phase_status` vai com as que devolvem só `text`). Nos três `match` de `src/cli/output.rs` e no de `src/tui/jobs.rs`, ao lado de `Phase::Correlating` (mesmo tratamento).
4. **CLI.** No braço `Command::List`, quando `upgradable && !json`, depois dos filtros e antes de `line.finish()`:

```rust
let notes = if upgradable && !json {
    let queries = upgrade_notes::queries(&rows, &crate::pins::load(&data));
    if queries.is_empty() {
        upgrade_notes::UpgradeNotes::default()
    } else {
        line.update(&ProgressEvent::Phase(Phase::CheckingUpgrades {
            count: queries.len(),
        }));
        let blobs = crate::cache::BlobStore::new(data.blobs_dir());
        let (catalog, http, blobs) = (&catalog, &http, &blobs);
        upgrade_notes::collect(queries, crate::bridge::host_arch(), |q| {
            // Read before the future: it then holds no catalog connection.
            let pkg = catalog.get("winget", &q.package_id);
            async move {
                let pkg = pkg?.context("pacote não encontrado no catálogo")?;
                let hash = pkg
                    .winget_hash
                    .as_deref()
                    .context("pacote winget sem hash de manifesto")?;
                let (m, _) = crate::sources::winget::details::merged_manifest(
                    http,
                    blobs,
                    crate::sources::winget::CDN_BASE,
                    &pkg.entry.id,
                    hash,
                    Some(&q.latest),
                )
                .await?;
                Ok(m)
            }
        })
        .await
    }
} else {
    upgrade_notes::UpgradeNotes::default()
};
```

   e na saída de texto: `let mut text = output::list_text(&rows, upgradable); text.push_str(&output::upgrade_notes_text(&notes)); emit::out_block(&text);`. `--json` e o `list` sem `--upgradable` não mudam (nenhuma busca de manifesto). Se `hash` não casar de tipo (`Option<Vec<u8>>` ⇒ `&[u8]`), ajuste como a TUI faz em `applicability`.
5. **Formatador** `pub fn upgrade_notes_text(n: &UpgradeNotes) -> String` em `src/cli/output.rs`: vazio quando `notes` e `unchecked` estão vazios; senão

```
\n{HEADER "Avisos do winget"} ({bold N}):\n
  {NAME nome}: {texto}\n          (uma por nota, na ordem)
\n{DIM "<k> não conferidas com o winget (sem manifesto ou sem rede): A, B"}\n   (só se unchecked não vazio)
```

   Sem cor, literalmente: `"\nAvisos do winget (2):\n  Studio: o editor não permite atualizar pelo winget\n  Anki: a versão nova só instala para usuário; a instalada é para máquina\n"` e, com `unchecked = ["Git"]`, a linha final `"\n1 não conferidas com o winget (sem manifesto ou sem rede): Git\n"` (contagem sem flexão, como `left_out_note`). Sem notas e só `unchecked`, não sai o título "Avisos do winget".

Testes (em `tests/it/winget_applicability.rs`, salvo o 6):

```rust
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wingetrs::bridge::MAX_APPLICABILITY_FETCH;
use wingetrs::cli::upgrade_notes::{
    EXPLICIT_ONLY_NOTE, NoteQuery, UpgradeNote, UpgradeNotes, collect, note_of, queries,
};

/// 1. Equivalence: the CLI checks exactly the rows (and fields) the TUI asks for.
#[test]
fn cli_checks_the_rows_the_tui_checks() {
    let rows = vec![
        brow("Git"),
        ListRow { origin: "wingetrs".into(), ..brow("Ours") },
        ListRow { state: UpdateState::UpToDate, ..brow("Current") },
        ListRow { origin: "scoop".into(), ..brow("Scooped") },
        brow("Pinned"),
        ListRow { scope: None, ..brow("NoScope") },
        ListRow { scope: Some(InstallScope::User), ..brow("Anki") },
    ];
    let mut pins = BTreeSet::new();
    pins.insert(wingetrs::pins::key("arp", "Pinned"));
    let mut a = App::new(Vec::new(), GlyphSet::Nerd);
    a.pins = pins.clone();
    let cmds = update(&mut a, Msg::InstalledLoaded(rows.clone()));
    let tui: Vec<(String, String, String, Option<InstallScope>)> = cmds
        .iter()
        .find_map(|c| match c {
            Cmd::FetchApplicability { rows } => Some(rows.clone()),
            _ => None,
        })
        .unwrap()
        .into_iter()
        .map(|q| (q.app_key, q.package_id, q.latest, q.scope))
        .collect();
    let cli: Vec<_> = queries(&rows, &pins)
        .into_iter()
        .map(|q| (q.app_key, q.package_id, q.latest, q.scope))
        .collect();
    assert_eq!(cli, tui);
    assert_eq!(cli.len(), 3, "Git, NoScope, Anki: {cli:?}");
}

/// 2. The note: refusal first, then explicit-only, else none.
#[test]
fn note_of_each_verdict() {
    let v = |applicability, explicit_only| WingetFit { applicability, explicit_only };
    assert_eq!(
        note_of(v(Applicability::Denied, true)).as_deref(),
        Some("o editor não permite atualizar pelo winget")
    );
    assert_eq!(
        note_of(v(
            Applicability::ScopeMismatch {
                installed: InstallScope::Machine,
                offered: InstallScope::User
            },
            false
        ))
        .as_deref(),
        Some("a versão nova só instala para usuário; a instalada é para máquina")
    );
    assert_eq!(note_of(v(Applicability::Ok, true)).as_deref(), Some(EXPLICIT_ONLY_NOTE));
    assert_eq!(note_of(v(Applicability::Unknown, true)).as_deref(), Some(EXPLICIT_ONLY_NOTE));
    assert_eq!(note_of(v(Applicability::Ok, false)), None);
    assert_eq!(note_of(v(Applicability::Unknown, false)), None);
}

fn q(name: &str, scope: Option<InstallScope>) -> NoteQuery {
    NoteQuery {
        app_key: format!("arp:{name}"),
        name: name.into(),
        package_id: format!("Pub.{name}"),
        latest: "2".into(),
        scope,
    }
}

/// 3. Verdicts in order, a failed fetch is unchecked and does not stop the others.
#[tokio::test]
async fn collect_notes_in_order_and_failures_unchecked() {
    let denied = manifest("UpgradeBehavior: deny\n", &["  Architecture: x64\n"]);
    let user_only = manifest("Scope: user\n", &["  Architecture: x64\n"]);
    let explicit = manifest("RequireExplicitUpgrade: true\n", &["  Architecture: x64\n"]);
    let fine = manifest("", &["  Architecture: x64\n"]);
    let m = Machine();
    let qs = vec![
        q("Studio", m), q("Git", m), q("Anki", m), q("Offline", m), q("Tool", m),
    ];
    let got = collect(qs, "x64", |q| {
        let r = match q.name.as_str() {
            "Studio" => Ok(denied.clone()),
            "Git" => Ok(fine.clone()),
            "Anki" => Ok(user_only.clone()),
            "Tool" => Ok(explicit.clone()),
            _ => Err(anyhow::anyhow!("sem rede")),
        };
        async move { r }
    })
    .await;
    let note = |name: &str, text: &str| UpgradeNote { name: name.into(), text: text.into() };
    assert_eq!(
        got,
        UpgradeNotes {
            notes: vec![
                note("Studio", "o editor não permite atualizar pelo winget"),
                note("Anki", "a versão nova só instala para usuário; a instalada é para máquina"),
                note("Tool", EXPLICIT_ONLY_NOTE),
            ],
            unchecked: vec!["Offline".into()],
        }
    );
}
#[allow(non_snake_case)]
fn Machine() -> Option<InstallScope> {
    Some(InstallScope::Machine)
}

/// 4. Parallel, but never more than the TUI's limit at a time.
#[tokio::test]
async fn collect_fetches_at_most_the_limit_at_a_time() {
    let fine = manifest("", &["  Architecture: x64\n"]);
    let now = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let qs: Vec<_> = (0..10).map(|i| q(&format!("App{i}"), None)).collect();
    let got = collect(qs, "x64", |_| {
        let (now, peak, fine) = (Arc::clone(&now), Arc::clone(&peak), fine.clone());
        async move {
            let n = now.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(n, Ordering::SeqCst);
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            now.fetch_sub(1, Ordering::SeqCst);
            Ok(fine)
        }
    })
    .await;
    assert_eq!(got, UpgradeNotes::default());
    assert_eq!(MAX_APPLICABILITY_FETCH, 4);
    assert_eq!(peak.load(Ordering::SeqCst), MAX_APPLICABILITY_FETCH);
}

/// 5. The text after the table.
#[test]
fn upgrade_notes_text_plain() {
    use wingetrs::cli::output::upgrade_notes_text;
    use wingetrs::cli::style::with_enabled;
    let note = |name: &str, text: &str| UpgradeNote { name: name.into(), text: text.into() };
    let both = UpgradeNotes {
        notes: vec![
            note("Studio", "o editor não permite atualizar pelo winget"),
            note("Anki", "a versão nova só instala para usuário; a instalada é para máquina"),
        ],
        unchecked: vec!["Git".into()],
    };
    assert_eq!(
        with_enabled(false, || upgrade_notes_text(&both)),
        "\nAvisos do winget (2):\n  Studio: o editor não permite atualizar pelo winget\n  Anki: a versão nova só instala para usuário; a instalada é para máquina\n\n1 não conferidas com o winget (sem manifesto ou sem rede): Git\n"
    );
    let only_unchecked = UpgradeNotes { notes: vec![], unchecked: vec!["Git".into(), "Zed".into()] };
    assert_eq!(
        with_enabled(false, || upgrade_notes_text(&only_unchecked)),
        "\n2 não conferidas com o winget (sem manifesto ou sem rede): Git, Zed\n"
    );
    assert_eq!(with_enabled(false, || upgrade_notes_text(&UpgradeNotes::default())), "");
}
```

6. Em `tests/it/install_progress.rs`, `new_phase_titles` ganha duas linhas: `(Phase::CheckingUpgrades { count: 1 }, "Conferindo 1 atualização com o winget · 00:02")` e `(Phase::CheckingUpgrades { count: 3 }, "Conferindo 3 atualizações com o winget · 00:02")`.

Os testes usam os helpers que o arquivo já tem (`brow`, `manifest`); importe o que faltar. O `fn Machine()` é só para encurtar; troque por uma `let` se o clippy reclamar.

**Files:** `src/cli/upgrade_notes.rs` (novo), `src/cli/mod.rs` (`pub mod upgrade_notes;` e o braço `Command::List`), `src/cli/output.rs` (`upgrade_notes_text` e o braço novo nos três `match` de `Phase`), `src/install/progress.rs` (variante, título, `phase_status`), `src/bridge/mod.rs` (só a constante), `src/tui/run.rs` (só trocar a constante), `src/tui/jobs.rs` (só o braço no `match` de `progress_msg`), `tests/it/winget_applicability.rs`, `tests/it/install_progress.rs`.

#### Checks beyond the steps

1. Mutação: em `queries`, tirar o filtro `job.tool == Tool::Winget` (ou trocar `bridge_target` por um filtro só de `UpdateState::Available`) ⇒ teste 1 **PEGA**.
2. Mutação: `.buffered(MAX_APPLICABILITY_FETCH)` ⇒ `.buffered(1)` ⇒ teste 4 **PEGA**; ⇒ `.buffered(usize::MAX)` ⇒ teste 4 **PEGA**.
3. Mutação: `note_of` sem o ramo `explicit_only` ⇒ teste 2 e 3 **PEGAM**.
4. Mutação: `Err(_)` de `fetch` descartado sem ir para `unchecked` ⇒ teste 3 **PEGA**.
5. Medida (orquestrador, máquina quieta, cache de blobs quente): `pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'list --upgradable' -Runs 15 -Baseline target/release/examples/noop.exe` na base e no HEAD; registrar o delta no ledger. Com cache frio, uma rodada só, anotar o tempo (rede).
6. Prova no binário (orquestrador): `wingetrs list --upgradable` num terminal mostra a fase "Conferindo N atualizações com o winget" com relógio e, se Android Studio/Anki estiverem desatualizados, a seção "Avisos do winget"; com o cabo desligado (ou `HTTPS_PROXY` inválido), a lista sai igual, com a linha "não conferidas".
7. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): list --upgradable says when winget would refuse or skip an upgrade`

**Relatório:** `.claude/tasks/task-225-report.md` com status, SHAs, saída dos testes novos (só nomes e `ok`), as quatro mutações com PEGA/SOBREVIVE e qualquer desvio do brief com o motivo.

---

### Task 226: Remover várias linhas ARP/MSIX marcadas na TUI, uma confirmação por item

**Modelo:** sonnet (revisor sonnet).

**Depois de:** 207 (`src/tui/app.rs`). Nenhum arquivo em comum com 225, 210 ou 211.

> Origem: `docs/superpowers/plans/BACKLOG.md`, item D. Desde a Task 187 a TUI remove uma linha ARP/MSIX por vez (`u` ou menu "Remover"), mas com linhas marcadas o `remove_marked` só aceita `origin == "wingetrs"` e as outras viram "pulados". Cada remoção nativa tem escolhas próprias (cópia de pastas compartilhadas, "só o registro" do Kindle), então não cabe numa confirmação de lote.

#### Where this fits

- `remove_selected` ⇒ com marcas `run_button(app, ActionId::Remove)` ⇒ `remove_marked(app, rows, hidden, left_out)` (`src/tui/app.rs`): só `r.origin == "wingetrs"` vira `Cmd::PlanRemove`; o resto vai para `skipped` ⇒ `start_batch(app, items, Vec::new(), skipped, hidden, left_out)` ⇒ toast "pulados: …". `open_actions` (menu de Enter com marcas) conta `removable` só com `origin == "wingetrs"` (item "Remover marcados (n)").
- `installed_actions` já habilita "Remover" para ARP/MSIX (`winget_removable(origin)` = `"arp" | "msix"`), então `ButtonTarget::possible` já conta essas linhas: hoje elas passam pelo filtro e caem no `skipped`.
- Remoção de uma linha: `remove_one` ⇒ `plan_winget_remove(app, &r, 0)` ⇒ `start_planning(app, Cmd::PlanWingetUninstall(RemovalRequest { .. }))`. `RemovalRequest::of_row(&row)` (`src/winget_install/previous.rs`) monta o mesmo pedido para o registro principal (é o que o CLI usa). `start_planning` recusa se `app.planning` e grava `app.planning_confirm = Some(settings.confirm)`.
- Resultado: `Msg::WingetUninstallPlanned(Result<UninstallPlan, String>)` ⇒ `on_winget_uninstall_planned`: `confirm = app.planning_confirm.take().unwrap_or(settings.confirm)`; caminho "só o registro" com recusa do winget ⇒ `confirm_modal(app, vec![Action::WingetRegistryOnly(plan)], vec![r], None)`; senão `admin_filter` ⇒ `start_ready(app, Action::WingetUninstall(plan), confirm, Vec::new())` (confirma se `needs_confirm(confirm, ..)` ou se há `shared`); `Err(message)` ⇒ toast de erro.
- Diálogos: `open_modal` empilha em `pending_modals` se já há um aberto; Enter/Esc numa confirmação fecham e `app.modal = app.pending_modals.pop_front()`. `cancel_confirm` trata Esc.
- `pub fn update(app, msg)` chama `apply` e depois `app.list_icon_cmds()` e `app.list_fetch_cmds()`: o lugar para um passo que roda depois de toda mensagem.
- Testes: `tests/it/wi_tui.rs` (`on_installed`, `installed_row(name, app_key, origin)`, `kindle_row()`, `kindle_plan(bool)`, `confirm_text`, `menu_items`, `pick`, `key`); `tests/it/tui_state.rs` (`on_tab`, `irow`, `space`, `ready_remove`, `plan_remove_cmd`, `removal`, `has_toast`, `confirm_open`). `UninstallPlan { target, name, version, command, elevation, shared }` (`src/winget_install/uninstall.rs`).

#### What binds this task

1. **Cadeia de remoções nativas.** Campo privado novo em `App`:

```rust
/// Marked ARP/MSIX rows removed one after another: each is planned only when no dialog is
/// open and confirmed on its own (`removal_chain_cmds`).
removal_chain: Option<RemovalChain>,
```

```rust
/// What is left of a marked removal of ARP/MSIX rows (Task 226).
#[derive(Debug, Default)]
struct RemovalChain {
    pending: VecDeque<crate::winget_install::previous::RemovalRequest>,
    total: usize,
    /// Position (from 1) and name of the item whose plan is in flight.
    current: Option<(usize, String)>,
}
```

2. **`remove_marked`**: linha com `winget_removable(&r.origin)` ⇒ `RemovalRequest::of_row(r)` na cadeia (não vai para `skipped`); as `wingetrs` seguem para o lote como hoje. Se já existe cadeia, os pedidos novos entram no fim (pulando `app_key` que já está nela) e `total` cresce. As marcas das linhas que entraram na cadeia saem de `app.marks[&Tab::Installed]` na hora (as `wingetrs` continuam saindo quando o lote é confirmado, como hoje). Linhas que não são nem `wingetrs` nem ARP/MSIX continuam em `skipped`.
3. **`open_actions`**: `removable` conta `r.origin == "wingetrs" || winget_removable(&r.origin)`.
4. **Passo depois de toda mensagem** `fn removal_chain_cmds(app: &mut App) -> Vec<Cmd>`, chamado em `update` junto de `list_icon_cmds`/`list_fetch_cmds`. Não faz nada se `app.planning`, se há `app.modal` ou `pending_modals` não vazio, ou se `app.quit_after_running`. Senão: tira o próximo pedido (fila vazia ⇒ `removal_chain = None`), grava `current = Some((total - pending.len(), req.name.clone()))`, chama `start_planning(app, Cmd::PlanWingetUninstall(req))` e **depois** força `app.planning_confirm = Some(ConfirmMode::Always)` (cada item da cadeia confirma, mesmo com "Nunca perguntar").
5. **`on_winget_uninstall_planned`** pega `current` da cadeia (`take`). Com `Some((k, name))`: a nota `format!("Remoção {k} de {total} marcadas")` entra nos `notes` da confirmação (nos dois caminhos: `start_ready` e o de "só o registro"); `Err(message)` vira toast `format!("{name}: {message}")`. Sem cadeia, nada muda.
6. Esc numa confirmação da cadeia pula só aquele item; o próximo é planejado quando o diálogo fecha. Nada muda no lote de apps do wingetrs nem na remoção de uma linha só.

Testes (`tests/it/wi_tui.rs`, salvo o 5):

```rust
fn simple_plan(name: &str) -> UninstallPlan {
    UninstallPlan {
        target: UninstallTarget::Arp { hive: Hive::User, key: name.into() },
        name: name.into(),
        version: Some("1".into()),
        command: None,
        elevation: Elevation::Never,
        shared: None,
    }
}

fn mark(a: &mut App, i: usize) {
    a.selected = i;
    update(a, key(KeyCode::Char(' ')));
}

fn plans_of(cmds: &[Cmd]) -> Vec<RemovalRequest> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::PlanWingetUninstall(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

/// 1. One plan at a time; the next only once the dialog closes; Esc skips only that item.
#[test]
fn marked_arp_and_msix_rows_removed_one_by_one() {
    let alfa = installed_row("Alfa", "arp/hkcu/Alfa", "arp");
    let beta = installed_row("Beta", "msix/Beta_abc", "msix");
    let mut a = on_installed(vec![alfa.clone(), beta.clone()]);
    mark(&mut a, 0);
    mark(&mut a, 1);
    let cmds = update(&mut a, key(KeyCode::Char('u')));
    assert_eq!(cmds, vec![Cmd::PlanWingetUninstall(RemovalRequest::of_row(&alfa))]);
    assert_eq!(a.marked_count(), 0, "marks go when the rows join the chain");
    // The first plan opens its confirmation; the second waits for it.
    let cmds = update(&mut a, Msg::WingetUninstallPlanned(Ok(simple_plan("Alfa"))));
    assert!(plans_of(&cmds).is_empty(), "{cmds:?}");
    assert!(confirm_text(&a).contains("Remoção 1 de 2 marcadas"), "{}", confirm_text(&a));
    // Enter runs it and plans the next.
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(cmds[0], Cmd::Execute(Action::WingetUninstall(simple_plan("Alfa"))));
    assert_eq!(plans_of(&cmds), vec![RemovalRequest::of_row(&beta)]);
    update(&mut a, Msg::WingetUninstallPlanned(Ok(simple_plan("Beta"))));
    assert!(confirm_text(&a).contains("Remoção 2 de 2 marcadas"), "{}", confirm_text(&a));
    // Esc skips it; the chain is over.
    let cmds = update(&mut a, key(KeyCode::Esc));
    assert!(plans_of(&cmds).is_empty() && a.modal.is_none(), "{cmds:?}");
    assert!(plans_of(&update(&mut a, Msg::Tick)).is_empty());
}

/// 2. Each item confirms even with "never ask"; a single row still follows the setting.
#[test]
fn chain_confirms_each_item_even_with_never() {
    let alfa = installed_row("Alfa", "arp/hkcu/Alfa", "arp");
    let beta = installed_row("Beta", "arp/hkcu/Beta", "arp");
    let mut a = on_installed(vec![alfa, beta]);
    a.settings.behavior.confirm = ConfirmMode::Never;
    mark(&mut a, 0);
    mark(&mut a, 1);
    update(&mut a, key(KeyCode::Char('u')));
    let cmds = update(&mut a, Msg::WingetUninstallPlanned(Ok(simple_plan("Alfa"))));
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::Execute(_))), "{cmds:?}");
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })));
    let mut one = on_installed(vec![installed_row("Alfa", "arp/hkcu/Alfa", "arp")]);
    one.settings.behavior.confirm = ConfirmMode::Never;
    update(&mut one, key(KeyCode::Char('u')));
    assert_eq!(
        update(&mut one, Msg::WingetUninstallPlanned(Ok(simple_plan("Alfa")))),
        vec![Cmd::Execute(Action::WingetUninstall(simple_plan("Alfa")))]
    );
}

/// 3. A plan that fails names its row and the chain goes on.
#[test]
fn chain_plan_error_names_the_row_and_moves_on() {
    let alfa = installed_row("Alfa", "arp/hkcu/Alfa", "arp");
    let beta = installed_row("Beta", "arp/hkcu/Beta", "arp");
    let mut a = on_installed(vec![alfa, beta.clone()]);
    mark(&mut a, 0);
    mark(&mut a, 1);
    update(&mut a, key(KeyCode::Char('u')));
    let cmds = update(&mut a, Msg::WingetUninstallPlanned(Err("sem desinstalador".into())));
    assert!(a.toasts.iter().any(|t| t.text == "Alfa: sem desinstalador"), "{:?}", a.toasts);
    assert_eq!(plans_of(&cmds), vec![RemovalRequest::of_row(&beta)]);
}

/// 4. Equivalence: the chain asks for the same removal as `u` on the row alone (other
/// records included), and the menu counts the rows.
#[test]
fn chain_request_equals_single_removal() {
    let kindle = kindle_row();
    let mut alone = on_installed(vec![kindle.clone()]);
    let single = plans_of(&update(&mut alone, key(KeyCode::Char('u'))));
    let mut a = on_installed(vec![kindle.clone(), installed_row("Zeta", "msix/Zeta_abc", "msix")]);
    mark(&mut a, 0);
    mark(&mut a, 1);
    update(&mut a, key(KeyCode::Enter));
    assert!(
        menu_items(&a).contains(&("Remover marcados (2)".to_string(), true, None)),
        "{:?}",
        menu_items(&a)
    );
    let chained = plans_of(&pick(&mut a, "Remover marcados (2)"));
    assert_eq!(chained, single);
    assert_eq!(single, vec![RemovalRequest::of_row(&kindle)]);
}

/// 6. Quitting stops the chain.
#[test]
fn quitting_stops_the_chain() {
    let mut a = on_installed(vec![
        installed_row("Alfa", "arp/hkcu/Alfa", "arp"),
        installed_row("Beta", "arp/hkcu/Beta", "arp"),
    ]);
    mark(&mut a, 0);
    mark(&mut a, 1);
    update(&mut a, key(KeyCode::Char('u')));
    update(&mut a, Msg::WingetUninstallPlanned(Ok(simple_plan("Alfa"))));
    a.quit_after_running = true;
    let cmds = update(&mut a, key(KeyCode::Esc));
    assert!(plans_of(&cmds).is_empty(), "{cmds:?}");
}
```

5. Em `tests/it/tui_state.rs` (helpers do lote de lá):

```rust
/// Mixed marks: the wingetrs batch plans first; the ARP row only after its confirmation.
#[test]
fn remove_marked_mixed_runs_batch_then_chain() {
    let gamma = ListRow {
        app_key: "arp/hkcu/Gamma".into(),
        ..irow("gamma", "arp", "winget", false)
    };
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("alpha", "wingetrs", "scoop:main", false), gamma],
    );
    mark_all(&mut a);
    let cmds = update(&mut a, key(KeyCode::Char('u')));
    assert_eq!(cmds, vec![Cmd::PlanBatch { items: vec![plan_remove_cmd("alpha")] }]);
    assert!(!has_toast(&a, "pulados"), "{:?}", a.toasts);
    let cmds = update(&mut a, Msg::BatchPlanned(vec![ready_remove("alpha")]));
    assert!(confirm_open(&a));
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::PlanWingetUninstall(_))), "{cmds:?}");
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(cmds[0], Cmd::Execute(removal("alpha")));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::PlanWingetUninstall(r) if r.app_key == "arp/hkcu/Gamma")),
        "{cmds:?}"
    );
}
```

Ajuste os `use` (`UninstallPlan`, `UninstallTarget`, `Elevation`, `Hive`, `RemovalRequest`, `ListRow` já estão no topo do `wi_tui.rs`). Se `installed_view` ordenar as linhas de outro jeito, marque pelo nome (procure a linha na vista) em vez de mudar a asserção.

**Files:** `src/tui/app.rs` (`App`, `RemovalChain`, `remove_marked`, `open_actions`, `update`, `removal_chain_cmds`, `on_winget_uninstall_planned`), `tests/it/wi_tui.rs`, `tests/it/tui_state.rs`.

#### Checks beyond the steps

1. Mutação: `removal_chain_cmds` sem a guarda de diálogo aberto (`app.modal.is_some() || !app.pending_modals.is_empty()`) ⇒ teste 1 **PEGA** (o 2º plano sai com a confirmação do 1º aberta) e teste 5 **PEGA**.
2. Mutação: sem `app.planning_confirm = Some(ConfirmMode::Always)` ⇒ teste 2 **PEGA**.
3. Mutação: `remove_marked` volta a mandar ARP/MSIX para `skipped` ⇒ testes 1 e 5 **PEGAM**.
4. Mutação: posição `total - pending.len()` ⇒ `pending.len() + 1` ⇒ teste 1 **PEGA**.
5. Mutação: sem a guarda `quit_after_running` ⇒ teste 6 **PEGA**.
6. Mutação: `open_actions` conta só `wingetrs` ⇒ teste 4 **PEGA**.
7. Prova na TUI (usuário, no fim da onda): marcar dois apps ARP de escopo usuário descartáveis, `u`, conferir "Remoção 1 de 2 marcadas", Esc no primeiro e Enter no segundo.
8. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(tui): remove marked ARP/MSIX rows one after another, each with its own confirmation`

**Relatório:** `.claude/tasks/task-226-report.md` com status, SHAs, nomes dos testes novos com `ok`, as seis mutações com PEGA/SOBREVIVE e desvios do brief com o motivo.

---

## Decisões abertas (só o usuário)

**C1. `list --upgradable --json` também leva as notas?**
- A. Não, o JSON fica como hoje (sem busca de manifesto). Prós: nada muda para quem já consome o JSON, e a saída continua sem rede. Contras: script que lê o JSON não sabe da recusa.
- B. Sim, com um campo `winget_note` por linha. Prós: informação completa. Contras: precisa de um tipo de saída novo em volta do `ListRow` (o `ListRow` é serializado direto) e torna o `--json` dependente de rede.
- **Recomendo A.** Hoje ninguém consome esse JSON, e B pode virar tarefa depois. O plano acima já segue A.

**D1. Esc numa confirmação da cadeia: pula só aquele item ou cancela os que faltam?**
- A. Pula só aquele (o plano acima). Prós: "uma confirmação por item" literal, e cada remoção é decidida sozinha. Contras: para desistir de 5 remoções, são 5 Esc.
- B. Esc cancela o resto, com toast "remoções restantes canceladas (N)". Prós: sai rápido. Contras: não dá para pular um item no meio sem perder os outros.
- C. Como A, mais um botão "Cancelar as restantes" na confirmação. Prós: dá as duas coisas. Contras: muda o diálogo de confirmação (mais código e mais teste em `view.rs`, arquivo da 207).
- **Recomendo A** nesta tarefa e C no BACKLOG se o uso mostrar que faz falta.

**D2. A cadeia confirma cada item mesmo com "Nunca perguntar"?**
- A. Sim (o plano acima). Prós: são desinstaladores de terceiros, e o BACKLOG pede uma confirmação por item. Contras: ignora a configuração de quem escolheu "Nunca".
- B. Segue a configuração (com "Nunca", só confirma o item que tem pastas compartilhadas). Prós: coerente com a remoção de uma linha só. Contras: marcar várias linhas e apertar `u` dispara vários desinstaladores sem nenhuma pergunta.
- **Recomendo A.**

---

## Bloco EH

## Itens E e H do BACKLOG (Tasks 227 e 228)

> Planejado em 2026-10-22. As duas seções abaixo já trazem a opção recomendada; as alternativas e os números ficam neste bloco, que o `brief.ps1` não extrai (fica fora de `### Task N`).

### Decisões

**d-E (Task 227): como o portão de scripts usa o parser real do PowerShell.**

Sonda de 2026-10-22 (scripts sintéticos no scratchpad; só `[Parser]::ParseInput` e leitura do AST, nada executado). A máquina estava carregada: `cmd /c exit` levou 198 ms (mediana de 7), então os números absolutos estão inflados; as razões valem.

| Medida (mediana) | pwsh 7.6 | Windows PowerShell 5.1 |
|---|---:|---:|
| 1 processo, 1 script (`-File`, com `ConvertTo-Json`) | 2042 ms (7 corridas) | 2012 ms |
| 1 processo, 20 scripts pelo stdin | 2596 ms (≈ 29 ms por script a mais) | 3322 ms (≈ 69 ms por script a mais) |
| 1 processo, 1 script, `-EncodedCommand`, ambiente zerado (só `SystemRoot` e `PATH`) | 1827 ms | 1772 ms |

O que a sonda mostrou:
- Os oito furos do léxico das Tasks 164 e 192 aparecem como dados no AST: `–Verb RunAs` (travessão) vira o parâmetro `Verb` com a constante `RunAs`; `Start-Job { Remove-Item C:\Windows\x }` e `ForEach-Object -Parallel { reg add HKCU\x }` trazem o comando de dentro (`FindAll(..., $true)`); `&("i"+"ex")` e `&(Get-Command iex)` são `CommandAst` sem nome com cabeça `ParenExpressionAst`; `.\setup.ps1 /S` tem nome `.\setup.ps1`; `cmd /c "del x"`, `sal z iex` e `Invoke-Expression -Command` têm nome e argumentos; `[IO.File]::WriteAllText(...)` é `InvokeMemberExpressionAst` estático; aspas tipográficas (`'abc’; Write-Output HIDDEN; ‘x'`) mostram o `Write-Output` escondido.
- **O 7 e o 5.1 leem o mesmo texto de jeitos diferentes:** `` i`ex `` é `iex` no 5.1 e `i<ESC>x` no 7 (`` `e `` é escape só no 6+). Por isso o parser tem de ser o **mesmo programa** que vai rodar o script (o de `PwshHost::locate`), nunca "o 7 se houver".
- `using module '<psm1 que grava um marcador>'` (com e sem `class`), `using assembly '<dll inexistente>'` e `[ValidateScript({ ... })]` em classe: o `ParseInput` não rodou nada (marcador ausente) nos dois PowerShell.
- O 5.1 sem `LOCALAPPDATA` escreve progresso CLIXML ("Preparando módulos para primeiro uso") no stderr quando o script usa um cmdlet (autoload do `ConvertTo-Json`). O script anfitrião de produção não usa cmdlet nenhum.

Opções:
- **A. Um pwsh por análise.** Cada `scan` abre um processo. Custo: ≈ 2 s por chamada na máquina carregada (estimativa ociosa 0,3–0,5 s, não medida); o plano, a tela de confirmação e o executor analisam o mesmo script, e cada arquivo chamado (Task 192) é outra chamada: um `install` de 10 apps com 2 scripts cada passa de 40 s carregado. Código: o menor (sem protocolo de várias requisições). Fraco no TUI (cada `show` paga a partida).
- **B. Um processo reaproveitado por execução do wingetrs (recomendada).** Sobe no primeiro script (paga a partida uma vez), recebe um script por linha no stdin, devolve os fatos e fica vivo até o fim do comando ou da TUI; cache por SHA-256 do texto (plano, confirmação e executor reusam). Custo: uma partida (≈ 1,8–2 s carregado) + ≈ 30 ms (7) ou 70 ms (5.1) por script. Código: ≈ 700–900 linhas com testes (módulo de regras puro + processo + fiação). Sem script no plano, custo zero (o processo nem sobe).
- **C. Manter o léxico próprio.** Custo zero de tempo e de código agora. Contra: oito furos seguidos, cada um achado só por revisão adversarial; o próximo também será (ARMADILHAS, entrada da Task 164).

Recomendação: **B**, com o AST **somando** ao léxico (vale o pior veredito dos dois; o AST nunca relaxa um veredito do léxico). Sem PowerShell nenhum, com o parser falhando, travando ou devolvendo lixo: o script é **recusado** (`Refuse`), não volta para o léxico sozinho; um script que o parser não lê não roda (o `PwshHost::run` usa o mesmo programa e falharia, ou o script tem erro de sintaxe). Sem pwsh 7 usa o 5.1, porque é ele que rodaria o script.

**d-H (Task 228): `winget uninstall` de pacote de escopo usuário num terminal de administrador.**

- **A. Token do Explorer + `CreateProcessWithTokenW` (recomendada, depois das sondas S1–S4 da seção).** O processo elevado abre o `explorer.exe` da sessão (`GetShellWindow` ⇒ `GetWindowThreadProcessId`), duplica o token primário (integridade média, o mesmo usuário) e roda o `winget.exe` de caminho absoluto (`elevate_win::winget_exe`) com esse token, ambiente de `CreateEnvironmentBlock` do token, pipes no stdout/stderr e Job object. Mantém progresso, código de saída e Ctrl+C. Precisa de `SeImpersonatePrivilege` (o token admin elevado tem) e do serviço `seclogon`. Falha de qualquer pré-condição (sem Explorer, Explorer de outro usuário, Explorer elevado com UAC desligado, `seclogon` parado) ⇒ a recusa de hoje, com texto melhor. Custo: ≈ 400–500 linhas (Win32 com `unsafe`), opus; prova só num terminal de administrador (usuário).
- **B. `runas /trustlevel:0x20000`.** Token SAFER "usuário básico": a ARMADILHAS (gate numa sessão elevada, 2026-10-20) mostra 9 de 17 testes que exigem não elevado falhando mesmo assim; o `runas` abre outra janela e não devolve saída nem código. Descartada.
- **C. `IShellDispatch2::ShellExecute` pelo shell do desktop** (o que o BACKLOG anotou). Não elevado de verdade, mas sem saída, sem código de saída e sem esperar o fim; precisaria de pipe nomeado ou arquivo para voltar o resultado, e o filho não morre com Ctrl+C. Custo parecido com A e resultado pior.
- **D. Só recusar, com mensagem melhor.** O texto da 218 ganha o comando exato a repetir (`wingetrs remove <id>`) num terminal sem administrador. ≈ 30 linhas, sonnet. Não resolve, só orienta.

Recomendação: **A**, com D como queda automática quando A não é possível (a seção já inclui o texto de D). **Scripts do Scoop num terminal de administrador continuam recusados (218), mesmo com A disponível**: o script rodaria como usuário comum, mas o wingetrs elevado aplicaria depois os efeitos em pastas do usuário que o script acabou de mexer (junção plantada ⇒ escrita como administrador), o mesmo risco que a regra de código elevado proíbe. Se um dia quiser, é outra tarefa.

Achado da leitura, entra na 228: `uninstall_by_winget` (`src/winget_install/uninstall.rs:597`) chama `ctx.bridge("winget", ...)`, que no CLI é `run_tool` ⇒ `bridge::spawn_win::run` ⇒ `Command::new("winget")` (busca no `PATH`). Num terminal de administrador a rota HKLM (permitida pela 218) faz um processo elevado procurar `winget` no `PATH` do usuário, o caso exato da regra de código elevado (revisão da 137).

### Task 227: Portão de scripts com o parser real do PowerShell

**Modelo:** opus (revisor opus, adversarial: a revisão tenta furar o portão com as formas da ARMADILHAS e com as do teste 12).

**Depois de:** 216 e 219/220 terminadas (nenhum arquivo em comum, mas `src/install/executor.rs` e `src/cli/output.rs` são quentes). Nenhum arquivo em comum com 228.

> Origem: BACKLOG item E. Decisão do usuário: opção B (um processo reaproveitado por execução, AST somando ao léxico, falha recusa).

#### Where this fits

- Portão de hoje: `src/install/script_scan.rs` (léxico próprio, ≈ 2400 linhas). API usada fora dele: `scan_called(script, supported, roots) -> (ScriptReport, Vec<CalledScript>)` (`:943`), `ps1_installer_report` (`:886`), `env_var_names` (`:1134`), `decode_script` (`:1084`). Tipos: `Verdict { Confirm, ConfirmStrong, Refuse }` (`:291`, ordenado, o pior vence), `FlagKind` (`:298`), `Flag { kind, verdict, detail, line }` (`:330`), `ScriptReport { commands, variables, flags, verdict }` (`:340`), `CallRoots` (`:903`), `CalledScript` (`:926`). Textos `D_*` (`:427-455`, privados hoje). Regras por linha em `line_rules` (`:673`): `runas` ⇒ `Elevation`/`Refuse`; HKLM ⇒ `Hklm`/`Refuse`; escrita em pasta do sistema ⇒ `SystemWrite`/`Refuse`; `msiexec` ⇒ `Msiexec`/`Refuse`; `iex`/`[scriptblock]::create`/`& $var` ⇒ `DynamicCode`/`ConfirmStrong`; download ⇒ `Download`/`ConfirmStrong`; HKCU/`reg` ⇒ `RegistryUser`/`ConfirmStrong`; tarefa/serviço ⇒ `ConfirmStrong`; caminho absoluto ⇒ `AbsolutePath`/`ConfirmStrong`; alias ⇒ `Alias`/`ConfirmStrong`; lançador (`cmd /c`, `Start-Process`) ⇒ `CalledFile`/`ConfirmStrong`; `__wr` ⇒ `PreludeInternal`/`Refuse`; redefinição do prelúdio ⇒ `PreludeRedefinition`/`Refuse`; `SetEnvironmentVariable` User/Machine ⇒ `Refuse`, processo ⇒ `ConfirmStrong`. Código dentro de `if ($global) { }` no runspace do runner vira aviso "só roda em instalação global" (`global_only`, `global_dead_mask` `:1745`).
- Chamadores de produção de `scan_called`: `src/install/planner.rs:248` (`hook_step`, recusa no plano ⇒ `PlanError::ScriptRefused`), `src/install/executor.rs:1123` (`scan_ps1_file`) e `:1145` (`refuse_called`, antes de rodar), `src/cli/output.rs:1071` (`script_items`, o que o CLI e a TUI mostram na confirmação; a TUI usa o mesmo `plan_third_party`).
- Execução: `src/install/script_host_win.rs`: `PwshHost::locate()` (`:47`, pwsh 7 em `<ProgramFiles>\PowerShell\7\pwsh.exe` pelo KnownFolder, senão `<System32>\WindowsPowerShell\v1.0\powershell.exe`), `program()` (`:83`), `child_path(program)` (`:154`, `PATH` fixo), `known(id)` (`:174`, privado). Recusa elevado: `script_policy::refuse_if_elevated`; política de grupo: `gpo_blocks`. Processo com pipes: `bridge::spawn_win::run_spec`/`RunSpec` (`src/bridge/spawn_win.rs:245`), Job object: `run_in_job` (`:493`).
- Fiação: `OpsCtx.runners: executor::Runners` (`src/install/ops.rs:131`), montado em `src/install/prod_win.rs:77-82` (`scripts: Some(Arc::new(PwshHost::locate()))`); `Runners::disabled()` nos testes. `PlanInput` (`src/install/planner.rs:28`) é montado em `src/install/ops.rs:616` e `:658` e em 28 lugares nos testes.
- Testes de hoje: `tests/it/script_scan.rs` (léxico, não muda), `tests/it/install_planner.rs`, `tests/it/cli_scripts.rs`, `tests/it/script_host_live.rs` (único lugar onde `PwshHost::locate` pode aparecer em teste; `check_rules` verifica).

#### What binds this task

1. **Fatos do AST, formato de fio.** Módulo puro novo `src/install/script_ast.rs`. O anfitrião PowerShell (`src/install/ast_facts.ps1`, `include_str!`) lê do stdin uma requisição por linha, `<id>\t<base64 do script em UTF-8>`, chama `[System.Management.Automation.Language.Parser]::ParseInput` e escreve um registro por linha, campos separados por TAB, terminando com `END\t<id>`. Texto livre nos campos é escapado só em `%` ⇒ `%25`, TAB ⇒ `%09`, CR ⇒ `%0D`, LF ⇒ `%0A`. Registros:
   - `ERR\t<linha>\t<ErrorId>` (cada `ParseError`);
   - `USE\t<linha>\t<module|assembly|namespace|type>` (cada `UsingStatementAst`, `UsingStatementKind` em minúsculas);
   - `CMD\t<linha>\t<op>\t<cabeça>\t<cadeia>\t<nome>\t<arg>...` para cada `CommandAst` (`FindAll` com blocos aninhados): `op` = `none|amp|dot` (`InvocationOperator`); `cabeça` = tipo do primeiro elemento sem o sufixo `ExpressionAst`/`Ast` (`StringConstant`, `ExpandableString`, `ScriptBlock`, `Paren`, `Variable`, `Member`, ...); `nome` = `GetCommandName()` ou vazio; cada `arg` (elementos depois do primeiro) = `p:<ParameterName>` (e, se `-p:valor`, mais um `arg` com o valor), `s:<valor da StringConstant>`, `e:<texto cru da ExpandableString>`, ou `?:<tipo>`;
   - `MEM\t<linha>\t<0|1 static>\t<cadeia>\t<alvo>\t<membro>\t<arg>...` para cada `InvokeMemberExpressionAst` e `MemberExpressionAst`: `alvo` = `[<TypeName.FullName>]` quando é `TypeExpressionAst`, senão o texto do `Extent`; `membro` = texto do `Member`; `arg` como em `CMD` (texto cru para o que não for constante);
   - `SET\t<linha>\t<cadeia>\t<variável>` para cada `AssignmentStatementAst` cujo lado esquerdo é `VariableExpressionAst` (`VariablePath.UserPath`);
   - `FN\t<linha>\t<cadeia>\t<nome>` para cada `FunctionDefinitionAst`.
   `cadeia` = ancestrais da raiz até o nó, uma letra cada: `B` (`ScriptBlockExpressionAst`), `F` (`FunctionDefinitionAst`), `G` (dentro do corpo da **primeira** cláusula de um `IfStatementAst` cuja condição é exatamente `$global` ou `$global -and $<nome>`); `.` quando vazia. `elseif`/`else` de um `if ($global)` não ganham `G`.
2. **Anfitrião sem efeitos.** O `ast_facts.ps1` começa com `$PSModuleAutoLoadingPreference = 'None'` e `$ProgressPreference = 'SilentlyContinue'`, usa só a linguagem e tipos .NET (nenhum cmdlet, nenhum `Invoke`, `GetScriptBlock`, `[scriptblock]::Create`, `.Invoke(`, `iex`, `&` de chamada), não grava arquivo e não abre rede. O script analisado nunca toca o disco.
3. **Processo do parser (opção B).** `src/install/script_ast_win.rs` (`#[cfg(windows)]`): `pub struct PwshAst` com `for_program(program: PathBuf) -> PwshAst` e o ponto de teste `with_checks(program, elevated: fn() -> io::Result<bool>, gpo: fn(&Path) -> GpoValues, first: Duration, each: Duration)`. Implementa `AstGate`. Regras:
   - **Mesmo programa do `PwshHost`**: em produção, `prod_win.rs` cria `PwshHost::locate()` e `PwshAst::for_program(host.program().to_path_buf())` do mesmo valor. Programa relativo ou inexistente ⇒ `AstError::NoPowerShell`.
   - Nunca sobe elevado: `elevated()` `Ok(true)` ou `Err` ⇒ `AstError::Elevated` sem iniciar processo (a 218 já recusa scripts num terminal de administrador). Política de grupo que bloqueia scripts (`gpo_blocks`) ⇒ `AstError::Policy(<o texto de gpo_blocks>)` sem iniciar processo.
   - Linha de comando fixa: `-NoLogo -NoProfile -NonInteractive -EncodedCommand <ast_facts.ps1 em UTF-16LE base64>` (sem `-File`, sem `-ExecutionPolicy`, sem argumento vindo do script). Ambiente montado do zero (`env_clear`): `SystemRoot` (KnownFolder Windows), `PATH` = `child_path(program)`, `POWERSHELL_TELEMETRY_OPTOUT=1`, `POWERSHELL_UPDATECHECK=Off`. Pasta de trabalho: `System32` (KnownFolder). Filho num Job object com `KILL_ON_JOB_CLOSE`.
   - Sobe no primeiro pedido e fica vivo (`Mutex<Option<...>>`) até o `Drop` do `PwshAst`, que fecha o stdin, espera até 2 s e mata o Job. Cache `HashMap<[u8; 32], Result<AstFacts, AstError>>` pelo SHA-256 do texto.
   - Prazo: primeira resposta em até 20 s (inclui a partida), as seguintes em até 10 s cada; passou ⇒ mata o processo, `AstError::Timeout` para aquele script; o próximo pedido sobe um processo novo. Processo morto, `END` com outro id, linha que não decodifica ou resposta acima de 1 MiB ⇒ `AstError::Protocol(..)` para aquele script e processo descartado. Leitura do stdout numa thread juntada em todos os caminhos (regra das threads antecipadas).
4. **Regras do AST**, função pura `pub fn ast_flags(facts: &AstFacts, supported: &[&str]) -> Vec<Flag>` em `script_ast.rs`, com os mesmos `FlagKind`, vereditos e textos `D_*` do léxico (os `D_*` passam a `pub(crate)`). Nome de comando comparado em minúsculas, sem `.exe` para os programas. Um nó cuja `cadeia` tem `G` e **não** tem `B` é código morto (o léxico já mostra o aviso de `global_only`) e não gera flag do AST; qualquer `B` na cadeia ⇒ analisado normalmente.
   | Fato | Flag |
   |---|---|
   | algum `ERR` | `SyntaxError` (novo), `Refuse`, "o PowerShell não entende o script (erro de sintaxe na linha N)" |
   | `USE` `module` ou `assembly` | `DynamicCode`, `Refuse`, "carrega módulo ou assembly (using)" |
   | `CMD` nome `iex`/`invoke-expression` | `DynamicCode`, `ConfirmStrong`, `D_DYNAMIC` |
   | `CMD` `amp`/`dot` com nome vazio e cabeça fora de `StringConstant`/`ExpandableString`/`ScriptBlock` | `CalledFile`, `ConfirmStrong`, `D_COMPUTED` |
   | `CMD` com algum arg `s:`/`e:` igual a `runas` (sem diferença de caixa), ou nome `sudo`/`gsudo` | `Elevation`, `Refuse`, `D_ELEVATION` |
   | `CMD` com arg `s:`/`e:` contendo `hklm:`, `hklm\` ou `hkey_local_machine`; `MEM` com alvo `[Microsoft.Win32.Registry]`/`[Registry]` e membro `LocalMachine` | `Hklm`, `Refuse`, `D_HKLM` |
   | `CMD` de `WRITE_CMDLETS` com arg contendo item de `SYSTEM_DIRS`; `MEM` `[IO.File]`/`[System.IO.File]` com membro `Write*`/`Append*`/`Copy`/`Move`/`Delete` e arg contendo item de `SYSTEM_DIRS` | `SystemWrite`, `Refuse`, `D_SYSTEM` |
   | `CMD` nome `msiexec`, ou `start-process`/`saps`/`start` com arg `msiexec` | `Msiexec`, `Refuse`, `D_MSIEXEC` |
   | `CMD` nome `scoop` | `ScoopCommand`, `Refuse`, `D_SCOOP` |
   | `CMD` nome em `SCOOP_FUNCTIONS` e fora de `supported` | `UnknownScoopFunction`, `Refuse` (texto do léxico) |
   | `CMD` nome em `DOWNLOAD_WORDS`; `MEM` membro `DownloadFile`/`DownloadString` | `Download`, `ConfirmStrong`, `D_DOWNLOAD` |
   | `CMD` nome em `TASK_WORDS` | `ScheduledTaskOrService`, `ConfirmStrong`, `D_TASK` |
   | `CMD` nome `set-alias`/`new-alias`/`sal`/`nal` | `Alias`, `ConfirmStrong`, `D_ALIAS` |
   | `CMD` nome `cmd` com arg `s:/c`/`s:/k`; `powershell`/`pwsh` com `p:c`/`p:command`/`p:file`; `start-process`/`saps`/`start` sem `msiexec`; `MEM` `[Diagnostics.Process]::Start` | `CalledFile`, `ConfirmStrong`, `D_LAUNCHED` |
   | `CMD` `powershell`/`pwsh` com `p:e`/`p:ec`/`p:enc`/`p:encodedcommand` | `HiddenCode`, `Refuse`, `D_HIDDEN` |
   | `MEM` `[ScriptBlock]::Create`, `[PowerShell]::Create`, `[RunspaceFactory]::*`, `$ExecutionContext.InvokeCommand.*` | `DynamicCode`, `ConfirmStrong`, `D_DYNAMIC` |
   | `MEM` `[Reflection.Assembly]::Load*`, `[Activator]::CreateInstance` | `DynamicCode`, `Refuse`, "carrega código .NET na hora" |
   | `MEM` `[Environment]::SetEnvironmentVariable` com 3º arg `s:User`/`s:Machine` (ou `e:` com `user`/`machine`) | `EnvironmentVariable`, `Refuse`, `D_ENV_PERSISTENT`; outro 3º arg ou nenhum ⇒ `ConfirmStrong`, `D_ENV_PROCESS` |
   | `SET` de `global` (prefixo de escopo ignorado), `CMD` `set-variable`/`new-variable`/`sv`/`nv` com arg `s:global` | `GlobalName`, `Refuse`, `D_GLOBAL_NAME` |
   | `__wr` em nome, arg, alvo, membro ou variável | `PreludeInternal`, `Refuse`, `D_PRELUDE_INTERNAL` |
   | `FN` com nome de `PRELUDE_FUNCTIONS` (sem diferença de caixa, prefixo de escopo ignorado) | `PreludeRedefinition`, `Refuse` (texto do léxico) |
   Tipos comparados em minúsculas, sem colchetes e sem o prefixo `system.`.
5. **União com o léxico, nunca relaxa.** `script_scan` ganha `pub fn scan_gated(script, supported, roots, ast: &dyn AstGate) -> (ScriptReport, Vec<CalledScript>)`: roda `scan_called` e, para o script e para cada `CalledScript`, junta `ast_flags` às flags do léxico (mesma `kind` e linha ⇒ fica a de pior veredito; `dedupe_reasons` como hoje), recalcula `verdict` como o pior de todas. `AstError` de um texto ⇒ uma flag `ParserFailed` (nova), `Refuse`, linha 1, com o texto do erro em português: `NoPowerShell` "PowerShell não encontrado: o script não pode ser analisado", `Elevated` "o wingetrs não analisa scripts com o processo elevado", `Policy(m)` = `m`, `Timeout` "a análise do PowerShell passou de N s", `Protocol(_)` "a análise do PowerShell falhou". `scan_called` continua existindo (testes do léxico e o próprio `scan_gated`).
6. **Uma porta só em produção.** `pub trait AstGate: Send + Sync { fn facts(&self, script: &str) -> Result<AstFacts, AstError>; }`. `Runners` ganha `pub ast: Arc<dyn AstGate>`; `Runners::disabled()` usa `script_ast::Unavailable` (devolve `AstError::NoPowerShell` ⇒ recusa: o padrão é seguro). `PlanInput` ganha `pub ast: &'a dyn AstGate` (de `ctx.runners.ast` em `ops.rs`). `hook_step`, `scan_ps1_file`, `refuse_called` e `script_items`/`plan_third_party` passam a `scan_gated` com o mesmo `AstGate`. Para testes existe `#[doc(hidden)] pub struct LexerOnly;` (fatos vazios); os testes que planejam scripts e esperam o veredito do léxico passam `&LexerOnly`.
7. **Duas regras novas no `check_rules`:** (a) `scan_called(` em `src/` fora de `src/install/script_scan.rs` falha ("use scan_gated"); (b) `LexerOnly` em `src/` fora de `src/install/script_ast.rs` falha. Os testes de autoverificação (`scripts/check_rules_selftest.ps1`) ganham um caso que planta cada uma.
8. **Medir antes de ligar.** `examples/gate_ast_delta.rs`: lê os scripts dos manifestos scoop do `catalog.db` (como `examples/oracle_2c.rs`), analisa cada um com `LexerOnly` e com um `PwshAst` de `PwshHost::locate()`, e imprime: total, contagem por veredito antes/depois, a lista de scripts cujo veredito piorou (app, hook, `kind`, linha), quantos falharam no parser, e o tempo (partida, total, por script). Só análise; nenhum script roda. Quem roda é o orquestrador, com pedido do usuário (toca scripts reais, mesmo sem executá-los). Se mais de 5% dos scripts hoje aceitos passarem a `Refuse`, a tarefa para e volta para decisão antes do commit da fiação.

Testes literais. Em `tests/it/script_ast.rs` (novo; `mod` no `main.rs`), com:
```rust
use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
use wingetrs::install::script_ast::{self, AstError, AstFacts, AstGate};
use wingetrs::install::script_scan::{self, CallRoots, FlagKind, Verdict};

fn flags(wire: &str) -> Vec<(FlagKind, Verdict)> {
    let facts = script_ast::decode(wire).expect("wire");
    script_ast::ast_flags(&facts, PRELUDE_FUNCTIONS)
        .into_iter()
        .map(|f| (f.kind, f.verdict))
        .collect()
}

struct Fake(std::collections::HashMap<String, Result<String, AstError>>);
impl AstGate for Fake {
    fn facts(&self, script: &str) -> Result<AstFacts, AstError> {
        match self.0.get(script) {
            Some(Ok(w)) => Ok(script_ast::decode(w).expect("wire")),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(AstFacts::default()),
        }
    }
}
```
1. `decode_round_trip`:
   ```rust
   let f = script_ast::decode("CMD\t3\tnone\tStringConstant\tB\tRemove-Item\ts:C:\\Windows\\x\nEND\t1\n").unwrap();
   assert_eq!(f.commands.len(), 1);
   assert_eq!(f.commands[0].line, 3);
   assert_eq!(f.commands[0].name.as_deref(), Some("Remove-Item"));
   assert_eq!(f.commands[0].chain, "B");
   assert_eq!(f.commands[0].args, vec![script_ast::Arg::Str(r"C:\Windows\x".into())]);
   assert_eq!(script_ast::decode("CMD\t%0A\n").unwrap_err().to_string().is_empty(), false);
   assert_eq!(script_ast::decode("CMD\t1\tnone\tStringConstant\t.\ta%09b\n").unwrap().commands[0].name.as_deref(), Some("a\tb"));
   ```
2. `backtick_iex_is_dynamic` (o furo do 5.1): `flags("CMD\t2\tnone\tStringConstant\t.\tiex\ts:x\n")` contém `(DynamicCode, ConfirmStrong)`.
3. `en_dash_runas_refused`: `flags("CMD\t1\tnone\tStringConstant\t.\tStart-Process\ts:notepad\tp:Verb\ts:RunAs\n")` contém `(Elevation, Refuse)`.
4. `job_block_is_live_code`: `flags("CMD\t3\tnone\tStringConstant\t.\tStart-Job\t?:ScriptBlock\nCMD\t3\tnone\tStringConstant\tB\tRemove-Item\ts:C:\\Windows\\x\n")` contém `(SystemWrite, Refuse)`.
5. `paren_head_is_computed`: `flags("CMD\t4\tamp\tParen\t.\t\ts:y\n")` contém `(CalledFile, ConfirmStrong)`.
6. `dead_global_branch_has_no_ast_flag`: `flags("CMD\t8\tnone\tStringConstant\tG\tSet-ItemProperty\ts:HKLM:\\x\n")` vazio; `flags("CMD\t8\tnone\tStringConstant\tBG\tSet-ItemProperty\ts:HKLM:\\x\n")` e `"GB"` contêm `(Hklm, Refuse)`.
7. `syntax_error_refused`: `flags("ERR\t5\tMissingEndCurlyBrace\n")` == `[(SyntaxError, Refuse)]`.
8. `using_module_refused`: `flags("USE\t1\tmodule\n")` contém `(DynamicCode, Refuse)`; `flags("USE\t1\tnamespace\n")` vazio.
9. `encoded_command_refused`: `flags("CMD\t1\tnone\tStringConstant\t.\tpwsh\tp:enc\ts:QQA=\n")` contém `(HiddenCode, Refuse)`.
10. `global_assignment_refused`: `flags("SET\t1\tB\tglobal\n")` e `flags("CMD\t1\tnone\tStringConstant\t.\tsv\ts:global\ts:1\n")` contêm `(GlobalName, Refuse)`.
11. `prelude_redefinition_refused`: `flags("FN\t1\t.\tenv_add_path\n")` (ou outro nome de `PRELUDE_FUNCTIONS`) contém `(PreludeRedefinition, Refuse)`.
12. `gated_union_never_relaxes`: `script_scan::scan_gated("Start-Process x -Verb RunAs", PRELUDE_FUNCTIONS, &CallRoots::default(), &Fake(Default::default())).0.verdict == Verdict::Refuse` (o léxico recusa, o AST vazio não relaxa).
13. `gated_adds_ast_refusal`: texto `"i`ex 'x'"` com `Fake` devolvendo `"CMD\t1\tnone\tStringConstant\t.\tiex\ts:x\n"` ⇒ o relatório tem `DynamicCode` mesmo que o léxico não ache; e com `"CMD\t1\tnone\tStringConstant\t.\tStart-Process\tp:Verb\ts:RunAs\n"` ⇒ `verdict == Refuse`.
14. `gated_parser_error_refuses`: `Fake` com `Err(AstError::Timeout(10))` para o texto `"Write-Output ok"` ⇒ `verdict == Refuse` e uma flag `ParserFailed`; o mesmo com `AstError::NoPowerShell` e `AstError::Elevated`.
15. `gated_scans_called_files_with_ast`: raiz temporária com `$dir\helper.ps1` contendo `Write-Output h`; `Fake` dá `"CMD\t1\tnone\tStringConstant\t.\tStart-Process\tp:Verb\ts:RunAs\n"` para `"Write-Output h"` ⇒ `scan_gated("& \"$dir\\helper.ps1\"", ..., &CallRoots { dir: Some(tmp), ..Default::default() }, &fake)` tem `verdict == Refuse`.
16. `unavailable_is_the_safe_default`: `script_ast::Unavailable.facts("x")` é `Err(AstError::NoPowerShell)`, e `Runners::disabled().ast.facts("x")` também.

Em `tests/it/install_planner.rs`:
17. `planner_refuses_on_ast_flag`: manifesto sintético com `post_install: ["Write-Output ok"]`, `PlanInput { ast: &fake, .. }` com `Fake` dando `Elevation` para `"Write-Output ok"` ⇒ `Err(PlanError::ScriptRefused { hook: "post_install", reasons })` com `reasons` contendo o texto de `D_ELEVATION`; com `&LexerOnly` ⇒ `Ok`.

Em `tests/it/cli_scripts.rs`:
18. `confirmation_and_plan_use_the_same_gate`: o mesmo `Fake` com `ConfirmStrong` (`Download`) para o script do manifesto ⇒ a confirmação mostrada (`plan_third_party(plan, ast)`) tem o veredito `ConfirmStrong` e a linha de `D_DOWNLOAD`; com `Refuse` ⇒ o plano recusa e nada é mostrado como "Continuar?".

Em `tests/it/script_ast_proc.rs` (novo; `.cmd` falso em tempdir, nunca pwsh real):
19. `fake_parser_reply_is_used`: `.cmd` que lê uma linha (`set /p L=`) e ecoa `CMD\t1\tnone\tStringConstant\t.\tiex` e `END\t1` ⇒ `PwshAst::with_checks(fake, not_elevated, no_gpo, 20s, 10s).facts("q")` tem um comando `iex`; a segunda chamada com o mesmo texto não manda nada ao processo (o `.cmd` grava um marcador por linha lida; um marcador só).
20. `fake_parser_timeout_refuses`: `.cmd` que não responde (`ping -n 30 127.0.0.1 >nul`), prazos de 1 s ⇒ `Err(AstError::Timeout(_))` em menos de 5 s, e nenhum `ping`/`cmd` filho continua vivo (o Job matou).
21. `fake_parser_dead_refuses`: `.cmd` que sai na hora ⇒ `Err(AstError::Protocol(_))`.
22. `elevated_starts_no_parser`: `elevated = || Ok(true)` e `.cmd` que grava um marcador ao iniciar ⇒ `Err(AstError::Elevated)` e marcador ausente; `elevated = || Err(..)` ⇒ o mesmo.
23. `encoded_host_fits_command_line`: o argumento `-EncodedCommand` tem menos de 30 000 caracteres.
24. `host_script_runs_nothing`: o texto de `script_ast::HOST_SCRIPT` não casa (sem diferença de caixa) com `Invoke`, `GetScriptBlock`, `::Create`, `iex`, `Set-Content`, `Out-File`, `New-Item`, `Start-Process`, nem com a regex `\b[A-Za-z]+-[A-Za-z]+\b` fora de comentários (nenhum cmdlet), e contém `$PSModuleAutoLoadingPreference = 'None'`.

Em `tests/it/script_host_live.rs` (`#[ignore = "live: real PowerShell parser"]`; único arquivo de teste com `PwshHost::locate`):
25. `real_parser_sees_the_holes`: `PwshAst::for_program(PwshHost::locate().program().into())` sobre o script sintético das linhas abaixo ⇒ linha 1 `Elevation`, linha 3 `SystemWrite`, linha 4 `CalledFile`, linha 6 `CalledFile`, linha 7 `Alias`, linha 9 `SystemWrite`, linha 12 `RegistryUser`; com o pwsh 7, a linha 2 **não** é `iex`, com o 5.1 é (o teste lê `program()` e escolhe o esperado):
    ```powershell
    Start-Process notepad –Verb RunAs
    i`ex 'x'
    Start-Job { Remove-Item C:\Windows\x }
    &("i" + "ex") 'y'
    .\setup.ps1 /S
    cmd /c "del x"
    sal z iex
    if ($global) { Set-ItemProperty HKLM:\x -Name a -Value 1 }
    [IO.File]::WriteAllText("$env:ProgramFiles\x", 'y')
    & $dir\helper.ps1
    $a = 'abc’; Write-Output HIDDEN; ‘x'
    ForEach-Object -Parallel { reg add HKCU\x } -InputObject 1
    ```
    (linha 8 é código morto: sem flag do AST; o léxico mantém o aviso de instalação global.)
26. `real_parser_runs_nothing`: tempdir com `mark.psm1` = `Set-Content '<tmp>\marker.txt' x`; analisar `"using module '<tmp>\mark.psm1'\nclass A { }\nSet-Content '<tmp>\marker2.txt' y"` ⇒ `marker.txt` e `marker2.txt` não existem, e o relatório tem `(DynamicCode, Refuse)` do `using module`.
27. `real_parser_batch_reuses_process`: 20 scripts distintos pelo mesmo `PwshAst` ⇒ todos `Ok`, e o tempo total menos o da primeira chamada é menor que 19 × o tempo da primeira (o processo foi reaproveitado; imprime os tempos para o relatório).

**Files:** `src/install/script_ast.rs` (novo), `src/install/script_ast_win.rs` (novo), `src/install/ast_facts.ps1` (novo), `src/install/mod.rs` (os `mod`), `src/install/script_scan.rs` (`scan_gated`, `D_*` `pub(crate)`, `FlagKind::SyntaxError`/`ParserFailed`), `src/install/script_host_win.rs` (só `known` `pub(crate)`), `src/install/executor.rs` (`Runners.ast`, `scan_ps1_file`, `refuse_called`), `src/install/planner.rs` (`PlanInput.ast`, `hook_step`), `src/install/ops.rs` (passa `ctx.runners.ast`), `src/install/prod_win.rs`, `src/cli/output.rs` (`script_items`/`plan_third_party`), chamadores de `plan_third_party` no CLI e na TUI (só o argumento novo), `scripts/check_rules.ps1`, `scripts/check_rules_selftest.ps1`, `examples/gate_ast_delta.rs` (novo), `tests/it/script_ast.rs` (novo), `tests/it/script_ast_proc.rs` (novo), `tests/it/script_host_live.rs`, `tests/it/install_planner.rs`, `tests/it/cli_scripts.rs`, `tests/it/main.rs` (os `mod`), e os `PlanInput { .. }`/`Runners { .. }` dos testes que só ganham o campo novo.

#### Checks beyond the steps

1. Mutação: `scan_gated` sem juntar as flags do AST (devolve só o léxico) ⇒ testes 13 e 15 **PEGAM**.
2. Mutação: `AstError` vira relatório vazio em vez de `ParserFailed` ⇒ teste 14 **PEGA**.
3. Mutação: código morto passa a ser `G` em qualquer posição da cadeia (ignora o `B`) ⇒ teste 6 **PEGA**.
4. Mutação: `PwshAst` sem a checagem de elevação ⇒ teste 22 **PEGA**.
5. Mutação: `Runners::disabled()` com `LexerOnly` ⇒ teste 16 **PEGA**; e `check_rules` regra (b) falha (plantar e colar a saída no relatório).
6. Mutação: `hook_step` de volta para `scan_called` ⇒ teste 17 **PEGA** e `check_rules` regra (a) falha.
7. Mutação: tirar a regra `runas` do `ast_flags` ⇒ testes 3 e 13 **PEGAM**.
8. Mutação: sem o `KILL_ON_JOB_CLOSE` no Job do parser ⇒ teste 20 **PEGA** (sobra filho vivo).
9. Orquestrador, com pedido do usuário: `cargo test --quiet --release -- --ignored real_parser` num terminal **sem** administrador (pwsh 7 e, se possível, com o 7 renomeado numa VM ou máquina sem 7, o 5.1); colar os tempos do teste 27.
10. Orquestrador, com pedido do usuário: `cargo run --release --example gate_ast_delta -- %LOCALAPPDATA%\wingetrs\catalog.db`; colar o resumo no relatório; parar se mais de 5% dos aceitos passarem a `Refuse` (passo 8 das regras).
11. Revisão adversarial (opus): tentar furar com as formas da ARMADILHAS (travessão, crase, aspas tipográficas, runspace, `&(…)`, `iex` por alias, caminho sem operador, `Start-Process`/`cmd /c`) e com formas que só o AST pega; cada furo novo vira teste.
12. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** dois commits: `feat(scripts): PowerShell AST facts and rules (parser process, union with the lexer)` e `feat(scripts): plan, confirmation and executor go through the AST gate`.

### Task 228: `winget uninstall` de escopo usuário sem elevação a partir de um terminal de administrador

**Modelo:** opus (revisor opus).

**Depois de:** 220 (`src/cli/winget_cmd.rs`, `src/tui/jobs.rs`). Nenhum arquivo em comum com 227.

> Origem: BACKLOG item H; achado da prova ao vivo de 2026-10-21 (remover kondo-ui e hyperfine num terminal de administrador ⇒ `0x8A15007D`). A Task 218 recusa no plano; esta tarefa faz a remoção funcionar rodando o `winget` com o token não elevado da sessão (Explorer), e mantém a recusa da 218, com o comando a repetir, quando isso não é possível. Scripts do Scoop num terminal de administrador continuam recusados (não mexer).

#### Where this fits

- Recusa de hoje (218): `src/cli/winget_cmd.rs:739` `pub fn winget_admin_refusal(plan: &UninstallPlan, elevated: bool) -> Option<String>` ⇒ `winget_install::uninstall::winget_user_scope(plan)` + `script_policy::admin_terminal_refusal(elevated, AdminBlock::WingetUserScope)` (`src/install/script_policy.rs:82`, texto em `:90`). "Está elevado?" por injeção: `script_policy::elevated_or_unknown(probe)` (`:96`, erro ⇒ elevado). TUI: o mesmo `winget_admin_refusal` (procure em `src/tui/`).
- Remoção pelo winget: `src/winget_install/uninstall.rs:579` `uninstall_by_winget(ctx, id, hive, key)` monta `uninstall --product-code <id> --exact --silent --disable-interactivity --accept-source-agreements` e chama `(ctx.bridge)("winget", &args, on_line)`. `UninstallCtx.bridge: ToolRunner<'a>` (`:474`; `(program, args, on_line) -> io::Result<(i32, Vec<String>)>`). Produção: `bridge: &winget_cmd::run_tool` (`src/cli/run_win.rs:311`, `:395`) ⇒ `bridge::spawn_win::run(program, ...)` ⇒ `Command::new("winget")`, **busca no `PATH`**. `winget_product_code` (`:237`), `ArpDetails.hive` (`Hive::User`/`MachineX64`/`MachineX86`).
- Caminho absoluto do winget: `src/bridge/elevate_win.rs:889` `pub fn winget_exe() -> io::Result<PathBuf>` (pacote do App Installer em `Program Files\WindowsApps`, nunca o alias do usuário).
- Pipes e Job: `src/bridge/spawn_win.rs` (`run_spec`/`RunSpec` `:245`, `run_in_job` `:493`, leitor com `PIPE_GRACE`). Token elevado: `src/winget_install/launch_win.rs:157` `is_elevated()`.
- Testes de hoje: `tests/it/admin_terminal.rs` (218: `winget_user_scope_refused_in_plan`, `winget_machine_scope_unchanged`, `cli_and_tui_same_refusal_text`, `token_error_counts_as_elevated`), `tests/it/wi_cli.rs:2129` `product_code_line_is_removed_by_winget`.

#### What binds this task

0. **Sondas antes do código (o usuário roda, num terminal de administrador; agente não eleva).** O implementador escreve primeiro `tests/it/unelevated_live.rs` (`#[ignore = "live: needs an elevated terminal"]`; cada teste sai cedo com `eprintln!("pulado: processo não elevado")` quando `is_elevated()` é `false`) e para, pedindo ao orquestrador a rodada:
   - S1 `shell_token_child_output_comes_back`: `spawn_as_shell_user(<System32>\whoami.exe, ["/groups"])` devolve código 0 e linhas no `on_line`;
   - S2 `shell_token_child_is_medium`: a saída de S1 contém `S-1-16-8192` (integridade média; SID independe do idioma) e não contém `S-1-16-12288`;
   - S3 `shell_token_child_dies_with_job`: filho `<System32>\ping.exe -n 30 127.0.0.1`, o `stop` cancela ⇒ volta em menos de 3 s e nenhum `ping.exe` daquele Job continua vivo;
   - S4 (manual, com pedido do usuário e um pacote de teste de escopo usuário que ele escolher): `wingetrs remove <id>` num terminal de administrador remove sem `0x8A15007D`.
   S1–S3 falhando ⇒ a tarefa para no passo 0 e volta para decisão (queda para a opção D do BACKLOG).
1. **Decisão pura** em `src/winget_install/uninstall.rs`:
   ```rust
   pub enum ShellUser { Usable, OtherUser, Elevated }
   pub enum ShellError { NoShell, Privilege, Seclogon, Other(String) }
   pub enum WingetRoute { Direct, AsShellUser, Refuse(String) }
   pub fn winget_route(plan: &UninstallPlan, elevated: bool, shell: impl FnOnce() -> Result<ShellUser, ShellError>) -> WingetRoute
   ```
   Não elevado, ou registro HKLM ⇒ `Direct` (sem chamar `shell`). Elevado + `winget_user_scope(plan)` ⇒ chama `shell()`: `Ok(Usable)` ⇒ `AsShellUser`; qualquer outro ⇒ `Refuse(texto)`, com o texto de `admin_terminal_refusal(true, AdminBlock::WingetUserScope)` seguido de `: wingetrs remove <id>` (o `id` do plano, `printable`), e, para `OtherUser`, a frase "o Explorer desta sessão é de outro usuário" antes do comando. `winget_admin_refusal` passa a ser `match winget_route(..) { Refuse(t) => Some(t), _ => None }` (CLI e TUI continuam com um texto só).
2. **`ShellUser` real** em `src/bridge/unelevated_win.rs` (novo, `#[cfg(windows)]`): `pub fn shell_user() -> Result<ShellUser, ShellError>`: `GetShellWindow` (nulo ⇒ `NoShell`) ⇒ `GetWindowThreadProcessId` ⇒ `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` ⇒ `OpenProcessToken(TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY)` ⇒ `TokenUser` igual ao `TokenUser` do processo atual (senão `OtherUser`) e `TokenElevation` falso (senão `Elevated`, ex.: UAC desligado). Handles fechados em todos os caminhos (guard com `Drop`).
3. **Lançamento com o token da sessão**: `pub fn spawn_as_shell_user(program: &Path, args: &[String], stop: &AtomicBool, on_line: &mut dyn FnMut(&str)) -> io::Result<(i32, Vec<String>)>`: `program` absoluto (senão `InvalidInput`); `DuplicateTokenEx(TokenPrimary)` do token do Explorer reaberto agora (não guardado de antes); ambiente de `CreateEnvironmentBlock(token, FALSE)` (nada do ambiente do processo elevado); pasta de trabalho `System32`; pipes herdáveis em `STARTF_USESTDHANDLES`; `CreateProcessWithTokenW(..., CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW, ...)` ⇒ `AssignProcessToJobObject` (Job com `KILL_ON_JOB_CLOSE`) ⇒ `ResumeThread`. Linhas lidas como no `run_spec` (mesmo decodificador, mesmo `PIPE_GRACE`, thread juntada em todos os caminhos). `stop` ⇒ fecha o Job. `ERROR_PRIVILEGE_NOT_HELD` ⇒ `ShellError::Privilege`; serviço `seclogon` indisponível (`ERROR_SERVICE_DISABLED`/`ERROR_SERVICE_NOT_ACTIVE`) ⇒ `Seclogon` (devolvidos como `io::Error` com a mensagem em português; o chamador vira `Refuse` com o texto da regra 1).
4. **Fiação.** `UninstallCtx` ganha `pub as_shell_user: Option<ToolRunner<'a>>` (produção: um `ToolRunner` que resolve `winget_exe()` e chama `spawn_as_shell_user`; testes: falso ou `None`). `uninstall_by_winget` recebe a `WingetRoute` decidida no plano: `AsShellUser` ⇒ `as_shell_user` (ausente ⇒ erro com o texto da recusa, nada roda); `Direct` ⇒ `bridge` como hoje. Nas duas rotas o programa é o caminho absoluto de `winget_exe()` em produção, nunca `"winget"` pelo `PATH` (corrige o achado: rota HKLM num terminal de administrador procurava `winget` no `PATH` com o processo elevado).
5. **Texto do plano.** Com `AsShellUser`, o plano do CLI e da TUI mostra a linha `Roda sem administrador (com a sua sessão do Windows)` antes da pergunta; sem pergunta nova.
6. Nada muda para scripts do Scoop, instalação e atualização (a recusa da 218 fica como está).

Testes (em `tests/it/admin_terminal.rs`, salvo os do passo 0):
1. `route_direct_when_not_elevated` (registro HKCU com `--product-code`, `elevated = false` ⇒ `Direct`, e o fechamento `shell` não é chamado: `|| panic!("shell")`).
2. `route_direct_for_machine_scope` (mesmo registro em `Hive::MachineX64`, elevado ⇒ `Direct`, `shell` não chamado).
3. `route_shell_user_when_usable` (HKCU, elevado, `|| Ok(ShellUser::Usable)` ⇒ `AsShellUser`).
4. `route_refuses_with_command` (HKCU, elevado, `|| Err(ShellError::NoShell)` ⇒ `Refuse(t)`, `t` começa com o texto da 218 e termina com `: wingetrs remove <id>`; o mesmo para `Privilege`, `Seclogon` e `Ok(ShellUser::Elevated)`).
5. `route_other_user_says_so` (`Ok(ShellUser::OtherUser)` ⇒ `Refuse(t)` com "o Explorer desta sessão é de outro usuário").
6. `user_scope_uses_shell_runner` (`uninstall_by_winget` com rota `AsShellUser`, `bridge` falso que entra em pânico se chamado e `as_shell_user` falso que grava `(program, args)` e devolve `(0, vec![])` ⇒ uma chamada com os mesmos argumentos de hoje).
7. `missing_shell_runner_runs_nothing` (rota `AsShellUser`, `as_shell_user: None` ⇒ `Err` com o texto da recusa; `bridge` não chamado).
8. `cli_plan_shows_unelevated_line` (CLI com terminal falso, elevado, `shell` `Usable` ⇒ a linha `Roda sem administrador (com a sua sessão do Windows)` aparece antes da pergunta).
9. `cli_and_tui_same_route_text` (atualiza o 8 da 218: `Refuse` dá o mesmo texto nos dois lados).
Os testes da 218 que esperavam recusa com elevado passam a injetar `shell = || Err(ShellError::NoShell)`; nenhum deles é apagado.

**Files:** `src/winget_install/uninstall.rs`, `src/bridge/unelevated_win.rs` (novo), `src/bridge/mod.rs` (o `mod` com `#[cfg(windows)]`), `src/cli/winget_cmd.rs` (`winget_admin_refusal`, linha do plano), `src/cli/run_win.rs` (fiação de `as_shell_user` e caminho absoluto), `src/tui/jobs.rs` e `src/tui/app.rs` (mesma fiação e linha), `tests/it/admin_terminal.rs`, `tests/it/unelevated_live.rs` (novo), `tests/it/main.rs` (o `mod`), `scripts/check_rules.ps1` (só a lista da regra do `Http::new`), `docs/cli-textos.md` (a linha nova e o texto da recusa).

#### Checks beyond the steps

1. Mutação: `winget_route` devolve `AsShellUser` também para HKLM ⇒ teste 2 **PEGA**.
2. Mutação: `Ok(ShellUser::OtherUser)` tratado como `Usable` ⇒ teste 5 **PEGA**.
3. Mutação: `uninstall_by_winget` ignora a rota e usa sempre `bridge` ⇒ teste 6 **PEGA**.
4. Mutação: erro do token do processo atual tratado como não elevado ⇒ `token_error_counts_as_elevated` (218) **PEGA**.
5. `check_rules`: incluir `bridge[\\/]unelevated_win` na lista de arquivos do lado elevado da regra `Http::new no lado elevado` (`scripts/check_rules.ps1:36`), plantar um `Http::new()` nele e colar a falha no relatório; `unelevated_win.rs` também não contém `Command::new(`.
6. Orquestrador, com pedido do usuário, num terminal de administrador: `cargo test --quiet --release -- --ignored shell_token` (S1–S3) e S4 com um pacote de escopo usuário que o usuário escolher; colar a saída no relatório.
7. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget): user-scope uninstall from an admin terminal runs with the session's unelevated token; absolute winget path`

---

## Bloco GI

Seções prontas para colar em `docs/superpowers/plans/2026-10-19-wingetrs-pos-2d.md`. Cada uma vai de `### Task N` ao próximo `### Task`, como o `scripts/brief.ps1` extrai.

**Numeração.** O `brief.ps1` recebe `[int]$Task` e procura `^### Task $Task\b`, então "229a"/"230a" não são extraídos. Usei inteiros: **229** (G, analisador + sonda), **230** (I, extratores), e proponho **231** (G, modo portátil no CLI) e **232** (I, evento + CLI + TUI). Renumere se 231/232 já foram dados a outro planejador.

**Ordem e conflitos** (205 em `src/bridge/mod.rs`, `src/tui/jobs.rs`, `src/cli/output.rs`; 207 em `src/tui/*`; 209 em `src/github/*`, `src/install/sniff.rs`):

| Task | Arquivos de produção | Pode rodar | Depois de |
|---|---|---|---|
| 230 | `src/install/extract.rs`, `extract_7z.rs`, `extract_cab.rs`, `extract_msi.rs`, `examples/extract_bench.rs` | já, em paralelo com 205/207/209 (só lê `sniff.rs`) | — |
| 229 | `src/winget_install/portable_msi.rs` (novo), `src/winget_install/mod.rs`, `src/install/extract_msi.rs` (só `pub(crate)`), `examples/portable_probe.rs` | depois da 230 (`extract_msi.rs` em comum) | 230 |
| 231 | `src/winget_install/portable.rs`, `portable_msi.rs`, `src/cli/winget_cmd.rs`, `src/cli/run_win.rs`, `src/cli/install_cmd.rs`, `src/cli/mod.rs`, `src/cli/output.rs`, `docs/cli-textos.md` | depois da 205 (`output.rs`, `docs/cli-textos.md`) | 229, 205, decisões D-G1..D-G5 |
| 232 | `src/install/progress.rs`, `src/install/executor.rs`, `src/cli/output.rs`, `src/tui/jobs.rs`, `src/tui/app.rs`, `src/tui/view.rs` | depois da 205 e da 207 | 230, 205, 207 |

Sequência recomendada: **230 → 229 → (sonda, com OK do usuário) → decisões → 231**; **232** assim que 205 e 207 fecharem. A 232 não depende da 229/231 e não tem arquivo em comum com elas (só `output.rs` com a 231: rodar uma depois da outra).

---

### Task 230: Extratores informam os bytes processados (zip, 7z, CAB, MSI, tar)

**Modelo:** sonnet (revisor sonnet).

**Depois de:** nada em andamento. Nenhum arquivo em comum com 205, 207 ou 209 (`extract.rs` só **chama** `sniff::*`; se a 209 renomear algo em `sniff.rs`, ajuste a chamada e cite no relatório).

> Origem: BACKLOG item I (decisão do usuário, 2026-10-21): barra de progresso também na extração. "Antes: evento novo em `ProgressEvent` com bytes descomprimidos/total por extrator, e medir se o callback custa no 7z/cab." Esta tarefa faz a metade de baixo (extratores + medição); o evento, o executor, o CLI e a TUI ficam na Task 232, porque uma variante nova de `ProgressEvent` obriga a mexer em `src/cli/output.rs` e `src/tui/jobs.rs`, que a 205 tem em andamento.

#### Where this fits

- Todo extrator já conta os bytes produzidos num `Budget` (`src/install/extract.rs:90-112`: `limit`, `used`, `for_file(file, limits)`, `charge(n)`), criado no início de cada um: zip `extract.rs:517`, tar `:592`, 7z `extract_7z.rs:32`, CAB `extract_cab.rs:27`, MSI `extract_msi.rs:43`, importação da árvore do `msiexec /a` `extract.rs:296` (esta fica **fora**: roda na fase `Placing`). O `charge` é chamado em `write_entry` (`extract.rs:504`, a cada leitura de até 64 KiB), no `drain` do 7z (`extract_7z.rs:440`) e do MSI (`extract_msi.rs:229`), nas entradas puladas do tar (`extract.rs:615`) e nos `folder_gaps` do CAB/MSI (`extract_cab.rs:70`, `extract_msi.rs:105`).
- O que cada extrator lê ou pula: zip lê **só** as entradas que o `Mapper` aceita (`extract.rs:539-549`; as de fora do `extract_dir` não são lidas nem cobradas); 7z decodifica e cobra **todas** (`place`, `extract_7z.rs:396-414`, `drain` para as de fora e para `.nsis`); CAB e MSI leem e cobram todas as entradas de todos os gabinetes mais os `folder_gaps` (`extract_cab.rs:64-75`, `extract_msi.rs:89-112`); tar não sabe o total antes de ler.
- Padrão de estado por thread já usado no módulo: `IN_DECODER` + `catch_decoder` com guard `Restore` (`extract.rs:20-41`). O executor chama a extração por um ponteiro `ExtractFn` (`src/install/executor.rs:53-54`) dentro de um worker (`executor.rs:2520-2555`); há 13 extratores falsos nos testes com essa assinatura (`tests/it/cli_cancel.rs`, `tests/it/install_executor.rs`, `tests/it/install_progress.rs`), por isso o progresso **não** entra pela assinatura.
- Unit test que monta `Budget` literal: `extract_7z.rs:507` (`Budget { limit: 1000, used: 0 }`).
- ARMADILHAS: "Teste que não consegue falhar", "Caso degenerado esconde a regra", "Desempenho: medir antes de mudar, e medir delta", "Corpo literal de teste no plano também passa pelo clippy".

#### What binds this task

1. **API em `src/install/extract.rs`:**
   - `pub type Report = std::sync::Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;` (`done`, `total`).
   - `pub const REPORT_STEP: u64 = 1 << 20;` (1 MiB; a medição do item 6 pode subir).
   - `pub fn reporting<T>(step: u64, report: Report, f: impl FnOnce() -> T) -> T`: guarda `(report, step)` num `thread_local!` enquanto `f` roda e **restaura o anterior** num guard com `Drop` (também em pânico), como `catch_decoder`.
   - Sem `reporting` ativo, nada é chamado e o custo é um `Option` vazio no `Budget`.
2. **`Budget` informa**: `for_file` copia o `(Report, step)` da thread (se houver). Campos novos `total: Option<u64>`, `reported: u64` e `last: Option<(u64, Option<u64>)>`. `expect(&mut self, total: u64)` fixa o total. Em `charge`, depois de somar e do teste de limite: se `used - reported >= step`, chama `report(used, total)` e guarda. `finish(&mut self)` (chamado no fim de cada extrator **só no sucesso**) chama `report(used, Some(used))`, a menos que o último evento já seja exatamente esse par. Sequência de `done` estritamente crescente: nenhum evento repetido. O `Budget` literal do teste de `extract_7z.rs` vira um construtor `#[cfg(test)] fn with_limit(limit: u64) -> Budget`.
3. **Total de cada formato**: o valor passado a `expect` antes do primeiro `charge`:
   - zip: soma de `size()` das entradas que o `Mapper` aceita. Acrescente `Mapper::accepts(&self, comps: &[String]) -> bool` (mesmo teste de prefixo de `map`, sem marcar `dir_seen`) e use-o no laço de validação (`extract.rs:524-534`).
   - 7z: soma de `size` de **todas** as entradas de `archive.files`.
   - CAB e MSI: soma dos `folder_gaps` mais `uncompressed_size` de todas as entradas de todos os gabinetes. Hoje o `charge(folder_gaps)` acontece dentro do laço que coleta os tamanhos: some os gaps numa variável, chame `expect(gaps + tamanhos)` e só então `charge(gaps)`.
   - tar (gz e xz): sem total (`expect` não é chamado); o `finish` fecha com `(used, Some(used))`.
   - Arquivo simples (`ArchiveKind::Plain`, `fs::copy`): nenhum evento.
4. Nada muda no resultado da extração, nos limites, nas mensagens de erro nem na assinatura de `extract`, `extract_all`, `extract_all_with` e `ExtractFn`.
5. O `reporting` **não** é chamado em produção nesta tarefa (o executor passa a usá-lo na 232). Nenhum `println!`; nenhuma thread nova.
6. **Medição** (antes do commit final, resultado colado no relatório): `examples/extract_bench.rs <arquivo> <nome-com-extensão> <rodadas>` extrai o arquivo `rodadas` vezes alternando **A** (sem `reporting`) e **B** (`reporting(REPORT_STEP, …)` com um `report` que manda `(done, total)` por `std::sync::mpsc::Sender`, como o executor fará), cada rodada numa pasta nova de `tempfile::tempdir()` apagada em seguida, e imprime a mediana de A, de B e o delta em %. Arquivos: os maiores 7z, MSI, CAB e zip do cache do usuário (`%LOCALAPPDATA%\wingetrs\cache\blobs`, somente leitura; formato pelos primeiros bytes: 7z `37 7A BC AF 27 1C`, zip `50 4B 03 04`, OLE/MSI `D0 CF 11 E0`, CAB `4D 53 43 46`). Formato sem arquivo no cache: "não medido" no relatório (o orquestrador baixa um). 7 rodadas por arquivo, `scripts/leftovers.ps1` antes. Critério: delta da mediana ≤ 2 % ou ≤ 50 ms; acima disso, `REPORT_STEP` vai a 4 MiB e mede de novo; os dois números vão para o relatório.

Testes (acrescentar; o `reported` do 7z é `pub(crate)` para o MSI usar, os dois arquivos estão no binário `tests/it`):

Em `tests/install_extract.rs` (binário próprio; usa `zip_with` e `targz_with` do arquivo):

```rust
fn reported(step: u64, f: impl FnOnce()) -> Vec<(u64, Option<u64>)> {
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    wingetrs::install::extract::reporting(
        step,
        Arc::new(move |done, total| sink.lock().unwrap().push((done, total))),
        f,
    );
    let out = seen.lock().unwrap().clone();
    out
}

#[test]
fn zip_reports_only_the_entries_it_writes() {
    let keep = vec![7u8; 200_000];
    let skip = vec![9u8; 300_000];
    let z = zip_with(&[("keep/a.bin", &keep), ("skip/b.bin", &skip)]);
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, z).unwrap();
    let dest = d.path().join("out");
    let ev = reported(1000, || {
        wingetrs::install::extract::extract(&f, "a.zip", Some("keep"), None, &dest).unwrap();
    });
    let (last, rest) = ev.split_last().unwrap();
    assert_eq!(*last, (200_000, Some(200_000)), "{ev:?}");
    assert!(!rest.is_empty(), "{ev:?}");
    assert!(rest.iter().all(|(_, t)| *t == Some(200_000)), "{ev:?}");
    assert!(ev.windows(2).all(|w| w[0].0 < w[1].0), "{ev:?}");
}

#[test]
fn tar_reports_without_a_total_until_the_end() {
    let a = vec![1u8; 150_000];
    let b = vec![2u8; 150_000];
    let t = targz_with(&[("x/a.bin", &a), ("x/b.bin", &b)], None);
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tar.gz");
    std::fs::write(&f, t).unwrap();
    let dest = d.path().join("out");
    let ev = reported(1000, || {
        wingetrs::install::extract::extract(&f, "a.tar.gz", None, None, &dest).unwrap();
    });
    let (last, rest) = ev.split_last().unwrap();
    assert_eq!(*last, (300_000, Some(300_000)), "{ev:?}");
    assert!(!rest.is_empty(), "{ev:?}");
    assert!(rest.iter().all(|(_, t)| t.is_none()), "{ev:?}");
}

#[test]
fn reporting_ends_with_its_scope_even_on_panic() {
    use std::sync::{Arc, Mutex};
    let z = zip_with(&[("a.bin", &vec![3u8; 100_000])]);
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, z).unwrap();
    let seen = Arc::new(Mutex::new(Vec::<(u64, Option<u64>)>::new()));
    let sink = Arc::clone(&seen);
    let r = std::panic::catch_unwind(|| {
        wingetrs::install::extract::reporting(
            1,
            Arc::new(move |d, t| sink.lock().unwrap().push((d, t))),
            || panic!("dentro do escopo"),
        )
    });
    assert!(r.is_err());
    wingetrs::install::extract::extract(&f, "a.zip", None, None, &d.path().join("out")).unwrap();
    assert!(seen.lock().unwrap().is_empty());
}
```

Em `tests/it/install_7z.rs` (usa `solid` e `pseudo_random` do arquivo):

```rust
pub(crate) fn reported(step: u64, f: impl FnOnce()) -> Vec<(u64, Option<u64>)> {
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    wingetrs::install::extract::reporting(
        step,
        Arc::new(move |done, total| sink.lock().unwrap().push((done, total))),
        f,
    );
    let out = seen.lock().unwrap().clone();
    out
}

#[test]
fn seven_zip_total_counts_the_entries_it_drains() {
    let a = solid(&[
        ("keep/x.bin", pseudo_random(100_000)),
        ("skip/y.bin", pseudo_random(50_000)),
    ]);
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.7z");
    std::fs::write(&f, a).unwrap();
    let dest = d.path().join("out");
    let ev = reported(1024, || {
        extract(&f, "a.7z", Some("keep"), None, &dest).unwrap();
    });
    let (last, rest) = ev.split_last().unwrap();
    assert_eq!(*last, (150_000, Some(150_000)), "{ev:?}");
    assert!(!rest.is_empty(), "{ev:?}");
    assert!(rest.iter().all(|(_, t)| *t == Some(150_000)), "{ev:?}");
}
```

Em `tests/it/install_msi.rs` (usa `build`, `tree_spec`, `cab`, `f_b`, `F_A`, `F_C` do arquivo):

```rust
#[test]
fn msi_reports_against_the_cabinet_total() {
    let bytes = build(&tree_spec());
    let mut out = None;
    let ev = crate::install_7z::reported(1024, || out = Some(run(&bytes, None)));
    let (_d, r) = out.unwrap();
    r.unwrap();
    let total = (F_A.len() + f_b().len() + F_C.len()) as u64;
    let (last, rest) = ev.split_last().unwrap();
    assert_eq!(*last, (total, Some(total)), "{ev:?}");
    assert!(!rest.is_empty(), "{ev:?}");
    assert!(rest.iter().all(|(_, t)| *t == Some(total)), "{ev:?}");
}

#[test]
fn cab_reports_against_its_entries() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("many.cab");
    std::fs::write(&f, cab("many.cab")).unwrap();
    let dest = d.path().join("out");
    let ev = crate::install_7z::reported(4096, || {
        wingetrs::install::extract::extract(&f, "many.cab", None, None, &dest).unwrap();
    });
    // 200 files × 50 lines of "file NNN\n" (9 bytes).
    let total = 200 * 50 * 9;
    let (last, rest) = ev.split_last().unwrap();
    assert_eq!(*last, (total, Some(total)), "{ev:?}");
    assert!(!rest.is_empty(), "{ev:?}");
    assert!(rest.iter().all(|(_, t)| *t == Some(total)), "{ev:?}");
}
```

Se `folder_gaps` de `three.cab` ou `many.cab` não for 0, o total esperado soma os gaps: imprima o valor, ajuste só o número no teste e explique no relatório (o teste continua exigindo o total **antes** do primeiro evento).

**Files:** `src/install/extract.rs`, `src/install/extract_7z.rs`, `src/install/extract_cab.rs`, `src/install/extract_msi.rs`, `examples/extract_bench.rs` (novo), `tests/install_extract.rs`, `tests/it/install_7z.rs`, `tests/it/install_msi.rs`.

#### Checks beyond the steps

1. Mutação: `finish` não é chamado no tar ⇒ `tar_reports_without_a_total_until_the_end` **PEGA**.
2. Mutação: total do zip soma todas as entradas (troca `accepts` por `true`) ⇒ `zip_reports_only_the_entries_it_writes` **PEGA**.
3. Mutação: total do 7z soma só as entradas aceitas pelo `Mapper` ⇒ `seven_zip_total_counts_the_entries_it_drains` **PEGA**.
4. Mutação: `charge` ignora o `step` (só o `finish` informa) ⇒ os testes de zip, tar, 7z, MSI e CAB **PEGAM** (`rest` vazio).
5. Mutação: o guard de `reporting` não restaura o anterior ⇒ `reporting_ends_with_its_scope_even_on_panic` **PEGA**.
6. Mutação: `expect` do CAB chamado depois do `charge(gaps)` com gaps forçados ≥ step (troque `step` por 1 no teste local, sem commit) mostra um evento com `None` antes dos outros: descreva no relatório o que viu.
7. Colar no relatório a tabela da medição (arquivo, formato, tamanho, mediana A, mediana B, delta) e o `REPORT_STEP` final.
8. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(extract): extractors report bytes processed against the archive's total`

---

### Task 232: Barra de progresso na extração (evento, executor, CLI e TUI)

**Modelo:** sonnet (revisor sonnet).

**Depois de:** 230 (API `extract::reporting`), 205 (`src/cli/output.rs`, `src/tui/jobs.rs`) e 207 (`src/tui/app.rs`, `src/tui/view.rs`, `src/tui/jobs.rs`). Arquivo em comum com a 231 (`src/cli/output.rs`): uma depois da outra.

> Origem: BACKLOG item I. A Task 216 deu barra só ao download; a extração mostra spinner, fase e relógio. Formatação única do CLI e da TUI: `DownloadMeter`, `download_parts` e `download_text` de `src/install/progress.rs` (Task 216).

#### Where this fits

- `ProgressEvent` (`src/install/progress.rs:26-35`: `Download { url, done, total }`, `Phase(Phase)`). Consumidores que casam o enum inteiro: `cli/output.rs` `Phases::update_at` (`:1474-1489`) e `PhaseLog::line` (`:1818-1829`); `tui/jobs.rs` `progress_msg` (`:100-130`); nos testes, `tests/it/install_progress.rs:195` (`trace_line`) e `tests/it/tui_jobs.rs:1385`. ARMADILHAS "Rótulo especial no callback de progresso só tratado num consumidor": todos ganham o braço novo e um teste.
- Medidor e texto: `DownloadMeter` (`progress.rs:275-346`, `record_at`, `is_empty`, `finished`, `rate_at`, `quiet_at`), `download_parts` (`:375`), `download_text` (`:415`), `phase_text` (`:192`, `"{título} · 00:12"`), `phase_status` (`:204`, igual a `phase_text` para `Extracting`, que é cancelável). Título: `Phase::Extracting { file, engine }` ⇒ `"Extraindo {file} ({engine})"` (`:137`).
- CLI: `Phases` (`output.rs:1454-1462`: `meter`, `phase`, `log`); `parts_at` (`:1619-1720`): com fase, a linha é `phase_text` sem barra (`Phase::Extracting` cai no grupo sem extras, `:1656`); sem fase, a barra de download. `LineParts` com `label`, `open`, `filled`, `empty`, `close`, `rest`, `warn`. Sem TTY: `PhaseLog` (uma linha por mudança de fase, também usada pelo transcript da Task 217).
- Executor: `Step::Extract` (`src/install/executor.rs:2516-2570`): worker `AwaitWorker::spawn(work, …)`, canal `phase_tx`/`phase_rx` e o laço `tokio::select!` que repassa fases (`run.enter(ctx, p)`); o `msiexec /a` de reserva roda no mesmo worker depois de `extract(...)` falhar. `ctx.progress` é `&dyn Fn` (não vai para o worker).
- TUI: `ProgressThrottle` (`tui/jobs.rs:45-98`, `admit` soma por rótulo e libera um agregado a cada `PROGRESS_GAP`); `Msg::InstallProgress` grava no `meter` do `RunProgress` (`tui/app.rs:1936-1948`); `RunProgress { label, phase, meter }` (`app.rs:357-365`), `phase_progress` zera o medidor a cada fase (`:368-374`), `METER_KEY` (`:377`); `progress_text` (`tui/view.rs:1496-1506`).
- Testes de referência para copiar a forma: `tests/it/install_progress.rs` `cli_and_tui_share_download_text` (`:1591`, helpers `tui_running` `:1569`, `after` `:1583`, `ascii` `:32`, `MIB` `:30`) e o harness `Tool` (`:81-190`, `cached_install_emits_phases_in_order` `:200`, zip `pkg/tool.exe` de 248 bytes com `extract_dir: "pkg"`).

#### What binds this task

1. **Evento**: `ProgressEvent::Extract { file: String, done: u64, total: Option<u64> }` (doc: bytes já gravados ou lidos pelo extrator próprio; o último de um arquivo bem extraído tem `done == total`).
2. **Executor**: no `Step::Extract`, um canal `tokio::sync::mpsc::unbounded_channel::<(u64, Option<u64>)>()`; o worker chama `extract::reporting(extract::REPORT_STEP, Arc::new(move |d, t| { let _ = tick_tx.send((d, t)); }), || extract(&file, …))` envolvendo **só** a chamada do extrator (o `msiexec /a` de reserva fica fora). O `select!` ganha o braço `Some((done, total)) = tick_rx.recv()` que chama `(ctx.progress)(&ProgressEvent::Extract { file: source.file_name.clone(), done, total })`. Depois que o worker termina, os eventos que ficaram no canal são repassados (`while let Ok(..) = tick_rx.try_recv()`) **antes** da próxima fase: o 100 % sempre chega antes de `Placing`. Inno (`Step::ExtractInno`) e arquivo simples não mandam bytes.
3. **CLI** (`Phases`): campo `extract: DownloadMeter`, zerado a cada mudança de fase. `update_at` com `Extract { file, .. }` grava em `extract` só se a fase atual for `Phase::Extracting { file: f, .. }` com `f == file`; fora disso, ignora (não muda a fase nem o medidor de download). `parts_at`: na fase `Extracting`, com `extract` não vazio e não `finished()`, a linha é `label = phase_text(fase, decorrido)` mais as partes de `download_parts(&self.extract, now, glyphs)` (`open`, `filled`, `empty`, `close`, `rest = " {tail}"`, `warn`), a mesma montagem da linha de download. Sem total (tar): sem barra, `"{label} {tail}"`. Medidor vazio ou terminado: só `phase_text`, como hoje. `PhaseLog::line` com `Extract` ⇒ `None` (nunca uma linha nova; o transcript continua com uma linha por fase).
4. **TUI**: `ProgressThrottle::admit_extract(&self, now, done, total) -> bool` com relógio próprio (`extract_last_ms`), sem misturar com as entradas de download: libera o primeiro, depois um por `PROGRESS_GAP`, e sempre o de `total == Some(done)`. `progress_msg` com `Extract` ⇒ `Msg::ExtractProgress { done, total }` quando `admit_extract` libera. `update` com `Msg::ExtractProgress` grava no `meter` do `RunProgress` (chave `METER_KEY`, `app.now`) só se a fase for `Extracting`; senão ignora. `progress_text`: fase `Extracting` com medidor não vazio e não terminado ⇒ `download_text(&phase_status(fase, decorrido), &download_parts(&p.meter, now, g))`.
5. Texto igual no CLI e na TUI (teste de equivalência abaixo). Nenhuma cor fora de `style.rs`/`theme.rs`; nenhum `println!`.
6. `docs/cli-textos.md`: a linha de progresso da extração ganha a forma nova (uma linha no inventário de progresso).

Testes (`tests/it/install_progress.rs`, acrescentar; nenhum relógio real, `Instant` base + `Duration`):

```rust
fn ex(done: u64, total: Option<u64>) -> ProgressEvent {
    ProgressEvent::Extract {
        file: "rust.msi".into(),
        done,
        total,
    }
}

fn extracting() -> Phase {
    Phase::Extracting {
        file: "rust.msi".into(),
        engine: "MSI".into(),
    }
}

#[test]
fn extract_line_has_phase_clock_and_bar() {
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let mut ph = Phases::new();
    ph.update_at(&ProgressEvent::Phase(extracting()), s(0));
    // Before any byte: the phase and its clock, as today.
    assert!(ph.line_at(0, ascii(), s(0)).unwrap().ends_with("Extraindo rust.msi (MSI) · 00:00"));
    for i in 0..=3 {
        ph.update_at(&ex(i * STEP, Some(27 * MIB)), s(i));
    }
    let line = ph.line_at(0, ascii(), s(3)).unwrap();
    assert!(
        line.contains("Extraindo rust.msi (MSI) · 00:03 [######--------------] 34%  9.3 / 27.0 MiB · 3.1 MiB/s"),
        "{line}"
    );
    // Bytes of another file, or outside the phase, are ignored.
    ph.update_at(&ProgressEvent::Extract { file: "outro.7z".into(), done: 27 * MIB, total: Some(27 * MIB) }, s(4));
    assert!(ph.line_at(0, ascii(), s(4)).unwrap().contains("34%"));
    // Done: the phase alone again.
    ph.update_at(&ex(27 * MIB, Some(27 * MIB)), s(5));
    assert!(ph.line_at(0, ascii(), s(5)).unwrap().ends_with("Extraindo rust.msi (MSI) · 00:05"));
}

#[test]
fn extract_bytes_add_no_phase_log_line() {
    let mut log = PhaseLog::new();
    assert_eq!(log.line(&ProgressEvent::Phase(extracting())).as_deref(), Some("Extraindo rust.msi (MSI)…"));
    assert_eq!(log.line(&ex(MIB, Some(27 * MIB))), None);
    assert_eq!(log.line(&ProgressEvent::Phase(Phase::Placing)).as_deref(), Some("Colocando os arquivos…"));
}

#[test]
fn cli_and_tui_share_extract_text() {
    use wingetrs::tui::app::{Msg, update};
    let base = Instant::now();
    let s = |n: u64| base + Duration::from_secs(n);
    let mut ph = Phases::new();
    let mut a = tui_running(base);
    ph.update_at(&ProgressEvent::Phase(extracting()), s(0));
    update(&mut a, Msg::InstallPhase(extracting()));
    for i in 0..=3 {
        ph.update_at(&ex(i * STEP, Some(27 * MIB)), s(i));
        a.now = s(i);
        update(&mut a, Msg::ExtractProgress { done: i * STEP, total: Some(27 * MIB) });
    }
    for (at, end) in [(3, " · 3.1 MiB/s · ~6 s"), (15, " · sem dados há 12 s")] {
        let cli = ph.line_at(0, ascii(), s(at)).unwrap();
        a.now = s(at);
        let tui = wingetrs::tui::view::running_text(&a, ascii()).unwrap();
        let (c, t) = (after(&cli, "(MSI) "), after(&tui, "(MSI) "));
        assert_eq!(c, t);
        assert!(c.ends_with(end), "{c}");
    }
}
```

Confira os textos exatos de `Placing` e do relógio com `phase_title`/`clock` antes de fixar (`"Colocando os arquivos"` e `00:03` vêm do código atual; se o título for outro, use o do código e cite no relatório). Se o `Msg::InstallPhase` reiniciar o relógio de outro jeito na TUI, ajuste só o instante base do teste.

Mais testes:
4. `tests/it/install_progress.rs`, `cached_install_emits_phases_in_order` passa a esperar, entre `"extract"` e `Placing`, a linha `"bytes tool-1.0.zip 248/Some(248)"` (o `trace_line` ganha o braço `ProgressEvent::Extract { file, done, total } => format!("bytes {file} {done}/{total:?}")`). Rode 20 vezes seguidas (`for i in 1..=20`) para provar que o dreno do canal deixa a ordem fixa.
5. `tests/it/tui_jobs.rs` (o `match` de `:1385` ganha o braço) `extract_ticks_are_throttled_apart_from_downloads`: `ProgressThrottle::new(base)`; um download completo (`admit(base, "a", 10, Some(10))`); depois `progress_msg(&t, base, &ProgressEvent::Extract { file: "x.zip".into(), done: 5, total: Some(10) })` casa `Some(Msg::ExtractProgress { done: 5, total: Some(10) })` (não 15); o mesmo com `done: 6` no mesmo instante ⇒ `None`; `done: 10` no mesmo instante ⇒ `Some(Msg::ExtractProgress { done: 10, .. })`.
6. `tests/it/tui_jobs.rs` `extract_bytes_outside_the_phase_are_ignored`: `RunProgress` na fase `Placing`; `update(Msg::ExtractProgress{..})` não muda `app.running` (compare o `RunProgress` antes e depois; ele é `PartialEq`).

**Files:** `src/install/progress.rs`, `src/install/executor.rs`, `src/cli/output.rs`, `src/tui/jobs.rs`, `src/tui/app.rs`, `src/tui/view.rs`, `docs/cli-textos.md` (uma linha), `tests/it/install_progress.rs`, `tests/it/tui_jobs.rs`.

#### Checks beyond the steps

1. Mutação: o executor não drena o canal depois do worker ⇒ teste 4 **PEGA** em alguma das 20 rodadas (se não pegar, diga no relatório; o teste de ordem continua valendo pela regra).
2. Mutação: `Phases::update_at` grava `Extract` sem conferir o arquivo ⇒ `extract_line_has_phase_clock_and_bar` **PEGA** (vai a 100 %).
3. Mutação: `PhaseLog::line` trata `Extract` como `Download` ⇒ `extract_bytes_add_no_phase_log_line` **PEGA**.
4. Mutação: `progress_msg` manda `Extract` pelo `admit` dos downloads ⇒ teste 5 **PEGA** (soma 15).
5. Mutação: `progress_text` da TUI ignora o medidor na fase `Extracting` ⇒ `cli_and_tui_share_extract_text` **PEGA**.
6. Prova manual (orquestrador): `wingetrs install` de um app do Scoop com 7z ou zip ≥ 100 MB num terminal mostra `Extraindo … · mm:ss [█████░░░…] NN%  x / y MiB · v MiB/s`, chega a 100 % e passa a `Colocando os arquivos`; com stderr redirecionado, uma linha só para a extração. A TUI mostra a mesma barra na pílula.
7. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(progress): extraction bar in the CLI and the TUI, fed by the extractors`

---

### Task 229: Análise de portabilidade de MSI e sonda no catálogo do winget

**Modelo:** opus (revisor opus).

**Depois de:** 230 (`src/install/extract_msi.rs`). Nenhum arquivo em comum com 205/207/209.

> Origem: BACKLOG item G (decisão do usuário, 2026-10-21): modo "portátil" para pacotes do winget sem UAC. "Antes: filtro pelas tabelas do MSI (`ServiceInstall`, driver, `CustomAction` relevante, `Registry` em HKLM fora do app), testar MSI dual-purpose (`ALLUSERS=2 MSIINSTALLPERUSER=1`) e Inno `/CURRENTUSER`; opt-in com aviso do que se perde." Esta tarefa faz o filtro (puro, testável com MSI sintético) e mede no catálogo real quantos pacotes ele aceita. A instalação fica na Task 231, depois das decisões do usuário que a sonda informa.

#### Regras do CLAUDE.md que se aplicam ao item G (para esta e para a 231)

- **Modo portátil por extração própria** (o caminho desta fase): nenhum instalador roda. A extração é a da 2b (`msi` + `cab`, código próprio) e o que se grava fica nos lugares da 2a (raiz do wingetrs, shims, `HKCU\Environment` para o PATH, atalhos em `Start Menu\Programs\wingetrs Apps`). Coberto pelas regras atuais, sem exceção nova. Já existe o mesmo caminho para `portable` do winget (Task 189, `src/winget_install/portable.rs`).
- **`msiexec /a` como reserva** quando o extrator próprio recusa: é a exceção da 2b (confirmação no item, fila do `msi.lock`), escrita para apps do Scoop. Usar em pacote do winget no modo portátil é caso **não previsto** ⇒ decisão D-G3.
- **Inno pelo innounp**: o innounp é auxiliar do bucket main usado em `innosetup: true` (2c). Aplicar a instalador Inno do winget é caso **não previsto** ⇒ D-G1. Agentes não rodam innounp sobre instaladores reais sem pedido do usuário.
- **MSI dual-purpose por usuário (`msiexec /i … ALLUSERS=2 MSIINSTALLPERUSER=1`) e Inno `/CURRENTUSER`**: rodam o instalador real, então caem na exceção da 2d (item 1: instalador com SHA-256 conferido, depois da confirmação; `msiexec` só com `/i`). As duas propriedades seriam constantes do código, nunca vindas do manifesto. Não é "portátil" (mantém ARP e atualizador): caso **não previsto** como modo ⇒ D-G1. Teste real só `#[ignore = "live: ..."]` com pedido do usuário.
- **Escopo**: o modo portátil não muda o escopo de instalador nenhum (não roda instalador); o app fica na raiz do usuário como qualquer app do wingetrs.

#### Where this fits

- MSI: `extract_msi.rs` lê `Directory`, `Component`, `File` por `tables(pkg)` (`:401`), com `rows` (`:328`), `text` (`:348`), `int` (`:355`), `required` (`:362`), `type Pkg` (`:21`), todos privados; `msi_layout::layout(dirs, comps, files) -> Vec<Placed { file, rel }>` (`src/install/msi_layout.rs:169`, `rel` = caminho da árvore do `msiexec /a`, pelo nome **de origem** do `DefaultDir`; é o que o extrator grava). `Mapper` alinha a caixa do `extract_dir` (`align_case`, `extract_msi.rs:178`).
- Pacotes do winget: `plan_with_mode` (`src/winget_install/plan.rs:98`) devolve `PlanOrBridge::Native(WingetPlan)` com `elevation: Elevation` (`launch.rs:58-62`; `install_elevation` `:68-100`: MSI/WiX/Burn sem `Scope: user` ⇒ `Helper`, que é o UAC), `PlanOrBridge::Portable(PortableSpec)` (Task 189) ou `Bridge`. Manifesto: `MergedManifest` e `EffectiveInstaller { kind, url, sha256, scope, elevation, commands, .. }` (`src/sources/winget/manifest.rs:300-320`). Busca do manifesto: `sources::winget::details::merged_manifest` (como em `portable::plan_from_catalog`, `portable.rs:192-230`); download conferido: `winget_install::fetch::fetch_installer_until` (`fetch.rs:184`, grava em `blobs\<sha256>`).
- Builder de MSI sintético nos testes: `tests/it/install_msi.rs:91-178` (`build(&MsiSpec)` com o crate `msi`: `Package::create`, `create_table`, `Column::build(..).primary_key().id_string(72)`, `insert_rows(Insert::into(..).row(..))`, `write_stream`); gabinete `tests/fixtures/msi/three.cab` (entradas `f_a`, `f_b`, `f_c`).
- ARMADILHAS: "Recusa nova sem rodar o corpus real (Task 138)" (por isso a sonda é parte desta tarefa), "Checagem de segurança por lista de proibidos falhou de novo e de novo" (aqui o filtro decide compatibilidade, não segurança: nada roda; mesmo assim, ação personalizada "que roda código" é uma lista fechada de tipos), "Cada checagem de validação precisa de uma fixture que falhe só nela".

#### What binds this task

1. `src/install/extract_msi.rs`: `type Pkg`, `rows`, `text`, `int`, `tables` e `Tables` passam a `pub(crate)` (nenhuma outra mudança).
2. Módulo novo `src/winget_install/portable_msi.rs` (`pub mod portable_msi;` em `winget_install/mod.rs`; nada de Windows: sem `_win`):
   - `pub fn analyze(file: &Path) -> Result<MsiFacts, ExtractError>`: abre o pacote como `extract_msi` (`BufReader<File>`, sob `catch_decoder`; pânico do parser vira `ExtractError::Decoder`).
   - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct MsiFacts { pub blockers: Vec<Blocker>, pub losses: Vec<Loss>, pub main_dir: Option<Vec<String>>, pub files: Vec<Vec<String>>, pub shortcuts: Vec<(String, Vec<String>)>, pub per_user_capable: bool }`. `files` e os alvos de `shortcuts` são relativos a `main_dir`, ordenados.
   - `pub enum Blocker { Service(String), Driver(String), DeferredAction(String), Com(String), SystemRegistry(String), SystemFiles(usize), Scattered }` e `pub enum Loss { ImmediateAction(String), Registry(usize), Associations(Vec<String>), Environment(Vec<String>) }`, os dois com `Display` em português (`"instala o serviço do Windows {0}"`, `"instala driver ({0})"`, `"ação personalizada {0} roda código durante a instalação"`, `"registra componente COM {0}"`, `"grava em HKLM\\{0}"`, `"{0} arquivo(s) para pastas do Windows"`, `"arquivos espalhados fora de uma pasta principal"`; perdas: `"ação personalizada {0} não roda"`, `"{0} valor(es) de registro não gravado(s)"`, `"associação de arquivos: {lista}"`, `"variáveis de ambiente: {lista}"`). `blockers` e `losses` sem repetição, na ordem das regras abaixo.
3. Regras (cada uma com fixture que falha só nela):
   - `ServiceInstall` com linhas ⇒ `Service(Name)` por linha.
   - `MsiDriverPackages` com linhas ⇒ `Driver(Component)` por linha.
   - `CustomAction`: só ações presentes em `InstallExecuteSequence`. Tipo base `Type & 0x3F` em `CODE_TYPES = [1, 2, 5, 6, 17, 18, 21, 22, 34, 37, 38, 50, 53, 54]` (DLL, EXE e script, em Binary, arquivo instalado, diretório, texto ou propriedade); com `Type & 0x400` (adiada, `msidbCustomActionTypeInScript`) ⇒ `DeferredAction(Action)`; sem ⇒ `Loss::ImmediateAction(Action)`. Os demais tipos (19 erro, 35 diretório, 51 propriedade) não aparecem. A política exata (só adiadas bloqueiam) é a D-G2: a sonda mede as duas contagens.
   - `Class`, `TypeLib` ou `AppId` com linhas ⇒ `Com(primeira coluna)` por linha; `Registry` com `Key` começando por `CLSID\` ou `Software\Classes\CLSID\` (sem caixa) ⇒ `Com(Key)`.
   - `Registry` com `Root == 2` e `Key` começando por `SYSTEM\` (sem caixa) ⇒ `SystemRegistry(Key)`. As outras linhas de `Registry` (que não são COM nem `SYSTEM\`) contam em `Loss::Registry(n)`.
   - Arquivos cujo diretório tem, na cadeia de `Directory_Parent`, um id em `SYSTEM_DIRS = ["SystemFolder", "System64Folder", "System16Folder", "WindowsFolder", "FontsFolder", "WindowsVolume"]` ⇒ `SystemFiles(n)` (a D-G5 decide se isso bloqueia sempre).
   - `main_dir` = maior prefixo comum (por componente) dos `rel` de `msi_layout::layout` dos arquivos que não são de pasta do Windows; com menos de 2 componentes ⇒ `Scattered` e `main_dir = None`.
   - `Extension` com linhas ⇒ `Loss::Associations` com `.{Extension}` ordenadas; `Environment` com linhas ⇒ `Loss::Environment` com o `Name` sem os prefixos `= - + ! *`, ordenadas.
   - `Shortcut`: `Name` longo (depois do `|`); alvo `[#ChaveDoArquivo]` ⇒ esse arquivo; outro texto (atalho anunciado, alvo é uma Feature) ⇒ o arquivo do `KeyPath` do `Component_`; alvo fora de `main_dir` ou sem arquivo ⇒ ignorado. Ordenados pela chave `Shortcut`.
   - `per_user_capable` = bit `0x8` do word count do resumo **e** (`ALLUSERS` = `2` ou existe a propriedade `MSIINSTALLPERUSER`). Só registrado (D-G1).
4. **Sonda** (`examples/portable_probe.rs`, roda depois dos testes verdes): para cada id da lista abaixo, busca o manifesto (`merged_manifest` com um `DataDir` temporário no scratchpad), planeja com `plan_with_mode(&m, &host_win::current(), ScopeWanted::Prefer(Scope::User), None, InstallerMode::default())`; se for `Native` com `elevation == Helper` e `kind` `Msi`/`Wix`: baixa com `fetch_installer_until` (SHA conferido; pula > 500 MB como "grande"), roda `analyze` e depois `extract::extract(arquivo, "<id>.msi", main_dir.join("\\"), None, tmp)` só para saber se o extrator próprio aceita (nada roda; apaga em seguida). Imprime TSV: id, tipo, elevação, veredito, bloqueios, perdas, `per_user_capable`, extração (`ok` ou o erro), MB. Para os não MSI: id, tipo, elevação e "fora". No fim apaga o `DataDir` temporário inteiro (ARMADILHAS "Disco cheio no meio do lote"). Lista inicial (o tipo real vem do manifesto): `7zip.7zip`, `Microsoft.PowerShell`, `VideoLAN.VLC`, `Mozilla.Firefox`, `KeePassXCTeam.KeePassXC`, `OpenJS.NodeJS.LTS`, `GoLang.Go`, `Amazon.Corretto.21.JDK`, `EclipseAdoptium.Temurin.21.JDK`, `Inkscape.Inkscape`, `TheDocumentFoundation.LibreOffice`, `Google.Chrome`, `WiresharkFoundation.Wireshark`, `Microsoft.AzureCLI`, `PuTTY.PuTTY`, `WinSCP.WinSCP`, `Zoom.Zoom`, `Notepad++.Notepad++`, `GIMP.GIMP`, `Audacity.Audacity`, `DBeaver.DBeaver.Community`, `Git.Git`, `Python.Python.3.12`, `Docker.DockerDesktop`, `Microsoft.VisualStudioCode`.
   **O orquestrador pede o OK do usuário antes de rodar a sonda** (baixa até ~3 GB de instaladores reais para uma pasta temporária e apaga no fim; nada é executado). Resultado em `docs/superpowers/reports/2026-10-2x-sonda-portatil.md`: a tabela, as contagens (MSI aceitos, recusados por regra, adiadas vs imediatas, `per_user_capable`, extração própria ok) e os números que as decisões D-G2, D-G3 e D-G5 pedem.

Testes (`tests/it/portable_msi.rs`, novo; o `mod portable_msi;` em `tests/it/main.rs` é do orquestrador). O builder é `pub(crate)` porque a 231 o reusa:

```rust
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! MSI portability analysis (Task 229). Synthetic MSIs built with the `msi` crate; the
//! cabinet is `tests/fixtures/msi/three.cab` (entries f_a, f_b, f_c).
use std::io::{Cursor, Write};
use std::path::Path;

use msi::{Column, Insert, Package, PackageType, Value};
use wingetrs::winget_install::portable_msi::{Blocker, Loss, MsiFacts, analyze};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/msi");

pub(crate) struct Extra {
    pub(crate) name: &'static str,
    pub(crate) columns: Vec<Column>,
    pub(crate) rows: Vec<Vec<Value>>,
}

pub(crate) struct AppMsi {
    /// Directory of `readme.txt` (f_c); `foo.exe` (f_a) is in INSTALLDIR, `lib.dll` (f_b) in BIN.
    pub(crate) f_c_dir: &'static str,
    pub(crate) word_count: i32,
    pub(crate) properties: Vec<(&'static str, &'static str)>,
    pub(crate) extra: Vec<Extra>,
}

pub(crate) fn app() -> AppMsi {
    AppMsi {
        f_c_dir: "INSTALLDIR",
        word_count: 2,
        properties: vec![("ProductName", "Foo")],
        extra: Vec::new(),
    }
}

pub(crate) fn build(s: AppMsi) -> Vec<u8> {
    let mut p = Package::create(PackageType::Installer, Cursor::new(Vec::new())).unwrap();
    p.summary_info_mut().set_word_count(s.word_count);
    p.create_table(
        "Directory",
        vec![
            Column::build("Directory").primary_key().id_string(72),
            Column::build("Directory_Parent").nullable().id_string(72),
            Column::build("DefaultDir").text_string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Component",
        vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("Directory_").id_string(72),
            Column::build("KeyPath").nullable().id_string(72),
        ],
    )
    .unwrap();
    p.create_table(
        "File",
        vec![
            Column::build("File").primary_key().id_string(72),
            Column::build("Component_").id_string(72),
            Column::build("FileName").text_string(255),
            Column::build("FileSize").int32(),
            Column::build("Version").nullable().string(72),
            Column::build("Attributes").nullable().int16(),
            Column::build("Sequence").int32(),
        ],
    )
    .unwrap();
    p.create_table(
        "Media",
        vec![
            Column::build("DiskId").primary_key().int16(),
            Column::build("LastSequence").int32(),
            Column::build("Cabinet").nullable().string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Property",
        vec![
            Column::build("Property").primary_key().id_string(72),
            Column::build("Value").text_string(0),
        ],
    )
    .unwrap();
    let dirs: [(&str, Option<&str>, &str); 5] = [
        ("TARGETDIR", None, "SourceDir"),
        ("ProgramFiles64Folder", Some("TARGETDIR"), "PFiles"),
        ("INSTALLDIR", Some("ProgramFiles64Folder"), "FOOT~1|Foo Target:FOOS~1|Foo Source"),
        ("BIN", Some("INSTALLDIR"), "bin"),
        ("System64Folder", Some("TARGETDIR"), "System64"),
    ];
    for (d, parent, default) in dirs {
        let parent = parent.map_or(Value::Null, Value::from);
        p.insert_rows(Insert::into("Directory").row(vec![d.into(), parent, default.into()]))
            .unwrap();
    }
    let comps = [
        ("C_INSTALLDIR", "INSTALLDIR", "f_a"),
        ("C_BIN", "BIN", "f_b"),
        ("C_OTHER", s.f_c_dir, "f_c"),
    ];
    for (c, d, key) in comps {
        p.insert_rows(Insert::into("Component").row(vec![c.into(), d.into(), key.into()]))
            .unwrap();
    }
    let files = [
        ("f_a", "C_INSTALLDIR", "FOO~1.EXE|foo.exe", 1),
        ("f_b", "C_BIN", "lib.dll", 2),
        ("f_c", "C_OTHER", "readme.txt", 3),
    ];
    for (key, comp, name, seq) in files {
        p.insert_rows(Insert::into("File").row(vec![
            key.into(),
            comp.into(),
            name.into(),
            0.into(),
            Value::Null,
            Value::Int(0),
            seq.into(),
        ]))
        .unwrap();
    }
    p.insert_rows(Insert::into("Media").row(vec![Value::Int(1), 100.into(), "#cab1".into()]))
        .unwrap();
    for (k, v) in &s.properties {
        p.insert_rows(Insert::into("Property").row(vec![(*k).into(), (*v).into()]))
            .unwrap();
    }
    for e in s.extra {
        p.create_table(e.name, e.columns).unwrap();
        for r in e.rows {
            p.insert_rows(Insert::into(e.name).row(r)).unwrap();
        }
    }
    let cab = std::fs::read(Path::new(FIXTURES).join("three.cab")).unwrap();
    p.write_stream("cab1").unwrap().write_all(&cab).unwrap();
    p.into_inner().unwrap().into_inner()
}

pub(crate) fn facts(bytes: &[u8]) -> MsiFacts {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x.msi");
    std::fs::write(&f, bytes).unwrap();
    analyze(&f).unwrap()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_string()).collect()
}

/// `(Action, Type, in InstallExecuteSequence)`.
fn custom_actions(rows: &[(&'static str, i32, bool)]) -> Vec<Extra> {
    vec![
        Extra {
            name: "CustomAction",
            columns: vec![
                Column::build("Action").primary_key().id_string(72),
                Column::build("Type").int16(),
                Column::build("Source").nullable().id_string(72),
                Column::build("Target").nullable().string(255),
            ],
            rows: rows
                .iter()
                .map(|(a, t, _)| vec![(*a).into(), Value::Int(*t), Value::Null, Value::Null])
                .collect(),
        },
        Extra {
            name: "InstallExecuteSequence",
            columns: vec![
                Column::build("Action").primary_key().id_string(72),
                Column::build("Condition").nullable().string(255),
                Column::build("Sequence").nullable().int16(),
            ],
            rows: rows
                .iter()
                .filter(|(_, _, seq)| *seq)
                .enumerate()
                .map(|(n, (a, _, _))| vec![(*a).into(), Value::Null, Value::Int(4000 + n as i32)])
                .collect(),
        },
    ]
}

fn registry(rows: &[(&'static str, i32, &'static str)]) -> Extra {
    Extra {
        name: "Registry",
        columns: vec![
            Column::build("Registry").primary_key().id_string(72),
            Column::build("Root").int16(),
            Column::build("Key").text_string(255),
            Column::build("Name").nullable().text_string(255),
            Column::build("Value").nullable().text_string(0),
            Column::build("Component_").id_string(72),
        ],
        rows: rows
            .iter()
            .map(|(r, root, key)| {
                vec![(*r).into(), Value::Int(*root), (*key).into(), "x".into(), "1".into(), "C_INSTALLDIR".into()]
            })
            .collect(),
    }
}

#[test]
fn plain_app_is_portable() {
    let f = facts(&build(app()));
    assert_eq!(f.blockers, Vec::<Blocker>::new());
    assert_eq!(f.main_dir, Some(s(&["PFiles", "Foo Source"])));
    assert_eq!(f.files, vec![s(&["bin", "lib.dll"]), s(&["foo.exe"]), s(&["readme.txt"])]);
    assert!(!f.per_user_capable);
}

#[test]
fn service_blocks() {
    let extra = vec![Extra {
        name: "ServiceInstall",
        columns: vec![
            Column::build("ServiceInstall").primary_key().id_string(72),
            Column::build("Name").text_string(255),
            Column::build("Component_").id_string(72),
        ],
        rows: vec![vec!["svc".into(), "FooSvc".into(), "C_INSTALLDIR".into()]],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Service("FooSvc".into())]);
}

#[test]
fn driver_blocks() {
    let extra = vec![Extra {
        name: "MsiDriverPackages",
        columns: vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("Flags").int32(),
        ],
        rows: vec![vec!["C_BIN".into(), 0.into()]],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Driver("C_BIN".into())]);
}

#[test]
fn deferred_code_action_blocks_immediate_is_a_loss() {
    let extra = custom_actions(&[
        ("Deferred", 1 | 0x400, true),
        ("Immediate", 34, true),
        ("SetDir", 35 | 0x400, true),
        ("SetProp", 51, true),
        ("UiOnly", 1 | 0x400, false),
    ]);
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::DeferredAction("Deferred".into())]);
    assert_eq!(f.losses, vec![Loss::ImmediateAction("Immediate".into())]);
}

#[test]
fn com_class_blocks() {
    let extra = vec![Extra {
        name: "Class",
        columns: vec![
            Column::build("CLSID").primary_key().string(38),
            Column::build("Context").primary_key().id_string(32),
            Column::build("Component_").primary_key().id_string(72),
        ],
        rows: vec![vec![
            "{00000000-0000-0000-0000-000000000001}".into(),
            "InprocServer32".into(),
            "C_BIN".into(),
        ]],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Com("{00000000-0000-0000-0000-000000000001}".into())]);
}

#[test]
fn com_registry_key_blocks() {
    let extra = vec![registry(&[("r1", 0, "CLSID\\{00000000-0000-0000-0000-000000000002}")])];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Com("CLSID\\{00000000-0000-0000-0000-000000000002}".into())]);
    assert_eq!(f.losses, Vec::<Loss>::new());
}

#[test]
fn system_registry_blocks_other_rows_are_a_loss() {
    let f = facts(&build(AppMsi {
        extra: vec![registry(&[("r1", 2, "Software\\Foo"), ("r2", 1, "Software\\Foo")])],
        ..app()
    }));
    assert_eq!(f.blockers, Vec::<Blocker>::new());
    assert_eq!(f.losses, vec![Loss::Registry(2)]);
    let f = facts(&build(AppMsi {
        extra: vec![registry(&[("r3", 2, "SYSTEM\\CurrentControlSet\\Services\\Foo")])],
        ..app()
    }));
    assert_eq!(f.blockers, vec![Blocker::SystemRegistry("SYSTEM\\CurrentControlSet\\Services\\Foo".into())]);
}

#[test]
fn system_files_block() {
    let f = facts(&build(AppMsi { f_c_dir: "System64Folder", ..app() }));
    assert_eq!(f.blockers, vec![Blocker::SystemFiles(1)]);
    assert_eq!(f.main_dir, Some(s(&["PFiles", "Foo Source"])));
}

#[test]
fn scattered_files_block() {
    let f = facts(&build(AppMsi { f_c_dir: "TARGETDIR", ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Scattered]);
    assert_eq!(f.main_dir, None);
}

#[test]
fn associations_and_environment_are_losses() {
    let extra = vec![
        Extra {
            name: "Extension",
            columns: vec![
                Column::build("Extension").primary_key().text_string(255),
                Column::build("Component_").primary_key().id_string(72),
            ],
            rows: vec![vec!["foo".into(), "C_INSTALLDIR".into()]],
        },
        Extra {
            name: "Environment",
            columns: vec![
                Column::build("Environment").primary_key().id_string(72),
                Column::build("Name").text_string(255),
                Column::build("Value").nullable().text_string(0),
                Column::build("Component_").id_string(72),
            ],
            rows: vec![vec!["e1".into(), "=-*FOO_HOME".into(), "[INSTALLDIR]".into(), "C_INSTALLDIR".into()]],
        },
    ];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, Vec::<Blocker>::new());
    assert_eq!(
        f.losses,
        vec![Loss::Associations(s(&[".foo"])), Loss::Environment(s(&["FOO_HOME"]))]
    );
}

#[test]
fn shortcut_targets_are_relative_to_the_main_dir() {
    let extra = vec![Extra {
        name: "Shortcut",
        columns: vec![
            Column::build("Shortcut").primary_key().id_string(72),
            Column::build("Directory_").id_string(72),
            Column::build("Name").text_string(128),
            Column::build("Component_").id_string(72),
            Column::build("Target").string(72),
        ],
        rows: vec![
            vec!["s1".into(), "ProgramMenuFolder".into(), "FOO|Foo App".into(), "C_INSTALLDIR".into(), "[#f_a]".into()],
            vec!["s2".into(), "DesktopFolder".into(), "Lib".into(), "C_BIN".into(), "DefaultFeature".into()],
        ],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(
        f.shortcuts,
        vec![("Foo App".to_string(), s(&["foo.exe"])), ("Lib".to_string(), s(&["bin", "lib.dll"]))]
    );
}

#[test]
fn per_user_capable_needs_the_flag_and_the_property() {
    let with = |wc: i32, props: Vec<(&'static str, &'static str)>| {
        facts(&build(AppMsi { word_count: wc, properties: props, ..app() })).per_user_capable
    };
    assert!(with(2 | 8, vec![("ProductName", "Foo"), ("ALLUSERS", "2")]));
    assert!(with(2 | 8, vec![("ProductName", "Foo"), ("MSIINSTALLPERUSER", "1")]));
    assert!(!with(2, vec![("ProductName", "Foo"), ("ALLUSERS", "2")]));
    assert!(!with(2 | 8, vec![("ProductName", "Foo"), ("ALLUSERS", "1")]));
}

#[test]
fn own_extraction_lands_where_the_analysis_says() {
    let bytes = build(app());
    let f = facts(&bytes);
    let d = tempfile::tempdir().unwrap();
    let msi = d.path().join("x.msi");
    std::fs::write(&msi, &bytes).unwrap();
    let dest = d.path().join("out");
    let dir = f.main_dir.clone().unwrap().join("\\");
    wingetrs::install::extract::extract(&msi, "x.msi", Some(&dir), None, &dest).unwrap();
    for rel in &f.files {
        let p = rel.iter().fold(dest.clone(), |p, c| p.join(c));
        assert!(p.is_file(), "{}", p.display());
    }
}
```

Se o crate `msi` recusar alguma coluna (ex.: `Class` com três chaves primárias), ajuste só o esquema da tabela no builder (a regra lida pelo analisador é a mesma) e cite no relatório. Os literais com `\\` são escritos pela ferramenta Write/Edit, nunca por heredoc (ARMADILHAS "Escape `\xNN` no texto de um comando vira byte bruto"); confira com `grep -n 'CLSID' tests/it/portable_msi.rs`.

**Files:** `src/install/extract_msi.rs` (só visibilidade), `src/winget_install/portable_msi.rs` (novo), `src/winget_install/mod.rs` (só o `mod`), `examples/portable_probe.rs` (novo), `tests/it/portable_msi.rs` (novo), `docs/superpowers/reports/2026-10-2x-sonda-portatil.md` (novo, da sonda).

#### Checks beyond the steps

1. Mutação: sem o filtro de `InstallExecuteSequence` ⇒ `UiOnly` aparece e `deferred_code_action_blocks_immediate_is_a_loss` **PEGA**; outra: sem o teste de `0x400` (toda ação de código bloqueia) ⇒ `Immediate` vira bloqueio e o mesmo teste **PEGA**; outra: `CODE_TYPES` com o `35` ⇒ `SetDir` bloqueia e **PEGA**.
2. Mutação: `main_dir` pelo nome de destino (`Foo Target`) em vez do de origem ⇒ `plain_app_is_portable` e `own_extraction_lands_where_the_analysis_says` **PEGAM**.
3. Mutação: `SystemFiles` contado pelo nome da pasta em vez do id na cadeia ⇒ `system_files_block` **PEGA** (`System64` é o `DefaultDir`, o id é `System64Folder`).
4. Mutação: `per_user_capable` sem o bit `0x8` ⇒ `per_user_capable_needs_the_flag_and_the_property` **PEGA**.
5. Mutação: atalho anunciado ignora o `KeyPath` ⇒ `shortcut_targets_are_relative_to_the_main_dir` **PEGA**.
6. A sonda roda só com o OK do usuário pedido pelo orquestrador; o relatório traz a tabela e as contagens. `scripts/leftovers.ps1` no fim e conferência de que a pasta temporária sumiu.
7. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget): MSI portability analysis (blockers, losses, main folder, shortcuts) and catalog probe`

---

### Task 231: `install <pacote do winget> --portable`: MSI sem UAC pela extração própria

**Modelo:** opus (revisor opus).

**Depois de:** 229 (analisador e sonda), 205 (`src/cli/output.rs`, `docs/cli-textos.md`) e as decisões **D-G1 a D-G5** (fim deste arquivo). Esta seção assume as recomendações (D-G1 = A, D-G2 = só adiadas bloqueiam, D-G3 = recusar, D-G4 = só a flag, D-G5 = bloquear); se o usuário decidir outra coisa, o orquestrador ajusta os itens marcados com (D-Gn) antes de gerar o brief. TUI fica para uma tarefa seguinte (fora deste plano).

> Origem: BACKLOG item G. O UAC dos pacotes MSI/WiX vem do escopo máquina escolhido pelo autor (`install_elevation` ⇒ `Helper`). O modo portátil põe o conteúdo do MSI na raiz do wingetrs como app da fase 2a (como os `portable` do winget na Task 189), sem rodar o instalador.

#### Where this fits

- Caminho de hoje: `install_cmd::install` (`src/cli/install_cmd.rs:913-930`) manda pacote do winget para `winget_cmd::install(winget, ui, &cand, scope, f, stop, say)` (`src/cli/winget_cmd.rs:534-600`), que escolhe `Engine::Native(plan)`, `Engine::Portable(spec)` (licenças e `flows.install_portable`, `:560-565`) ou `Engine::Bridge`. `--scope` só para winget: erro `Advice` em `install_cmd.rs:925-930` (copie a forma para `--portable` em app do Scoop). `Command::Install` (`src/cli/mod.rs:203-225`) e a desmontagem em `install_cmd.rs:733-748`. Há 6 chamadas de `winget_cmd::install` (1 em `src`, 5 em `tests/it/wi_cli.rs`).
- `WingetFlows` (`winget_cmd.rs:~286-330`, `async fn` no trait; impls: `run_win.rs:165` e 4 nos testes: `admin_terminal.rs:157`, `cli_scripts.rs:94`, `wi_cli.rs:164`, `wi_portable.rs:711`). Um método novo com **corpo padrão** não obriga a mexer nos 4 de teste.
- `PortableSpec { id, name, version, manifest, agreements }` e `portable::spec` (`src/winget_install/portable.rs:96-170`) montam o manifesto no formato do Scoop (`url` com `#/<arquivo>`, `hash`, `bin`); `install_plan` (`:182`) chama `plan_manifest` com o bucket `WINGET_BUCKET`; `plan_from_catalog` (`:188-230`) replaneja a atualização e recusa o que deixou de ser `Portable` (`:224-228`). Apps com bucket `winget` são reconhecidos no `upgrade` do CLI (`install_cmd.rs:1179`), em `ops::upgradable` (`src/install/ops.rs:1199`), na correlação (`src/installed/correlate.rs:205`) e na TUI (`src/tui/jobs.rs:343`, `:423`, via `portable_upgrade` ⇒ `plan_from_catalog`).
- Download conferido do instalador: `fetch::fetch_installer_until(http, data, id, version, &installer, progress, stop)` (`src/winget_install/fetch.rs:184`), usado por `ops::fetch_verified` (`ops.rs:268`); o passo `Download` da fase 2a usa `download::fetch_all` com o índice `blobs\index.json` (`src/install/download.rs:119-170`, `:281`).
- MSI pela extração própria: `.msi` no fragmento da URL ⇒ `ArchiveKind::Msi` ⇒ `extract_msi` com `extract_dir` alinhado sem caixa. Reserva `msiexec /a` só com `Flags.allow_msiexec` (`install_cmd.rs:519-545`).
- Textos do CLI: todos em `src/cli/output.rs` e inventariados em `docs/cli-textos.md`.

#### What binds this task

1. **Flag** `--portable` em `Command::Install` (help: `Instala um pacote MSI do winget sem administrador, extraindo o conteúdo para a pasta do wingetrs`). Vai por parâmetro novo de `install_cmd::install` e de `winget_cmd::install` (`portable: bool`, depois de `scope`), não por `Flags` (12 construções literais). Erros `Advice` (texto em `output.rs`): app do Scoop ⇒ `"--portable só vale para pacotes do winget"` / `"apps do Scoop já ficam na pasta do wingetrs; rode sem --portable"`; com `--scope machine` ⇒ `"--portable não combina com --scope machine"`; motor `winget` no config ⇒ `"--portable precisa do motor do wingetrs"` / `"mude winget_engine para wingetrs nas configurações"`.
2. **Escolha** em `winget_cmd::install` com `portable`: `Engine::Native(plan)` com `kind` `Msi` ou `Wix` ⇒ fluxo portátil (item 3); `Engine::Portable(spec)` ⇒ o caminho de hoje (já é portátil); `Engine::Native` de outro tipo ⇒ `Advice("o modo portátil só vale para instaladores MSI (este é {tipo})", "rode sem --portable")` (D-G1); `Engine::Bridge(why)` ⇒ `Advice("o modo portátil não se aplica: {why}", "rode sem --portable")`. Sem `--portable`, nada muda.
3. **Fluxo portátil**: método novo `async fn portable_msi(&self, plan: &WingetPlan, stop: …) -> anyhow::Result<(PortableSpec, MsiFacts)>` em `WingetFlows`, com corpo padrão `bail!("modo portátil indisponível")`. Produção (`run_win.rs`): fase `Preparing`, `fetch_installer_until` (barra de download, Ctrl+C para entre pedaços), `portable_msi::analyze` dentro de `spawn_blocking` com o handle aguardado, e `portable_msi::spec_for(plan, &facts)`. Com `blockers` não vazio ⇒ erro com a lista (`output::portable_refused(&facts)`: `"{nome} não pode ser instalado no modo portátil:"` e uma linha `  • {bloqueio}` por item), nada instalado. Senão, antes de `install_portable` (que mostra o plano 2a e pede a confirmação de sempre), `say` imprime `output::portable_losses(&facts)`: `"No modo portátil o {nome}:"` e as linhas fixas `  • não aparece em Aplicativos instalados do Windows (remova com wingetrs uninstall)`, `  • não recebe as atualizações do próprio app` e uma por `Loss`.
4. **`spec_for(plan, facts) -> Result<PortableSpec, Refusal>`** em `portable_msi.rs`: mesmas checagens de `portable::spec` para id, versão, URL e SHA (extraia as que já existem para funções comuns em `portable.rs`, sem cópia); manifesto `{"version", "description": nome, "url": "<url>#/<id>.msi", "hash", "extract_dir": main_dir com "\\", "bin": [...], "shortcuts": [...], "homepage"?}`. `bin`: para cada `Commands` do instalador, o arquivo de `facts.files` cujo nome é `<comando>.exe` (sem caixa) ⇒ `[rel, comando]`; comando sem arquivo ⇒ fica fora e vira linha de perda `comando {c} não encontrado no pacote`. `shortcuts`: `[[rel do alvo, nome]]` de `facts.shortcuts`. `main_dir` `None` ⇒ `Refusal`. Todo caminho volta a passar pelo `safepath` no planejador 2a.
5. **Sem `msiexec /a`** (D-G3): `install_portable` do fluxo portátil passa `allow_msiexec: false` mesmo com `--allow-msiexec`; se o extrator próprio recusar o MSI, o erro diz `o extrator do wingetrs não suporta este MSI; instale sem --portable`.
6. **Atualização**: `portable::plan_from_catalog_with(ctx, cdn, host, id, keep_old, progress)` (o `plan_from_catalog` atual delega com progresso vazio; a TUI continua chamando o atual): se o manifesto gravado do app (`layout.current(id)\manifest.json`, campo `url`) termina em `.msi` e o replanejamento devolve `Native` MSI/WiX ⇒ baixa, analisa e monta pelo item 4 (bloqueio novo ⇒ erro com a lista, nada muda); app `portable` comum que passou a MSI continua recusado com a mensagem de hoje. O `upgrade` do CLI (`install_cmd.rs:1179`) usa a versão com progresso.
7. **Um download só**: o arquivo baixado no item 3 é o mesmo que o passo `Download` do plano 2a usa (mesmo índice de blobs); se não for, registre o arquivo no índice depois do `fetch_installer_until`. Teste com `expect(1)` no wiremock.
8. `docs/cli-textos.md`: textos novos da flag, dos erros, da recusa e das perdas.

Testes (`tests/it/wi_msi_portable.rs`, novo; `mod wi_msi_portable;` do orquestrador; reusa `crate::portable_msi::{app, build, facts, AppMsi, Extra}` da Task 229 e o `World`/`publish` de `tests/it/wi_portable.rs` onde servir; copie a forma do `PortableFlows` de `wi_portable.rs:705-760` para o Flows falso):
1. `spec_from_facts_builds_the_scoop_manifest` (puro: `facts(&build(app()))` com atalho `("Foo App", ["foo.exe"])` acrescentado à mão e plano com `commands: ["foo", "falta"]`, id `Acme.Foo`, versão `1.0` ⇒ JSON com `"extract_dir": "PFiles\\Foo Source"`, `url` terminando em `#/Acme.Foo.msi`, `bin == [["foo.exe","foo"]]`, `shortcuts == [["foo.exe","Foo App"]]`; a perda `comando falta não encontrado no pacote` aparece no texto de `portable_losses`).
2. `blocked_msi_is_refused_with_reasons` (Flows falso que devolve facts com `Blocker::Service("FooSvc")`: `winget_cmd::install(.., portable: true, ..)` ⇒ erro cujo texto contém `instala o serviço do Windows FooSvc`; `install_portable` nunca é chamado).
3. `portable_flag_needs_an_msi` (plano `Native` de `Inno` ⇒ erro `o modo portátil só vale para instaladores MSI (este é inno)`; `Bridge("x")` ⇒ `o modo portátil não se aplica: x`).
4. `portable_flag_on_scoop_app_is_advice` e `portable_with_scope_machine_is_advice` (binário real como em `tests/it/cli_install.rs`, sem rede além do wiremock).
5. `losses_are_shown_before_the_confirmation` (Flows falso com `Loss::Registry(2)`: a ordem do que o `say` recebeu tem `No modo portátil o Foo:` antes da chamada de `install_portable`).
6. `msi_portable_installs_as_an_app` (wiremock serve `build(app())` com SHA no manifesto; fluxo de produção até o `execute_install`, com `FakeShortcuts`/`FakeEnv`/`WINGETRS_TEST_*` como em `wi_portable.rs`: `apps\Acme.Foo\1.0\foo.exe` existe, shim `foo` existe, atalho `Foo App` no diretório falso, o wiremock recebeu **1** GET do `.msi`, e `state.db` tem o app com bucket `winget`).
7. `msiexec_fallback_is_never_used` (MSI que o extrator recusa, ex. word count sem o bit de compressão: com `allow_msiexec: true` nas flags, o `Msiexec` falso nunca é chamado e o erro diz `instale sem --portable`).
8. `upgrade_of_an_msi_portable_app` (app instalado pelo teste 6; catálogo publica `1.1` MSI ⇒ `plan_from_catalog_with` devolve plano 2a com download do `.msi` novo; um app `portable` `.exe` cuja versão nova é MSI ⇒ recusado com a mensagem de hoje).
9. Ao vivo, `#[ignore = "live: baixa e instala um MSI real no modo portátil"]`: `wingetrs install KeePassXCTeam.KeePassXC --portable --yes` (ou o primeiro aceito pela sonda) ⇒ app instalado sem UAC, `keepassxc` abre; remove no fim. Só o orquestrador roda, com o OK do usuário.

**Files:** `src/winget_install/portable.rs`, `src/winget_install/portable_msi.rs`, `src/cli/mod.rs` (flag), `src/cli/install_cmd.rs` (repasse, `upgrade`), `src/cli/winget_cmd.rs`, `src/cli/run_win.rs`, `src/cli/output.rs` (textos), `docs/cli-textos.md`, `tests/it/wi_msi_portable.rs` (novo), `tests/it/wi_cli.rs` (só as 5 chamadas com o parâmetro novo).

#### Checks beyond the steps

1. Mutação: o fluxo portátil ignora `blockers` ⇒ teste 2 **PEGA**.
2. Mutação: `extract_dir` sem o `main_dir` (árvore inteira) ⇒ teste 6 **PEGA** (o shim aponta para `foo.exe`, que fica em `PFiles\Foo Source\`).
3. Mutação: o fluxo portátil repassa `allow_msiexec` do usuário ⇒ teste 7 **PEGA**.
4. Mutação: perdas impressas depois do `install_portable` ⇒ teste 5 **PEGA**.
5. Mutação: o `upgrade` replaneja MSI-portátil como `Native` ⇒ teste 8 **PEGA**.
6. O revisor confere que nada roda instalador nem `msiexec` no caminho novo (`git diff <base>..HEAD -- src | grep -n "msiexec\|Launcher\|runas"`), e que nenhum texto novo saiu fora de `output.rs`.
7. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget): --portable installs an MSI package without UAC through the own extractor`

---

## Decisões do usuário (G e I)

**D-G1. O que entra no "modo portátil".**
- **A. Só MSI/WiX pela extração própria** (Tasks 229/231). Vantagens: nenhum instalador roda, nenhuma exceção nova do CLAUDE.md, reaproveita a 2b e a 2a. Desvantagens: perde ARP, atualizador e o que as ações personalizadas fariam; Inno, NSIS e Burn ficam de fora.
- **B. A + MSI dual-purpose por usuário** (`msiexec /i … ALLUSERS=2 MSIINSTALLPERUSER=1` quando o MSI declara, `per_user_capable`). Vantagens: instalação "de verdade" por usuário, com ARP em HKCU, desinstalador e atualizador. Desvantagens: roda o instalador real (exceção 2d); o MSI pode ignorar as propriedades e pedir UAC mesmo assim; só se testa ao vivo.
- **C. B + Inno `/CURRENTUSER`** (e, para Inno sem suporte, extração pelo innounp). Vantagens: cobre muitos apps de desktop. Desvantagens: precisa ler o cabeçalho do Inno (`PrivilegesRequiredOverridesAllowed`) ou tentar e ver; innounp em instalador do winget é caso novo; mais superfície.
- **Recomendo A agora**, e B numa tarefa seguinte se a sonda mostrar `per_user_capable` em boa parte dos MSI. C fica no BACKLOG.

**D-G2. Ações personalizadas (CustomAction).**
- **Estrita**: qualquer ação que roda código na `InstallExecuteSequence` bloqueia. Vantagem: o app nunca depende de algo que não rodou. Desvantagem: deve recusar a maioria dos MSI do WiX, que trazem ações imediatas inofensivas.
- **Só adiadas bloqueiam** (`0x400`, as que mexem no sistema durante a instalação); imediatas viram "não roda" no aviso. Vantagem: aceita mais pacotes. Desvantagem: uma imediata pode ser necessária (ex.: gravar config) e o app quebra em silêncio.
- **Recomendo decidir depois da sonda**, com "só adiadas" como padrão se as imediatas forem quase todas das bibliotecas do WiX.

**D-G3. `msiexec /a` como reserva no modo portátil.**
- **Recusar** (o erro manda instalar sem `--portable`). Vantagem: o modo nunca toca o Windows Installer nem a exceção 2b. Desvantagem: MSI que só o `msiexec /a` abre (gabinete externo, transform) fica de fora.
- **Permitir com `--allow-msiexec`** (estende a exceção 2b a pacotes do winget). Vantagem: cobre mais MSI. Desvantagem: o `/a` não se cancela, segura o mutex global e esbarra no `MAX_PATH` (ARMADILHAS).
- **Recomendo recusar**; a sonda conta quantos o extrator próprio aceita.

**D-G4. Como o usuário liga o modo.**
- **Só a flag `--portable` por instalação** (e, depois, uma ação "Instalar portátil" na TUI). Vantagem: opt-in explícito, com o aviso do que se perde a cada vez. Desvantagem: é preciso lembrar da flag.
- **Também uma opção no config** (ex. usar o portátil sempre que o pacote pediria UAC). Vantagem: sem UAC por padrão. Desvantagem: perde ARP e atualizador sem o usuário perceber pacote a pacote.
- **Recomendo só a flag.**

**D-G5. Arquivos para pastas do Windows** (`System32`, fontes; ex. runtime do VC++ em merge module).
- **Bloquear**. Vantagem: nada do app fica faltando em silêncio. Desvantagem: recusa pacotes que só levam o runtime.
- **Copiar para a pasta do app** (as DLLs ao lado do exe costumam ser achadas). Vantagem: aceita mais pacotes. Desvantagem: fontes e serviços não funcionam assim, e uma DLL do sistema antiga ao lado do exe pode carregar no lugar da do Windows.
- **Recomendo bloquear agora** e rever com a contagem da sonda.

**D-I1. Total do tar na barra.**
- **A. Sem total** (só tamanho e velocidade, sem barra; 100 % no fim). Vantagem: nada novo; o número é o que foi gravado. Desvantagem: sem barra nem estimativa para `.tar.gz`/`.tar.xz`.
- **B. Bytes comprimidos lidos sobre o tamanho do arquivo** (um leitor contador antes do descompressor). Vantagem: barra e estimativa reais. Desvantagem: o número mostrado é de bytes comprimidos (diferente dos outros formatos) e é um segundo caminho de contagem.
- **Recomendo A** (como está na Task 230); quantos apps do Scoop usam tar: não medido.

**D-I2. Ordem.** A 230 pode começar já; a 232 espera 205 e 207. Alternativa: fazer a 232 inteira de uma vez depois de 205/207 (sem a 230 separada). **Recomendo separar**: a medição do custo do callback (pedida no BACKLOG) sai antes e decide o `REPORT_STEP`.

---


## Bloco 234

### Task 234: Caminhos de sistema por KnownFolder, não pelo ambiente (`reg.exe`, `msiexec.exe`)

**Modelo:** sonnet (revisor opus: segurança). Origem: revisão da Task 233 (2026-10-22).
- `src/install/registry_win.rs` (`reg_exe()`, perto da linha 402) monta o caminho de `reg.exe` a partir de `%SystemRoot%` do ambiente. Num terminal de administrador, o ambiente vem do usuário (o `HKCU\Environment` é gravável sem admin). Um `SystemRoot` trocado faria o `export` rodar como admin o `reg.exe` que o usuário quiser. Se o Windows deixa o HKCU sobrescrever `SystemRoot` não foi verificado; a correção vale de qualquer jeito.
- `src/install/msiexec_win.rs` (perto da linha 57) usa `Command::new("msiexec.exe")` com nome solto. A busca do std olha a pasta do exe antes da System32.

**Files:** `src/install/registry_win.rs`, `src/install/msiexec_win.rs`, os testes do módulo.

**Contrato:**
- `reg_exe()` usa `FOLDERID_System`, como `orphans_win.rs` (perto da linha 20). Nenhuma leitura de `SystemRoot`/`windir` do ambiente para montar caminho de executável.
- O `msiexec` da extração usa `launch_win::msiexec_path()` (KnownFolder) ou um equivalente já existente.
- Uma regra do `check_rules.ps1` recusa `env::var("SystemRoot")`, `env::var("windir")` e `Command::new("<nome>.exe")` com nome solto em `src/` fora de `#[cfg(test)]`. Use lista de exceções só se houver caso legítimo, justificado no relatório. Prove a regra plantando a violação, vendo o gate falhar e restaurando.

#### Verificação (cole cada `[OK] mutação PEGA`)
- Um teste confere que `reg_exe()` termina em `System32\reg.exe` mesmo com `SystemRoot` trocado no ambiente do teste (`std::env::set_var` dentro de um teste serializado, restaurado no fim). Mutação: voltar ao `%SystemRoot%` faz o teste falhar.
- O selftest do `check_rules` ganha um caso de cada padrão proibido.

**Relatório:** `.claude/tasks/task-234-report.md`.
