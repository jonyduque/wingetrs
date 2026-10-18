# wingetrs — Fase 2c: scripts PowerShell, innosetup e installer.file/args (plano)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-18-wingetrs-fase2c.md`.

**Spec:** `docs/superpowers/specs/2026-10-15-wingetrs-fase2c-design.md` (decidida em 2026-10-18; **D1 = B, prelúdio próprio**: nada da lib do Scoop é baixado nem carregado).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- Agentes nunca executam scripts de manifesto reais nem o innounp sobre instaladores reais (só testes `#[ignore = "live: ..."]`).
- Testes sem rede (fixtures e `wiremock`), sem PATH/registro reais.
- Sem crate nova. Texto ao usuário em português.

## Review Focus

1. Chave de script só na variante de arquitetura (`architecture.64bit.pre_install`) e ausente na raiz.
2. Manifesto com várias chaves ao mesmo tempo (innosetup + post_install): a camada mais arriscada vence.
3. Hook como string única × array de linhas; linhas vazias; comentários.
4. Manifesto que já planeja na 2a/2b nunca é contado como 2c.
5. Catálogo com manifesto JSON inválido: contado à parte, nunca derruba o oráculo.

---

### Task 142: Oráculo da 2c — classificação por sub-caso e funções do Scoop usadas

**Modelo:** opus (revisor sonnet).

#### Where this fits

- `src/install/manifest.rs`: `SCRIPT_KEYS` (~:206, 7 chaves), recusa de `innosetup` (~:225–228 "usa innosetup: suportado na fase 2c"), `resolve(raw_json, host)` (~:292) com a regra de variante de arquitetura (`field(root, variant, ...)`).
- `examples/oracle_2a.rs`: oráculo da 2a sobre o `catalog.db` (`is_simple`, leitura dos manifestos do Scoop do catálogo, contagem por motivo). Copie o modo de abrir o catálogo e de iterar os manifestos dele.
- Spec 2c, tabela de sub-casos (seção a): `S-psm` (psmodule), `S-inno` (innosetup), `S-file` (`installer.file`/`args` com `.exe`/`.msi`/`.cmd`, sem script), `S-ps1` (`installer.file` em `.ps1`), `S-hook` (hooks de script: `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`, `installer.script`, `uninstaller.script`). Ordem de risco crescente: S-psm < S-inno < S-file < S-ps1 < S-hook.

#### What binds this task

1. `pub fn classify(raw_json: &str) -> Classification` em `src/install/classify_2c.rs` (novo, puro; `mod` em `src/install/mod.rs`): devolve o conjunto de sub-casos presentes (raiz **ou** qualquer variante de `architecture`) e o mais arriscado (`worst`). Manifesto sem nenhum ⇒ `None` (não é 2c). JSON inválido ⇒ `Err`.
2. `pub fn script_commands(raw_json: &str) -> Vec<String>`: para todos os scripts do manifesto (hooks e `installer.script`/`uninstaller.script`, string ou array, raiz e variantes), os **nomes de comando** chamados: primeira palavra de cada comando (início de linha, depois de `|`, `;`, `{`, `(`, `&&`, `||`, `=`), ignorando comentários `#`, strings literais e here-strings (`@' '@`, `@" "@`). Léxico simples e documentado; não precisa ser perfeito (o oráculo mede, não decide segurança).
3. `examples/oracle_2c.rs` sobre o `catalog.db` real (só leitura, como o `oracle_2a`): imprime
   - por bucket (main, extras, demais somados): total de manifestos, quantos são 2c, contagem por `worst` e por presença de cada sub-caso;
   - quantos 2c **só** têm S-psm/S-inno/S-file (entregáveis sem PowerShell);
   - ranking dos comandos de script: nome, nº de manifestos que usam, e se é função definida no Scoop (lista fixa no exemplo, ver item 4) ou cmdlet/outra coisa;
   - cobertura acumulada: "implementando as N funções do Scoop mais usadas, X% dos manifestos S-hook/S-ps1 ficam só com funções suportadas" para N = 5, 10, 20, 30, todas.
4. A lista de funções do Scoop vem de uma tabela no próprio exemplo, extraída do fonte do Scoop (`lib/*.ps1`, `function <nome>`), com o commit citado em comentário. Pode ler o fonte em `~/scoop/apps/scoop/current/lib` se existir (só leitura) ou do GitHub (`ScoopInstaller/Scoop`, branch master); diga qual usou.
5. Saída do oráculo colada no relatório e salva em `docs/superpowers/reports/2026-10-18-oraculo-2c.md` (tabelas em Markdown).

**Files:**
- Create: `src/install/classify_2c.rs`, `examples/oracle_2c.rs`, `docs/superpowers/reports/2026-10-18-oraculo-2c.md`
- Modify: `src/install/mod.rs`
- Test: `tests/it/classify_2c.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/classify_2c.rs`, JSON escrito no teste):
1. `none_for_simple_manifest`: manifesto só com `url`/`bin` ⇒ `None`.
2. `one_fixture_per_layer`: um manifesto por sub-caso ⇒ o sub-caso certo.
3. `variant_only_key_counts`: `architecture.64bit.post_install` sem raiz ⇒ S-hook.
4. `worst_wins`: innosetup + post_install ⇒ `worst = S-hook`, conjunto com os dois.
5. `installer_file_ps1_vs_exe`: `installer.file = "x.ps1"` ⇒ S-ps1; `"setup.exe"` com `args` ⇒ S-file; `installer.script` ⇒ S-hook.
6. `commands_skip_comments_strings_herestrings`: script com `# Add-Path x`, `'shim'`, here-string com `ensure` e chamadas reais `Add-Path $dir`, `Remove-Item x | Out-Null` ⇒ só `Add-Path`, `Remove-Item`, `Out-Null`.
7. `invalid_json_is_err`.

#### Checks beyond the steps

1. Mutação: ignorar as variantes de `architecture` ⇒ teste 3 **PEGA**.
2. Mutação: não pular here-strings ⇒ teste 6 **PEGA**.
3. Rode `cargo run --release --example oracle_2c -- <catalog.db real>` (caminho em `%LOCALAPPDATA%\wingetrs\catalog.db`) e cole a saída.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): phase 2c oracle (sub-cases and Scoop functions used by manifest scripts)`

---

## Decisões do plano que completam a spec (aceitas pelo orquestrador em 2026-10-18)

| # | Ponto | Decisão no rascunho | Motivo / evidência |
|---|---|---|---|
| R1 | `$ErrorActionPreference='Stop'` no runner (spec c) | **Não** muda: o hook roda com o padrão (`Continue`), dentro de `try/catch` que sai com 1 em erro terminante | `grep ErrorActionPreference` em `bin/scoop.ps1`, `lib/core.ps1`, `lib/install.ps1` (e6aa3b3): nenhuma ocorrência. Com `Stop`, `Remove-Item` de arquivo ausente derrubaria hooks que passam no Scoop |
| R2 | `Expand-MsiArchive` no prelúdio | usa o auxiliar **lessmsi** (ramo `USE_LESSMSI` do Scoop), nunca `msiexec` | o ramo padrão do Scoop é `msiexec.exe` (`decompress.ps1:191-195`); o CLAUDE.md só permite `msiexec /a` pela fila do `msi.lock` com confirmação por item |
| R3 | `installer.file` terminando em `.msi` | recusado com motivo | o Scoop roda o instalador com `UseShellExecute = $false` (`core.ps1`, `Invoke-ExternalCommand`), e o `CreateProcess` não abre `.msi`: nem o Scoop executa |
| R4 | "auxiliares marcados `helper=1`" (spec c) | sem coluna nova; auxiliar = dependência oculta protegida pelo `dependents` | YAGNI: nada da spec lê a marca; evita mexer no `state.rs` (que a 2d também altera) |
| R5 | `spawn_win` "passa para `src/proc/`" | fica em `src/bridge/spawn_win.rs`, ganha `RunSpec`/`run_spec` (ambiente, pasta, decodificador) | mover mexe em `bridge/elevate_win.rs` e `tui/run.rs` sem ganho |
| R6 | `scoop` como comando externo dentro de hook | **recusado** pelo portão com motivo claro; nada mapeado para operações do wingetrs | o oráculo achou só 4 manifestos no catálogo (main+extras) chamando `scoop`; os 367 do nerd-fonts não estão no catálogo |
| R7 | Lugar da recusa por "ainda não suportado" | sai de `manifest::resolve` e vai para `planner::plan_install` (`PlanError::Unsupported`), uma recusa por chave, removida pela tarefa que implementa a chave | assim o modelo tipado entra primeiro sem ligar nada pela metade |
| R8 | Consentimento | `Runners` no `ExecCtx`: corredores de código de terceiros (`installer`, `scripts`) são `Option`; `None` = sem consentimento ⇒ o executor recusa **antes de qualquer passo**. Até a Task 161 nenhum caminho de produção concede consentimento | mesmo desenho do `Msiexec` (`executor.rs:52`); impossível rodar script com só `--yes` enquanto a CLI/TUI não existem |
| R9 | Codificação do `prelude.ps1` | só ASCII (teste); `runner.ps1` gravado com BOM UTF-8 | o PowerShell 5.1 lê `.ps1` sem BOM como ANSI; o caminho do usuário pode ter acento |

## Ondas e lanes (arquivos disjuntos dentro de cada onda)

| Onda | Tarefas | Observação |
|---|---|---|
| 1 | **143** | base de tudo (modelo tipado) |
| 2 | L1: **144 → 145** · L2: **146** · L3: **149** · L4: **150** · L5: **152** · L6: **155** | PARALELO entre lanes |
| 3 | L1: **147** (precisa de 145 e 146) · L5: **153** (precisa de 152) | PARALELO; 147 toca literais de teste da CLI/TUI: nenhuma tarefa de CLI/TUI (2c ou 2d) em paralelo |
| 4 | L1: **148 → 151** · L5: **154** (precisa de 147 e 153) | PARALELO |
| 5 | L1: **156 → 157 → 158 → 159** · L7: **160** (depois de 156) | 160 em PARALELO com 157–159 |
| 6 | **161 → 162 → 163** | sozinhas (arquivos da CLI/TUI) |

S-psm fica pronto em 145; S-inno em 148 (147 + auxiliar automático). Nenhuma das duas precisa de PowerShell nem de consentimento.

`src/install/mod.rs` e `tests/it/main.rs`: o orquestrador cria **antes do despacho** cada arquivo novo de `src/install/` e de `tests/it/` com uma linha `//! <assunto> (Task N).` e as linhas `pub mod <x>;` / `mod <x>;` (os agentes não editam esses dois arquivos).

## Arquivos compartilhados com a fase 2d (não rodar em paralelo com tarefas da 2d que toquem o mesmo arquivo)

| Arquivo | Tarefas da 2c | Por quê |
|---|---|---|
| `src/install/state.rs` | 144, 145 | a 2d cria a tabela `winget_ops` no `state.db` |
| `src/install/ops.rs` | 147, 148, 151, 156, 159, 161 | a 2d tem `install/winget/ops.rs` próprio, mas pode rotear por aqui |
| `src/install/prod_win.rs` | 147, 151, 160 | contexto de produção comum |
| `src/cli/install_cmd.rs` | 147 (só o literal do `OpsCtx`), 161 | confirmação/flags da 2d |
| `src/cli/mod.rs`, `src/cli/output.rs` | 161 | flags e resumo do plano |
| `src/tui/app.rs`, `view.rs`, `regions.rs`, `jobs.rs` | 162 | confirmação da 2d (licenças) |
| `tests/it/tui_jobs.rs`, `tests/it/cli_cancel.rs`, `tests/it/install_progress.rs`, `tests/it/install_ops.rs`, `tests/it/install_executor.rs` | 147 (uma linha `runners: Runners::disabled()` em cada literal) | literais de `OpsCtx`/`ExecCtx` |

`src/config.rs` e `src/install/executor.rs`: nenhuma tarefa da 2c toca o `config.rs`; o `executor.rs` é da lane L1 (assumindo que a 2d não o toca).

---

### Task 143: Modelo tipado da 2c (hooks, installer, psmodule, innosetup, auxiliares)

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/install/manifest.rs`: `pub struct Resolved` (:115), `fn field(root, variant, key)` (:150, variante sobrepõe raiz: `variant.and_then(|v| v.get(key)).or_else(|| root.get(key)).filter(|v| !v.is_null())`), `const SCRIPT_KEYS` (:206), `fn check_keys` (:216, recusa as 7 chaves e o `innosetup` em :225–228 com `"usa innosetup: suportado na fase 2c"`), `pub fn resolve(raw_json, host)` (:292, chama `check_keys(root)` e `check_keys(v)` da variante), literal `Ok(Resolved { ... })` (:371).
- `src/install/planner.rs`: `pub enum PlanError` (:33), `pub fn plan_install` (:190).
- `src/install/ops.rs`: `OpsError::Unsupported` (`#[error("não suportado: {0}")]`, :186), `From<ManifestError>` (:235, mapeia `ManifestError::Unsupported{reason}` para `OpsError::Unsupported(reason)`), `OpsError::Plan(#[from] PlanError)`.
- `src/install/classify_2c.rs:216`: `pub fn commands_in(script: &str) -> Vec<String>` (léxico: comandos em posição de comando, sem comentários, strings e here-strings).
- `src/install/safepath.rs:6`: `pub fn safe_components(raw) -> Option<Vec<String>>` (recusa `..`, absoluto, drive, UNC, ADS, nomes reservados).
- Scoop (e6aa3b3, só leitura): `lib/install.ps1:143-169` `Invoke-HookScript` junta o script com `` $script -join "`r`n" ``; `lib/depends.ps1:74` `Get-InstallationHelper`: `$script = $pre_install + $installer.script + $post_install`; `7zip` se `*Expand-7zipArchive *`, `lessmsi` se `*Expand-MsiArchive *` (e `USE_LESSMSI`), `innounp` se `innosetup` ou `*Expand-InnoArchive *`, `dark` se `*Expand-DarkArchive *`; `lib/psmodules.ps1` lê `$manifest.psmodule` (só raiz); `lib/decompress.ps1` `Invoke-Extraction` lê `$Manifest.innosetup` (só raiz).

#### What binds this task

1. Tipos novos em `src/install/manifest.rs` (todos `pub`, `#[derive(Debug, Clone, PartialEq, Eq)]`, enums também `Copy, PartialOrd, Ord, Hash`):
   ```rust
   pub enum HookKind { PreInstall, InstallerScript, PostInstall, PreUninstall, UninstallerScript, PostUninstall }
   // HookKind::key(): "pre_install", "installer.script", "post_install", "pre_uninstall", "uninstaller.script", "post_uninstall"
   #[derive(Default)] pub struct Hooks { pub pre_install: Option<String>, pub post_install: Option<String>, pub pre_uninstall: Option<String>, pub post_uninstall: Option<String> }
   pub struct InstallerSpec { pub file: Option<String>, pub args: Vec<String>, pub keep: bool, pub script: Option<String> }
   pub struct PsModule { pub name: String }
   pub enum Helper { SevenZip, Lessmsi, Innounp, Dark }   // Helper::app(): "7zip", "lessmsi", "innounp", "dark"
   ```
   `Resolved` ganha: `hooks: Hooks`, `installer: Option<InstallerSpec>`, `uninstaller: Option<InstallerSpec>`, `psmodule: Option<PsModule>`, `innosetup: bool`, `helpers: BTreeSet<Helper>`, `raw_json: String` (o texto recebido, sem reformatar).
2. Valor de script (hooks e `installer.script`/`uninstaller.script`): texto ⇒ o próprio texto; lista de textos ⇒ unidos com `"\r\n"`; ausente, `null`, `""` ou `[]` ⇒ `None`; outro tipo ⇒ `ManifestError::Invalid("<chave>: deve ser texto ou lista de textos")`.
3. Variante de arquitetura sobrepõe a raiz **chave a chave**, pela `field()` existente, para `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`, `installer`, `uninstaller` (o objeto `installer` inteiro, sem mesclar campos). Linha literal obrigatória: `let installer = installer_spec(field(root, variant, "installer"), "installer")?;` (e a análoga para `"uninstaller"`). `psmodule` e `innosetup` vêm **só da raiz** (como o Scoop).
4. `installer`/`uninstaller`: objeto com `file` (texto), `args` (texto ⇒ `vec![texto]`; lista de textos), `keep` (bool, padrão `false`), `script` (regra do item 2). Objeto sem `file`, `args` nem `script` ⇒ `None`. `file` passa por `safe_components`; recusado ⇒ `Invalid("<chave>.file: caminho inseguro")`. `args` com caractere de controle ⇒ `Invalid("<chave>.args: caractere de controle")`.
5. `psmodule`: objeto com `name` texto não vazio que seja **um** componente seguro (`safe_components(name)` com exatamente 1 item) ⇒ `Some`. Sem `name` ⇒ `Invalid("psmodule sem name")` (texto exato). Nome com `..`, `\` ou `/` ⇒ `Invalid("psmodule.name: nome inseguro")`.
6. `innosetup`: `true` ⇒ `true`; `false`, `null` ou ausente ⇒ `false`; outro tipo ⇒ `Invalid("innosetup deve ser true ou false")`.
7. `helpers` pela regra do `Get-InstallationHelper`, sobre `pre_install` + `installer.script` + `post_install` já resolvidos pela variante, comparando os comandos de `classify_2c::commands_in` em minúsculas por **igualdade** com a tabela literal:
   `const HELPER_COMMANDS: [(&str, Helper); 4] = [("expand-7ziparchive", Helper::SevenZip), ("expand-msiarchive", Helper::Lessmsi), ("expand-innoarchive", Helper::Innounp), ("expand-darkarchive", Helper::Dark)];`
   mais `Innounp` quando `innosetup`. A URL **não** conta (o extrator próprio cuida de 7z/MSI da URL). Hooks de desinstalação não contam. `Lessmsi` sempre que houver `Expand-MsiArchive` (o prelúdio nunca usa `msiexec`).
8. `resolve()` deixa de recusar essas chaves: apague `SCRIPT_KEYS` e `check_keys`. Se `ManifestError::Unsupported`/`fn unsupported` ficarem sem uso (`grep -n "unsupported(" src/install/manifest.rs`), remova também o braço correspondente do `From<ManifestError>` em `ops.rs`; se houver outro uso, mantenha.
9. A recusa passa para o planejador: `PlanError::Unsupported(String)` com `#[error("não suportado: {0}")]` e `fn refuse_2c(m: &Resolved) -> Result<(), PlanError>`, chamada no início de `plan_install`, com **um `if` por chave, nesta ordem** (cada tarefa futura apaga o seu):
   `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall` (hook `Some`), `installer`, `uninstaller` (`Some`), `psmodule` (`Some`) ⇒ `"usa <chave> (PowerShell/instalador próprio): suportado na fase 2c"`; `innosetup` ⇒ linha literal `return Err(PlanError::Unsupported("usa innosetup: suportado na fase 2c".into()));`.
   O texto final ao usuário fica idêntico ao de hoje (`não suportado: usa innosetup: suportado na fase 2c`).
10. `examples/oracle_2a.rs` continua compilando (usa `resolve` + `plan_install`; o filtro `is_simple` é independente).

**Files:**
- Modify: `src/install/manifest.rs`, `src/install/planner.rs`, `src/install/ops.rs` (só se o item 8 exigir)
- Test: `tests/it/manifest_2c.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/manifest_2c.rs`, JSON escrito no teste; host `Arch::X64` salvo indicação; para o `PlanInput` copie o construtor de `tests/it/install_planner.rs`):
1. `hooks_string_and_array`: `pre_install: "a"` ⇒ `Some("a")`; `post_install: ["l1","l2"]` ⇒ `Some("l1\r\nl2")`.
2. `empty_hook_is_none`: `pre_install: ""` e `post_install: []` ⇒ ambos `None`; `pre_install: 3` ⇒ `Err(Invalid(..))`.
3. `variant_installer_overrides_root`: raiz `installer.file = "root.exe"`, `architecture.64bit.installer.file = "v64.exe"` ⇒ `"v64.exe"`; outro manifesto com a variante 64bit sem `installer` ⇒ `"root.exe"`.
4. `variant_hook_overrides_root_key_by_key`: raiz `pre_install "R"`, `post_install "RP"`; variante `post_install "V"` ⇒ `pre_install == Some("R")`, `post_install == Some("V")`.
5. `installer_fields`: `{"file":"setup.exe","args":["/S","/D=$dir"],"keep":true}` ⇒ `InstallerSpec{file: Some("setup.exe"), args: ["/S","/D=$dir"], keep: true, script: None}`; `args: "/S"` ⇒ `["/S"]`.
6. `installer_file_unsafe_refused`: `file` = `"..\\x.exe"`, `"C:\\x.exe"`, `"a/../../x.exe"` ⇒ cada um `Err(Invalid(..))`.
7. `uninstaller_script_only`: `uninstaller: {"script":["a","b"]}` ⇒ `script == Some("a\r\nb")`, `file == None`.
8. `psmodule_rules`: `{"name":"PSFzf"}` ⇒ `Some`; `{}` ⇒ `Err(ManifestError::Invalid("psmodule sem name".into()))`; `{"name":"..\\x"}` ⇒ `Err`.
9. `psmodule_in_variant_only_is_ignored`: só `architecture.64bit.psmodule` ⇒ `psmodule == None`.
10. `innosetup_values`: `true` ⇒ `true`; `false` ⇒ `false`; ausente ⇒ `false`; `"yes"` ⇒ `Err`.
11. `helpers_follow_scoop_rule`: `innosetup:true` ⇒ `{Innounp}`; `post_install: "Expand-7zipArchive \"$dir\\x.7z\""` ⇒ `{SevenZip}`; `pre_install: "expand-msiarchive x"` ⇒ `{Lessmsi}`; `installer.script: "Expand-DarkArchive x"` ⇒ `{Dark}`; `pre_uninstall: "Expand-7zipArchive x"` ⇒ `{}`; `post_install: "# Expand-InnoArchive x"` ⇒ `{}`; `post_install: "Expand-InnoArchiveX x"` ⇒ `{}`.
12. `raw_json_kept_verbatim`: entrada com espaços e ordem de chaves incomuns ⇒ `raw_json == entrada`.
13. `planner_refuses_each_2c_key_with_old_text`: um manifesto por chave (as 7 + `innosetup`) ⇒ `plan_install` devolve `Err(PlanError::Unsupported(..))` e `to_string()` igual a `"não suportado: usa <chave> (PowerShell/instalador próprio): suportado na fase 2c"` / `"não suportado: usa innosetup: suportado na fase 2c"`.
14. `simple_manifest_still_plans`: sem chave da 2c ⇒ `Ok`.

#### Checks beyond the steps

1. Mutação em `src/install/manifest.rs`: âncora `field(root, variant, "installer")` → `root.get("installer").filter(|v| !v.is_null())` ⇒ teste 3 **PEGA**.
2. Mutação em `src/install/manifest.rs`: âncora `.or_else(|| root.get(key))` → `.xor(root.get(key))` ⇒ testes 3/4 **PEGAM** (variante e raiz definidas ⇒ `None`).
3. Mutação em `src/install/manifest.rs`: âncora `("expand-innoarchive", Helper::Innounp)` → `("expand-innoarchivex", Helper::Innounp)` ⇒ teste 11 **PEGA**.
4. Mutação em `src/install/planner.rs`: âncora `return Err(PlanError::Unsupported("usa innosetup: suportado na fase 2c".into()));` → `{}` ⇒ teste 13 **PEGA**.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): typed phase 2c manifest model (hooks, installer, psmodule, innosetup, helpers)`

---

### Task 144: `WriteManifest` e leitura do manifesto gravado

**Modelo:** sonnet (revisor sonnet). **Lane L1** (onda 2, antes da 145). **Compartilhado com a 2d:** `src/install/state.rs`.

#### Where this fits

- `src/install/plan.rs`: `pub enum Step` (:53), `InstallPlan::step_line` (:197, linha do dry-run por passo).
- `src/install/planner.rs:190` `plan_install`: empurra `Step::RecordState` perto do fim (antes de `RemoveOldVersion`).
- `src/install/executor.rs`: `fn step_kind` (:269), `fn step_phase` (:289, `Phase::Recording` para `RecordState`), `fn step_label` (:311), `enum Undo` (:165, `Undo::RemovePath(PathBuf)`), `fn check_order` (:456, só exige `RecordState` como último antes das remoções), `async fn run_step` (:692), braço `Step::RecordState` (:932).
- `src/install/state.rs`: `pub fn write_install_json` (:227, grava `.tmp` e renomeia com `crate::fs_retry::retry("renomear", ...)`), `pub enum StateError` (:8).
- `src/install/ops.rs` `plan_for`: o `manifest_sha256` do plano é `crate::cache::sha256_hex(n.raw.as_bytes())`, os mesmos bytes que `Resolved.raw_json` (campo `raw_json: String` já existente em `Resolved`, texto do manifesto sem reformatar).

#### What binds this task

1. `Step::WriteManifest { json: String }`, empurrado imediatamente antes de `Step::RecordState`, com a linha literal `steps.push(Step::WriteManifest { json: m.raw_json.clone() });`.
2. No executor: grava `<version_dir>\manifest.json` com os bytes exatos de `json` (sem reformatar), via `manifest.json.tmp` + `fs_retry::retry("renomear", ...)`; depois do sucesso, linha literal `run.push(Undo::RemovePath(dest.clone()));`. Falha ⇒ apaga o `.tmp` e o passo falha (rollback).
3. `step_kind` = `"manifest"`, `step_label` = `"gravar manifesto"`, `step_phase` = `Some(Phase::Recording)`, `step_line` = `"gravar <version_dir>\manifest.json"`.
4. `pub fn installed_manifest(layout: &Layout, app: &str, version: &str, expected_sha256: &str) -> Result<Option<String>, StateError>` em `state.rs`: lê `layout.version_dir(app, version).join("manifest.json")`; ausente ⇒ `Ok(None)` (apps instalados antes da 2c); não UTF-8 ou SHA-256 diferente ⇒ `Err(StateError::ManifestMismatch(app))` com `#[error("o manifesto gravado de {0} não confere com o registro")]`; linha literal da checagem: `if crate::cache::sha256_hex(&bytes) != expected_sha256 {`.
5. Nada mais muda nesta tarefa (a remoção passa a usar essa função nas tarefas de remoção com scripts).

**Files:**
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/state.rs`
- Test: `tests/it/install_written_manifest.rs` (**já criado pelo orquestrador** com uma linha `//!`). Para o servidor falso, o `World` e o `FailAt`, copie o padrão de `tests/it/install_executor.rs` (:99–:270), não importe dele.

Testes (`tests/it/install_written_manifest.rs`):
1. `install_writes_manifest_json_byte_exact`: manifesto com `{ "version" :"1.0",  "url":... }` (espaços e ordem incomuns) ⇒ bytes de `apps\<app>\1.0\manifest.json` == entrada.
2. `plan_puts_write_manifest_right_before_record_state`: no plano, `WriteManifest` está no índice `i` e `RecordState` em `i + 1`.
3. `rollback_removes_manifest_json`: `FailAt` no índice do `RecordState` ⇒ `apps\<app>` não existe depois.
4. `installed_manifest_is_the_written_one`: depois de instalar o texto A, `installed_manifest(.., sha(A))` == `Some(A)`; com `sha(B)` de outro texto ⇒ `Err(StateError::ManifestMismatch(app))`.
5. `installed_manifest_missing_is_none`: pasta da versão sem `manifest.json` ⇒ `Ok(None)`.
6. `upgrade_keep_old_has_both_manifests`: instala 1.0 (texto A) e atualiza com `keep_old` para 2.0 (texto B) ⇒ `1.0\manifest.json` == A e `2.0\manifest.json` == B.

#### Checks beyond the steps

1. Mutação em `src/install/planner.rs`: âncora `steps.push(Step::WriteManifest { json: m.raw_json.clone() });` → `let _ = &m.raw_json;` ⇒ testes 1/2 **PEGAM**.
2. Mutação em `src/install/executor.rs`: âncora `run.push(Undo::RemovePath(dest.clone()));` → `let _ = &dest;` ⇒ teste 3 **PEGA**.
3. Mutação em `src/install/state.rs`: âncora `if crate::cache::sha256_hex(&bytes) != expected_sha256 {` → `if false {` ⇒ teste 4 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): write manifest.json into the version folder and read it back`

---

### Task 145: `psmodule` nativo (S-psm)

**Modelo:** sonnet (revisor opus). **Lane L1** (onda 2, depois da 144). **Compartilhado com a 2d:** `src/install/state.rs`.

#### Where this fits

- `src/install/planner.rs`: `fn refuse_2c` (criada para recusar chaves da 2c ainda não implementadas; tem um `if` por chave, um deles `m.psmodule.is_some()` com `"usa psmodule (PowerShell/instalador próprio): suportado na fase 2c"`); laço dos atalhos (`for s in &m.shortcuts`) seguido do laço `for (name, value) in &m.env_set` em `plan_install` (:190).
- `src/install/manifest.rs`: `Resolved.psmodule: Option<PsModule>` (`PsModule { name }`, nome já validado como um componente seguro).
- `src/install/executor.rs`: `enum Undo` (:165, tem `RemovePath`, `RestoreEnv { name, previous }`), braço `Step::EnvSet` (:901, padrão de `RestoreEnv` + `Artifact`), `fn remove_artifact` (:1353), `fn execute_uninstall` (:1407).
- `src/install/state.rs`: `pub enum ArtifactKind` (:18) com `as_str`/`parse`; `pub fn path_users` (:194) como modelo de consulta.
- `src/install/plan.rs:151` `fn artifact_kind` (rótulo da remoção).
- `src/install/layout.rs`; `src/install/link.rs` (`link::create(target, link)`, `link::target`, `link::remove`); `src/install/path_edit.rs` (`norm` :3, `path_add` :9, `path_remove` :21, `path_contains` :30); `src/install/traits.rs` `EnvStore::{get,set,remove}`, `EnvValue{value, expand}`.
- Scoop `lib/psmodules.ps1`: junção `modulesdir\<name>` → `$dir` (o `current`, pois roda depois do `link_current`); se o link existe é substituído; `ensure_in_psmodulepath`: lê `PSModulePath` do usuário, se ausente usa `"$env:USERPROFILE\Documents\WindowsPowerShell\Modules"`, e se não contém a pasta grava `"<modules>;<anterior>"`; a remoção do Scoop não mexe no `PSModulePath`.

#### What binds this task

1. `Layout::modules()` = `<raiz>\modules`.
2. Apague o `if` do `psmodule` em `refuse_2c`. `Step::PsModule { name: String }` vai depois do último `Step::Shortcut` e antes do primeiro `Step::EnvSet` (ordem do Scoop: atalhos → psmodule → env).
3. Execução: `ensure_dirs(modules)`; se `<modules>\<name>` já existe: é de **outro** app no `state.db` (artefato `PsModule` com esse alvo) ⇒ passo falha `"o módulo {name} pertence a {outro}"`; é do registro anterior do mesmo app (upgrade) ⇒ remove e recria, com `Undo` que restaura o link anterior; não é de ninguém ⇒ falha `"{caminho} já existe e não é do wingetrs"` (não apaga o que não é nosso). Cria a junção para `layout.current(app)` e registra `Undo::RemovePath(link)`.
4. `PSModulePath`: lê `EnvStore::get("PSModulePath")`; ausente ⇒ base = `std::env::var_os("USERPROFILE")` + o literal `r"Documents\WindowsPowerShell\Modules"`; se `path_edit::path_contains(base, modules)` (sem diferenciar caixa e barra final) ⇒ nada; senão grava `format!("{};{}", modules.display(), base)` preservando o `expand` do valor anterior, com `Undo::RestoreEnv { name: "PSModulePath", previous }`.
5. `ArtifactKind::PsModule` (`"psmodule"`; rótulo em `plan.rs` `"módulo PowerShell"`), alvo = `name`. `StateDb::apps_with_artifact(kind: ArtifactKind) -> Result<Vec<String>, StateError>`.
6. Remoção (`remove_artifact`): apaga a junção `<modules>\<name>` (ausente é ok); depois, se nenhum **outro** app tem artefato `PsModule` (linha literal `if others.is_empty() {`), tira `<modules>` do `PSModulePath` com `path_remove`, preservando as outras entradas e a ordem; valor vazio ⇒ `EnvStore::remove`.

**Files:**
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/state.rs`, `src/install/layout.rs`
- Test: `tests/it/install_psmodule.rs` (**já criado pelo orquestrador**; copie o `World`/`FailAt` de `tests/it/install_executor.rs`; ambiente sempre `FakeEnv`, nunca o registro real)

Testes:
1. `installs_junction_to_current`: `link::target(<modules>\PSFzf)` == `apps\x\current`.
2. `psmodulepath_prepended_and_preserved`: `PSModulePath = "C:\\A;C:\\B"` ⇒ `"<modules>;C:\\A;C:\\B"`.
3. `psmodulepath_absent_seeds_documents`: ausente ⇒ `"<modules>;<USERPROFILE>\\Documents\\WindowsPowerShell\\Modules"`.
4. `psmodulepath_with_entry_unchanged`: já contém `<MODULES>\` (caixa e barra diferentes) ⇒ valor idêntico e nenhuma escrita (`FakeEnv` sem mudança).
5. `rollback_restores_psmodulepath_and_removes_junction`: `FailAt` no passo seguinte ao `PsModule` ⇒ `PSModulePath` volta ao valor inicial (não o padrão: parta de `"C:\\Z"`) e a junção some.
6. `last_module_removes_entry_only_then`: apps A e B com módulos; remover A ⇒ `<modules>` continua no `PSModulePath`; remover B ⇒ some, e `"C:\\A"` continua.
7. `module_of_other_app_refused`: B tenta o mesmo `name` de A ⇒ instalação de B falha, junção continua apontando para A.
8. `step_order`: `PsModule` depois do último `Shortcut` e antes do primeiro `EnvSet` (manifesto com 2 atalhos e 1 `env_set`).

#### Checks beyond the steps

1. Mutação em `src/install/executor.rs`: âncora `if others.is_empty() {` → `if true {` ⇒ teste 6 **PEGA**.
2. Mutação em `src/install/executor.rs`: âncora `r"Documents\WindowsPowerShell\Modules"` → `r"Documents\Modules"` ⇒ teste 3 **PEGA**.
3. Mutação em `src/install/executor.rs`: âncora `format!("{};{}", modules.display(), base)` → `format!("{}", modules.display())` ⇒ teste 2 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): native psmodule (junction + PSModulePath)`

---

### Task 146: `run_spec` no `spawn_win` (ambiente, pasta, decodificador de bytes)

**Modelo:** opus (revisor opus). **Lane L2** (onda 2, PARALELO).

#### Where this fits

- `src/bridge/spawn_win.rs`: `PIPE_GRACE` (:22), `pub fn run` (:158), `pub fn run_with_grace` (:168), `pub fn run_in` (:180, programa absoluto, sem PATH, pasta), `fn run_command` (:191, `CREATE_NO_WINDOW`, stdin nulo, leitores com `join` em todos os caminhos), `fn read_loop`/`spawn_reader` (:105/:135).
- `src/bridge/mod.rs:511` `pub struct Tail` (`push` :524): divide linhas, corta `\r`, decodifica com `String::from_utf8_lossy(seg)`, corta em `LINE_MAX` na fronteira de caractere.
- Sonda D4 (relatório 2026-10-18-sondas-2c): o innounp escreve em OEM CP850 com CRLF e tudo no stdout; a decodificação UTF-8 com perda troca `ã` (`0xC6`) por `U+FFFD`.
- Usuários atuais: `bridge/elevate_win.rs:544` e `:898` (`run_in`), `tui/run.rs:1379` (`run`). O comportamento deles não muda.

#### What binds this task

1. Em `spawn_win.rs`:
   ```rust
   pub struct RunSpec<'a> {
       pub program: &'a Path, pub args: &'a [String], pub dir: &'a Path,
       pub env: &'a [(OsString, OsString)], pub env_remove: &'a [OsString],
       pub decode: fn(&[u8]) -> String, pub grace: Duration,
   }
   pub fn run_spec(spec: &RunSpec<'_>, on_line: impl FnMut(&str)) -> std::io::Result<(i32, Vec<String>)>
   pub fn utf8_lossy(b: &[u8]) -> String
   ```
   Programa relativo ⇒ `ErrorKind::InvalidInput` (`"caminho do programa precisa ser absoluto"`), com a linha literal `if !spec.program.is_absolute() {`. Ambiente: herda o do processo, aplica `cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));` (linha literal) e `env_remove` de cada nome.
2. `Tail::with_decoder(decode: fn(&[u8]) -> String) -> Tail`; `Tail::default()` usa `utf8_lossy`. Na decodificação, a linha literal `(self.decode)(seg)` substitui `String::from_utf8_lossy(seg)`; o corte em `LINE_MAX` continua na fronteira de caractere.
3. `run`, `run_with_grace` e `run_in` continuam com a mesma assinatura e o mesmo comportamento (montam o `Tail` padrão).
4. Threads: os leitores continuam juntados em todos os caminhos, pânico propagado (`resume_unwind`).

**Files:**
- Modify: `src/bridge/spawn_win.rs`, `src/bridge/mod.rs`
- Test: `tests/it/spawn_env.rs` (**já criado pelo orquestrador**; comece com `#![cfg(windows)]`). Os `.cmd` falsos ficam numa pasta temporária do teste.

Testes:
1. `env_is_passed_and_removed`: `t.cmd` = `@echo off` / `echo A=%WR_A%` / `echo B=%OS%`; `env = [("WR_A","1")]`, `env_remove = ["OS"]` ⇒ linhas `A=1` e `B=%OS%` (o `cmd` deixa literal a variável indefinida).
2. `cwd_is_dir`: `t.cmd` = `@echo off` / `cd` ⇒ a linha é a pasta temporária.
3. `decoder_gets_raw_bytes`: `data.bin` com bytes `6E C6 6F 0D 0A`, `t.cmd` = `@type data.bin`; decodificador do teste que mapeia `0xC6` para `ã` ⇒ linha `"não"`; com `utf8_lossy` ⇒ `"n\u{FFFD}o"`.
4. `relative_program_refused`: `program = Path::new("t.cmd")` ⇒ `ErrorKind::InvalidInput`.
5. `exit_code_propagates`: `exit /b 3` ⇒ código 3.

#### Checks beyond the steps

1. Mutação em `src/bridge/spawn_win.rs`: âncora `cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));` → `let _ = spec.env;` ⇒ teste 1 **PEGA**.
2. Mutação em `src/bridge/mod.rs`: âncora `(self.decode)(seg)` → `String::from_utf8_lossy(seg).into_owned()` ⇒ teste 3 **PEGA**.
3. Mutação em `src/bridge/spawn_win.rs`: âncora `if !spec.program.is_absolute() {` → `if false {` ⇒ teste 4 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): run_spec with environment, working dir and byte decoder`

---

### Task 147: `ExtractInno` (S-inno) e `Runners` no contexto de execução

**Modelo:** opus (revisor opus). **Lane L1** (onda 3). **Compartilhado com a 2d:** `src/install/ops.rs`, `src/install/prod_win.rs`, `src/cli/install_cmd.rs` (só o literal do `OpsCtx`), `tests/it/tui_jobs.rs`, `tests/it/cli_cancel.rs`, `tests/it/install_progress.rs`, `tests/it/install_ops.rs`, `tests/it/install_executor.rs` (uma linha cada). Não rode em paralelo com tarefa que toque esses arquivos.

**Pré-requisitos (confira com `grep`; faltando ⇒ `BLOCKED`):** `pub fn run_spec` e `pub struct RunSpec` em `src/bridge/spawn_win.rs`; `Resolved.innosetup: bool` e o `if` do `innosetup` em `planner::refuse_2c`.

#### Where this fits

- `src/install/executor.rs`: `pub struct Msiexec` (:52) com ponteiros de função e `Msiexec::DISABLED` (:67) — o modelo para corredores injetáveis; `pub struct ExecCtx` (:75); braço `Step::Extract { index }` (:726), que roda a extração num worker bloqueante (`struct AwaitWorker` :1111) dentro do `catch_decoder`; `fn step_kind`/`step_phase`/`step_label` (:269/:289/:311).
- `src/install/plan.rs:53` `enum Step`; `src/install/planner.rs` laço `for index in 0..m.sources.len() { steps.push(Step::Extract { index }); }` em `plan_install`.
- `src/install/ops.rs`: `pub struct OpsCtx` (:113), `fn exec_ctx` (:269). `src/install/prod_win.rs:56` `build_ctx` (em :65, com `WINGETRS_TEST_ENV_KEY` usa `Msiexec::DISABLED`).
- Literais a atualizar: `ExecCtx {` em `tests/it/cli_cancel.rs`, `tests/it/install_executor.rs`, `tests/it/install_progress.rs`, `src/install/ops.rs`; `OpsCtx {` em `tests/it/install_ops.rs`, `tests/it/install_progress.rs`, `tests/it/tui_jobs.rs`, `src/cli/install_cmd.rs`, `src/install/prod_win.rs`.
- `src/install/extract.rs`: `ExtractError::MissingExtractDir(String)` (:48), `fn merge_into` (:621, privada), `catch_decoder` (:31).
- Scoop `lib/decompress.ps1:227-265` `Expand-InnoArchive`: `@('-x', "-d$DestinationPath", $Path, '-y')` + `-c{app}\<ExtractDir>` (sem `{` no início), `-c<ExtractDir>` (com `{`), `-c{app}` sem `extract_dir`; `Invoke-Extraction` só usa o innounp para nomes que terminam em `.exe` quando `innosetup`.
- Sonda D4 (xming, innounp 2.71.2): código 0 + 399 arquivos; **código 0 + zero arquivos** quando o filtro `-c` não casa (`{app}\fonts`, `{tmp}`); código 2 = "not an Inno Setup installation"; código 3 = "Cannot open or read the specified file" + mensagem do sistema em **CP850** (`0xC6` = `ã`); CRLF; stderr vazio; `{app}` some do caminho e o `extract_dir` sai achatado em `<dest>`.

#### What binds this task

1. Em `executor.rs`:
   ```rust
   /// `(exe, args, dir)` → exit code and decoded output lines.
   pub type ToolRun = fn(&Path, &[String], &Path) -> io::Result<(i32, Vec<String>)>;
   #[derive(Clone)]
   pub struct Runners { pub innounp: ToolRun }
   impl Runners { pub fn disabled() -> Runners { /* innounp: Err(io::Error::other("innounp desativado neste ambiente")) */ } }
   ```
   `ExecCtx` e `OpsCtx` ganham `pub runners: Runners`; `exec_ctx` copia `ctx.runners.clone()`. Cada literal listado acima ganha **só** `runners: Runners::disabled(),`. `build_ctx`: com `WINGETRS_TEST_ENV_KEY` ⇒ `Runners::disabled()`; senão `Runners { innounp: inno_win::run }`. Campos futuros entram em `Runners` e em `disabled()`, sem tocar de novo nos literais.
2. `src/install/inno.rs` (puro):
   - `pub fn inno_args(installer: &Path, dest: &Path, extract_dir: Option<&str>) -> Result<Vec<String>, ExtractError>` = `["-x", "-d<dest>", "<installer>", "-y", "-c..."]` pela regra do Scoop, com a linha literal `if dir.starts_with('{') {`; `/` vira `\`; o resto do caminho passa por `safe_components` (o `{nome}` inicial é aceito só como primeiro componente); recusado ⇒ `ExtractError::UnsafePath`.
   - `pub fn decode_cp850(bytes: &[u8]) -> String`: ASCII igual; `0x80..=0xFF` pela tabela CP850 completa (128 entradas).
   - `pub fn count_files(dir: &Path) -> io::Result<u64>`: recursivo, sem seguir pontos de reanálise.
   - `pub fn check_exit(code: i32, files: u64, extract_dir: Option<&str>, lines: &[String]) -> Result<(), ExtractError>`: `0` e `files > 0` ⇒ `Ok`; `0` e zero arquivos ⇒ `MissingExtractDir(extract_dir.unwrap_or("{app}"))`, com a linha literal `if files == 0 {`; `2` ⇒ `Archive("o innounp não reconhece o arquivo como instalador Inno Setup")`; `3` ⇒ `Io("o innounp não conseguiu abrir o instalador: <última linha>")`; outro ⇒ `Archive("innounp saiu com código <c>: <última linha>")`.
3. `src/install/inno_win.rs` (`#[cfg(windows)]` no `mod`): `pub fn run(exe, args, dir)` = `spawn_win::run_spec` com `decode: inno::decode_cp850`, sem ambiente extra, `grace: PIPE_GRACE`.
4. Planejador: apague o `if` do `innosetup` em `refuse_2c`. Para cada fonte, se `m.innosetup` e `source.file_name.to_lowercase()` termina em `".exe"` (linha literal `.ends_with(".exe")`) ⇒ `Step::ExtractInno { index }`; senão `Step::Extract { index }`.
5. Executor `Step::ExtractInno`: exe = `layout.current("innounp").join("innounp.exe")`; ausente ⇒ passo falha com `"innounp não está instalado (auxiliar do bucket main)"` sem chamar o corredor. Destino temporário novo `<staging>\.inno-<index>`; roda `ctx.runners.innounp` no worker bloqueante (mesmo padrão do `Extract`, com `catch_decoder` em volta da leitura); conta arquivos; `check_exit`; move o conteúdo para o staging respeitando o `extract_to` (via `safe_components`) com `merge_into` (passe a `pub(crate)`). Qualquer erro ⇒ apaga o temporário e o passo falha (rollback). Fase `Phase::Extracting { file }`, rótulo `"extrair [n] (Inno Setup)"`.
6. O innounp real nunca roda em teste: os testes injetam `ToolRun` falsos (um `fn` por teste, contador `static` próprio de cada teste) que leem o `-d<dest>` dos argumentos e criam arquivos lá.

**Da revisão da Task 146 (2026-10-18):** `run_spec` herda o ambiente do usuário (o programa é absoluto, mas um filho acha ferramentas pelo PATH do usuário). Esta tarefa decide o ambiente do processo que ela lança: `PATH` fixo (`%SystemRoot%\System32` por KnownFolder + as pastas do próprio app/auxiliar), sem herdar o PATH do usuário, salvo motivo medido; teste do ambiente montado.

**Files:**
- Create: `src/install/inno.rs`, `src/install/inno_win.rs` (**o orquestrador cria os arquivos com uma linha `//!` e as linhas `pub mod` em `src/install/mod.rs`**)
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/extract.rs` (só a visibilidade de `merge_into`), `src/install/ops.rs`, `src/install/prod_win.rs`, `src/cli/install_cmd.rs`, `tests/it/cli_cancel.rs`, `tests/it/install_executor.rs`, `tests/it/install_progress.rs`, `tests/it/install_ops.rs`, `tests/it/tui_jobs.rs` (só a linha `runners`)
- Test: `tests/it/install_inno.rs` (**já criado pelo orquestrador**)

Testes:
1. `args_without_extract_dir`: ⇒ `["-x", "-d<dest>", "<inst>", "-y", "-c{app}"]` (caminhos do temporário do teste).
2. `args_with_subdir`: `Some("xkb/types")` ⇒ último `"-c{app}\\xkb\\types"`.
3. `args_with_brace_dir`: `Some("{tmp}")` ⇒ `"-c{tmp}"`.
4. `args_unsafe_dir_refused`: `Some("..\\x")` ⇒ `Err(UnsafePath)`.
5. `cp850_decodes_pt_br`: `[0x6E, 0xC6, 0x6F]` ⇒ `"não"`; `0x87` ⇒ `"ç"`; `0x82` ⇒ `"é"`; `0xA0` ⇒ `"á"`; `0x90` ⇒ `"É"`; `b"abc"` ⇒ `"abc"`.
6. `exit_zero_with_zero_files_is_missing_extract_dir`: `check_exit(0, 0, Some("fonts"), &[])` ⇒ `MissingExtractDir("fonts")`; `check_exit(0, 0, None, &[])` ⇒ `MissingExtractDir("{app}")`.
7. `exit_codes_2_and_3`: `2` ⇒ texto com `"Inno Setup"`; `3` com última linha `"Não foi possível"` ⇒ texto termina com `"Não foi possível"`.
8. `plan_uses_extract_inno_only_for_exe`: `innosetup:true` com `a.exe` e `b.zip` ⇒ `[ExtractInno{0}, Extract{1}]`; com `innosetup:false` ⇒ `[Extract{0}, Extract{1}]`.
9. `install_with_fake_innounp_places_files`: `apps\innounp\current\innounp.exe` (arquivo vazio) presente; o falso cria `<dest>\Xming.exe` e `<dest>\locale\x.txt` ⇒ a pasta da versão tem os dois; o falso viu `-c{app}`.
10. `fake_exit_zero_no_files_rolls_back`: falso sai 0 sem criar nada ⇒ instalação falha com `MissingExtractDir`; `apps\<app>` não existe.
11. `missing_innounp_fails_without_calling_runner`: sem `apps\innounp` ⇒ erro com `"innounp não está instalado"`; contador do falso == 0.

#### Checks beyond the steps

1. Mutação em `src/install/inno.rs`: âncora `if files == 0 {` → `if false {` ⇒ testes 6 e 10 **PEGAM**.
2. Mutação em `src/install/inno.rs`: âncora `if dir.starts_with('{') {` → `if false {` ⇒ teste 3 **PEGA** (no PowerShell, aspas simples dobradas: `''{''`).
3. Mutação em `src/install/planner.rs`: âncora `.ends_with(".exe")` → `.ends_with("")` ⇒ teste 8 **PEGA**.
4. Recusa nova em extrator (ARMADILHAS, Task 138): a prova contra um instalador real (o `xming` da sonda D4) é um teste `#[ignore = "live: innounp sobre instalador real"]` que só roda **com autorização do usuário**; deixe-o escrito e diga no relatório que não rodou.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): innosetup extraction through innounp (S-inno) and injectable runners`

---

### Task 148: Auxiliares (innounp, 7zip, lessmsi, dark) como dependências ocultas

**Modelo:** sonnet (revisor sonnet). **Lane L1** (onda 4). **Compartilhado com a 2d:** `src/install/ops.rs`.

#### Where this fits

- `src/install/ops.rs`: `fn find_dependency` (:418, `"main/<app>"` explícito escolhe o bucket main), `struct DepWalk` e `fn visit` (:478, pós-ordem; pula o que já está instalado ou já visto; resolve o manifesto da dependência e visita `manifest.depends`), `pub fn plan_for` (:521, `for d in &target.depends { walk.visit(&bucket, d)?; }`), `fn raw_depends` (:745), `fn dependents` (:760, outros apps cujo manifesto do catálogo depende de `app`), `fn refuse_if_needed` (:785), `pub fn plan_uninstall` (:795).
- `src/install/manifest.rs`: `Resolved.helpers: BTreeSet<Helper>`; `Helper::app()` = `"7zip"`, `"lessmsi"`, `"innounp"`, `"dark"`.
- Scoop `lib/depends.ps1:45`: `$deps = @(Get-InstallationHelper ...) + @($manifest.depends)` (auxiliares antes das dependências declaradas).

#### What binds this task

1. Em `plan_for`, **antes** do laço de `target.depends`, linha literal `for h in &target.helpers {` que visita `format!("main/{}", h.app())`. Em `DepWalk::visit`, antes do laço de `manifest.depends`, linha literal `for h in &manifest.helpers {` com a mesma visita (auxiliares das dependências também).
2. Auxiliar já instalado ⇒ não entra no plano (regra existente do `visit`). Auxiliar ausente do catálogo ⇒ `OpsError::DependencyUnsupported { app: "<auxiliar>", reason: "não encontrada no catálogo" }` (regra existente).
3. `dependents`: além de `raw_depends`, um app instalado depende de `app` se `manifest::resolve(&raw, Arch::host())` der `Ok(m)` e algum `m.helpers` tiver `h.app()` igual a `app` (sem caixa). Linhas literais: `let uses_helper = ...;` e `if depends_on_app || uses_helper {`. `resolve` falhando ⇒ só `raw_depends` (com `tracing::warn!`).
4. Sem coluna nova no `state.db` (decisão R4 do rascunho).

**Files:**
- Modify: `src/install/ops.rs`
- Test: `tests/it/install_helpers.rs` (**já criado pelo orquestrador**; catálogo e estado montados como em `tests/it/install_ops.rs`, sem rede)

Testes:
1. `innosetup_app_plans_innounp_first`: main tem `innounp` e `x` (`innosetup:true`) ⇒ planos `[innounp, x]`.
2. `installed_helper_not_planned`: `innounp` no estado ⇒ planos `[x]`.
3. `helper_of_dependency_planned_before_it`: `x` depende de `y`; `y.post_install` chama `Expand-7zipArchive` ⇒ `[7zip, y, x]`.
4. `helper_comes_from_main`: `x` no extras; `innounp` no main e outro `innounp` no extras ⇒ o plano usa o do main (`bucket == "main"`).
5. `removing_helper_with_dependent_refused`: estado com `innounp` e `x` (catálogo de `x` com `innosetup`) ⇒ `plan_uninstall("innounp")` = `Err(HasDependents(["x"]))`.
6. `removing_helper_without_dependents_ok`.
7. `helper_missing_from_catalog`: ⇒ `DependencyUnsupported { app: "innounp", .. }`.

#### Checks beyond the steps

1. Mutação: âncora `for h in &target.helpers {` → `for h in target.helpers.iter().take(0) {` ⇒ teste 1 **PEGA**.
2. Mutação: âncora `for h in &manifest.helpers {` → `for h in manifest.helpers.iter().take(0) {` ⇒ teste 3 **PEGA**.
3. Mutação: âncora `if depends_on_app || uses_helper {` → `if depends_on_app {` ⇒ teste 5 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): install helpers (innounp, 7zip, lessmsi, dark) as hidden dependencies`

---

### Task 149: Guardar `scripts/` dos buckets no `update`

**Modelo:** sonnet (revisor opus). **Lane L3** (onda 2, PARALELO).

#### Where this fits

- `src/sources/scoop/bucket.rs`: `pub struct BucketImport { packages, skipped }` (:9; literais só em `bucket.rs` e `update.rs`), `fn manifest_file` (:34, aceita `<top>/bucket/<app>.json`), `pub fn read_tarball` (:42, `tar::Archive` sobre `GzDecoder`), `pub async fn fetch_bucket` (:81).
- `src/update.rs`: `fn fetch_one` (:142), aplicação de `Fetched::Scoop(imp, entry)` (:255); o `prev: Option<&HttpEntry>` decide o GET condicional.
- `src/cache/mod.rs:33` `pub struct DataDir` (`tmp_dir` :92 como modelo).
- `src/buckets.rs:182` `pub fn remove_and_cleanup`.
- `src/install/safepath.rs:6` `safe_components`.
- Sonda D6: main 15 arquivos / 11,9 KB em `scripts/`; extras 98 / 195 KB; os manifestos usam `$bucketsdir\<bucket>\scripts\<app>\...` com o nome do bucket literal; no Scoop `$bucketsdir = "$scoopdir\buckets"` (`lib/buckets.ps1:1`). O tempo extra no `update` não foi medido.

#### What binds this task

1. `BucketImport` ganha `pub scripts: Vec<(String, Vec<u8>)>`: caminho relativo a `scripts/` (com `/`) e conteúdo, para entradas **regulares** `<top>/scripts/**`. Link simbólico ou físico ⇒ pulado e anotado em `skipped` como `"scripts/<caminho>"`.
2. Cada caminho passa por `safe_components` (linha literal `let Some(parts) = safe_components(rel) else {`); recusado ⇒ anotado em `skipped`, sem abortar o resto.
3. Tetos por bucket, aplicados **antes** de ler o conteúdo: 2000 arquivos e 8 MiB no total; ao passar, o resto é pulado com uma nota `"scripts: limite excedido"`.
4. `DataDir::buckets_dir()` = `<raiz>\buckets`. `pub fn write_scripts(bucket_dir: &Path, files: &[(String, Vec<u8>)], tag: &str) -> io::Result<()>`: grava em `<bucket_dir>\.scripts-new-<tag>`, renomeia o `scripts` antigo para `.scripts-old-<tag>`, renomeia o novo para `scripts`, apaga o antigo (`fs_retry`); falha ⇒ apaga o novo e mantém o antigo intacto. Depois do sucesso grava o marcador `<bucket_dir>\.scripts-ok` (também com zero arquivos). O `update` chama com `tag = <pid>`, só para `Fetched::Scoop`. Nome do bucket validado como um componente seguro.
5. Marcador ausente ⇒ o `update` não passa o `HttpEntry` anterior daquele bucket (GET incondicional), com a linha literal `if !marker.exists() {`.
6. `remove_and_cleanup` apaga `<raiz>\buckets\<bucket>` se existir.
7. Span `tracing::debug_span!("update.scripts")` em volta do `write_scripts` (a medição fica para a validação).

**Files:**
- Modify: `src/sources/scoop/bucket.rs`, `src/update.rs`, `src/cache/mod.rs`, `src/buckets.rs`
- Test: `tests/it/bucket_scripts.rs` (**já criado pelo orquestrador**; tarballs montados no teste com `tar` + `flate2`; `wiremock` como em `tests/it/update.rs`)

Testes:
1. `scripts_collected_with_relative_paths`: `Main-master/scripts/7-zip/a.reg` ⇒ `("7-zip/a.reg", bytes)`; `Main-master/bucket/x.json` continua em `packages`.
2. `unsafe_paths_skipped_rest_kept`: `scripts/../x`, `scripts/C:/x`, `scripts/aux.reg` pulados e anotados; `scripts/ok.reg` mantido.
3. `symlink_entry_skipped`.
4. `file_limit`: 2001 arquivos de 1 byte ⇒ 2000 mantidos + nota `"scripts: limite excedido"`.
5. `swap_replaces_old`: `scripts\old.reg` existente ⇒ depois só os novos; `old.reg` sumiu; nenhum `.scripts-*` sobrando; marcador presente.
6. `failed_write_keeps_old`: um **arquivo** `.scripts-new-T` criado antes ⇒ `write_scripts(.., "T")` = `Err` e `scripts\old.reg` intacto.
7. `missing_marker_forces_unconditional_get`: `http.json` com ETag do bucket e sem marcador ⇒ o servidor falso recebe o pedido **sem** `If-None-Match`; com marcador ⇒ com `If-None-Match`.
8. `bucket_remove_deletes_scripts_dir`.

#### Checks beyond the steps

1. Mutação em `src/sources/scoop/bucket.rs`: âncora `safe_components(rel)` → `Some(rel.split('/').map(str::to_string).collect::<Vec<_>>())` ⇒ teste 2 **PEGA**.
2. Mutação em `src/update.rs`: âncora `if !marker.exists() {` → `if false {` ⇒ teste 7 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(update): store bucket scripts/ folders atomically`

---

### Task 150: Portão estático `script_scan`

**Modelo:** opus (revisor opus). **Lane L4** (onda 2, PARALELO).

#### Where this fits

- `src/install/classify_2c.rs:216` `pub fn commands_in(script) -> Vec<String>` (léxico documentado; pula comentários `#` e `<# #>`, strings e here-strings `@' '@`/`@" "@`).
- `examples/oracle_2c.rs:15` `const SCOOP_FUNCTIONS: &[&str]` (262 nomes de `lib/*.ps1` do Scoop e6aa3b3).
- `src/install/manifest.rs`: `HookKind` (6 tipos, `key()`), `Resolved.hooks`, `Resolved.installer/uninstaller` (`script`).
- Oráculo 2026-10-18: `Expand-7ZipArchive` 236 manifestos, `reg` 39, `Start-Process` 42, `scoop` 4; 36 funções do Scoop usadas no catálogo.
- Spec (c) "Portão estático": `Refuse` (elevação, HKLM, `-Global`, escrita em ProgramFiles/windir, função fora do prelúdio), `ConfirmStrong` (`iex`, `& $var`, download, `reg import`, `Set-ItemProperty HKCU:`, tarefas agendadas, serviços, caminho absoluto fora de `$dir`/`$persist_dir`), `Confirm` (o resto). O mesmo relatório alimenta dry-run, confirmação, `show` e oráculo. A UI nunca diz "seguro".

#### What binds this task

1. `src/install/script_scan.rs` (puro):
   ```rust
   pub const SCOOP_FUNCTIONS: &[&str] = &[ /* os 262 de examples/oracle_2c.rs, mesma ordem */ ];
   #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub enum Verdict { Confirm, ConfirmStrong, Refuse }
   pub enum FlagKind { Elevation, Hklm, GlobalSwitch, SystemWrite, UnknownScoopFunction, ScoopCommand, Msiexec,
                       DynamicCode, Download, RegistryUser, ScheduledTaskOrService, AbsolutePath }
   pub struct Flag { pub kind: FlagKind, pub verdict: Verdict, pub detail: String, pub line: usize }
   pub struct ScriptReport { pub commands: Vec<String>, pub variables: BTreeSet<String>, pub flags: Vec<Flag>, pub verdict: Verdict }
   pub fn scan(script: &str, supported: &[&str]) -> ScriptReport
   pub fn scan_manifest(m: &Resolved, supported: &[&str]) -> Vec<(HookKind, ScriptReport)>
   pub fn env_var_names(script: &str) -> BTreeSet<String>
   ```
   `examples/oracle_2c.rs` passa a usar `script_scan::SCOOP_FUNCTIONS` (apague a cópia local).
2. Texto analisado: linha literal `let text = strip_here_strings(&strip_comments(script));` — tira comentários e o corpo das here-strings, **mantém** o conteúdo das strings entre aspas (um caminho `'HKLM:\...'` conta). Números de linha do script original. Comandos vêm de `commands_in`. Comparações sem caixa.
3. Regras (`detail` em português, texto exato entre aspas):
   - `Refuse`: `Elevation` — `-Verb RunAs`, `-RunAs`, comando `sudo` ou `gsudo` ⇒ `"pede elevação (RunAs/sudo)"`; `Hklm` — `HKLM:` ou `HKEY_LOCAL_MACHINE` ⇒ `"mexe no registro da máquina (HKLM)"`; `GlobalSwitch` — parâmetro `-Global` ⇒ `"usa -Global (instalação para todos os usuários)"`; `SystemWrite` — linha com cmdlet de escrita (`New-Item`, `Copy-Item`, `Move-Item`, `Set-Content`, `Add-Content`, `Out-File`, `Remove-Item`, `Rename-Item`) e `$env:ProgramFiles`, `${env:ProgramFiles(x86)}`, `$env:windir`, `$env:SystemRoot`, `C:\Program Files` ou `C:\Windows` ⇒ `"grava em Program Files/Windows"`; `UnknownScoopFunction` — comando em `SCOOP_FUNCTIONS` e fora de `supported` (linha literal `!supported.iter().any(|s| s.eq_ignore_ascii_case(name))`) ⇒ `"função do Scoop ainda não suportada: <nome canônico de SCOOP_FUNCTIONS>"`, um flag por nome; `ScoopCommand` — comando `scoop` ⇒ `"chama o comando scoop (instalaria no ~/scoop); não suportado"`; `Msiexec` — comando `msiexec`/`msiexec.exe` ou `Start-Process msiexec` ⇒ `"chama o msiexec; o wingetrs só usa msiexec /a pela fila própria"`.
   - `ConfirmStrong`: `DynamicCode` — `iex`, `Invoke-Expression`, `[scriptblock]::Create`, `& $var`, `. $var`; `Download` — `Invoke-WebRequest`, `iwr`, `Invoke-RestMethod`, `irm`, `Start-BitsTransfer`, `curl`, `wget`, `Net.WebClient`, `DownloadFile`, `DownloadString`; `RegistryUser` — comando `reg` com `import`/`add`/`delete`, ou `Set-ItemProperty`/`New-ItemProperty`/`Remove-ItemProperty`/`New-Item`/`Remove-Item` com `HKCU:`; `ScheduledTaskOrService` — `Register-ScheduledTask`, `schtasks`, `New-Service`, `sc.exe`, `Set-Service`, `Start-Service`, `Stop-Service`; `AbsolutePath` — literal `X:\...`, UNC `\\...`, `$env:USERPROFILE`, `$env:APPDATA`, `$env:LOCALAPPDATA`, `[Environment]::GetFolderPath` ⇒ `"caminho absoluto fora da pasta do app"`.
   - Sem flag ⇒ `Confirm` (todo script exige confirmação).
4. Veredito: linha literal `let verdict = flags.iter().map(|f| f.verdict).max().unwrap_or(Verdict::Confirm);`. `flags` ordenados por linha.
5. `variables`: nomes `$x` em minúsculas sem `$` (`$env:X` vira `"env:x"`). `env_var_names`: primeiro argumento literal (posicional ou `-Name`) de `Get-EnvVar`/`Set-EnvVar`, caixa preservada.
6. Nenhum texto do relatório contém "segur".
7. Corpus real (ARMADILHAS, recusa nova): acrescente ao `oracle_2c` uma tabela "veredito do portão" (manifestos S-hook por veredito, com `supported = &[]` e com `supported = SCOOP_FUNCTIONS`), rode sobre o `catalog.db` real (só leitura, nada executado) e cole no relatório.

**Files:**
- Create: `src/install/script_scan.rs` (**o orquestrador cria com uma linha `//!` e a linha `pub mod`**)
- Modify: `examples/oracle_2c.rs`
- Test: `tests/it/script_scan.rs` (**já criado pelo orquestrador**)

Testes (`supported = &["ensure"]` salvo indicação):
1. `plain_script_is_confirm`: `Remove-Item "$dir\x" -Force` ⇒ `Confirm`, sem flags.
2. `runas_refused`: `Start-Process "$dir\x.exe" -Verb RunAs` ⇒ `Refuse`, `Elevation`.
3. `sudo_refused`: `sudo Remove-Item x` ⇒ `Elevation`.
4. `hklm_refused`: `Set-ItemProperty 'HKLM:\SOFTWARE\X' -Name a -Value 1` ⇒ `Hklm`.
5. `hklm_in_comment_ignored`: `# HKLM:\x` + `Remove-Item x` ⇒ `Confirm`.
6. `hklm_in_here_string_ignored`: `$t = @'` / `HKLM:\x` / `'@` / `Set-Content "$dir\a.reg" $t` ⇒ nenhum `Hklm`.
7. `global_switch_refused`: `Add-Path -Path "$dir\bin" -Global`.
8. `program_files_write_refused`: `Copy-Item "$dir\x" "$env:ProgramFiles\X"`.
9. `unknown_scoop_function_refused_with_name`: `Expand-DarkArchive "$dir\a.exe" "$dir\x"` ⇒ `detail == "função do Scoop ainda não suportada: Expand-DarkArchive"`; `APPDIR foo` ⇒ detalhe com `appdir`.
10. `supported_scoop_function_ok`: o mesmo do 9 com `supported` contendo `"Expand-DarkArchive"` ⇒ sem esse flag.
11. `scoop_command_refused`: `scoop install sudo`.
12. `msiexec_refused`: `Start-Process msiexec -ArgumentList '/i x.msi'` e `msiexec /i x` ⇒ `Msiexec` nos dois.
13. `iex_strong`: `iex (Get-Content x -Raw)` ⇒ `ConfirmStrong`.
14. `call_operator_with_variable_strong`: `& $exe /S`.
15. `download_strong`: `Invoke-WebRequest https://x -OutFile "$dir\a"`.
16. `reg_import_strong`: `reg import "$bucketsdir\extras\scripts\vscode\install-context.reg"`.
17. `hkcu_strong`: `New-ItemProperty -Path 'HKCU:\Software\X' -Name a -Value 1`.
18. `task_and_service_strong`: `schtasks /create ...` e `Stop-Service x` ⇒ `ScheduledTaskOrService` nos dois.
19. `absolute_path_strong`: `Copy-Item "$dir\a" 'D:\Tools\a'`.
20. `worst_verdict_wins`: `iex x` + `Set-ItemProperty 'HKLM:\X' a 1` ⇒ `Refuse`, os dois flags presentes.
21. `env_var_names_literal`: `Get-EnvVar -Name 'JAVA_HOME'; Set-EnvVar 'X' 'v'` ⇒ `{"JAVA_HOME", "X"}`.
22. `scan_manifest_reports_per_hook`: `pre_install` e `uninstaller.script` ⇒ `[(PreInstall, ..), (UninstallerScript, ..)]`.
23. `no_text_says_seguro`: para os scripts dos testes 1–20, nenhum `detail` contém `"segur"`.
24. `scoop_functions_table_matches_oracle_count`: `SCOOP_FUNCTIONS.len() == 262` e contém `"Expand-7zipArchive"`, `"ensure"`, `"Expand-ZstdArchive"`.

#### Checks beyond the steps

1. Mutação: âncora `let text = strip_here_strings(&strip_comments(script));` → `let text = strip_comments(script);` ⇒ teste 6 **PEGA**.
2. Mutação: a mesma âncora → `let text = strip_here_strings(script);` ⇒ teste 5 **PEGA**.
3. Mutação: âncora `!supported.iter().any(|s| s.eq_ignore_ascii_case(name))` → `true` ⇒ teste 10 **PEGA**.
4. Mutação: âncora `.max().unwrap_or(Verdict::Confirm)` → `.min().unwrap_or(Verdict::Confirm)` ⇒ teste 20 **PEGA**.
5. Saída do `cargo run --release --example oracle_2c -- <catalog.db real>` (tabela de vereditos) colada no relatório.
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): static gate for manifest scripts (script_scan)`

---

### Task 151: `RunInstaller`/`RunUninstaller` sem `.ps1` (S-file)

**Modelo:** sonnet (revisor opus). **Lane L1** (onda 4, depois da 148). **Compartilhado com a 2d:** `src/install/ops.rs`, `src/install/prod_win.rs`.

**Pré-requisitos (confira com `grep`; faltando ⇒ `BLOCKED`):** `pub struct Runners` com `disabled()` em `src/install/executor.rs`; `pub fn installed_manifest` em `src/install/state.rs`; `pub fn run_spec` em `src/bridge/spawn_win.rs`.

#### Where this fits

- `src/install/executor.rs`: `pub type ToolRun = fn(&Path, &[String], &Path) -> io::Result<(i32, Vec<String>)>`, `pub struct Runners { pub innounp: ToolRun }` + `Runners::disabled()`, `pub struct ExecCtx` (:75, campo `runners`), `pub enum ExecError` (:137), `pub async fn execute_install_with` (:487), `pub fn execute_uninstall` (:1407), worker bloqueante `AwaitWorker` (:1111).
- `src/install/planner.rs`: `fn refuse_2c` (um `if` por chave; os de `installer` e `uninstaller` dizem `"usa installer (PowerShell/instalador próprio): suportado na fase 2c"`); `Step::Promote` empurrado depois das extrações.
- `src/install/state.rs`: `pub fn installed_manifest(layout, app, version, expected_sha256) -> Result<Option<String>, StateError>`; `StateDb::get(app)` devolve o `AppRecord` com `manifest_sha256` e `arch`.
- `src/install/ops.rs:269` `fn exec_ctx`; `src/install/prod_win.rs:56` `build_ctx`.
- Scoop `lib/install.ps1` `Invoke-Installer`: programa = `"$Path\$(coalesce $installer.file $Name[0])"` (padrão: o primeiro arquivo baixado); fora do app ⇒ aborta; inexistente ⇒ aborta; substitui **só** `$dir`, `$global` (switch ⇒ `False`) e `$version` (`substitute`, `String.Replace` com caixa); `.ps1` roda no próprio PowerShell (fora desta tarefa); o resto via `Invoke-ExternalCommand`; falha ⇒ aborta; apaga o instalador se não houver `keep` (só no ramo não `.ps1`); depois roda `installer.script`.
- Scoop `lib/core.ps1` `Invoke-ExternalCommand`: tira as aspas `"` e divide cada argumento com `'(?<=(?<![:\w])[/-]\w+) | (?=[/-])'`; `UseShellExecute = $false` (por isso `.msi` não roda).

#### What binds this task

1. `Runners` ganha `pub installer: Option<ToolRun>`; `disabled()` ⇒ `None`. `None` = sem consentimento. `ops::exec_ctx` põe `installer: None` sempre nesta tarefa (nenhum caminho de produção concede consentimento ainda); `build_ctx` põe `Some(installer_win::run)` em `OpsCtx.runners` (fora do `WINGETRS_TEST_ENV_KEY`).
2. `ExecError::NeedsScriptConsent { app: String, what: String }` com `#[error("{app}: roda código de terceiros ({what}); é preciso confirmar com --allow-scripts")]`. Plano com `Step::RunInstaller` e `installer: None` ⇒ esse erro **antes de qualquer passo** (nenhum download), com a linha literal `if needs_consent && ctx.runners.installer.is_none() {`.
3. Planejador: o `if` do `installer` em `refuse_2c` passa a recusar **só** `file` terminando em `.ps1` ou `script` presente (texto antigo mantido); idem `uninstaller`. `Step::RunInstaller { file: String, args: Vec<String>, keep: bool }` logo depois de `Step::Promote` e antes do primeiro `Step::Persist`. `file` = `installer.file` ou `sources[0].file_name`; `.msi` ⇒ `PlanError::Unsupported("installer.file .msi não roda direto (o Scoop também não executa)".into())`.
4. `src/install/installer_args.rs` (puro): `const INSTALLER_VARS: [&str; 3] = ["$dir", "$global", "$version"];` (linha literal); `pub fn installer_args(args: &[String], dir: &Path, version: &str) -> Result<Vec<String>, PlanError>`: troca `$dir` pelo caminho da versão, `$global` por `False`, `$version` pela versão (com caixa, uma passada por variável); caractere de controle ⇒ `UnsafeArgs`; depois tira `"` e divide pela regex do Scoop (`fancy_regex`), descartando vazios.
5. Executor `RunInstaller`: programa = `version_dir.join(file)`; precisa existir (senão `"instalador {file} não existe"`) e ficar dentro da pasta da versão depois de normalizado; roda `installer` no worker bloqueante com pasta = `version_dir`; código ≠ 0 ⇒ passo falha com `"o instalador saiu com código {c}"` + o aviso D5 `"; o instalador pode ter alterado arquivos fora da pasta do app, e isso não foi desfeito"` (rollback normal). Código 0 ⇒ se não `keep` (linha literal `if !keep {`) apaga o arquivo; falha ao apagar é só `tracing::warn!`. Fase `Phase::Placing`, rótulo `"rodar instalador {file}"`, `step_kind` `"installer"`.
6. `src/install/installer_win.rs`: `pub fn run(exe, args, dir)` = `spawn_win::run_spec` com `utf8_lossy`, sem ambiente extra, `PIPE_GRACE`. `.cmd`/`.bat`: o `std` recusa argumentos perigosos ⇒ o passo falha com a mensagem do erro.
7. Remoção: `execute_uninstall` lê `state.installed_manifest(..)` com o `manifest_sha256` e o `arch` do registro; `None` ⇒ remove como antes. Com `uninstaller` S-file: `installer: None` ⇒ `NeedsScriptConsent` antes de remover qualquer coisa; senão roda o desinstalador (`$dir` = pasta da versão instalada) **antes** de remover artefatos; código ≠ 0 ⇒ `ExecError::Failed` e nada removido.

**Files:**
- Create: `src/install/installer_args.rs`, `src/install/installer_win.rs` (**o orquestrador cria com `//!` e `pub mod`**)
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/ops.rs`, `src/install/prod_win.rs`
- Test: `tests/it/install_file_installer.rs` (**já criado pelo orquestrador**; `World`/`FailAt` copiados de `tests/it/install_executor.rs`; `ToolRun` falsos com contador `static` próprio de cada teste)

Testes:
1. `args_substitute_only_dir_global_version`: `["/D=$dir", "$global", "$version", "$persist_dir", "$app"]` ⇒ `["/D=<vdir>", "False", "1.0", "$persist_dir", "$app"]`.
2. `args_split_like_scoop`: `["/S /D=C:\\x y"]` ⇒ `["/S", "/D=C:\\x y"]`; `["\"q\""]` ⇒ `["q"]`; `["C:/Program Files/x"]` ⇒ inalterado.
3. `file_defaults_to_download_name`: sem `file`, fonte `setup.exe` ⇒ `RunInstaller { file: "setup.exe", .. }`.
4. `msi_file_refused`.
5. `ps1_or_script_still_refused_with_old_text`: `file: "x.ps1"` e `{"script":"a"}` ⇒ `"não suportado: usa installer (PowerShell/instalador próprio): suportado na fase 2c"`.
6. `runs_with_consent_and_deletes_file`: falso sai 0 ⇒ arquivo apagado; com `keep: true` ⇒ mantido; o falso viu `cwd == vdir` e os argumentos do item 1.
7. `exit_3_rolls_back_with_warning`: mensagem contém `"código 3"` e `"não foi desfeito"`; `apps\<app>` não existe.
8. `no_consent_fails_before_anything`: `installer: None` ⇒ `NeedsScriptConsent`; o servidor falso recebeu 0 pedidos; contador do falso == 0.
9. `installer_outside_version_dir_refused`: `file: "..\\x.exe"` ⇒ erro de manifesto/plano.
10. `uninstaller_runs_before_removal`: falso do desinstalador sai 1 ⇒ app e shims continuam; sai 0 ⇒ removido, e o falso viu `$dir` = pasta da versão.

#### Checks beyond the steps

1. Mutação em `src/install/installer_args.rs`: âncora `["$dir", "$global", "$version"]` → `["$dir", "$global", "$versionX"]` ⇒ teste 1 **PEGA**.
2. Mutação em `src/install/executor.rs`: âncora `if needs_consent && ctx.runners.installer.is_none() {` → `if false {` ⇒ teste 8 **PEGA** (o servidor recebe pedidos).
3. Mutação em `src/install/executor.rs`: âncora `if !keep {` → `if true {` ⇒ teste 6 (`keep`) **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): run installer/uninstaller file and args (S-file), gated by consent`

---

### Task 152: Prelúdio próprio, grupo 1 (mensagens, caminhos, configuração)

**Modelo:** sonnet (revisor opus). **Lane L5** (onda 2, PARALELO).

#### Where this fits

- Decisão do usuário (spec 2c, D1 = B): nada da lib do Scoop é baixado nem carregado; o runner dot-source um prelúdio do wingetrs que implementa só as funções que os manifestos chamam; função do Scoop fora do prelúdio é recusada pelo portão com o nome dela.
- Oráculo 2026-10-18 (manifestos do catálogo que usam): `ensure` 76, `info` 53, `error` 43, `is_admin` 34, `warn` 23, `get_config` 18, `appdir` 10, `versiondir` 8, `Split-PathLikeEnvVar` 7, `abort` 5, `currentdir` 4, `installed` 4, `movedir` 3, `ConvertToPrettyJson` 2, `Get-CommandPath` 2, `Get-HelperPath` 2, `Select-CurrentVersion` 2, `shimdir` 2, `Find-BucketDirectory` 1; nas sondas (buckets locais) também `appsdir`, `persistdir`, `friendly_path`, `Get-AbsolutePath`.
- Scoop (e6aa3b3, só leitura em `~/scoop/apps/scoop/current/lib`; porte com uma linha de comentário citando `arquivo:linha`): `core.ps1:311` `abort` (`Write-Host $msg; exit $exit_code`), `:312` `error` (`"ERROR $msg"`), `:313` `warn` (`"WARN  $msg"`, dois espaços), `:314` `info` (`"INFO  $msg"`, dois espaços), `:304` `is_admin`, `:622` `ensure`, `:118` `get_config`, `:367-385` `appsdir`/`shimdir`/`appdir`/`versiondir`/`currentdir`/`persistdir`, `:369` `modulesdir` (**só função**; não existe `$modulesdir`), `:407` `installed`, `:476` `Get-HelperPath`, `:519` `Get-CommandPath`, `:628` `Get-AbsolutePath`, `:663` `friendly_path`, `:864` `movedir`; `versions.ps1:31` `Select-CurrentVersion`; `buckets.ps1:3` `Find-BucketDirectory`; `system.ps1:76` `Split-PathLikeEnvVar`; `json.ps1:6` `ConvertToPrettyJson`.
- Raiz do wingetrs (`src/install/layout.rs`): `apps\<app>\<versão>`, `apps\<app>\current`, `persist\<app>`, `shims`, `modules`.

#### What binds this task

1. `src/install/prelude.ps1` (texto **só ASCII**) e `src/install/prelude.rs`: `pub const PRELUDE: &str = include_str!("prelude.ps1");`, `pub const PRELUDE_FUNCTIONS: &[&str]` (nomes canônicos do Scoop implementados até aqui) e `pub const CONTRACT_VARS: &[&str]` = `["__wr_root", "__wr_effects", "__wr_env_seed", "__wr_helpers", "__wr_shortcut_root"]` — variáveis que o runner define **antes** de dot-source o prelúdio (documentadas no cabeçalho do `.ps1`). O prelúdio só define funções: nenhum efeito ao ser carregado.
2. Funções deste grupo: `info`, `warn`, `error` (textos exatos do Scoop), `abort($msg, [int]$exit_code = 1)` com a linha literal `throw "abort: $msg"`, `is_admin` com a linha literal `function is_admin { return $false }` (o wingetrs recusa rodar elevado), `ensure`, `get_config`, `appsdir`, `appdir`, `versiondir`, `currentdir`, `persistdir`, `shimdir`, `modulesdir`, `installed`, `Select-CurrentVersion`, `Find-BucketDirectory`, `Get-HelperPath`, `Get-CommandPath`, `Get-AbsolutePath`, `friendly_path`, `movedir`, `Split-PathLikeEnvVar`, `ConvertToPrettyJson`. Caminhos sob `$__wr_root`; argumento `$global` verdadeiro ⇒ `throw "global não suportado pelo wingetrs"`. `Get-HelperPath` lê `$__wr_helpers[<nome>]` (`$null` se ausente). `Find-BucketDirectory` usa `$bucketsdir` (definido pelo runner) com a semântica do original (inclusive `-Root`).
3. `get_config`: antes de implementar, liste por consulta **só leitura** ao `catalog.db` os nomes literais passados a `get_config` nos manifestos (teste `#[ignore]` ou trecho temporário do `oracle_2c`; cole a lista no relatório). Tabela fixa para esses nomes (ex.: `NO_JUNCTION` ⇒ `$false`, `USE_LESSMSI` ⇒ `$true`, `USE_EXTERNAL_7ZIP` ⇒ `$false`); nome desconhecido ⇒ o `$default` recebido.
4. Testes rodam o PowerShell de verdade **só com scripts sintéticos numa raiz temporária** (nada de manifesto real, rede ou registro): `<pwsh> -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <tmp>\t.ps1`, onde `t.ps1` define as `CONTRACT_VARS`, dot-source `<tmp>\prelude.ps1` (gravado de `PRELUDE`) e roda o corpo do teste; `<pwsh>` = `wingetrs::bridge::find_pwsh(std::env::var_os("PATH").as_deref())`.

**Files:**
- Create: `src/install/prelude.ps1`, `src/install/prelude.rs` (**o orquestrador cria o `.rs` com `//!` e a linha `pub mod prelude;`**)
- Test: `tests/it/prelude_core.rs` (**já criado pelo orquestrador**)

Testes:
1. `prelude_functions_all_defined`: depois do dot-source, cada nome de uma **lista literal independente no teste** (as 24 funções do item 2) responde a `Get-Command <n> -CommandType Function`; e `PRELUDE_FUNCTIONS` como conjunto == essa lista.
2. `prelude_is_ascii_and_loads_without_side_effects`: `PRELUDE.is_ascii()`; carregar numa raiz vazia não cria nada nela (listagem igual antes e depois) e não escreve no stdout.
3. `messages_match_scoop`: `info x`, `warn x`, `error x` ⇒ linhas `"INFO  x"`, `"WARN  x"`, `"ERROR x"`.
4. `abort_throws_and_exits_nonzero`: corpo `abort 'falhou'; 'depois'` ⇒ código ≠ 0, saída com `"abort: falhou"`, sem `"depois"`.
5. `is_admin_false`.
6. `ensure_creates_and_returns_path`.
7. `paths_under_root`: `appdir foo`, `versiondir foo 1.0`, `currentdir foo`, `persistdir foo`, `shimdir`, `appsdir`, `modulesdir` ⇒ caminhos sob a raiz; `appdir foo $true` ⇒ erro com `"global não suportado"`.
8. `modulesdir_variable_not_defined`: `Test-Path variable:modulesdir` ⇒ `False`.
9. `get_config_table_and_default`: uma chave da tabela ⇒ o valor fixo; `get_config 'ZZZ_X' 'pad'` ⇒ `pad`.
10. `installed_and_select_current_version`: junção `apps\foo\current` → `apps\foo\1.2` ⇒ `installed foo` = `True`, `Select-CurrentVersion foo` = `1.2`; app ausente ⇒ `False` / vazio.
11. `bucket_helper_command_paths`: `Find-BucketDirectory main` (com e sem `-Root`), `Get-HelperPath 7zip` (presente e ausente), `Get-CommandPath` de um `.cmd` em `shims`.
12. `misc_utils`: `movedir`, `ConvertToPrettyJson`, `friendly_path`, `Get-AbsolutePath`, `Split-PathLikeEnvVar` com entradas e saídas do original.
13. `loads_on_windows_powershell_5_1`: o teste 1 com `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` (caminho absoluto).

#### Checks beyond the steps

1. Mutação em `src/install/prelude.ps1`: âncora `function is_admin { return $false }` → `function is_admin { return $true }` ⇒ teste 5 **PEGA**.
2. Mutação em `src/install/prelude.ps1`: âncora `throw "abort: $msg"` → `Write-Host "abort: $msg"` ⇒ teste 4 **PEGA**.
3. Mutação em `src/install/prelude.ps1`: âncora `function modulesdir` → `$modulesdir = 1; function modulesdir` ⇒ teste 8 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): own PowerShell prelude, group 1 (messages, paths, config)`

---

### Task 153: Prelúdio, grupo 2 (funções com efeito global) e `effects.jsonl`

**Modelo:** opus (revisor opus). **Lane L5** (onda 3, depois da 152, PARALELO com a 147).

#### Where this fits

- `src/install/prelude.ps1` / `src/install/prelude.rs`: `PRELUDE`, `PRELUDE_FUNCTIONS`, `CONTRACT_VARS` (`__wr_root`, `__wr_effects`, `__wr_env_seed`, `__wr_helpers`, `__wr_shortcut_root`), definidas pelo runner antes do dot-source; prelúdio só ASCII.
- Oráculo: `Set-EnvVar` 11, `Get-EnvVar` 10, `Add-Path` 9, `Remove-Path` 9, `shortcut_folder` 8, `rm_shim` 5, `shim` 5, `startmenu_shortcut` 5; sondas: `add_first_in_path` 2, `remove_from_path` 2, `env` 0.
- Scoop (porte com citação): `system.ps1:30` `Get-EnvVar`, `:46` `Set-EnvVar` (grava `HKCU\Environment` + broadcast), `:96` `Add-Path` (`-Path [string[]]`, `-TargetEnvVar` padrão `PATH`, insere na frente, também mexe no `$env:PATH` da sessão), `:122` `Remove-Path`, `:153` `env`, `:168` `add_first_in_path`, `:173` `remove_from_path`; `core.ps1:949` `shim` (chama `Add-Path` da pasta de shims em :952), `install.ps1:195` `rm_shim`; `shortcuts.ps1:22` `shortcut_folder` (cria a pasta), `:31` `startmenu_shortcut` (`.lnk` via `WScript.Shell`).
- Spec (c): essas funções só gravam uma linha em `effects.jsonl`; o executor aplica os efeitos pela via da 2a com `Undo`.

#### What binds this task

1. Funções interceptadas (acrescente a `PRELUDE_FUNCTIONS`): `Set-EnvVar`, `Get-EnvVar`, `Add-Path`, `Remove-Path`, `add_first_in_path`, `remove_from_path`, `env`, `shim`, `rm_shim`, `shortcut_folder`, `startmenu_shortcut`. Nenhuma toca o registro, o PATH real nem cria `.lnk`. `-Global` (ou `$global` verdadeiro) ⇒ `throw "global não suportado pelo wingetrs"`.
2. Cada efeito vira uma linha JSON (UTF-8 sem BOM, `\n`) acrescentada a `$__wr_effects` por uma função interna `__wr_effect` (hashtable ⇒ `ConvertTo-Json -Compress`):
   `{"op":"set_env","name":..,"value":..|null}`, `{"op":"add_path","var":"PATH","entries":[..],"prepend":true}`, `{"op":"remove_path","var":..,"entries":[..]}`, `{"op":"shim","path":..,"name":..,"args":..|null}`, `{"op":"rm_shim","name":..}`, `{"op":"shortcut","target":..,"name":..,"args":..|null,"icon":..|null}`.
3. `Get-EnvVar`: lê do JSON em `$__wr_env_seed` (objeto nome ⇒ valor ou `null`, sem caixa), sobreposto pelas escritas da própria sessão (`$script:__wr_env_overlay`), com a linha literal `if ($script:__wr_env_overlay.ContainsKey($Name))`; nome ausente das duas fontes ⇒ `throw "Get-EnvVar: a variável '$Name' não foi pré-carregada pelo wingetrs"`. `Set-EnvVar` grava efeito e sobreposição. `Add-Path`/`Remove-Path` também ajustam a variável da sessão (`$env:PATH` ou a `-TargetEnvVar`), como o original.
4. `shim` grava só o efeito `shim` (**não** chama `Add-Path`; o executor garante a pasta de shims no PATH). `shortcut_folder` devolve `$__wr_shortcut_root` (criando a pasta se faltar, como o original); `startmenu_shortcut` grava só o efeito.
5. `src/install/script_effects.rs` (puro):
   ```rust
   pub enum ScriptEffect { SetEnv { name: String, value: Option<String> }, AddPath { var: String, entries: Vec<String>, prepend: bool },
       RemovePath { var: String, entries: Vec<String> }, Shim { path: String, name: String, args: Option<String> },
       RmShim { name: String }, Shortcut { target: String, name: String, args: Option<String>, icon: Option<String> } }
   pub enum EffectsError { Invalid(usize), UnknownOp(usize), TooMany, TooLong(usize) }  // thiserror, mensagens em português
   pub fn parse_effects(text: &str) -> Result<Vec<ScriptEffect>, EffectsError>
   ```
   Tetos checados **antes** de decodificar: 1000 linhas e 64 KiB por linha. BOM inicial aceito e ignorado; linhas vazias ignoradas; `op` desconhecido ⇒ `UnknownOp(nº da linha, base 1)` por um `match` explícito no texto do `op`, cujo braço final é a linha literal `other => return Err(EffectsError::UnknownOp(n)),`.

**Files:**
- Create: `src/install/script_effects.rs` (**o orquestrador cria com `//!` e `pub mod`**)
- Modify: `src/install/prelude.ps1`, `src/install/prelude.rs`
- Test: `tests/it/prelude_effects.rs` (**já criado pelo orquestrador**; mesmo arnês de PowerShell da `tests/it/prelude_core.rs` — copie, não importe; scripts sintéticos numa raiz temporária)

Testes:
1. `set_env_records_and_overlay`: `Set-EnvVar 'A' 'v'; Get-EnvVar 'A'` ⇒ imprime `v`; efeitos == `[SetEnv{A, Some(v)}]` (via `parse_effects`).
2. `set_env_null_removes`: `Set-EnvVar 'A' $null` ⇒ `SetEnv{A, None}`.
3. `get_env_from_seed_and_unknown_throws`: semente `{"JAVA_HOME":"C:\\J"}` ⇒ `C:\J`; `Get-EnvVar 'ZZ'` ⇒ código ≠ 0 com `"não foi pré-carregada"`.
4. `global_throws_for_each`: `Set-EnvVar a b -Global`, `Get-EnvVar a -Global`, `Add-Path x -Global`, `Remove-Path x -Global` ⇒ cada um código ≠ 0 com `"global não suportado"`.
5. `add_path_records_and_updates_session`: `Add-Path -Path "$dir\bin"` ⇒ `AddPath{var:"PATH", entries:[<dir>\bin], prepend:true}` e `$env:PATH` começa com a entrada.
6. `add_path_target_env_var`: `-TargetEnvVar 'PSModulePath'` ⇒ `var == "PSModulePath"`.
7. `remove_path_records`.
8. `deprecated_aliases`: `add_first_in_path $dir $false` ⇒ `AddPath`; `remove_from_path $dir $false` ⇒ `RemovePath`; `env 'X' $false 'v'` ⇒ `SetEnv{X, Some(v)}`.
9. `shim_records_without_touching_path`: `shim "$dir\x.exe" $false 'x'` ⇒ `Shim{..}` e `$env:PATH` igual ao de antes.
10. `rm_shim_records`.
11. `shortcuts`: `shortcut_folder $false` == raiz de atalhos do teste; `startmenu_shortcut ...` ⇒ `Shortcut{..}` e nenhum `.lnk` criado.
12. `parser_limits_and_errors`: 3ª linha com `op` `"zz"` ⇒ `UnknownOp(3)`; 2ª linha inválida ⇒ `Invalid(2)`; 1001 linhas ⇒ `TooMany`; linha com 65 537 bytes ⇒ `TooLong(1)`; BOM + linha válida ⇒ `Ok` com 1 item; `""` ⇒ `Ok([])`.
13. `prelude_still_ascii`: `PRELUDE.is_ascii()`.

#### Checks beyond the steps

1. Mutação em `src/install/prelude.ps1`: âncora `if ($script:__wr_env_overlay.ContainsKey($Name))` → `if ($false)` ⇒ teste 1 **PEGA**.
2. Mutação em `src/install/prelude.ps1`: âncora `__wr_effect @{ op = 'shim'` → `Add-Path -Path (Split-Path $path); __wr_effect @{ op = 'shim'` ⇒ teste 9 **PEGA**.
3. Mutação em `src/install/script_effects.rs`: âncora `other => return Err(EffectsError::UnknownOp(n)),` → `_ => continue,` ⇒ teste 12 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): prelude group 2 (intercepted global effects) and effects.jsonl parser`

---

### Task 154: Prelúdio, grupo 3 (extração por auxiliar, programas externos, persist)

**Modelo:** sonnet (revisor opus). **Lane L5** (onda 4).

**Pré-requisitos (confira com `grep`; faltando ⇒ `BLOCKED`):** `pub fn inno_args` em `src/install/inno.rs`; `__wr_effect` e `CONTRACT_VARS` em `src/install/prelude.*`.

#### Where this fits

- `src/install/prelude.ps1`/`prelude.rs` (`PRELUDE_FUNCTIONS`, `CONTRACT_VARS`; `$__wr_helpers` = hashtable `7zip`/`lessmsi`/`innounp`/`dark` ⇒ caminho absoluto do exe do auxiliar).
- `src/install/inno.rs`: `pub fn inno_args(installer, dest, extract_dir) -> Result<Vec<String>, ExtractError>` (regra do Scoop para `-c{app}`).
- Oráculo: `Expand-7ZipArchive` 236 (a função mais usada), `Invoke-ExternalCommand` 41, `Expand-MsiArchive` 29, `Expand-DarkArchive` 15, `Expand-InnoArchive` 14, `Expand-ZipArchive` 2, `persist_data` 2, `persist_permission` 2, `unlink_persist_data` 2.
- Scoop (porte com citação): `decompress.ps1:63` `Expand-7zipArchive` (`-Overwrite`, `-ExtractDir`, `-Switches`, `-Removal`), `:169` `Expand-MsiArchive` (**use só o ramo `USE_LESSMSI`**; o ramo `msiexec.exe` de :195 não é portado), `:227` `Expand-InnoArchive`, `:267` `Expand-ZipArchive`, `:303` `Expand-DarkArchive`; `core.ps1:713` `Invoke-ExternalCommand` (`-ArgumentList`, `-RunAs`, `-Quiet`, `-Activity`, `-ContinueExitCodes`, `-LogPath`); `install.ps1:444` `persist_data`, `:496` `unlink_persist_data`, `:521` `persist_permission` (só age com `$global`).
- Sonda D4: o innounp sai 0 com zero arquivos quando o filtro não casa.

#### What binds this task

1. Funções (acrescente a `PRELUDE_FUNCTIONS`): `Expand-7zipArchive`, `Expand-MsiArchive`, `Expand-InnoArchive`, `Expand-DarkArchive`, `Expand-ZipArchive`, `Invoke-ExternalCommand`, `persist_data`, `unlink_persist_data`, `persist_permission`.
2. `Expand-*` chamam o auxiliar por `$__wr_helpers[...]` (`7zip` ⇒ `7z.exe`, `lessmsi` ⇒ `lessmsi.exe`, `innounp` ⇒ `innounp.exe`, `dark` ⇒ `dark.exe`); ausente ou arquivo inexistente ⇒ `throw "auxiliar '<nome>' não instalado"`. Argumentos iguais aos do original; código ≠ 0 ⇒ `throw` com o código. `Expand-InnoArchive` monta os mesmos argumentos de `inno_args` e, com código 0, conta os arquivos do destino: linha literal `if ($count -eq 0) {` ⇒ `throw "innounp: nenhum arquivo extraído"`. `Expand-ZipArchive` usa `System.IO.Compression.ZipFile` como o original.
3. O texto do prelúdio não contém o token `msiexec` (teste).
4. `Invoke-ExternalCommand`: porte do original, com a linha literal `if ($RunAs) { throw "elevação não suportada pelo wingetrs" }`; o teste de código usa a linha literal `ContainsKey($code)` sobre `-ContinueExitCodes`; devolve `$true`/`$false` como o original.
5. `persist_data`/`unlink_persist_data`: porte (junção/hardlink só sob `persist\<app>` e `$dir`); `persist_permission`: sem efeito (o wingetrs nunca roda com `$global`).

**Files:**
- Modify: `src/install/prelude.ps1`, `src/install/prelude.rs`
- Test: `tests/it/prelude_extract.rs` (**já criado pelo orquestrador**; arnês de PowerShell copiado; auxiliares falsos são `.cmd` na raiz temporária que gravam `%*` num arquivo e criam arquivos no destino; nada de auxiliar real)

Testes:
1. `expand_7zip_calls_helper_with_scoop_args`: argumentos gravados == os do original para `Overwrite` padrão.
2. `expand_7zip_extract_dir_moves_subdir`: o falso cria `<o>\sub\a.txt`; `-ExtractDir 'sub'` ⇒ `<dest>\a.txt`.
3. `expand_7zip_removal_deletes_archive`.
4. `expand_7zip_nonzero_exit_throws`.
5. `msi_uses_lessmsi_never_msiexec`: o falso do lessmsi foi chamado; `PRELUDE.to_lowercase()` não contém `"msiexec"`.
6. `inno_args_equal_rust_and_zero_files_throws`: argumentos gravados == `inno_args(..)` para `None`, `Some("sub")` e `Some("{tmp}")` (equivalência entre os dois caminhos); falso sai 0 sem criar nada ⇒ erro `"nenhum arquivo extraído"`.
7. `dark_args`.
8. `zip_native`: zip pequeno montado no teste (crate `zip`) ⇒ arquivos no destino.
9. `invoke_external_command_codes`: `.cmd` que sai 0 ⇒ `True`; sai 3 ⇒ `False`; sai 3 com `-ContinueExitCodes @{3='ok'}` ⇒ `True`.
10. `invoke_external_command_runas_throws`.
11. `missing_helper_throws_named`: sem `7zip` em `$__wr_helpers` ⇒ `"auxiliar '7zip' não instalado"`.
12. `persist_data_moves_and_links`: `$manifest.persist = 'data'`, `$dir\data` existe, `persist\app` vazio ⇒ movido e ligado.

#### Checks beyond the steps

1. Mutação: âncora `if ($count -eq 0) {` → `if ($false) {` ⇒ teste 6 **PEGA**.
2. Mutação: âncora `if ($RunAs) { throw "elevação não suportada pelo wingetrs" }` → `if ($false) { throw "x" }` ⇒ teste 10 **PEGA**.
3. Mutação: âncora `ContainsKey($code)` → `ContainsKey(-12345)` ⇒ teste 9 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): prelude group 3 (helper extraction, external commands, persist)`

---

### Task 155: Gerador do runner (puro) e tabela variável × hook

**Modelo:** opus (revisor opus). **Lane L6** (onda 2, PARALELO).

#### Where this fits

- `src/install/manifest.rs`: `HookKind` (`PreInstall`, `InstallerScript`, `PostInstall`, `PreUninstall`, `UninstallerScript`, `PostUninstall`), `Arch::key()` (`"64bit"`, `"32bit"`, `"arm64"`, :17).
- Sonda D3 (tabela variável × hook, Scoop e6aa3b3): `$cmd` vem do `exec` por escopo dinâmico (`install`, `update`, `uninstall`); **no upgrade os hooks de instalação da versão nova também veem `'update'`**; `$dir` = pasta da versão em `pre_install`/`installer.script`, **junção `current` em `post_install`** com `$original_dir` = pasta da versão; nos hooks de desinstalação `$dir` = pasta da versão (nunca `current`), sem `$original_dir` nem `$fname`; **nos hooks de desinstalação durante o upgrade `$version` é a versão NOVA, `$manifest` o manifesto ANTIGO** e há `$old_version`; `$purge` só no uninstall; `installer.script` tem também `$Path` (= `$dir`), `$Name` (= `$fname`), `$AppName`, `$Uninstall` (`$false`); `uninstaller.script` tem `$Path`, `$AppName`, `$Uninstall = $true` e nenhum `$Name`; `$global` é `$false`; `$bucketsdir` = `<raiz>\buckets`; `$scoopdir`, `$globaldir`, `$cachedir` globais.
- `Invoke-HookScript` (`install.ps1:165`): `Invoke-Command ([scriptblock]::Create($script))`, escopo filho, sem `$ErrorActionPreference` próprio (o Scoop não define `Stop`: nenhuma ocorrência em `bin/scoop.ps1`, `lib/core.ps1`, `lib/install.ps1`).
- Sonda D7: o PowerShell trata `‘ ’ ‚ ‛` (U+2018–U+201B) como aspas simples.

#### What binds this task

1. `src/install/runner.rs` (puro):
   ```rust
   pub enum ScriptOp { Install, Update, Uninstall }
   pub struct ScriptContext { pub op: ScriptOp, pub app: String, pub version: String, pub old_version: Option<String>,
       pub bucket: Option<String>, pub arch: Option<Arch>, pub root: PathBuf, pub version_dir: PathBuf, pub current: PathBuf,
       pub persist_dir: PathBuf, pub fnames: Vec<String>, pub purge: bool, pub manifest_path: PathBuf, pub bucketsdir: PathBuf }
   pub enum VarValue { Str(String), Bool(bool), StrArray(Vec<String>), JsonFile(PathBuf), Null }
   pub fn script_vars(kind: HookKind, cx: &ScriptContext) -> Vec<(String, VarValue)>
   pub fn ps_single_quoted(s: &str) -> String
   pub enum ScriptBody<'a> { Hook { script_path: &'a Path }, Ps1File { path: &'a Path, args: &'a [String] } }
   pub struct RunnerInput<'a> { pub contract: &'a [(String, VarValue)], pub vars: &'a [(String, VarValue)], pub prelude_path: &'a Path, pub body: ScriptBody<'a> }
   pub fn runner_ps1(input: &RunnerInput<'_>) -> Result<String, RunnerError>
   ```
2. `script_vars` segue a tabela acima; `$cmd` por `ScriptOp` com as linhas literais `ScriptOp::Install => "install",`, `ScriptOp::Update => "update",`, `ScriptOp::Uninstall => "uninstall",`; `$dir` com a linha literal `HookKind::PostInstall => cx.current.clone(),`; `$manifest` = `JsonFile(cx.manifest_path)` (o chamador passa o manifesto antigo nos hooks de desinstalação do upgrade); `$version` = `cx.version` sempre; variável marcada "—" na tabela **não** aparece no vetor.
3. `ps_single_quoted`: envolve em `'...'` e dobra `'` e cada um de `'\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}'` (linha literal com esse padrão).
4. `runner_ps1`, nesta ordem: (a) `[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)` e `$OutputEncoding = [Console]::OutputEncoding`; (b) não define `$ErrorActionPreference` (decisão R1); (c) atribuições das `contract` e depois das `vars` (`$nome = <literal>`; nome fora de `^[A-Za-z_][A-Za-z0-9_]*$` ⇒ `Err(RunnerError::BadName)`; `Str` ⇒ `ps_single_quoted`, `Bool` ⇒ `$true`/`$false`, `Null` ⇒ `$null`, `StrArray` ⇒ `@('a','b')`, `JsonFile` ⇒ `(Get-Content -LiteralPath '<p>' -Raw -Encoding UTF8 | ConvertFrom-Json)`); (d) `. '<prelude_path>'`; (e) `Hook`: `$__wr_hook = [System.IO.File]::ReadAllText('<script_path>')` e `try { Invoke-Command ([scriptblock]::Create($__wr_hook)) } catch { [Console]::Error.WriteLine($_.ToString()); exit 1 }`; `Ps1File`: `try { & '<path>' @(<args literais>) } catch { ...; exit 1 }`; (f) `exit 0`. O texto do hook nunca entra no runner.

**Files:**
- Create: `src/install/runner.rs` (**o orquestrador cria com `//!` e `pub mod`**)
- Test: `tests/it/script_runner.rs` (**já criado pelo orquestrador**)

Testes:
1. `quotes_ascii_and_typographic_escaped`: `"a'b\u{2018}c\u{2019}d\u{201A}e\u{201B}f"` ⇒ `"'a''b\u{2018}\u{2018}c\u{2019}\u{2019}d\u{201A}\u{201A}e\u{201B}\u{201B}f'"`.
2. `hook_never_interpolated`: o arquivo do hook contém `MARKER_ZZZ_9`; o runner não contém `MARKER_ZZZ_9` e contém o caminho do arquivo.
3. `order_vars_prelude_hook`: índice da última atribuição < índice de `. '<prelude>'` < índice de `ReadAllText`.
4. `vars_pre_install`: `$dir` = pasta da versão, `$original_dir` = pasta da versão, `$cmd` = `install`, `$fname` = `StrArray`.
5. `vars_post_install_dir_is_current`: `$dir` = `current`, `$original_dir` = pasta da versão.
6. `vars_update_install_hooks_cmd_update`: `op: Update`, `PreInstall` ⇒ `$cmd` = `update`.
7. `vars_update_uninstall_hooks`: `op: Update`, `PreUninstall`, `version: "2.0"`, `old_version: Some("1.0")`, `version_dir` = pasta da 1.0, `manifest_path` = manifesto antigo ⇒ `$version` = `2.0`, `$old_version` = `1.0`, `$dir` = pasta da 1.0, `$manifest` = `JsonFile(antigo)`, `$cmd` = `update`; sem `$original_dir`, `$fname`, `$purge`.
8. `vars_uninstall`: `$cmd` = `uninstall`, `$purge` presente; sem `$original_dir`, `$fname`.
9. `installer_and_uninstaller_script_extras`: `InstallerScript` ⇒ `$Path`, `$Name`, `$AppName`, `$Uninstall = $false`; `UninstallerScript` ⇒ `$Uninstall = $true` e sem `$Name`.
10. `bad_var_name_err`: nome `"a b"` ⇒ `Err(BadName)`.
11. `end_to_end_pwsh` (roda o PowerShell só com arquivos sintéticos numa raiz temporária; prelúdio = um `.ps1` vazio do teste): hook `"$cmd|$dir|$version|$($manifest.version)"` ⇒ linha exata; hook `throw 'x'` ⇒ código 1; hook `Remove-Item '<tmp>\nao-existe'` ⇒ código 0 (semântica do Scoop); valor com `’` vira a variável certa.

#### Checks beyond the steps

1. Mutação: âncora `HookKind::PostInstall => cx.current.clone(),` → `HookKind::PostInstall => cx.version_dir.clone(),` ⇒ teste 5 **PEGA**.
2. Mutação: âncora `'\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}'` → `'\u{2018}'` ⇒ teste 1 **PEGA** (no PowerShell, aspas simples dobradas).
3. Mutação: âncora `ScriptOp::Update => "update",` → `ScriptOp::Update => "install",` ⇒ testes 6 e 7 **PEGAM**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): pure runner.ps1 generator and variable-by-hook table`

---

### Task 156: `ScriptHost` no executor (hooks de instalação)

**Modelo:** opus (revisor opus). **Lane L1** (onda 5). **Compartilhado com a 2d:** `src/install/ops.rs`.

**Pré-requisitos (confira com `grep`; faltando ⇒ `BLOCKED`):** `script_scan::{scan, env_var_names}`, `prelude::{PRELUDE, PRELUDE_FUNCTIONS, CONTRACT_VARS}`, `runner::{script_vars, runner_ps1, ScriptContext, ScriptOp, ScriptBody, RunnerInput, VarValue}`, `Runners` com `installer: Option<ToolRun>`, `ExecError::NeedsScriptConsent`, `Step::RunInstaller`.

#### Where this fits

- `src/install/executor.rs`: `pub struct Runners { innounp, installer }` + `disabled()`; `ExecError::NeedsScriptConsent { app, what }`; `fn check_order` (:456); `async fn run_step` (:692); `AwaitWorker` (:1111); `struct RemoveWork(PathBuf)` (:961, guard que apaga uma pasta no `Drop`).
- `src/install/planner.rs`: `fn refuse_2c` (os `if` de `pre_install`, `post_install` e o de `installer` que ainda recusa `script`); `Step::RunInstaller` logo depois de `Step::Promote`; `Step::WriteManifest { json }` antes de `RecordState`.
- `src/install/script_scan.rs`: `scan(script, supported) -> ScriptReport { verdict, flags, .. }`, `env_var_names(script)`; `src/install/runner.rs` (tabela de variáveis e gerador puro); `src/install/prelude.rs` (`PRELUDE` só ASCII, `CONTRACT_VARS` = `__wr_root`, `__wr_effects`, `__wr_env_seed`, `__wr_helpers`, `__wr_shortcut_root`).
- Ordem do Scoop (`install_app`, D3): `pre_install` → `installer.file/args` **e depois** `installer.script` → `link_current` → shims → atalhos → psmodule → env → persist → `post_install` → grava o manifesto.
- Spec D5: em falha, desfazer os nossos passos e listar os efeitos detectados.

#### What binds this task

1. Em `executor.rs`:
   ```rust
   pub struct ScriptJob { pub app: String, pub hook: HookKind, pub workdir: PathBuf, pub runner: PathBuf, pub env: Vec<(OsString, OsString)> }
   pub struct ScriptOutcome { pub code: i32, pub lines: Vec<String> }
   pub trait ScriptHost: Send + Sync { fn run(&self, job: &ScriptJob) -> io::Result<ScriptOutcome>; }
   ```
   `Runners` ganha `pub scripts: Option<Arc<dyn ScriptHost>>` (`disabled()` ⇒ `None`); `ops::exec_ctx` põe `scripts: None` (sem consentimento ainda).
2. Planejador: apague os `if` de `pre_install` e `post_install` e a parte `script` do `if` do `installer` em `refuse_2c` (os de desinstalação continuam). `Step::Hook { kind: HookKind, script: String }`. Ordem: `Promote` → `Hook(PreInstall)` → `RunInstaller` → `Hook(InstallerScript)` → `Persist*` → `LinkCurrent` → `Shim*` → `Shortcut*` → `PsModule` → `EnvSet*` → `EnvAddPath*` → `EnsureShimsOnPath` → `Hook(PostInstall)` → `WriteManifest` → `RecordState` → `RemoveOldVersion?`.
3. Portão no planejamento: cada hook passa por `scan(script, PRELUDE_FUNCTIONS)`; veredito `Refuse` ⇒ `PlanError::ScriptRefused { hook: String, reasons: Vec<String> }` com `#[error("o script {hook} foi recusado: {}", .reasons.join("; "))]`.
4. Consentimento: plano com `Step::Hook` e `scripts: None` ⇒ `NeedsScriptConsent` **antes de qualquer passo**, com a linha literal `if has_hooks && ctx.runners.scripts.is_none() {`.
5. `Step::Hook`: pasta nova `<raiz>\tmp\script-<app>-<hook>-<n>` com guard `RemoveWork` (apagada em sucesso, erro e pânico). Grava: `prelude.ps1` (= `PRELUDE`), `hook.ps1` (o script), `manifest.json` (o `json` do `Step::WriteManifest` do mesmo plano), `env.json` (nomes `Path`, `PSModulePath` e `env_var_names(script)`, valores de `ctx.effects.env.get`), `effects.jsonl` vazio, e `runner.ps1` **com BOM UTF-8** (`EF BB BF`) gerado por `runner_ps1` com `script_vars(kind, &cx)` e as `CONTRACT_VARS` (todas presentes; faltando uma ⇒ erro interno). `ScriptContext`: `op = Update` se `plan.upgrade_from` é `Some`, senão `Install`; `fnames` dos `Step::Download`; `bucketsdir = <raiz>\buckets`; `__wr_helpers` com os auxiliares que existem em `<raiz>\apps\<auxiliar>\current\<exe>`; `__wr_shortcut_root` = `ctx.effects.shortcuts.root()`. `ScriptJob.env`: `SCOOP=<raiz>`, `SCOOP_GLOBAL=<raiz>\.global-disabled`, `SCOOP_CACHE=<raiz>\cache`. Roda o host no worker bloqueante.
6. Código ≠ 0 ⇒ o passo falha com `"o script {hook} saiu com código {c}: {últimas 5 linhas}"` + D5: compara os valores de `env.json` antes e depois e acrescenta `"; variáveis alteradas pelo script e não desfeitas: A, B"` quando houver; rollback normal.
7. Código 0 ⇒ por ora **nenhum efeito é aplicado**: `effects.jsonl` não vazio ⇒ o passo falha com `"efeitos de script ainda não suportados"` (a aplicação dos efeitos vem na tarefa seguinte da lane; este braço será trocado lá).

**Files:**
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/ops.rs`
- Test: `tests/it/install_scripts.rs` (**já criado pelo orquestrador**; `World`/`FailAt` copiados de `tests/it/install_executor.rs`; `FakeHost` em `Arc` que registra os `ScriptJob`, lê o `runner.ps1` e devolve um código; nada de PowerShell)

Testes:
1. `hooks_run_in_scoop_order`: `pre_install`, `installer` (`file`+`args`), `installer.script` e `post_install` ⇒ registro comum `[PreInstall, installer, InstallerScript, PostInstall]`.
2. `dir_per_hook`: `runner.ps1` do `PreInstall` tem `$dir = '<vdir>'`; o do `PostInstall` tem `$dir = '<current>'` e `$original_dir = '<vdir>'`.
3. `upgrade_install_hooks_see_cmd_update`: plano com `upgrade_from` ⇒ `$cmd = 'update'`.
4. `no_consent_fails_before_anything`: `scripts: None` ⇒ `NeedsScriptConsent`; servidor falso com 0 pedidos; host não chamado.
5. `refused_script_fails_at_planning`: `post_install: "scoop install x"` ⇒ `PlanError::ScriptRefused` com razão contendo `"scoop"`.
6. `unknown_scoop_function_refused_with_name`: `post_install: "Expand-ZstdArchive x"` ⇒ razão `"função do Scoop ainda não suportada: Expand-ZstdArchive"`.
7. `nonzero_exit_rolls_back_and_lists_changed_env`: o host grava `X` no `FakeEnv` (simula o script mexendo no registro) e sai 1 ⇒ erro contém `"código 1"` e `"X"`; `apps\<app>` não existe.
8. `failure_at_each_step_restores_initial_state`: para cada `k` dos passos, `FailAt(k)` ⇒ ambiente, pastas e atalhos iguais ao estado inicial (padrão de `failure_at_any_step_restores_the_initial_state_exactly`).
9. `workdir_removed_on_success_and_failure`: depois dos dois casos, `<raiz>\tmp` sem `script-*`.
10. `runner_has_bom_and_prelude_is_ascii`: `runner.ps1` começa com `EF BB BF`; `PRELUDE.is_ascii()`.
11. `env_seed_contains_script_names`: `Get-EnvVar -Name 'JAVA_HOME'` no script ⇒ `env.json` com chaves `Path`, `PSModulePath`, `JAVA_HOME` e os valores do `FakeEnv`.
12. `effects_not_yet_supported_fail`: host grava uma linha em `effects.jsonl` ⇒ falha com `"ainda não suportados"` e rollback.

#### Checks beyond the steps

1. Mutação: âncora `if has_hooks && ctx.runners.scripts.is_none() {` → `if false {` ⇒ teste 4 **PEGA** (o servidor recebe pedidos).
2. Mutação: âncora `HookKind::PostInstall => cx.current.clone(),` em `src/install/runner.rs` → `HookKind::PostInstall => cx.version_dir.clone(),` ⇒ teste 2 **PEGA** (prova que o executor passa pela tabela).
3. Mutação: troque a ordem das linhas que empurram `Step::RunInstaller` e `Step::Hook { kind: HookKind::InstallerScript, .. }` (`-AnchorFile`/`-ReplacementFile` com as duas linhas literais do planejador) ⇒ teste 1 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): script host in the executor for install hooks`

---

### Task 157: Aplicar `effects.jsonl` pela via da 2a

**Modelo:** opus (revisor opus). **Lane L1** (onda 5, depois da 156).

#### Where this fits

- `src/install/executor.rs`: braço `Step::Hook` (hoje falha com `"efeitos de script ainda não suportados"` quando `effects.jsonl` não está vazio); `fn add_path` (:1215, journal `Undo::PathRemove`), `fn make_shim` (:1265), braço `Step::Shortcut` (:872, `Undo::RemoveShortcut` + `Artifact`), braço `Step::EnvSet` (:901, `Undo::RestoreEnv` + `Artifact` com `previous`), `fn remove_artifact` (:1353).
- `src/install/script_effects.rs`: `parse_effects(text) -> Result<Vec<ScriptEffect>, EffectsError>`; `ScriptEffect::{SetEnv, AddPath, RemovePath, Shim, RmShim, Shortcut}`.
- `src/install/planner.rs`: `fn shim_kind` (:112) e `fn shortcut_link` (:165), privadas.
- `src/install/path_edit.rs`: `norm` (:3), `path_add`, `path_remove`, `path_contains`.
- Spec (c): "Depois de código 0: o executor aplica `effects.jsonl` pela via da 2a, com `Undo` e registro na state.db. Efeito ilegível ou fora do app ⇒ rollback." O Scoop recusa `env_add_path` fora do app.

#### What binds this task

1. Troque o braço "ainda não suportados" por: `parse_effects` (erro ⇒ passo falha `"efeitos do script ilegíveis: {e}"`, rollback) e aplicação **em ordem**, cada efeito com `Undo` e `Artifact` como os passos da 2a.
2. Regras (`app_dir = layout.app_dir(app)`, `persist_dir = layout.persist_dir(app)`, comparação por `path_edit::norm`), com a linha literal `if !is_inside(&entry, &app_dir) && !is_inside(&entry, &persist_dir) {`:
   - `SetEnv` ⇒ como `Step::EnvSet` (`value: None` ⇒ remove, `RestoreEnv`); nome `Path`/`PATH` (sem caixa) ⇒ falha `"o script tentou gravar o PATH inteiro; use Add-Path"`.
   - `AddPath{var: PATH}` ⇒ cada entrada dentro de `app_dir` ou `persist_dir`, senão falha `"o script tentou pôr {entry} no PATH (fora da pasta do app)"`; aplica com `add_path`. `var` `PSModulePath` ⇒ mesma regra, `prepend` com `RestoreEnv`. Outra variável ⇒ falha `"Add-Path em {var} não suportado"`.
   - `RemovePath` ⇒ remove só entradas que este app registrou (artefato desta execução ou do registro anterior); as outras são ignoradas com `tracing::warn!`.
   - `Shim{path, name, args}` ⇒ `path` dentro de `app_dir`; `make_shim(ShimSpec { name, target, args, kind: shim_kind(path) })` (`shim_kind` e `shortcut_link` passam a `pub(crate)`).
   - `RmShim{name}` ⇒ só se o shim é deste app; senão ignora com aviso.
   - `Shortcut` ⇒ `target` dentro de `app_dir`; `link = shortcut_link(root, name)`; cria como `Step::Shortcut`.
3. Os artefatos entram em `run.artifacts` e vão para o `state.db` no `RecordState` (a remoção da 2a desfaz).

**Da revisão da Task 153 (2026-10-18): o efeito é dado não confiável (escrito por script de terceiro).** Antes de aplicar: (1) tamanho do `effects.jsonl` conferido antes de ler (teto 64 MiB); (2) `shortcut.name` (ex.: `..\..\Startup\x`), `shim.name`/`path`, `rm_shim.name` e cada entrada de PATH pelo `install::safepath` e dentro do app/raiz; (3) `set_env` e `add_path.var` só em variáveis permitidas (nunca `PATH` inteiro, `PSModulePath` só pela via do psmodule; lista explícita e teste); (4) `remove_path` com padrões `-like` como no Scoop, mas só removendo entradas dentro da raiz do wingetrs. Um teste de recusa por item.

**Da revisão da Task 156 (2026-10-18):** acrescente em tests/it/install_scripts.rs (ou no seu arquivo de teste) um caso de recusa de hook vindo de variante de arquitetura (`architecture.64bit.post_install: "scoop install x"` ⇒ `PlanError::ScriptRefused`).

**Files:**
- Modify: `src/install/executor.rs`, `src/install/planner.rs` (só visibilidade)
- Test: `tests/it/install_script_effects.rs` (**já criado pelo orquestrador**; `World`/`FailAt` copiados; `FakeHost` que grava um `effects.jsonl` escolhido pelo teste e sai 0)

Testes:
1. `add_path_inside_app_applied_and_recorded`: `AddPath "<app_dir>\current\bin"` ⇒ no PATH do `FakeEnv`; artefato `Path`; a remoção tira.
2. `add_path_outside_app_rolls_back`: `AddPath "C:\\Windows"` ⇒ falha com `"fora da pasta do app"`; PATH igual ao inicial; `apps\<app>` não existe.
3. `set_env_applied_restored_on_rollback`: `SetEnv X=v` sobre `X=antigo` e `FailAt` no passo seguinte ⇒ `X == "antigo"`.
4. `set_env_path_refused`.
5. `psmodulepath_add_path`: `AddPath{var:"PSModulePath"}` dentro do app ⇒ aplicado na frente.
6. `other_var_add_path_refused`.
7. `remove_path_only_own_entries`: `RemovePath` de entrada de outro app ⇒ ignorada (continua no PATH).
8. `shim_effect_created_and_removed_on_uninstall`.
9. `rm_shim_of_other_app_ignored`.
10. `shortcut_effect_created_and_removed`.
11. `unreadable_effects_rolls_back`: `effects.jsonl` = `"lixo"` ⇒ falha com `"ilegíveis"`.
12. `failure_at_each_step_with_effects`: `FailAt(k)` para cada `k` com um hook que gera `AddPath` + `SetEnv` + `Shim` ⇒ estado inicial restaurado.

#### Checks beyond the steps

1. Mutação: âncora `if !is_inside(&entry, &app_dir) && !is_inside(&entry, &persist_dir) {` → `if false {` ⇒ teste 2 **PEGA**.
2. Mutação: remova o `Undo` do `SetEnv` de script (linha `run.push(Undo::RestoreEnv {` do braço novo → `let _ = (` com o resto equilibrado, via `-AnchorFile`) ⇒ teste 3 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): apply script effects through the phase 2a paths`

---

### Task 158: `installer.file` em `.ps1` (S-ps1)

**Modelo:** sonnet (revisor opus). **Lane L1** (onda 5, depois da 157).

#### Where this fits

- `src/install/planner.rs`: `fn refuse_2c` (o `if` do `installer` ainda recusa `file` terminando em `.ps1` com `"usa installer (PowerShell/instalador próprio): suportado na fase 2c"`); `Step::RunInstaller` e `Step::Hook(InstallerScript)` depois de `Promote`.
- `src/install/installer_args.rs`: `installer_args(args, dir, version)` (só `$dir`, `$global`, `$version`; divisão do Scoop).
- `src/install/runner.rs`: `ScriptBody::Ps1File { path, args }` (gera `& '<path>' @(<args>)` dentro de `try/catch`).
- `src/install/executor.rs`: braço `Step::Hook` (pasta de trabalho, `runner.ps1` com BOM, host no worker, efeitos aplicados, D5); `Runners.scripts: Option<Arc<dyn ScriptHost>>`; `ExecError::NeedsScriptConsent`.
- Scoop `Invoke-Installer`: `.ps1` roda com `& $progName @fnArgs` no próprio processo; **não é apagado** mesmo sem `keep` (o `Remove-Item` fica no ramo não `.ps1`); depois roda `installer.script`.
- `script_scan` não vê o conteúdo do `.ps1` (não está no manifesto).

#### What binds this task

1. `refuse_2c`: o `if` do `installer` deixa de recusar `.ps1` (o do `uninstaller` continua até a tarefa de remoção).
2. `Step::RunPs1Installer { file: String, args: Vec<String> }` no lugar de `RunInstaller` quando `file` termina em `.ps1` (sem caixa); mesma posição (depois de `Hook(PreInstall)`, antes de `Hook(InstallerScript)`).
3. Execução pelo mesmo caminho do `Step::Hook` (pasta de trabalho, BOM, efeitos, D5, guard) com `ScriptBody::Ps1File { path: <vdir>\<file>, args: installer_args(..) }`, `HookKind::InstallerScript` para a tabela de variáveis; `scripts: None` ⇒ `NeedsScriptConsent` antes de qualquer passo. Arquivo ausente ou fora da pasta da versão ⇒ falha. O `.ps1` nunca é apagado (linha literal `// Scoop keeps .ps1 installers even without keep`).
4. Relatório do portão para esse passo: veredito `ConfirmStrong` com o detalhe `"instalador .ps1: conteúdo não analisado"` (função `pub fn ps1_installer_report(file: &str) -> ScriptReport` em `script_scan.rs`).

**Files:**
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/script_scan.rs`
- Test: `tests/it/install_ps1.rs` (**já criado pelo orquestrador**; `FakeHost` como na `tests/it/install_scripts.rs`, copiado)

Testes:
1. `ps1_runs_via_host_with_ps1_body`: `runner.ps1` contém `& '<vdir>\setup.ps1' @('/S','<vdir>')` para `args: ["/S", "$dir"]`.
2. `ps1_not_deleted_without_keep`.
3. `ps1_needs_consent`: `scripts: None` ⇒ `NeedsScriptConsent`, servidor com 0 pedidos.
4. `ps1_then_installer_script`: `installer: {file:"setup.ps1", script:"..."}` ⇒ registro `[ps1, InstallerScript]`.
5. `ps1_nonzero_rolls_back`.
6. `ps1_report_is_confirm_strong`.

#### Checks beyond the steps

1. Mutação: âncora do braço que escolhe `RunPs1Installer` (linha literal `if file.to_lowercase().ends_with(".ps1") {`) → `if false {` ⇒ teste 1 **PEGA** (vira `RunInstaller` sem host).
2. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): run .ps1 installers through the script host (S-ps1)`

---

### Task 159: Remoção e upgrade com hooks e `$cmd`

**Modelo:** sonnet (revisor opus). **Lane L1** (onda 5, depois da 158). **Compartilhado com a 2d:** `src/install/ops.rs`.

#### Where this fits

- `src/install/executor.rs`: `pub fn execute_uninstall` (:1407, hoje: em uso ⇒ recusa; roda o desinstalador S-file antes das remoções; remove artefatos; apaga `apps\<app>`; `purge`; registro), braço `Step::Hook` (pasta, runner com BOM, host, efeitos, D5), `Runners { innounp, installer, scripts }`, `ExecError::{NeedsScriptConsent, Hook(String)}`.
- `src/install/state.rs`: `installed_manifest(layout, app, version, expected_sha256)` (checa o SHA-256 do `manifest.json` gravado).
- `src/install/planner.rs`: `pub struct InstalledInfo { pub version }` (:14; literais em `ops.rs`, `planner.rs`, `tests/it/install_executor.rs`, `tests/it/install_planner.rs`), `fn refuse_2c` (restam os `if` de `pre_uninstall`, `post_uninstall` e `uninstaller` com `script` ou `.ps1`).
- `src/install/ops.rs`: `plan_for` (:521, preenche `InstalledInfo`), `uninstall` (:806).
- `src/install/runner.rs`: `ScriptOp::{Update, Uninstall}`, `ScriptContext { version, old_version, version_dir, manifest_path, purge, .. }`.
- Ordem do Scoop (D3): uninstall = `pre_uninstall` → desinstalador `file/args` → `uninstaller.script` → remoções → `post_uninstall` (pasta já apagada); update = baixa a nova → `pre_uninstall` (manifesto antigo) → desinstalador antigo → `uninstaller.script` antigo → remoções → `post_uninstall` antigo → instala a nova com `$cmd = 'update'`. Nos hooks de desinstalação do update: `$version` = **nova**, `$manifest` = **antigo**, `$dir` = pasta da versão antiga, `$old_version`.

#### What binds this task

1. Apague os `if` restantes de `refuse_2c` (`pre_uninstall`, `post_uninstall`, `uninstaller` com `script`/`.ps1`); `refuse_2c` some se ficar vazia. Hooks de desinstalação passam pelo portão (`ScriptRefused`) no planejamento da remoção também (ler em `ops::plan_uninstall` via `installed_manifest`).
2. Remoção: `execute_uninstall` lê `installed_manifest` com o SHA-256 do registro (divergente ⇒ falha sem remover nada; ausente ⇒ remove sem scripts). Com scripts e `scripts: None` ⇒ `NeedsScriptConsent` antes de tudo (linha literal `if has_uninstall_scripts && ctx.runners.scripts.is_none() {`). Ordem: `Hook(PreUninstall)` → desinstalador `file/args` ou `.ps1` → `Hook(UninstallerScript)` → remoções da 2a → `Hook(PostUninstall)`. `op = Uninstall`, `$dir` = pasta da versão, `purge` do plano. Falha antes das remoções ⇒ nada removido. Falha no `post_uninstall` ⇒ registro já removido e `Err(ExecError::Hook("removido, mas o post_uninstall falhou: ..."))`.
3. Efeitos de hooks de desinstalação: aplicados só se tocam artefatos deste app (`RemovePath` de entrada registrada, `SetEnv` de variável registrada, `RmShim` de shim do app); o resto é ignorado com `tracing::warn!`.
4. Upgrade: `InstalledInfo` ganha `pub old_manifest: Option<Resolved>` (`ops::plan_for` preenche com `installed_manifest(..)` + `resolve(.., Arch::host())`; ausente ⇒ `None`). Passos novos, depois das extrações e antes de `Promote`: `Step::UpgradeUninstallHook { kind, script }` (`PreUninstall`, depois `UninstallerScript`, depois `PostUninstall`) e `Step::UpgradeRunUninstaller { file, args }` entre o `PreUninstall` e o `UninstallerScript`. Contexto: `op = Update`, `version = plan.version` (nova), `old_version = plan.upgrade_from`, `version_dir` = pasta da versão antiga, `manifest_path` = cópia do manifesto antigo na pasta de trabalho. Os hooks de instalação da nova continuam com `$cmd = 'update'`.
5. Falha da instalação nova depois que os hooks antigos rodaram ⇒ a mensagem acrescenta `"; os scripts de desinstalação da versão antiga já rodaram e não foram desfeitos"`.

**Da revisão da Task 151 (2026-10-18):** confira no `scoop-update.ps1` (e6aa3b3) se a atualização roda o desinstalador (`uninstaller.file`/`script`) da versão antiga antes de instalar a nova; se rodar, esta tarefa faz o mesmo (com o manifesto gravado da versão antiga, nunca o do catálogo), com teste da ordem.

**Files:**
- Modify: `src/install/plan.rs`, `src/install/planner.rs`, `src/install/executor.rs`, `src/install/ops.rs`, `tests/it/install_planner.rs`, `tests/it/install_executor.rs` (só o campo novo de `InstalledInfo`)
- Test: `tests/it/uninstall_scripts.rs` (**já criado pelo orquestrador**; `FakeHost` e `World` copiados)

Testes:
1. `uninstall_order_and_cmd`: registro `[PreUninstall, uninstaller, UninstallerScript, PostUninstall]`; no `PostUninstall` a pasta do app já não existe; `$cmd = 'uninstall'` nos três.
2. `uninstall_uses_written_manifest_not_catalog`: catálogo trocado por manifesto sem hooks ⇒ hooks rodam.
3. `uninstall_manifest_sha_mismatch_refuses`: `manifest.json` alterado ⇒ erro; nada removido.
4. `pre_uninstall_failure_keeps_everything`.
5. `post_uninstall_failure_reports_after_removal`: registro removido; erro com `"removido, mas o post_uninstall falhou"`.
6. `uninstall_without_consent_refused_before_anything`: manifesto com `uninstaller.file` **e** `post_uninstall`, `scripts: None` ⇒ `NeedsScriptConsent`; desinstalador falso não chamado; app intacto.
7. `upgrade_old_hooks_see_new_version_old_manifest`: runner do `PreUninstall` antigo com `$version = '2.0'`, `$old_version = '1.0'`, `$dir = '<pasta 1.0>'`, `$cmd = 'update'`, e o `manifest.json` da pasta de trabalho == texto da 1.0; depois `PreInstall` da 2.0 com `$cmd = 'update'`; os antigos antes do `Promote`.
8. `upgrade_failure_after_old_hooks_mentions_warning`.
9. `pre_2c_install_without_manifest_json_uninstalls_without_scripts`.
10. `uninstall_hook_effects_only_for_own_artifacts`.

#### Checks beyond the steps

1. Mutação em `src/install/state.rs`: âncora `if crate::cache::sha256_hex(&bytes) != expected_sha256 {` → `if false {` ⇒ teste 3 **PEGA**.
2. Mutação em `src/install/executor.rs`: âncora `if has_uninstall_scripts && ctx.runners.scripts.is_none() {` → `if false {` ⇒ teste 6 **PEGA**.
3. Mutação: no contexto do `UpgradeUninstallHook`, `version: plan.version.clone(),` → `version: plan.upgrade_from.clone().unwrap_or_default(),` ⇒ teste 7 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): uninstall and upgrade with manifest hooks and $cmd`

---

### Task 160: `ScriptHost` real no Windows (PowerShell, política, elevação)

**Modelo:** opus (revisor opus). **Lane L7** (onda 5, depois da 156, PARALELO com 157–159). **Compartilhado com a 2d:** `src/install/prod_win.rs`.

#### Where this fits

- `src/install/executor.rs`: `pub trait ScriptHost { fn run(&self, job: &ScriptJob) -> io::Result<ScriptOutcome>; }`, `ScriptJob { app, hook, workdir, runner, env }`, `Runners.scripts: Option<Arc<dyn ScriptHost>>`.
- `src/install/prod_win.rs:56` `build_ctx` (com `WINGETRS_TEST_ENV_KEY` nada real roda).
- `src/bridge/spawn_win.rs`: `RunSpec`/`run_spec` (programa absoluto, `env`, `env_remove`, `decode`, `grace`), `PIPE_GRACE` (2 s, carência para neto), `utf8_lossy`. `src/bridge/mod.rs:321` `find_pwsh` (busca `pwsh.exe` no PATH; o código **não elevado** pode usar o PATH).
- `scripts/check_rules.ps1` (regras por padrão de linha sobre `src/`; a regra de relógio também varre `tests/`).
- Sonda D7: a política bloqueia `.ps1` em disco (`-File`, dot-source), não `-Command`; `-ExecutionPolicy Bypass` no filho resolve `Restricted`/`AllSigned`/`RemoteSigned`, **exceto GPO** (`MachinePolicy`/`UserPolicy`); chaves `HKLM`/`HKCU\SOFTWARE\Policies\Microsoft\Windows\PowerShell` (`EnableScripts`, `ExecutionPolicy`) e `...\Microsoft\PowerShellCore`; erro típico com `UnauthorizedAccess`/`PSSecurityException`/`about_Execution_Policies`, saída 1. O pwsh 7 exporta um `PSModulePath` próprio que atrapalha um filho 5.1.

#### What binds this task

1. `src/install/script_policy.rs` (puro): `pub struct GpoValues { pub machine_enable_scripts: Option<u32>, pub machine_policy: Option<String>, pub user_enable_scripts: Option<u32>, pub user_policy: Option<String> }` e `pub fn gpo_blocks(v: &GpoValues) -> Option<String>`: máquina antes de usuário; `EnableScripts == 0` ou política `Restricted`/`AllSigned` (sem caixa; linha literal `"allsigned" | "restricted"`) ⇒ `Some("a política de grupo (GPO) do PowerShell ({valor}) impede rodar scripts; peça ao administrador")`; `RemoteSigned`/`Unrestricted`/`Bypass`/ausente ⇒ `None`. `pub fn policy_error(lines: &[String]) -> Option<String>`: linha com `UnauthorizedAccess`, `PSSecurityException` ou `about_Execution_Policies` ⇒ `"a política de execução do PowerShell impediu o script; veja Get-ExecutionPolicy -List"`. `pub fn refuse_if_elevated(elevated: bool) -> Result<(), String>` ⇒ `"o wingetrs não roda scripts de manifesto com o processo elevado"`.
2. `src/install/script_host_win.rs` (`#[cfg(windows)]` no `mod`): `pub struct PwshHost { program: PathBuf }`; `PwshHost::locate()` = primeiro `pwsh.exe` nas pastas do PATH (caminho absoluto), senão `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`; `PwshHost::with_program(p)` (testes). `run`: recusa se elevado (`GetTokenInformation(TokenElevation)`), lê as chaves de GPO (só leitura) e aplica `gpo_blocks` **antes** de iniciar; argumentos `["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", <runner>]`; `env` = `job.env`; `env_remove` = linha literal `const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePath"];`; pasta = `job.workdir`; `decode = utf8_lossy`; `grace = PIPE_GRACE`; sem tempo-limite. Saída 1 com `policy_error(..)` ⇒ `io::Error` com esse texto (sem despejar o script).
3. `build_ctx`: com `WINGETRS_TEST_ENV_KEY` ⇒ `scripts: None`; senão `Some(Arc::new(PwshHost::locate()))`.
4. `scripts/check_rules.ps1`: regra nova — `PwshHost` só aparece em `src/install/script_host_win.rs`, `src/install/prod_win.rs` e, em `tests/`, só em `tests/it/script_host_live.rs`, onde todo `#[test]`/`#[tokio::test]` vem seguido de `#[ignore = "live:`. Prove com violação plantada (inclusive num arquivo de uma linha) e restaure.
5. Teste ao vivo em `tests/it/script_host_live.rs` com `#[ignore = "live: roda o PowerShell real com script sintético"]`: hook sintético grava um arquivo em `$dir` e chama `Add-Path`; confere `effects.jsonl`, que `%USERPROFILE%\scoop` não mudou (listagem do primeiro nível antes e depois) e que o `HKCU` real não foi tocado (`WINGETRS_TEST_ENV_KEY`). **Agentes não rodam esse teste sem pedido do usuário.**

**Da revisão da Task 146 (2026-10-18):** `run_spec` herda o ambiente do usuário (o programa é absoluto, mas um filho acha ferramentas pelo PATH do usuário). Esta tarefa decide o ambiente do processo que ela lança: `PATH` fixo (`%SystemRoot%\System32` por KnownFolder + as pastas do próprio app/auxiliar), sem herdar o PATH do usuário, salvo motivo medido; teste do ambiente montado.

**Da revisão da Task 156 (2026-10-18):** o host não consulta o `LaunchGate`; um Ctrl+C durante um hook bloqueia até o PowerShell terminar e o log do drop diz "cancelada durante a extração". Nesta tarefa: Ctrl+C antes de lançar o pwsh cancela; depois de lançado, o hook roda até o fim (como o msiexec, sem matar), com aviso; o texto do log do drop fica genérico ("cancelada durante um passo em andamento"). Teste com o `fake_pwsh.cmd`.

**Files:**
- Create: `src/install/script_policy.rs`, `src/install/script_host_win.rs` (**o orquestrador cria com `//!` e `pub mod`**)
- Modify: `src/install/prod_win.rs`, `scripts/check_rules.ps1`
- Test: `tests/it/script_host_win.rs` e `tests/it/script_host_live.rs` (**já criados pelo orquestrador**; o primeiro com `#![cfg(windows)]` e um `fake_pwsh.cmd` na pasta temporária que grava `%*`, `set SCOOP`, `set PSModulePath` e `cd` num arquivo e sai com o código pedido)

Testes (`tests/it/script_host_win.rs`):
1. `args_exact`: o falso gravou `-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <runner>`.
2. `env_scoop_set_psmodulepath_removed`: `SCOOP=<raiz>` presente; `PSModulePath` ausente.
3. `cwd_is_workdir`.
4. `exit_code_and_lines`: código 3 e as linhas do falso.
5. `policy_error_mapped`: falso imprime `... UnauthorizedAccess ...` e sai 1 ⇒ erro com `"política de execução"`.
6. `gpo_blocks_cases`: `EnableScripts 0` ⇒ bloqueia; `AllSigned` ⇒ bloqueia; `Restricted` ⇒ bloqueia; `RemoteSigned` ⇒ `None`; nada ⇒ `None`; máquina `RemoteSigned` + usuário `AllSigned` ⇒ bloqueia pelo usuário; máquina `AllSigned` + usuário `Bypass` ⇒ bloqueia pela máquina.
7. `elevated_refused`: `refuse_if_elevated(true)` ⇒ `Err` com o texto; `false` ⇒ `Ok`.
8. `workdir_removed_with_code_1`: instalação com `PwshHost::with_program(fake)` que sai 1 ⇒ `<raiz>\tmp` sem `script-*`.

#### Checks beyond the steps

1. Mutação em `src/install/script_host_win.rs`: âncora `"Bypass"` → `"RemoteSigned"` ⇒ teste 1 **PEGA**.
2. Mutação: âncora `const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePath"];` → `const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePathX"];` ⇒ teste 2 **PEGA**.
3. Mutação em `src/install/script_policy.rs`: âncora `"allsigned" | "restricted"` → `"restricted"` ⇒ teste 6 **PEGA**.
4. Instrumento: violação plantada da regra nova do `check_rules.ps1` ⇒ o gate falha; restaurado ⇒ verde (cole as duas saídas).
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): real PowerShell script host (Bypass child, GPO and elevation checks)`

---

### Task 161: Confirmação de scripts na CLI (`--allow-scripts`)

**Modelo:** sonnet (revisor opus). Onda 6, sozinha. **Compartilhado com a 2d:** `src/cli/mod.rs`, `src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/install/ops.rs`.

#### Where this fits

- `src/cli/mod.rs`: `Install` (:188; `yes` :200, `allow_msiexec` :203), `Uninstall` (:214), `Upgrade` (:238; `yes`, `allow_msiexec`).
- `src/cli/install_cmd.rs`: `struct Flags { yes, dry_run, allow_msiexec }` (:412) com `allow()`; `fn proceed` (:430: `--yes` pula; sem terminal e sem `--yes` ⇒ `"a confirmação exige um terminal interativo; use --yes"`).
- `src/cli/output.rs:371–433`: resumo do plano por passo (dry-run e confirmação).
- `src/install/ops.rs`: `pub enum AllowMsiexec` (:134), `apply`/`apply_allowing` (:607/:612), `apply_groups_allowing`, `fn exec_ctx` (:269, hoje põe `installer: None` e `scripts: None`), `plan_uninstall` (:795), `uninstall` (:806).
- `src/install/script_scan.rs`: `scan_manifest`, `ScriptReport { flags, verdict }`, `ps1_installer_report`; `src/install/state.rs::installed_manifest` (scripts da remoção).
- Spec D3 = 1: script nunca aceito só com `--yes`; a CLI exige `--allow-scripts`. Cabeçalho: "Script de terceiro (<hook>), roda com os seus privilégios e pode alterar qualquer arquivo seu"; mostra os flags e o script numerado; a UI nunca diz "seguro".

#### What binds this task

1. `--allow-scripts` em `install`, `upgrade` e `uninstall` (texto de ajuda: `"Permite rodar scripts e instaladores de terceiros do manifesto (mostrados antes)"`); `Flags.allow_scripts`.
2. `ops`: `pub struct Allow { pub msiexec: AllowMsiexec, pub scripts: bool }` e `apply_with(ctx, plans, &Allow)` / `apply_groups_with` / `uninstall_with(ctx, plan, scripts: bool)`; as funções antigas delegam com `scripts: false` (TUI inalterada). `exec_ctx` mascara: linha literal `scripts: if allow.scripts { ctx.runners.scripts.clone() } else { None },` (idem `installer`).
3. Um plano "tem código de terceiros" se contém `RunInstaller`, `RunPs1Installer`, `Hook`, `UpgradeUninstallHook` ou `UpgradeRunUninstaller`; na remoção, se o manifesto gravado tem hooks de desinstalação ou `uninstaller`.
4. Sem `--allow-scripts` e com código de terceiros ⇒ erro (mesmo com `--yes`) `"<app> roda código de terceiros (<o quê>); revise com --dry-run e rode de novo com --allow-scripts"`, sem baixar nada, com a linha literal `if needs_scripts && !f.allow_scripts {`. Com `--allow-scripts`: mostra o bloco e segue a regra do `proceed` (sem terminal e sem `--yes` ⇒ erro existente).
5. Bloco (também no `--dry-run`): por hook, `Script de terceiro (<hook>), roda com os seus privilégios e pode alterar qualquer arquivo seu`, os flags (`  ! <detail> (linha N)`) e o script numerado (`  1 | ...`); instalador: `Instalador de terceiro: <file> <args>`; `.ps1`: o relatório de `ps1_installer_report`. Nenhum texto contém "segur".

**Da revisão da Task 151 (2026-10-18):** `execute_uninstall` devolve as notas de desinstalador pulado (manifesto gravado ausente/alterado/irresolvível), mas `ops::uninstall` só as registra no log. Esta tarefa (e a 162 na TUI) leva essas notas ao usuário no resultado da remoção, com teste.

**Da Task 160 (2026-10-18):** o texto `UNCANCELLABLE` do CLI (aviso do Ctrl+C recusado) ainda fala só de "Windows Installer"; deixe genérico para hooks e instaladores ("um passo que não pode ser interrompido está rodando: <qual>; esperando terminar"), com teste.

**Files:**
- Modify: `src/cli/mod.rs`, `src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/install/ops.rs`
- Test: `tests/it/cli_scripts.rs` (**já criado pelo orquestrador**; `TestUi`/contexto como em `tests/it/cli_install.rs`, `FakeHost` em `Arc` com contador)

Testes:
1. `yes_without_allow_scripts_errors`: `install x --yes` com `post_install` ⇒ o texto exato do item 4; servidor com 0 pedidos; host não chamado.
2. `allow_scripts_yes_runs`: host chamado uma vez.
3. `no_tty_without_yes_errors`: `--allow-scripts`, sem terminal ⇒ erro do `proceed`.
4. `confirmation_block_has_header_flags_numbered_script`: saída contém o cabeçalho, `"  ! "` com o detalhe de um flag e `"  1 | "`.
5. `dry_run_shows_block_without_running`.
6. `uninstall_with_scripts_needs_allow_scripts`.
7. `upgrade_needs_allow_scripts_when_old_or_new_has_scripts`: só o manifesto antigo com `pre_uninstall` ⇒ exige.
8. `plan_without_scripts_needs_no_flag`.
9. `text_never_says_seguro`.
10. `help_lists_allow_scripts`: ajuda de `install`, `upgrade` e `uninstall`.
11. `ops_masks_runners_without_consent`: `apply_with(.., scripts: false)` num plano com hook ⇒ `NeedsScriptConsent`, host não chamado.

#### Checks beyond the steps

1. Mutação em `src/cli/install_cmd.rs`: âncora `if needs_scripts && !f.allow_scripts {` → `if false {` ⇒ teste 1 **PEGA** (o texto exato muda para o do executor).
2. Mutação em `src/install/ops.rs`: âncora `scripts: if allow.scripts { ctx.runners.scripts.clone() } else { None },` → `scripts: ctx.runners.scripts.clone(),` ⇒ teste 11 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): --allow-scripts confirmation with gate flags and numbered scripts`

---

### Task 162: Confirmação de scripts na TUI

**Modelo:** sonnet (revisor opus). Onda 6, depois da 161, sozinha. **Compartilhado com a 2d:** `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/regions.rs`, `src/tui/jobs.rs`.

#### Where this fits

- `src/tui/app.rs`: `pub enum Action` (:192; `ApplyMsiexec { groups, app }` como modelo de ação autorizada), `MSIEXEC_NOTE` (:244), `pub struct ConfirmItem` (:316), `pub enum Modal` (:343), `fn confirm_items` (:2253), `fn confirm_modal` (:2326); `planning_confirm: Option<ConfirmMode>` (o ajuste `confirm` pode pular o diálogo).
- `src/tui/regions.rs`: `pub fn confirm_geom(items, notes, area)` (:151), `pub fn confirm_window(scroll, len, rows)` (:208).
- `src/tui/view.rs`: `fn draw_modal` (:1172), `fn confirm_text` (:1274), `fn draw_confirm` (:1295). Cores só por `tui/theme.rs`.
- `src/tui/jobs.rs`: `Action::Apply(groups) | Action::ApplyMsiexec { .. }` (:282), `Action::ApplyMsiexec { groups, app }` (:341).
- `src/install/ops.rs`: `apply_groups_with(ctx, groups, &Allow { msiexec, scripts })`, `uninstall_with(ctx, plan, scripts)`; `script_scan::scan_manifest`, `ps1_installer_report`; `state::installed_manifest`.
- ARMADILHAS: "Regra por item que remonta a lista inteira" — monte o texto dos scripts **uma vez** por confirmação; código chamado no desenho ganha bench `#[ignore]`.

#### What binds this task

1. `Action::ApplyScripts { groups }` e `Action::RemoveScripts(UninstallPlan)`: criados só pela confirmação que mostrou os scripts; `jobs.rs` chama `apply_groups_with(.., scripts: true)` / `uninstall_with(.., true)`. `Action::Apply`/`Remove` continuam com `scripts: false`.
2. Plano com código de terceiros ⇒ o diálogo aparece **sempre**, mesmo com o ajuste `confirm` que o pularia.
3. Seção rolável "Scripts de terceiros" no diálogo: por hook o cabeçalho `Script de terceiro (<hook>), roda com os seus privilégios e pode alterar qualquer arquivo seu`, os flags e as linhas numeradas; geometria em `regions.rs` (`confirm_geom` ganha o número de linhas da seção e devolve a área dela); `PgUp`/`PgDn` e a roda do mouse rolam a seção (`confirm_window`). Texto montado uma vez quando o modal abre (guardado no `Modal::Confirm`).
4. Nenhum texto contém "segur".

**Da revisão da Task 151 (2026-10-18):** `execute_uninstall` devolve as notas de desinstalador pulado (manifesto gravado ausente/alterado/irresolvível), mas `ops::uninstall` só as registra no log. Esta tarefa (e a 162 na TUI) leva essas notas ao usuário no resultado da remoção, com teste.

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/regions.rs`, `src/tui/jobs.rs`
- Test: `tests/it/tui_scripts.rs` (**já criado pelo orquestrador**; `TestBackend` e montagem do `App` como em `tests/it/tui_state.rs`/`tui_view.rs`)

Testes:
1. `dialog_forced_even_when_confirm_setting_skips`.
2. `confirm_yields_apply_scripts`: Enter no botão de confirmar ⇒ `Action::ApplyScripts`.
3. `cancel_yields_nothing`.
4. `plan_without_scripts_keeps_apply`: ⇒ `Action::Apply`.
5. `script_lines_numbered_and_header_rendered`.
6. `scroll_moves_script_window`: `PgDn` desloca a janela; a primeira linha visível muda de `1 |` para outra.
7. `remove_with_scripts_yields_remove_scripts`.
8. `regions_has_script_area`: `confirm_geom` com 40 linhas de script em área 80x24 ⇒ área da seção com altura > 0 e dentro do modal.
9. `bench_confirm_300_lines` (`#[ignore = "bench"]`): desenhar 300 linhas.

#### Checks beyond the steps

1. Mutação em `src/tui/app.rs`: a linha que força o diálogo (literal `if has_third_party_code(&actions) {`) → `if false {` ⇒ teste 1 **PEGA**.
2. Mutação em `src/tui/jobs.rs`: no braço `Action::ApplyScripts`, `scripts: true` → `scripts: false` ⇒ teste de integração do `jobs` (acrescente `apply_scripts_reaches_host` com `FakeHost`) **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(tui): scrollable third-party script section in the confirmation`

---

### Task 163: Validação da fase 2c

**Modelo:** opus (orquestrador revisa).

#### Where this fits

- `examples/oracle_2c.rs` (Task 142, com a tabela de vereditos do portão) e `docs/superpowers/reports/2026-10-18-oraculo-2c.md` (antes: 1403 manifestos 2c de 4118; `worst`: S-psm 10, S-inno 93, S-file 4, S-ps1 3, S-hook 1293).
- `scripts/bench.ps1` (sempre com `-Baseline target/release/examples/noop.exe`); `scripts/leftovers.ps1`.
- Spec 2c (f) riscos; texto proposto da exceção D2-1 para o `CLAUDE.md` (na spec, seção b).

#### What binds this task

1. Oráculo depois: acrescente ao `oracle_2c` a coluna "instalável pela 2c" = planeja com `plan_install` sem erro e sem `ScriptRefused`, com `supported = PRELUDE_FUNCTIONS`; tabela por bucket e por sub-caso, antes × depois, e as 10 razões de recusa mais frequentes do portão (com contagem). Rode sobre o `catalog.db` real (só leitura; nada é executado).
2. Bench do custo do host: `examples/bench_script_host.rs` que roda um `runner.ps1` sintético vazio pelo `PwshHost` (pwsh 7 e powershell 5.1), 15 execuções, com `-Baseline`; relate a mediana do delta e diga "não medido" para o que não rodou.
3. Relatório `docs/superpowers/reports/<data>-fase2c-validacao.md`: números acima, lista das emendas à spec (as decisões R1–R9 deste plano), riscos que ficaram.
4. Proposta (não aplicada) do texto D2-1 para o `CLAUDE.md`, para o orquestrador aplicar **com confirmação do usuário**.
5. Prova ao vivo **só com autorização explícita do usuário**, registrada no relatório: um S-psm, o `xming` (S-inno, o menor da sonda D4) e um S-hook de veredito `Confirm`; antes e depois, listagem de `%USERPROFILE%\scoop`, diff de `HKCU\Environment` e `scripts/leftovers.ps1`. Sem autorização, a seção diz "não executada".

**Da Task 157 (2026-10-18):** o `python` do main é recusado pelo portão porque o `installer.script` dele tem `-Global` dentro de `if ($global) { ... }` (ramo que nunca roda na instalação por usuário). Na validação, meça quantos manifestos S-hook do corpus são recusados só por construções dentro de `if ($global)` e traga ao orquestrador a proposta (com o risco) de ignorar esse bloco quando `$global` é fixo em `$false` e nenhum trecho reatribui `global` — a decisão é do usuário.

**Files:**
- Create: `examples/bench_script_host.rs`, `docs/superpowers/reports/<data>-fase2c-validacao.md`
- Modify: `examples/oracle_2c.rs`
- Test: nenhum módulo novo (no mutation proof: tarefa de medição)

#### Checks beyond the steps

1. Saídas reais do oráculo e do bench coladas no relatório.
2. `pwsh -File scripts/leftovers.ps1` limpo depois do bench.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `docs(report): phase 2c validation (oracle before/after, script host cost)`

### Task 164: `$global` constante e blocos `if ($global)` informativos (opção E)

**Modelo:** opus (revisor opus). Decisão do usuário em 2026-10-18, depois da Task 163 (relatório `docs/superpowers/reports/2026-10-18-fase2c-validacao.md`, seção 3).

#### Where this fits

- `src/install/runner.rs`: `runner_ps1` (~:205) grava `$global = $false` como linha comum (vem de `put("global", VarValue::Bool(false))`, ~:103) e roda o hook com `Invoke-Command ([scriptblock]::Create($__wr_hook))` (escopo filho).
- `src/install/prelude.ps1`: funções com parâmetro `$global` (`appsdir`, `shimdir`, `modulesdir`, `appdir`, `versiondir`, `currentdir`, `persistdir`, `installed`, `env`, ~:42–112 e ~:504) e com `[switch]$Global` (~:82, ~:417, ~:430, ~:445, ~:473); todas jogam erro se o valor for verdadeiro.
- `src/install/script_scan.rs`: `scan` (~:433) e `line_rules` (~:542) aplicam as regras linha a linha sobre o texto sem comentários e sem here-strings, e de novo sobre a linha crua (fail closed, ~:515).
- Testes existentes: `tests/it/script_scan.rs` (`global_switch_refused` ~:77, `global_passthrough_only_with_the_exact_variable` ~:481), `tests/it/script_runner.rs`, `tests/it/prelude_core.rs`.
- Oráculo: `examples/oracle_2c.rs` (Task 163: 1318/1403 planejam, 77 recusados, 38 só por código dentro de `if ($global)`).

**Sonda do orquestrador (2026-10-18, pwsh 7.6 e PowerShell 5.1):** com `Set-Variable -Name global -Value $false -Option Constant,AllScope -Scope Global`, 14 formas de mudar o valor (atribuição, `$script:`/`$local:`/`$global:`, `${global}`, `Set-Variable -Force`, `-Scope Local`, `New-Variable -Force`, `Remove-Variable -Force`, `iex`, escopo aninhado, `Invoke-Command` de scriptblock, provider `variable:`, `foreach ($global in ...)`) dão erro ou deixam `False`. **Mas** qualquer função com parâmetro chamado `global` passa a dar erro na chamada (mesmo com `$false` ou sem argumento). Com `[Alias('global')]` num parâmetro de outro nome (`$__wr_g`), `f 'x' $global`, `f 'x' -global $global`, `sw -Global:$global` e `sw -Global` funcionam (a variável dentro da função é `$__wr_g`). Sem `AllScope`, `$global = $true` no escopo filho cria outra variável e o bloco roda: `AllScope` é obrigatório.

#### What binds this task

1. **Runner:** `$global` deixa de ser linha comum. `runner_ps1` grava, antes do dot-source do prelúdio, `Set-Variable -Name global -Value $false -Option Constant,AllScope -Scope Global` (e não grava mais `$global = $false`). Os outros nomes do contrato não mudam.
2. **Prelúdio:** todo parâmetro `$global`/`$Global` do `prelude.ps1` vira `[Alias('global')]$__wr_g` (ou `[Alias('Global')][switch]$__wr_gs` nos switches), mantendo a posição; o corpo testa a variável nova e continua jogando o mesmo erro se for verdadeira. Nenhum `$global` como parâmetro, laço ou atribuição sobra no prelúdio (teste que lê o texto).
3. **Portão:** `scan` acha os blocos `if (<cond>) { ... }` (e `elseif (<cond>) { ... }`) cuja condição é exatamente `$global` ou começa com `$global -and ` (sem diferenciar maiúsculas; espaços livres). As linhas **dentro** desses blocos geram `Flag` com `verdict: Confirm` e `detail` "só roda em instalação global; não roda no wingetrs (<motivo original>)", em vez do veredito original. O `else` e tudo fora do bloco continuam com as regras de hoje.
4. **Fail closed:** se o portão não consegue delimitar o bloco com certeza (chave de abertura fora da mesma linha da condição ou da seguinte, chaves desbalanceadas, here-string, string ou comentário com `{`/`}` dentro do bloco, condição com `-or`, `-not`, `!`, parênteses extras), o bloco **não** é tratado como morto: as linhas seguem as regras de hoje.
5. **Portão, novas recusas:** o script do manifesto que **declara** parâmetro, laço (`foreach ($global`), `param(... $global ...)` ou função com parâmetro `global` recebe `Flag` `Refuse` com detail "usa `$global` como nome próprio (incompatível com o `$global` fixo do wingetrs)". Atribuição a `global` já cai no erro do PowerShell; não precisa de regra, mas meça no oráculo quantos manifestos a teriam.
6. **Oráculo:** rode `oracle_2c` de novo; relate no relatório da tarefa o novo total que planeja (esperado ~1356/1403, +38 da Task 163) e quantos caem na regra do item 5. Se o número divergir de +38, explique cada diferença.

#### Tests (literais)

- `runner_global_is_constant`: o texto de `runner_ps1` contém `Set-Variable -Name global -Value $false -Option Constant,AllScope -Scope Global` antes da linha do dot-source e não contém `$global = `.
- `prelude_has_no_global_parameter`: nenhuma linha do `prelude.ps1` casa `(?i)\$global\b` fora de `[Alias('global')]`/`[Alias('Global')]` e de comentários.
- `global_block_is_informational`: `if ($global) { $c = $c -replace 'HKEY_CURRENT_USER', 'HKEY_LOCAL_MACHINE' }` ⇒ veredito do relatório `Confirm`, um `Flag` com detail começando por "só roda em instalação global".
- `global_block_multiline`: o mesmo com `if ($global) {` numa linha, o corpo na seguinte e `}` na terceira.
- `global_block_else_still_scanned`: `if ($global) { ... } else { Set-ItemProperty 'HKLM:\Software\X' ... }` ⇒ `Refuse` pela linha do `else`.
- `global_and_condition`: `if ($global -and $x) { HKLM... }` ⇒ informativo; `if ($global -or $x) { HKLM... }` ⇒ `Refuse`; `if (!$global) { HKLM... }` ⇒ `Refuse`.
- `global_block_unbalanced_fails_closed`: `if ($global) { "}" ; HKLM... }` (chave dentro de string) ⇒ `Refuse`.
- `own_global_parameter_refused`: `function f($global) { }` e `foreach ($global in $a) { }` ⇒ `Refuse` com o detail do item 5.
- `#[ignore = "live: pwsh"] constant_global_blocks_reassignment` em `tests/it/script_host_live.rs`: runner real com hook `$global = $true; if ($global) { New-Item (Join-Path $dir 'RAN') }` ⇒ o hook falha e o arquivo `RAN` não existe na pasta temporária.

Mutações obrigatórias (cole `[OK] mutação PEGA`): (a) trocar `Constant,AllScope` por `Constant` no runner; (b) aceitar `-or` como condição de bloco morto; (c) desligar o fail closed da chave dentro de string.

**Files:**
- Modify: `src/install/runner.rs`, `src/install/prelude.ps1`, `src/install/script_scan.rs`, `examples/oracle_2c.rs`
- Test: `tests/it/script_scan.rs`, `tests/it/script_runner.rs`, `tests/it/prelude_core.rs`, `tests/it/script_host_live.rs`

#### Checks beyond the steps

1. Os testes do prelúdio que já existem (grupos 1–3) passam sem mudar asserção.
2. Saída real do oráculo colada no relatório.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(scripts): constant $global and informational if ($global) blocks`


### Task 191: Correções da revisão de fase da 2c (executor, consentimento, efeitos)

**Modelo:** opus (revisor opus). Origem: revisão de fase inteira da 2c em 2026-10-18 (orquestrador), sobre o HEAD f71ac10.

#### Where this fits

- `src/install/installer_win.rs` (~:14 `run`) e `src/install/inno_win.rs` (~:12): rodam `installer.file`/`uninstaller.file` e o innounp **sem** `refuse_if_elevated`; só o `PwshHost::run` checa (`script_host_win.rs` ~:91). Chamadores: `src/install/executor.rs` ~:2307 (instalar/atualizar) e ~:3496 (remover).
- `src/cli/output.rs` ~:620–627 `removal_third_party`: lista o desinstalador só quando `uninstaller.file` existe; o executor (`planner::uninstaller_command`, `planner.rs` ~:299) também roda o primeiro download quando só `args` existe; a ordem mostrada (script, depois arquivo) é a inversa da executada (arquivo, depois script).
- `src/install/executor.rs` ~:2159–2172 (efeito `Shim`): `args` vindo do `effects.jsonl` não é checado; o planner recusa caracteres de controle no `bin` do manifesto (`planner.rs` ~:428); `shim.rs` ~:210/217/219 grava `args` cru no texto do `.cmd`/`.ps1`.
- `src/install/executor.rs` ~:1912 `is_inside`: checagem só léxica; uma junção `$dir\x` criada pelo hook faz `Add-Path "$dir\x"` pôr qualquer pasta no PATH.
- `src/install/executor.rs` ~:3472–3510 com ~:3347: efeitos de remoção aplicados sem desfazer; se um passo seguinte falha, o erro não avisa que os scripts de desinstalação já rodaram.
- `src/tui/jobs.rs` ~:375 `check_shown_manifest` e `src/install/ops.rs` ~:976 `uninstall_with`: o sha mostrado é comparado antes do `install.lock`; `uninstall_with` só compara a versão; a CLI não compara sha.

#### What binds this task

1. **Nunca elevado:** `installer_win::run` e o lançamento do innounp chamam `refuse_if_elevated(is_elevated()?)` (mesmo erro e texto do `PwshHost`), com o `elevated` injetável já usado nos testes do host.
2. **Consentimento mostra o que roda, na ordem em que roda:** `removal_third_party` monta a lista a partir de `planner::uninstaller_command` (o mesmo que o executor usa), na ordem do executor.
3. **`args` de shim vindo de efeito:** `apply_effect` recusa em `args` caracteres de controle e `"` (mesma regra do `bin`), com motivo.
4. **Sem fuga por junção:** antes de aplicar `AddPath`/`Shim`/`Shortcut`, o caminho é canonicalizado e comparado com a pasta do app/persist canonicalizada; reparse point no caminho ⇒ recusa com motivo.
5. **Remoção que falha depois dos scripts:** o erro diz "os scripts de desinstalação já rodaram e alteraram: <lista dos efeitos aplicados>", como o `OLD_HOOKS_RAN` da atualização.
6. **sha sob a trava:** `uninstall_with` recebe o `manifest_sha256` mostrado (TUI e CLI) e compara **sob** o `install.lock`; diferente ⇒ recusa sem rodar nada.

#### Tests (literais)

- `file_installer_refused_when_elevated` e `innounp_refused_when_elevated` (elevated injetado = true ⇒ erro, corredor falso com 0 chamadas).
- `removal_consent_lists_args_only_uninstaller`: `uninstaller: {args: "/S"}` ⇒ a lista mostrada contém o programa; e `removal_consent_order_matches_executor`.
- `shim_effect_args_with_control_char_refused` e `shim_effect_args_with_quote_refused`.
- `effect_path_through_junction_refused` (junção em tempdir).
- `removal_failure_after_scripts_mentions_effects`.
- `uninstall_with_sha_mismatch_runs_nothing` (sha diferente sob a trava ⇒ 0 chamadas ao corredor).

Mutações obrigatórias (cole `[OK] mutação PEGA`): (a) tirar o `refuse_if_elevated` do `installer_win::run`; (b) `removal_third_party` voltar a olhar só `uninstaller.file`; (c) tirar a canonicalização do item 4.

**Files:**
- Modify: `src/install/installer_win.rs`, `src/install/inno_win.rs`, `src/install/executor.rs`, `src/install/ops.rs`, `src/install/shim.rs` (só se preciso), `src/cli/output.rs`, `src/cli/` (passar o sha), `src/tui/jobs.rs`
- Test: módulos existentes em `tests/it/` desses arquivos.

#### Checks beyond the steps

1. Outro agente edita `src/install/{runner.rs,script_scan.rs,prelude.ps1}` (Task 164) e outro `src/bridge/*` e `src/winget_install/{switches,elevated_rules}.rs` (Task 190): não toque nesses arquivos.
2. Testes nunca rodam pwsh, innounp nem instalador reais.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(install): phase 2c review (no elevation for file installers, consent order, effect paths, sha under lock)`

### Task 192: Portão — código puxado de arquivo e apelidos (revisão de fase da 2c)

**Modelo:** opus (revisor opus). Depois da Task 164 (mesmo arquivo).

#### Where this fits

- `src/install/script_scan.rs` ~:911 `invokes_variable`: `& "$dir\setup.ps1"` e `. "$bucketsdir\extras\scripts\foo.ps1"` (aspas entre `&`/`.` e `$`) não geram flag; o arquivo chamado roda sem análise e sem ser mostrado. O mesmo vale para `.ps1` que um `installer.file` analisado dot-source de `$PSScriptRoot`.
- `src/install/script_scan.rs` ~:592–605: regras `PreludeInternal`/`PreludeRedefinition` contornáveis por `Set-Alias shim Out-Null`, `New-Item alias:Add-Path`, `Set-Variable ('__w'+'r_effects')`, `Add-Content .\effects.jsonl`. Sem impacto hoje (o Rust revalida cada efeito por lista branca), mas recusas do lado do prelúdio não são fronteira.

#### What binds this task

1. `&` ou `.` seguido de string (aspas simples ou duplas) ou de caminho ⇒ `Flag` `ConfirmStrong` "roda um arquivo que não é mostrado: <alvo>".
2. Alvo literal resolvível (`$dir\<nome>.ps1`, `$bucketsdir\<bucket>\scripts\<nome>.ps1` com `<nome>` passando por `safepath`): o arquivo é lido (bucket guardado pela Task 149; `$dir` só se já extraído no momento da análise), analisado pelo mesmo `scan` (o pior veredito vence) e incluído no texto mostrado na confirmação. Não resolvível ⇒ fica no `ConfirmStrong` do item 1.
3. `Set-Alias`, `New-Alias`, `alias:` ⇒ `ConfirmStrong` "redefine comando por apelido".
4. Comentário no topo do `prelude.ps1` e na seção do portão do código: recusas do prelúdio são conveniência, não fronteira; a fronteira é a revalidação dos efeitos no Rust.

#### Tests (literais)

- `call_quoted_path_is_confirm_strong` (`& "$dir\setup.ps1"`), `dot_source_bucket_script_scanned` (arquivo de `scripts/` com `HKLM:` ⇒ `Refuse` do hook que o chama), `unresolvable_target_confirm_strong`, `alias_redefinition_confirm_strong`.

Mutação obrigatória: tirar a regra do item 1 ⇒ `call_quoted_path_is_confirm_strong` PEGA.

**Files:**
- Modify: `src/install/script_scan.rs`, `src/install/prelude.ps1` (só o comentário), e onde a confirmação monta o texto mostrado (CLI `src/cli/output.rs`, TUI pela mesma função)
- Test: `tests/it/script_scan.rs`, testes de confirmação existentes.

**Commit:** `fix(scripts): gate follows called script files and flags aliases`
