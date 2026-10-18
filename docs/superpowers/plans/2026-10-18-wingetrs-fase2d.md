# wingetrs — Fase 2d: instaladores do winget (plano)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-18-wingetrs-fase2d.md`.

**Spec:** `docs/superpowers/specs/2026-10-15-wingetrs-fase2d-design.md` (decidida em 2026-10-18). **Sondas:** `docs/superpowers/reports/2026-10-18-sondas-2d.md` (S2: motor nativo cobre 56,5% da amostra; zip/portable ~32%, `Dependencies` 15%, Agreements 0/290. S3: 331 entradas ARP; Kindle 2.8.3 em HKLM 32, 2.9.1 em HKCU; 3 sobras prováveis; `reg export` de HKLM funciona sem admin).

**Goal:** instalar, remover e atualizar pacotes do catálogo winget com o próprio wingetrs (motor nativo para msi/wix/burn/inno/nullsoft/msix/appx/exe com silencioso), a ponte `winget install` para o resto (com motivo), e limpar registros ARP órfãos com backup `.reg`. Um UAC por sessão (auxiliar elevado da Task 137 estendido).

**Decisões do usuário (2026-10-18), aplicadas neste plano:**
- **D1 = C (híbrido).** Motor nativo para msi, wix, burn, inno, nullsoft, msix, appx e exe com `Silent`/`SilentWithProgress`; ponte `winget install` para o resto, sempre com o motivo; Ajustes "Motor do winget: wingetrs / winget". Desinstalação ARP e órfãos **sempre nativos**. Ordem: puras (P1–P4) primeiro, ponte de instalação (P13a) cedo.
- **D2 = A + Ajustes.** Escopo `user` preferido; `--scope user|machine` na CLI (exige o escopo); Ajustes/`config.toml` trocam a preferência para `machine`. "pede UAC" na confirmação.
- **D3 = B para tudo.** O `wingetrs bridge-worker` da Task 137 ganha operações **fechadas**: rodar instalador conferido por hash, rodar desinstalador lido do ARP HKLM revalidado, apagar chave HKLM exportada e revalidada. Regra do `CLAUDE.md` para código elevado vale integralmente.
- **D4–D7** como recomendado na spec. **D4 instalar = B:** `exe` sem silencioso roda interativo com aviso (não é recusa).
- **D8:** `Dependencies` ⇒ ponte com o motivo. Agreements mostrados na confirmação; **`--yes` aceita as licenças**; com `--yes` a CLI imprime uma linha por licença (nome e link) e o `job.start` registra.
- **S1 não foi feita:** a Task 137 já provou `runas` + pipe; os pontos restantes da S1 entram na validação ao vivo (Task 188).

## Desenho (decidido pelo orquestrador; o implementador não muda sem perguntar)

- **Módulo próprio:** `src/winget_install/` (puro) + `*_win.rs` dentro dele; nada da 2d entra em `src/install/` (a fase 2c mexe em `src/install/manifest.rs`, `executor.rs`, `ops.rs`, `mod.rs` e no host de scripts). A 2d **lê** `install::download`, `install::msi_queue`, `install::msiexec_win`, `install::msiexec_log`, `install::safepath`, `install::executor::LaunchGate`, sem editá-los.
- **Cadeia de confiança:** catálogo (`winget_hash`) → versionData → manifesto mesclado (`sources::winget::details::merged_manifest`) → `InstallerSha256` → arquivo baixado e conferido → cópia conferida de novo antes de rodar.
- **TOCTOU do instalador elevado (D3), decidido aqui:** o auxiliar **copia** o arquivo para uma pasta de execução própria em `<FOLDERID_ProgramFiles>\wingetrs\staging\<128 bits hex>\`, com DACL protegida `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`, confere o SHA-256 **da cópia** por um handle aberto só com `FILE_SHARE_READ`, mantém o handle até o fim e roda de lá. Motivo decisivo: um instalador elevado rodando da pasta do usuário (`%LOCALAPPDATA%\wingetrs\installers\…`) carrega DLLs plantadas ao lado dele (ordem de busca de DLL começa na pasta do executável); o handle com compartilhamento negado não impede isso. A Task 180 tem a sonda que confirma as premissas de ACL e para com `BLOCKED` se alguma falhar.
- **Execução sem elevação:** `ShellExecuteExW` verbo `open` (nunca `CreateProcess`: exe `requireAdministrator` daria 740), `SW_SHOW` em produção, `SW_HIDE` só nos testes, numa thread STA sempre juntada; sem tempo-limite e sem cancelar depois de lançado.
- **Quando eleva (pelo auxiliar):** regra pura `install_elevation` da Task 178. Escopo `user` **nunca** é elevado (instalaria no perfil do administrador quando a elevação usa outra conta).
- **Arquivos fora de `%LOCALAPPDATA%\wingetrs`:** só os da emenda de segurança da spec, mais a pasta de execução do auxiliar (preparação do orquestrador, item 1).

## Preparação do orquestrador (antes de despachar)

1. **Emenda no `CLAUDE.md`** (seção "Comandos proibidos", depois da exceção da 2b), texto proposto:
   > **Exceção da fase 2d (instaladores do winget):** depois da confirmação do usuário naquele item, o wingetrs pode (1) rodar um instalador com SHA-256 conferido ou um desinstalador lido do ARP, aceitando o que eles gravam; (2) implantar e remover MSIX pelo `PackageManager`; (3) apagar chaves `…\CurrentVersion\Uninstall\<sub>` (HKCU, HKLM 32/64) uma a uma, depois do `.reg` exportado e conferido; (4) o auxiliar elevado cria e apaga só `<Program Files>\wingetrs\staging\<aleatório>\` (DACL só SYSTEM e Administradores). Nunca `msiexec` com ação diferente de `/i` na instalação e `/x` na desinstalação. **Testes nunca** rodam instalador real, nunca usam `runas`, nunca tocam o ARP real nem `Program Files`: `.cmd` em tempdir, `Launcher` falso e registro falso; o resto é `#[ignore = "live: ..."]`.
2. **Fontes do winget-cli** no commit de `docs/reference/winget-cli/SOURCE_COMMIT.txt` (`39739564a4aaf1071b17d163ec332a08b9bcf05c`), baixados de `https://raw.githubusercontent.com/microsoft/winget-cli/<commit>/src/...` e gravados achatados (como os existentes): `AppInstallerCommonCore_Manifest_ManifestCommon.cpp`, `AppInstallerCLICore_Workflows_ShellExecuteInstallerHandler.cpp`, `AppInstallerCLICore_Workflows_MsiInstallFlow.cpp`, `AppInstallerCommonCore_Deployment.cpp`, `AppInstallerCommonCore_Manifest_ManifestComparator.cpp`, `AppInstallerCLICore_Workflows_InstallFlow.cpp`. Se algum caminho não existir naquele commit, procure pelo nome na árvore do commit e anote o caminho real numa linha do `SOURCE_COMMIT.txt`. Um commit.
3. **Esqueleto:** `pub mod winget_install;` em `src/lib.rs`; `src/winget_install/mod.rs` com `//! Native install/uninstall of winget packages (phase 2d).` e os `pub mod` de cada onda (os `*_win` com `#[cfg(windows)]`), cada arquivo novo com uma linha `//! <assunto> (Task N).`. Onda 2: `select`, `host_win`, `switches`, `codes`, `fetch`, `verified_win`, `launch`, `launch_win`. Onda 1: `uninstall_cmd`, `arp_read_win`. Depois: `msix`, `msix_win` (179); `staging_win` em `src/bridge/` (180); `plan`, `ops`, `post_check`, `prod_win` (181); `elevated_rules` (182); `orphans`, `orphans_win` (183); `uninstall` (184); `engine` (185). `src/cli/winget_cmd.rs` (176) com `pub mod winget_cmd;` em `src/cli/mod.rs`.
4. **Módulos de teste** (uma linha `//!` cada, `mod` no `tests/it/main.rs`, um commit por onda): `wi_manifest` (170), `wi_settings` (171), `wi_uninstall_cmd` (172), `wi_select` (173), `wi_switches` (174), `wi_codes` (175), `wi_bridge_install` (176), `wi_fetch` (177), `wi_launch` (178), `wi_msix` (179), `elevated_install` (180), `wi_install_flow` (181), `elevated_uninstall` (182), `wi_orphans` (183), `wi_uninstall_flow` (184), `wi_upgrade` (185), `wi_cli` (186), `wi_tui` (187), `wi_portable` (189).

## Ordem, faixas paralelas e arquivos compartilhados

| Onda | Tarefas (paralelas dentro da onda) | Depende de |
|---|---|---|
| 1 | 170 (manifesto) ‖ 171 (config + Ajustes) ‖ 172 (comando de desinstalação ARP) | preparação |
| 2 | 173 (seleção) ‖ 174 (switches) ‖ 175 (códigos) ‖ 176 (ponte `winget install`) ‖ 177 (download + arquivo conferido) ‖ 178 (lançador) | 170 |
| 3 | 179 (MSIX) ‖ 180 (auxiliar: rodar instalador) | 175 / 174+177+178 |
| 4 | 181 (fluxo de instalação nativa) ‖ 182 (auxiliar: desinstalador e apagar chave) | 173–180 / 172+180 |
| 5 | 183 (órfãos) | 182 |
| 6 | 184 (fluxo de desinstalação + Kindle) | 181+183 |
| 7 | 185 (atualização nativa + escolha do motor) | 181+184 |
| 8 | 186 (CLI) ‖ 187 (TUI) | 185 |
| 9 | 188 (validação ao vivo, orquestrador + usuário) | 186+187 |
| 10 | 189 (opcional: portable e zip→portable) | **decisão do usuário** |

**Arquivos compartilhados** (só uma tarefa por vez; com a 2c em paralelo, o agente usa `cargo fmt -- <arquivos>` e `git add <arquivos>`):
- Com a 2c (risco): `src/cli/install_cmd.rs` (176, 186), `src/cli/mod.rs` (176, 186), `src/install/progress.rs` + consumidores `src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/tui/jobs.rs`, `src/tui/app.rs` (181), `src/install/target.rs` (176), `src/tui/app.rs`/`run.rs`/`jobs.rs`/`view.rs` (176, 187), `src/config.rs` + `src/tui/settings.rs` + snapshots do Ajustes em `tests/it/snapshots/` (171), `src/job_log.rs` (176), `tests/it/main.rs` (preparação).
- Entre faixas da 2d: `src/bridge/worker.rs`, `elevated.rs`, `elevate_win.rs` (176 → 180 → 182, sempre em ondas diferentes); testes com literal `BridgeJob { .. }`: `tests/it/bridge.rs`, `elevated_worker.rs`, `job_log.rs`, `tui_jobs.rs`, `tui_state.rs` (176); `src/installed/arp.rs` (172).
- A 2d **não edita**: `src/install/manifest.rs`, `executor.rs`, `ops.rs`, `mod.rs`, `state.rs`, `download.rs` (só lê).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa (itens que mais pesam aqui: "Teste que não consegue falhar", "Caso degenerado esconde a regra", "Teste com limiar de relógio quebra o gate sob carga", "API Win32 receitada no brief sem sonda", "Versão comparada como texto", "Rótulo especial no callback de progresso só tratado num consumidor", "Escape `\xNN` no texto de um comando vira byte bruto" — barras invertidas em literais de caminho e pipe).
- **Testes nunca** rodam instalador real, nunca usam `runas`, nunca tocam o ARP real, o HKLM ou `Program Files`; sem rede (wiremock). Instalador falso = `.cmd` em tempdir com hash calculado no teste (D7). Testes que precisam da máquina: `#[ignore = "live: <motivo>"]`, rodados em `--release` só pelo usuário/orquestrador.
- Código da 2d em `src/winget_install/` (e `src/bridge/` para o auxiliar, `src/cli/winget_cmd.rs` para a CLI). Código Win32/WinRT/winreg só em `*_win.rs` com `#[cfg(windows)]` no `mod` (o `check_rules` barra `windows::`/`winreg::` fora deles e `cfg!(windows)`).
- **Código que roda elevado** (`bridge-worker`): caminhos só de `SHGetKnownFolderPath` (nunca de variável de ambiente, que vem do usuário), executáveis por caminho absoluto (nunca PATH), nada criado/gravado/apagado em pastas do usuário (lê o blob do usuário, só isso), resultado e log voltam pelo pipe. Toda operação nova do protocolo é fechada (`deny_unknown_fields`, enums em vez de texto livre), validada antes de agir, e uma requisição inválida fecha o canal ou responde `error` sem executar nada.
- Dado externo (manifesto, registro, pipe, `.reg`, arquivo baixado): sem `unwrap`/`expect`, com teto antes de alocar. Toda thread criada é juntada em todos os caminhos; pânico de worker propagado com `resume_unwind`; handle de `spawn_blocking` sempre aguardado.
- Sem crate nova. Features do `windows` (0.62.2) já habilitadas: `Management_Deployment`, `Foundation_Collections`, `Win32_UI_Shell`, `Win32_System_Com`, `Win32_Foundation`, `Win32_Storage_FileSystem`, `Win32_System_Threading`, `Win32_System_Pipes`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_Security_Cryptography`, `Win32_System_Registry`, `Win32_Storage_Packaging_Appx`. Feature nova só com o motivo no relatório (e o orquestrador confere que o `Cargo.lock` não muda de pacote).
- Versões sempre por `crate::core::version` (nunca `==`/`<` em `String`); códigos de saída comparados `as u32`.
- Cada função portada do winget-cli cita o arquivo de origem em comentário de uma linha (fontes em `docs/reference/winget-cli/`, já baixadas pelo orquestrador). Não portar mais do que a tarefa pede.
- Texto ao usuário em português; identificadores e comentários em inglês.
- A fase 2c roda em paralelo no mesmo checkout: nunca `cargo fmt` sem lista de arquivos, nunca `git add -A`/`.`; não edite `src/install/manifest.rs`, `executor.rs`, `ops.rs`, `mod.rs`, `state.rs`, `download.rs`.

## Review Focus

1. **Auxiliar elevado:** cada operação nova recusa campo extra, alvo fora do formato, caminho relativo, `..`, aspas e switches de `msiexec` fora da lista; programa sempre por caminho absoluto de pasta de administrador; nada gravado em pasta do usuário; a cópia conferida é a que roda (troca entre a conferência e a execução não passa).
2. **Escopo `user` nunca elevado**, em nenhum caminho (instalação nativa, ponte, desinstalação HKCU).
3. **View 32/64** preservada da linha até o `reg export`, a releitura e o `RegDeleteKey` (mutação de troca em cada ponto).
4. **Hash divergente ou URL `http` ⇒ nada lançado**; `exit 0` sem o app no ARP/MSIX ⇒ aviso, nunca "Instalado".
5. **Órfãos:** nunca pré-marcados; `.reg` conferido (cabeçalho e nome da chave) antes de apagar; chave que deixou de ser sobra não é apagada; `--yes` sem `--orphans` não limpa nada.
6. **Esperas com relógio injetado** (chave sumir, 1618): nenhum `elapsed() <` sem `clock-ok`, nenhum teste que só fica lento quando a regra quebra.
7. **Arquivos compartilhados com a 2c**: o diff só toca o que a tarefa lista.

---

### Task 170: Manifesto mesclado completo (P1)

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `src/sources/winget/manifest.rs:6` `MergedManifest` (raiz: `installer_type`, `scope`, `upgrade_behavior`, `require_explicit_upgrade`, `installers`) e `:67` `Installer` (`architecture`, `installer_type`, `scope`, `upgrade_behavior`, `require_explicit_upgrade`, `installer_url`, `installer_sha256`), ambos `#[serde(rename_all = "PascalCase")]`; `:86` `parse` (serde-saphyr). `into_details` (~:92) e `bridge::winget_applicability` (`src/bridge/mod.rs:154`) leem esses campos e **não mudam de comportamento**.
- Chaves YAML do winget (schema 1.6+): raiz e instalador aceitam `InstallerType`, `NestedInstallerType`, `NestedInstallerFiles` (`RelativeFilePath`, `PortableCommandAlias`), `Scope`, `InstallerLocale`, `Platform`, `MinimumOSVersion`, `InstallModes`, `InstallerSwitches` (`Silent`, `SilentWithProgress`, `Interactive`, `InstallLocation`, `Log`, `Upgrade`, `Custom`), `InstallerSuccessCodes`, `ExpectedReturnCodes` (`InstallerReturnCode`, `ReturnResponse`, `ReturnResponseUrl`), `UpgradeBehavior`, `RequireExplicitUpgrade`, `ElevationRequirement`, `ProductCode`, `PackageFamilyName`, `AppsAndFeaturesEntries` (`DisplayName`, `Publisher`, `DisplayVersion`, `ProductCode`, `UpgradeCode`, `InstallerType`), `Dependencies` (`WindowsFeatures`, `WindowsLibraries`, `PackageDependencies` com `PackageIdentifier`/`MinimumVersion`, `ExternalDependencies`). Só na raiz (locale padrão mesclado): `Agreements` (`AgreementLabel`, `Agreement`, `AgreementUrl`).
- **Armadilha de nome:** `rename_all = "PascalCase"` transforma `minimum_os_version` em `MinimumOsVersion`; a chave real é `MinimumOSVersion` ⇒ `#[serde(rename = "MinimumOSVersion")]`.

#### What binds this task

1. Structs novas em `manifest.rs`, todas `Debug, Clone, Default, serde::Deserialize`, PascalCase, campos `Option`/`Vec` com `#[serde(default)]`: `InstallerSwitches`, `ExpectedReturnCode { installer_return_code: i64, return_response: String, return_response_url: Option<String> }`, `Dependencies`, `PackageDependency`, `Agreement`, `ArpEntry` (AppsAndFeaturesEntries), `NestedInstallerFile`.
2. Os campos da lista acima entram **em `MergedManifest` (raiz) e em `Installer`** como `Option<...>` (listas como `Option<Vec<_>>`, para distinguir "ausente" de "vazio"); `agreements: Vec<Agreement>` só na raiz. Códigos de retorno são `i64` (aceita negativos e valores acima de `i32::MAX`).
3. `pub enum InstallerKind { Msi, Wix, Burn, Inno, Nullsoft, Msix, Appx, Exe, Zip, Portable, Font, Pwa, Msstore, Unknown(String) }` com `pub fn parse(s: Option<&str>) -> InstallerKind` (sem distinção de caixa; `None` ou vazio ⇒ `Unknown(String::new())`) e `pub fn name(&self) -> &str` (minúsculo, como no manifesto).
4. `pub struct EffectiveInstaller` (campos: `architecture`, `kind: InstallerKind`, `nested_kind: Option<InstallerKind>`, `nested_files: Vec<NestedInstallerFile>`, `scope`, `locale`, `platform: Vec<String>`, `minimum_os_version`, `install_modes: Vec<String>`, `switches: InstallerSwitches`, `success_codes: Vec<i64>`, `expected_codes: Vec<ExpectedReturnCode>`, `upgrade_behavior`, `require_explicit_upgrade: bool`, `elevation: Option<String>`, `product_code`, `package_family_name`, `arp_entries: Vec<ArpEntry>`, `dependencies: Option<Dependencies>`, `url: String`, `sha256: String`) e `impl MergedManifest { pub fn effective(&self) -> Vec<EffectiveInstaller> }`, um por instalador na ordem do manifesto.
5. Herança: cada campo é `installer.x.clone().or(root.x.clone())`. `InstallerSwitches` mescla **chave por chave** (o do instalador vence em cada chave). `Dependencies` do instalador substitui a da raiz inteira. `require_explicit_upgrade` ausente nos dois ⇒ `false`.

**Files:**
- Modify: `src/sources/winget/manifest.rs`
- Test: `tests/it/wi_manifest.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_manifest.rs`, YAML escrito no teste, `manifest::parse(yaml.as_bytes())?.effective()`):
1. `installer_field_falls_back_to_root`: raiz `InstallerType: inno`, `Scope: machine`, `ElevationRequirement: elevatesSelf`, `ProductCode: "{A}"`; instaladores `[x64 com Scope: user, x86 sem nada]` ⇒ `[0].scope == Some("user")`, `[0].kind == Inno`, `[0].elevation == Some("elevatesSelf")`; `[1].scope == Some("machine")`, `[1].product_code == Some("{A}")`.
2. `switches_merge_key_by_key`: raiz `InstallerSwitches: {Silent: /s-root, Custom: /c-root}`, instalador `{Silent: /s-inst, Log: /l}` ⇒ `silent == /s-inst`, `custom == /c-root`, `log == /l`, `upgrade == None`.
3. `codes_and_expected_parse`: `InstallerSuccessCodes: [3010, -1, 4294967295]`, `ExpectedReturnCodes: [{InstallerReturnCode: 1, ReturnResponse: packageInUse}]` ⇒ `success_codes == [3010, -1, 4294967295]`, `expected_codes[0].installer_return_code == 1`, `return_response == "packageInUse"`.
4. `minimum_os_version_key_is_exact`: `MinimumOSVersion: 10.0.22000.0` no instalador ⇒ `Some("10.0.22000.0")`.
5. `agreements_and_dependencies_parse`: raiz `Agreements: [{AgreementLabel: EULA, AgreementUrl: https://x/eula}]` e `Dependencies: {PackageDependencies: [{PackageIdentifier: Microsoft.VCRedist.2015+.x64, MinimumVersion: "14.0"}]}`; instalador 2 com `Dependencies: {WindowsFeatures: [NetFx3]}` ⇒ `[0].dependencies` tem 1 pacote; `[1].dependencies` tem 0 pacotes e `windows_features == ["NetFx3"]`; `manifest.agreements[0].agreement_url == Some("https://x/eula")`.
6. `installer_kind_parse_case_insensitive`: `MSI`→`Msi`, `wix`→`Wix`, `Nullsoft`→`Nullsoft`, `appx`→`Appx`, `None`→`Unknown("")`, `weird`→`Unknown("weird")`; `InstallerKind::parse(Some("Inno")).name() == "inno"`.
7. `nested_installer_inherits`: raiz `NestedInstallerType: portable`, `NestedInstallerFiles: [{RelativeFilePath: 'bin\x.exe', PortableCommandAlias: x}]`, instalador `InstallerType: zip` ⇒ `kind == Zip`, `nested_kind == Some(Portable)`, `nested_files[0].relative_file_path == "bin\\x.exe"`.
8. `explicit_upgrade_defaults_false`: nenhum `RequireExplicitUpgrade` ⇒ `false`; raiz `true` e instalador ausente ⇒ `true`; raiz `true` e instalador `false` ⇒ `false`.

#### Checks beyond the steps

1. Mutação: em `effective`, a herança de `scope` vira `root.scope.clone().or(i.scope.clone())` ⇒ teste 1 **PEGA**.
2. Mutação: a mescla de `silent` vira raiz-primeiro ⇒ teste 2 **PEGA**.
3. Mutação: tirar o `rename = "MinimumOSVersion"` ⇒ teste 4 **PEGA**.
4. Binário real: `target/release/wingetrs.exe show Git.Git` e `show Microsoft.PowerToys` continuam funcionando (o manifesto real com os campos novos ainda parseia); cole as duas primeiras linhas de cada.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget): full merged manifest with installer-over-root inheritance`

---

### Task 171: Ajustes e config — motor do winget e escopo padrão (D1, D2)

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `src/config.rs:35` `Behavior` (`Copy`, `#[serde(default)]`, campos `confirm`, `purge`, `keep_old`, `residues`, `allow_http_redirect`, `mouse`) e `:46` `Default`; enums de exemplo `ConfirmMode` (`:15`) e `ResidueMode` (`:25`) com `rename_all = "lowercase"`.
- `src/tui/settings.rs:6` `SettingId`, `:20` `ORDER: [SettingId; 10]`, `label`/`options`/`description`, `:149` `value_index`, `:183` `cycle`. `src/tui/jobs.rs:605` `save_settings` grava `behavior` inteiro (não precisa mudar).
- Os snapshots `insta` da aba Ajustes (`tests/it/snapshots/`, testes em `tests/it/tui_view.rs`) listam as linhas do `ORDER`.

#### What binds this task

1. `config.rs`: `pub enum WingetEngine { #[default] Wingetrs, Winget }` e `pub enum DefaultScope { #[default] User, Machine }`, ambos `Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize`, `rename_all = "lowercase"`. `Behavior` ganha `winget_engine: WingetEngine` e `default_scope: DefaultScope` (com o `Default`). `config.toml` antigo sem os campos carrega com os padrões.
2. `settings.rs`: `SettingId::WingetEngine` — rótulo `Motor do winget`, opções `["wingetrs", "winget"]`, descrição `Quem instala e remove pacotes do winget: o próprio wingetrs (mais rápido; usa o winget quando não sabe instalar um tipo) ou sempre o winget.`; `SettingId::DefaultScope` — rótulo `Escopo padrão (winget)`, opções `["usuário", "máquina"]`, descrição `Para quem instalar quando o pacote oferece os dois: só você, ou todos os usuários (pede UAC).`. `ORDER` passa a 12 entradas, as duas novas logo depois de `HttpRedirect`. `value_index` e `cycle` cobrem as duas.
3. Snapshot do Ajustes que mudar: revise o diff (só as duas linhas novas) e aceite só esse snapshot; diga no relatório quais arquivos de snapshot mudaram.

**Files:**
- Modify: `src/config.rs`, `src/tui/settings.rs`, snapshots do Ajustes em `tests/it/snapshots/` (só os que mudarem)
- Test: `tests/it/wi_settings.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_settings.rs`):
1. `defaults_are_wingetrs_and_user`: `Behavior::default()` ⇒ `winget_engine == Wingetrs`, `default_scope == User`.
2. `config_roundtrip_new_fields`: TOML `[behavior]\nwinget_engine = "winget"\ndefault_scope = "machine"\n` num tempfile ⇒ `Config::load` traz `Winget`/`Machine`; `save` + `load` devolve o mesmo `Behavior`.
3. `old_config_without_fields_loads`: TOML `[behavior]\nconfirm = "always"\n` ⇒ carrega, os dois campos no padrão.
4. `invalid_engine_is_parse_error`: `winget_engine = "x"` ⇒ `Err(ConfigError::Parse { .. })`.
5. `cycle_engine_and_scope`: `SettingsState::new(Behavior::default(), ..)` com `selected` na posição de `WingetEngine` ⇒ `cycle(true)` ⇒ `behavior.winget_engine == Winget` **e** `default_scope` inalterado; de novo ⇒ `Wingetrs`. Mesmo para `DefaultScope` (e `winget_engine` inalterado).
6. `order_places_new_settings_after_redirect`: posição de `HttpRedirect` + 1 é `WingetEngine`, + 2 é `DefaultScope`; `ORDER.len() == 12`.
7. `save_settings_writes_fields`: `jobs::save_settings(tmp, behavior com Winget/Machine, None, None, None, "dark")` ⇒ o arquivo contém `winget_engine = "winget"` e `default_scope = "machine"`.

#### Checks beyond the steps

1. Mutação: o braço `WingetEngine` de `cycle` escreve em `default_scope` ⇒ teste 5 **PEGA**.
2. Mutação: `value_index(DefaultScope)` sempre 0 ⇒ teste 5 **PEGA**.
3. Binário real: abra `target/release/wingetrs.exe` na aba Ajustes **só se o terminal permitir**; senão diga "não verificado na TUI" e cole o snapshot novo.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(config): winget engine and default scope settings`

---

### Task 172: Comando de desinstalação a partir do ARP (P10) e leitura de uma chave

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/installed/arp.rs:108` `missing_uninstaller(cmd, expand, exists)`: separa o programa (entre aspas, ou até o primeiro `.exe` que fecha um token) e expande `%VAR%` com `expand_vars` (`:92`, privado). `GENERIC_LAUNCHERS` (`:82`).
- `src/installed/mod.rs:21` `Hive { MachineX64, MachineX86, User }`; `:52` `RawArpEntry` (não tem `QuietUninstallString` nem os sinais da D4).
- `src/installed/arp_win.rs:8–9` raízes `UNINSTALL` e `UNINSTALL_X86`, `:105` `read_arp` (HKLM com `KEY_READ | KEY_WOW64_64KEY`, inclusive o caminho `WOW6432Node`; HKCU com `KEY_READ`). Não muda.
- `src/installed/guid.rs` `pack_guid` (valida `{8-4-4-4-12}`).
- S3: 175 entradas MSI (nenhuma com Quiet), 50 com Quiet (todas as Inno `_is1` e Burn já têm Quiet), 12 NSIS só pelo marcador; Kindle 2.8.3 `uninstall.exe` NSIS sem Quiet.

#### What binds this task

1. `src/installed/arp.rs`: `pub fn split_command(cmd: &str) -> Option<(String, String)>` (programa **sem expandir**, resto aparado) com a mesma regra de hoje; `missing_uninstaller` passa a usá-la (comportamento idêntico); `expand_vars` vira `pub fn`.
2. `src/winget_install/uninstall_cmd.rs` (puro): `pub struct ArpDetails { hive: Hive, key: String, display_name, publisher, display_version: Option<String>, windows_installer: bool, system_component: bool, quiet_uninstall_string, uninstall_string, inno_app_path, bundle_cache_path, bundle_upgrade_code, install_location: Option<String> }`; `pub enum Rule { Msi, Quiet, Inno, Burn, Nsis, Interactive }` (serde `snake_case`, para o pipe); `pub struct UninstallCommand { rule: Rule, program: String /* "msiexec" ou o programa sem expandir */, args: String, interactive: bool }`; `pub fn uninstall_command(d: &ArpDetails, nsis_from_manifest: bool) -> Option<UninstallCommand>`.
3. Ordem (spec P10, D4): (1) `windows_installer` **e** `key` é GUID (`pack_guid` aceita) ⇒ `Msi`, `program "msiexec"`, args `/x{KEY} /quiet /norestart` (a chave como está); (2) `QuietUninstallString` ⇒ `Quiet`, `split_command`; (3) chave termina em `_is1` (sem distinção de caixa) **e** `inno_app_path` ⇒ `Inno`, `UninstallString` + ` /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`; (4) `bundle_cache_path` ou `bundle_upgrade_code` ⇒ `Burn`, `UninstallString` + ` /quiet /norestart`; (5) `nsis_from_manifest` ⇒ `Nsis`, `UninstallString` + ` /S`; (6) `UninstallString` ⇒ `Interactive` (`interactive: true`); nada ⇒ `None`. Switch acrescentado não é duplicado se o resto já o contém (comparação por token, sem caixa).
4. `src/winget_install/arp_read_win.rs`: `pub fn read_details(hive: Hive, key: &str) -> std::io::Result<Option<ArpDetails>>` lendo **uma** chave com a view explícita (HKLM64: `UNINSTALL` + `KEY_WOW64_64KEY`; HKLM32: caminho `WOW6432Node` + `KEY_WOW64_64KEY`; HKCU: `UNINSTALL`), valores `QuietUninstallString`, `UninstallString`, `WindowsInstaller`, `SystemComponent`, `Inno Setup: App Path`, `BundleCachePath`, `BundleUpgradeCode`, `InstallLocation`, `DisplayName`, `Publisher`, `DisplayVersion`; chave inexistente ⇒ `Ok(None)`. `pub fn valid_arp_key(key: &str) -> bool` (em `uninstall_cmd.rs`, puro): não vazia, sem `\`, sem `/`, ≤ 255, diferente de `.`/`..`; `read_details` recusa chave inválida com `InvalidInput`. E `pub fn shell_split(cmd: &str) -> std::io::Result<(String, String)>` com `SHEvaluateSystemCommandTemplate` (só para o teste ao vivo de equivalência).

**Files:**
- Create: `src/winget_install/uninstall_cmd.rs`, `src/winget_install/arp_read_win.rs` (já existem com uma linha `//!`)
- Modify: `src/installed/arp.rs`
- Test: `tests/it/wi_uninstall_cmd.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_uninstall_cmd.rs`, `ArpDetails` montado no teste; `G = "{11111111-2222-3333-4444-555555555555}"`):
1. `msi_beats_quiet`: `windows_installer`, `key = G`, `quiet = "C:\Q\q.exe /s"` ⇒ `Rule::Msi`, `program == "msiexec"`, `args == "/x{11111111-2222-3333-4444-555555555555} /quiet /norestart"`.
2. `msi_flag_without_guid_key_falls_through`: `windows_installer`, `key = "Foo"`, `uninstall = r#""C:\X\u.exe""#` ⇒ `Interactive`, `program == r"C:\X\u.exe"`.
3. `quiet_rule`: `quiet = r#""C:\Program Files\A\unins000.exe" /SILENT"#` ⇒ `Quiet`, `program == r"C:\Program Files\A\unins000.exe"`, `args == "/SILENT"`, `interactive == false`.
4. `inno_rule_needs_is1_and_app_path`: `key = "App_is1"`, `inno_app_path = Some`, `uninstall = r#""C:\P\unins000.exe""#` ⇒ `Inno`, `args == "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART"`; mesma entrada com `key = "App"` ⇒ `Interactive`; com `key = "App_is1"` e sem `inno_app_path` ⇒ `Interactive`.
5. `burn_rule`: `bundle_cache_path = Some`, `uninstall = r#""C:\ProgramData\Package Cache\{g}\setup.exe" /uninstall"#` ⇒ `Burn`, `args == "/uninstall /quiet /norestart"`; com `uninstall` já terminando em `/quiet /norestart` ⇒ args sem duplicar.
6. `nsis_only_with_manifest_hint`: `uninstall = r#""C:\P\K\uninstall.exe""#`: `hint = false` ⇒ `Interactive`; `true` ⇒ `Nsis`, `args == "/S"`.
7. `unquoted_with_spaces`: `uninstall = r"C:\Program Files\X\uninst.exe /x"` ⇒ `program == r"C:\Program Files\X\uninst.exe"`, `args == "/x"`.
8. `nothing_is_none`: sem Quiet nem Uninstall nem MSI ⇒ `None`.
9. `split_command_agrees_with_missing_uninstaller`: para 6 comandos (aspas, sem aspas com espaço, `foo.exe.d`, `%ProgramFiles%\a.exe /x`, `msiexec.exe /x{G}`, relativo `a.exe`) ⇒ `missing_uninstaller(cmd, expand, |_| false)` é `Some(p)` exatamente quando `split_command` + `expand_vars` dá um caminho absoluto não genérico, e `p` é esse caminho (equivalência dos dois caminhos).
10. `valid_arp_key_cases`: `""`, `"a\\b"`, `"a/b"`, `".."`, 256 caracteres ⇒ `false`; `G`, `"KLiteCodecPack_is1"`, `"Steam App 255710"` ⇒ `true`.
11. `live_split_matches_shell` `#[ignore = "live: SHEvaluateSystemCommandTemplate"]`: os comandos dos testes 3, 5, 7 ⇒ o programa de `shell_split` (sem caixa) == o de `split_command`.

#### Checks beyond the steps

1. Mutação: Quiet antes de MSI na ordem ⇒ teste 1 **PEGA**.
2. Mutação: ignorar `nsis_from_manifest` (sempre `/S`) ⇒ teste 6 **PEGA**.
3. Mutação: `_is1` sozinho basta (sem `inno_app_path`) ⇒ teste 4 **PEGA**.
4. Os testes existentes de `missing_uninstaller` (`tests/it/installed.rs`) continuam verdes sem mudança.
5. Real: `cargo test --release --quiet live_split_matches_shell -- --ignored` (só chama a API do shell) e cole o resultado.
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): uninstall command from an ARP key (msi, quiet, inno, burn, nsis)`

---

### Task 173: Escolha do instalador (P2)

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/sources/winget/manifest.rs`: `MergedManifest::effective() -> Vec<EffectiveInstaller>` e `InstallerKind` (Task 170; campos `architecture`, `kind`, `nested_kind`, `scope`, `locale`, `platform`, `minimum_os_version`, `switches`, `dependencies`, …).
- `src/bridge/mod.rs:120` `host_arch()` (`x64`/`x86`/`arm64`) e `:130` `runs_on` (a mesma preferência: x64 ⇒ x64, x86; arm64 ⇒ arm64, x64, x86; `neutral` serve a todos).
- `src/installed/arp_win.rs` usa `winreg` só leitura (modelo para `host_win.rs`).
- Porte: `docs/reference/winget-cli/AppInstallerCommonCore_Manifest_ManifestComparator.cpp` (filtros de arquitetura, plataforma, versão mínima, locale, escopo).
- S2: 48% sem `Scope`; 7,7% com tipo por instalador; `Dependencies` 15% (todas `PackageDependencies`); 3 exe sem silencioso.

#### What binds this task

1. `src/winget_install/select.rs` (puro): `pub struct Host { pub arch: String, pub os: [u32; 4], pub locale: String }`; `pub enum ScopeWanted { Prefer(Scope), Only(Scope) }` com `pub enum Scope { User, Machine }`; `pub enum Verdict { Native { interactive: bool }, Bridge(String) }`; `pub struct Selection { pub index: usize, pub installer: EffectiveInstaller, pub verdict: Verdict }`; `pub struct Refusal(pub String)`; `pub fn choose(installers: &[EffectiveInstaller], host: &Host, wanted: ScopeWanted) -> Result<Selection, Refusal>`.
2. Filtros (instalador fora ⇒ descartado): arquitetura aplicável ao host (`neutral` sempre); `platform` vazia ou contendo `Windows.Desktop`; `minimum_os_version` ≤ `host.os` comparando **número a número** (parte ausente = 0; texto inválido ⇒ descartado); com `Only(s)`, escopo igual a `s` ou ausente.
3. Ordenação dos que sobram (o primeiro vence; empate fica com a ordem do manifesto): (a) escopo — igual à preferência < ausente < outro; (b) arquitetura — nativa < `neutral` < reservas na ordem de `runs_on`; (c) locale — igual ao do host (sem caixa) < mesma língua (`pt-PT` para `pt-BR`) < ausente < outro. **Escopo vem antes de arquitetura.**
4. Nenhum sobra ⇒ `Refusal` com motivo em português: `nenhum instalador para esta arquitetura (<arch>)`, `exige Windows <versão> ou mais novo`, `nenhum instalador para escopo máquina|usuário`.
5. Veredito do escolhido: tipo `zip`, `portable`, `font`, `pwa`, `msstore` ou `Unknown` ⇒ `Bridge("tipo <nome>: instalado pelo winget")` (`Unknown("")` ⇒ `"tipo desconhecido: instalado pelo winget"`); `dependencies` com qualquer lista não vazia ⇒ `Bridge("tem dependências: instalado pelo winget")` (vale também para tipos nativos); `exe` sem `Silent` e sem `SilentWithProgress` ⇒ `Native { interactive: true }`; demais (msi, wix, burn, inno, nullsoft, msix, appx, exe com silencioso) ⇒ `Native { interactive: false }`.
6. `src/winget_install/host_win.rs`: `pub fn current() -> Host` — arquitetura de `bridge::host_arch()`, versão de `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion` (`CurrentMajorVersionNumber`, `CurrentMinorVersionNumber`, `CurrentBuildNumber`, `UBR`), locale de `HKCU\Control Panel\International\LocaleName`; leitura falha ⇒ `[10,0,0,0]`/`"en-US"` com `tracing::warn!`. Só `winreg` leitura (nenhuma feature nova).

**Files:**
- Create: `src/winget_install/select.rs`, `src/winget_install/host_win.rs` (já existem com uma linha `//!`)
- Test: `tests/it/wi_select.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_select.rs`, manifestos YAML no teste via `parse(..)?.effective()`; host padrão `x64`, `[10,0,22631,0]`, `pt-BR`):
1. `native_arch_beats_earlier_fallback`: `[x86, x64]` ⇒ índice 1; host `x86` ⇒ índice 0; `[x86]` só ⇒ índice 0 em host `x64` (reserva).
2. `arm64_falls_back_x64_then_x86`: host `arm64`: `[x86, x64]` ⇒ 1; `[x86]` ⇒ 0; `[x64, arm64]` ⇒ 1.
3. `user_preferred_over_earlier_machine`: `[x64 machine, x64 user]`: `Prefer(User)` ⇒ 1; `Prefer(Machine)` ⇒ 0.
4. `only_scope_refuses_when_absent`: `[x64 user]` com `Only(Machine)` ⇒ `Err`, texto contém `escopo máquina`; `[x64 sem escopo]` com `Only(Machine)` ⇒ `Ok(0)`.
5. `scope_beats_arch`: `[x64 machine, x86 user]` com `Prefer(User)` ⇒ índice 1.
6. `locale_exact_then_language_then_any`: `[en-US, pt-PT, pt-BR]` ⇒ 2; `[en-US, pt-PT]` ⇒ 1; `[en-US]` ⇒ 0.
7. `min_os_above_host_is_skipped`: `[x64 MinimumOSVersion 10.0.99999.0, x86]` ⇒ 1; só o primeiro ⇒ `Err` contendo `exige Windows`.
8. `os_version_numeric_not_text`: `MinimumOSVersion: 10.0.9.0` em host `[10,0,22631,0]` ⇒ aceito.
9. `bridge_reasons_one_fixture_each`: um manifesto por caso — `zip`, `portable`, `font`, `pwa`, `msstore`, tipo ausente, `msi` com `Dependencies.PackageDependencies` ⇒ `Verdict::Bridge(m)` com `m` contendo, respectivamente, `tipo zip`, `tipo portable`, `tipo font`, `tipo pwa`, `tipo msstore`, `tipo desconhecido`, `dependências`.
10. `exe_silent_rules`: `exe` sem switches ⇒ `Native { interactive: true }`; `exe` com só `SilentWithProgress` ⇒ `interactive: false`; `exe` com `Silent` ⇒ `false`.
11. `classic_types_are_native`: `msi`, `wix`, `burn`, `inno`, `nullsoft`, `msix`, `appx` ⇒ `Native { interactive: false }`.
12. `platform_filter`: `[x64 Platform: [Windows.Universal], x86 Platform: [Windows.Desktop]]` ⇒ 1.
13. `host_current_prints` `#[ignore = "live: lê o registro"]`: imprime `host_win::current()`; afirma só `os[0] >= 10` e `locale` não vazio.

#### Checks beyond the steps

1. Mutação: tirar `x86` das reservas de `x64` ⇒ teste 1 **PEGA**.
2. Mutação: ordenar arquitetura antes de escopo ⇒ teste 5 **PEGA**.
3. Mutação: comparar `MinimumOSVersion` como texto ⇒ teste 8 **PEGA**.
4. Mutação: ignorar `Dependencies` no veredito ⇒ teste 9 **PEGA**.
5. Real: `cargo test --release --quiet host_current_prints -- --ignored --nocapture` (teste 13); cole a linha.
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): installer selection (arch, scope, locale, refusals)`

---

### Task 174: Switches padrão e linha de comando (P3)

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `src/sources/winget/manifest.rs`: `InstallerKind`, `InstallerSwitches { silent, silent_with_progress, interactive, install_location, log, upgrade, custom }` (Task 170).
- Porte: `docs/reference/winget-cli/AppInstallerCommonCore_Manifest_ManifestCommon.cpp` (switches padrão por tipo) e `AppInstallerCLICore_Workflows_ShellExecuteInstallerHandler.cpp` (montagem dos argumentos e a forma `msiexec.exe /i "<arquivo>" …`).
- `src/install/msi_queue.rs:68` `admin_args` (modelo de aspas; não muda).

#### What binds this task

1. `src/winget_install/switches.rs` (puro): `pub enum Mode { Silent, SilentWithProgress, Interactive }`; `pub enum Program { Msiexec, Installer }`; `pub struct CommandLine { pub program: Program, pub args: String }`; `pub const LOG_TOKEN: &str = "<LOGPATH>";`; `pub fn mode_for(kind: &InstallerKind, sw: &InstallerSwitches) -> Mode`; `pub fn install_args(kind: &InstallerKind, sw: &InstallerSwitches, mode: Mode, upgrade: bool) -> Result<CommandLine, ArgsError>`; `pub fn fill_log(args: &str, log: &Path) -> Result<String, ArgsError>`; `pub fn msiexec_params(file: &Path, args: &str) -> String` (`/i "<file>" <args>`, sem espaço sobrando quando `args` é vazio); `pub fn validate(args: &str) -> Result<(), ArgsError>`.
2. Padrões (spec P3): msi/wix/burn — Silent `/quiet /norestart`, SilentWithProgress `/passive /norestart`, Log `/log "<LOGPATH>"`; nullsoft — Silent e SilentWithProgress `/S`, sem Log; inno — Silent `/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`, SilentWithProgress `/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART`, Log `/LOG="<LOGPATH>"`; exe — nenhum. Interactive padrão vazio para todos. msi/wix ⇒ `Program::Msiexec`; os demais ⇒ `Installer`; msix/appx e tipos da ponte ⇒ `Err(ArgsError::NoCommandLine)`.
3. Switch do manifesto **substitui** o padrão da mesma chave; `Custom` sempre acrescentado; `Upgrade` só com `upgrade == true`. Ordem: modo, Log, Custom, Upgrade, separados por um espaço, sem partes vazias. **InstallLocation fica fora** (sem `--location` nesta fase). Se o `ShellExecuteInstallerHandler.cpp` portado montar em outra ordem, ou acrescentar argumentos por escopo para MSI, **pare com `BLOCKED`** citando a linha do fonte (o orquestrador corrige os testes).
4. `mode_for`: tipo com padrão Silent ⇒ `Silent`; exe: manifesto `Silent` ⇒ `Silent`, senão `SilentWithProgress` ⇒ `SilentWithProgress`, senão `Interactive`.
5. `validate`: recusa NUL, `\r`, `\n` e mais de 32 KiB (`ArgsError::Invalid`); `install_args` valida o resultado. `fill_log`: troca todas as ocorrências de `LOG_TOKEN` pelo caminho; caminho com `"`, `\r`, `\n` ou NUL ⇒ `Err`.

**Files:**
- Create: `src/winget_install/switches.rs` (já existe com uma linha `//!`)
- Test: `tests/it/wi_switches.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_switches.rs`, `InstallerSwitches::default()` como base):
1. `msi_silent_default`: Msi, Silent, `upgrade=false` ⇒ `Program::Msiexec`, args `/quiet /norestart /log "<LOGPATH>"`.
2. `wix_and_burn_share_msi_defaults`: Wix ⇒ igual ao teste 1; Burn ⇒ `Program::Installer`, mesmos args.
3. `inno_silent_and_progress`: Silent ⇒ `/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>"`; SilentWithProgress ⇒ `/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>"`.
4. `nullsoft_silent`: ⇒ `/S`.
5. `manifest_silent_replaces_default_custom_appends`: Inno, manifesto `Silent: "/VERYSILENT /NOCANCEL"`, `Custom: "/MERGETASKS=!desktopicon"` ⇒ `/VERYSILENT /NOCANCEL /LOG="<LOGPATH>" /MERGETASKS=!desktopicon`.
6. `upgrade_only_on_upgrade`: Msi, `Custom: "ALLUSERS=1"`, `Upgrade: "REINSTALL=ALL"`: `false` ⇒ `/quiet /norestart /log "<LOGPATH>" ALLUSERS=1`; `true` ⇒ `/quiet /norestart /log "<LOGPATH>" ALLUSERS=1 REINSTALL=ALL`.
7. `exe_only_manifest`: Exe `Silent: "/q"` ⇒ modo Silent, args `/q`; Exe sem switches ⇒ `mode_for == Interactive`, args `""`; Exe só `SilentWithProgress: "/passive"` ⇒ modo `SilentWithProgress`, args `/passive`; Exe `Interactive: "/i"` em modo Interactive ⇒ `/i`.
8. `refuses_bad_args`: `Custom` com `\n`, com `\0`, e com 33 KiB ⇒ `Err(Invalid)`.
9. `fill_log_quotes_and_refuses_quote_in_path`: `fill_log(r#"/log "<LOGPATH>""#, r"C:\a b\x.log")` ⇒ `/log "C:\a b\x.log"`; caminho `C:\a"b\x.log` ⇒ `Err`.
10. `msiexec_params_form`: `msiexec_params(r"C:\x\a.msi", "/quiet")` ⇒ `/i "C:\x\a.msi" /quiet`; com `""` ⇒ `/i "C:\x\a.msi"`.
11. `no_command_line_types`: Msix, Appx, Zip, Portable ⇒ `Err(NoCommandLine)`.

#### Checks beyond the steps

1. Mutação: tirar `/norestart` do Silent de MSI ⇒ teste 1 **PEGA**.
2. Mutação: acrescentar `Upgrade` sempre ⇒ teste 6 **PEGA**.
3. Mutação: `Custom` antes do Log ⇒ teste 5 **PEGA**.
4. No relatório: as linhas do fonte portado que fixam cada padrão (arquivo:linha).
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): default switches per installer type and argument line`

---

### Task 175: Classificador de códigos de saída (P4)

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `src/sources/winget/manifest.rs`: `InstallerKind`, `ExpectedReturnCode { installer_return_code: i64, return_response: String, .. }` (Task 170).
- Porte: `docs/reference/winget-cli/AppInstallerCommonCore_Manifest_ManifestCommon.cpp` (`ExpectedReturnCodeEnum`, tabela padrão por tipo — `GetDefaultKnownReturnCodes` ou nome equivalente) e `AppInstallerCLICore_Workflows_InstallFlow.cpp` (precedência: sucesso, depois esperados).
- `src/install/msi_queue.rs:58` `attempt_from_code` (0/3010/1618 para o `msiexec /a`; não muda).
- S2: `ExpectedReturnCodes` em 2%, `InstallerSuccessCodes` em 1%.

#### What binds this task

1. `src/winget_install/codes.rs` (puro): `pub enum Category` com os valores do `ExpectedReturnCodeEnum` portado (no mínimo `PackageInUse`, `PackageInUseByApplication`, `InstallInProgress`, `FileInUse`, `MissingDependency`, `DiskFull`, `InsufficientMemory`, `InvalidParameter`, `NoNetwork`, `ContactSupport`, `RebootRequiredToFinish`, `RebootRequiredForInstall`, `RebootInitiated`, `CancelledByUser`, `AlreadyInstalled`, `Downgrade`, `BlockedByPolicy`, `SystemNotSupported`, `Custom`) mais `Unknown`; `pub enum Outcome { Ok, OkReboot { initiated: bool }, Failed { category: Category, code: u32, message: String } }`.
2. `pub fn classify_install(kind: &InstallerKind, code: i32, success: &[i64], expected: &[ExpectedReturnCode]) -> Outcome`: precedência (1) `0` ou `success` ⇒ `Ok`; (2) `expected` do manifesto; (3) tabela padrão do tipo (portada); (4) `Failed { Unknown }` com a mensagem `o instalador terminou com o código <n> (0x<hex>)`. **Toda comparação é `as u32`** (`i64` do manifesto truncado com `as u32`, `i32` do processo com `as u32`). Categoria `RebootRequiredToFinish` ⇒ `OkReboot { initiated: false }`; `RebootInitiated` ⇒ `OkReboot { initiated: true }`; as demais ⇒ `Failed` com `message(category)`.
3. `pub fn classify_uninstall(msiexec: bool, code: i32) -> Outcome`: só `0` é `Ok`; com `msiexec`, `3010` ⇒ `OkReboot { false }`, `1641` ⇒ `OkReboot { true }`, `1618` ⇒ `Failed { InstallInProgress }`, `1602` ⇒ `Failed { CancelledByUser }`.
4. `pub fn classify_msix(hresult: i32) -> Outcome` com a tabela MSIX portada; `pub fn category_of(response: &str) -> Category` (nomes do manifesto, `camelCase`, sem distinção de caixa; desconhecido ⇒ `Custom`); `pub fn message(c: Category) -> &'static str` em português (ex.: `InstallInProgress` ⇒ `outro instalador está rodando; tente de novo em instantes`); `pub const UAC_REFUSED: &str = "Permissão de administrador recusada.";`.
5. A tabela padrão vem do fonte portado, com o comentário de origem. Ela precisa conter, para msi/wix/burn: 1618 InstallInProgress, 1602 CancelledByUser, 1625 BlockedByPolicy, 112 DiskFull, 3010 RebootRequiredToFinish, 1641 RebootInitiated, 1638 AlreadyInstalled; para inno: 2 e 5 CancelledByUser, 8 RebootRequiredForInstall. Se o fonte divergir disso, **BLOCKED** com a linha.

**Files:**
- Create: `src/winget_install/codes.rs` (já existe com uma linha `//!`)
- Test: `tests/it/wi_codes.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_codes.rs`):
1. `zero_and_success_codes_are_ok`: `(Exe, 0, [], [])` ⇒ `Ok`; `(Msi, 3010, [3010], [])` ⇒ `Ok` (sucesso declarado vence o reinício padrão).
2. `msi_defaults`: Msi 3010 ⇒ `OkReboot{false}`; 1641 ⇒ `OkReboot{true}`; 1618 ⇒ `InstallInProgress`; 1602 ⇒ `CancelledByUser`; 1625 ⇒ `BlockedByPolicy`; 112 ⇒ `DiskFull`; 1638 ⇒ `AlreadyInstalled`; Wix 1618 e Burn 3010 iguais.
3. `inno_defaults_are_per_type`: Inno 2 ⇒ `CancelledByUser`, 5 ⇒ `CancelledByUser`, 8 ⇒ `Failed { RebootRequiredForInstall }`; Nullsoft 2 ⇒ `Unknown`; Msi 2 ⇒ `Unknown`.
4. `manifest_expected_beats_default`: Msi 1618 com `expected [{1618, contactSupport}]` ⇒ `ContactSupport`.
5. `success_beats_expected`: Inno 5 com `success [5]` e `expected [{5, cancelledByUser}]` ⇒ `Ok`.
6. `compares_as_u32`: `success [-1]` e código `-1` ⇒ `Ok`; `expected [{4294967295, diskFull}]` e código `-1` ⇒ `DiskFull`; `expected [{-2147024891, blockedByPolicy}]` e código `0x80070005_u32 as i32` ⇒ `BlockedByPolicy`.
7. `unknown_code_message`: Exe 7 ⇒ `Failed { Unknown, code: 7 }`, mensagem contém `código 7` e `0x7`.
8. `uninstall_only_zero`: `(false, 3010)` ⇒ `Failed`; `(true, 3010)` ⇒ `OkReboot{false}`; `(true, 1641)` ⇒ `OkReboot{true}`; `(false, 0)` ⇒ `Ok`; `(true, 1618)` ⇒ `InstallInProgress`.
9. `msix_table_entry`: `classify_msix(0x80073D02_u32 as i32)` (ERROR_PACKAGES_IN_USE) ⇒ `PackageInUse`; se a tabela portada não tiver essa entrada, **BLOCKED** com o trecho do fonte.
10. `response_names_case_insensitive`: `category_of("PackageInUse") == category_of("packageInUse") == PackageInUse`; `category_of("xyz") == Custom`.

#### Checks beyond the steps

1. Mutação: tabela padrão antes dos esperados do manifesto ⇒ teste 4 **PEGA**.
2. Mutação: comparar como `i64` em vez de `as u32` ⇒ teste 6 **PEGA**.
3. Mutação: `3010` de desinstalação sem msiexec vira `OkReboot` ⇒ teste 8 **PEGA**.
4. No relatório: arquivo:linha de cada tabela portada.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): exit code classification (success, expected, defaults per type)`

---

### Task 176: Ponte `winget install` cedo (P13a), CLI `--scope` e TUI "Instalar" de pacotes winget

**Modelo:** opus (revisor opus — mexe no protocolo do auxiliar elevado).

#### Where this fits

- `src/bridge/mod.rs:47` `BridgeJob` (`tool`, `target`, `name`, `installed`, `available`, `scope`), `:63` `bridge_target` (só upgrades), `:225` `valid_target`, `:243` `command` (só `upgrade`), `:142` `parse_scope`.
- `src/bridge/worker.rs:16` `Request { Upgrade { target }, SourceUpdate {}, Quit {} }` (`deny_unknown_fields`; variantes struct, ver comentário em `:20`), `:92` `serve` (monta um `BridgeJob` e chama `command`).
- `src/bridge/elevated.rs:82` `exec_for` (eleva winget com `scope == Some(Machine)`), `:213` `request_for` (mapeia o comando de volta para a `Request`; outro comando ⇒ `PermissionDenied`).
- `src/tui/jobs.rs:375` `run_bridge`, `:388` `run_bridge_elevated`, `:410` `run_bridge_as` (`JobInfo { kind: "bridge", .. }`), `:455` `bridge_run` (conferência de versão e nova tentativa da 136, hoje só para upgrade).
- `src/tui/app.rs:192` `Action` (`Bridge(BridgeJob)`), `:2122` `install_one` (toast `instalação na fase 2d` em `:2130`), `:2747` `search_actions` (nota `instalação na fase 2d` em `:2755`), `:2300`–`:2335` itens e notas da confirmação; `src/tui/run.rs:1108` `applicability` (modelo: lê o pacote, `merged_manifest` com `CDN_BASE`), `:1397` `spawn_bridge`.
- `src/install/target.rs:24` `WINGET_NOTE` e `:36` `candidate` (winget não selecionável); `src/cli/install_cmd.rs:527` `choose`, `:604` `install`; `src/cli/mod.rs:188` `Command::Install`.
- `src/job_log.rs:38` `JobInfo`, `:109` `JobLog::start` (`job.start`).
- Literais `BridgeJob { .. }` em testes: `tests/it/bridge.rs:78`, `elevated_worker.rs:25`, `job_log.rs:17`, `tui_jobs.rs:964`, `tui_state.rs:4048` e `:4718`.

#### What binds this task

1. `bridge/mod.rs`: `pub enum BridgeOp { Upgrade, Install { scope: Option<InstallScope> } }`; `BridgeJob` ganha `op: BridgeOp` (os literais existentes recebem `op: BridgeOp::Upgrade`; `bridge_target` monta `Upgrade`). `command` para `Install`: `winget install --id <target> --exact --source winget --silent --disable-interactivity --accept-package-agreements --accept-source-agreements`, mais `--scope user|machine` **só** quando `scope` é `Some` (passado só por `--scope` explícito; a preferência do Ajustes não vira `--scope`).
2. Protocolo: `Request::Install { target: String, scope: Option<ScopeArg> }` com `pub enum ScopeArg { User, Machine }` (`rename_all = "lowercase"`). `serve` valida o alvo com `valid_target` como no `Upgrade` e monta o `BridgeJob` com `op: Install`; `request_for` reconhece o comando de instalação. Nenhuma outra operação nova.
3. Elevação da instalação pela ponte: `BridgeJob.scope = Some(Machine)` quando `--scope machine` **ou** quando todos os instaladores do manifesto aplicáveis ao host (`bridge::runs_on` via `winget_applicability`, ou filtro equivalente por arquitetura) têm escopo efetivo `machine` (`MergedManifest::effective`, Task 170). Caso contrário `None` (o winget/instalador pede o UAC sozinho). `user` nunca elevado.
4. `tui/jobs.rs`: `run_bridge_as` usa `kind: "install"` para `Install`; `bridge_run` para `Install`: sucesso = `winget_updated(code)` e a versão escolhida (`winget_chosen_version`) comparada com `available` por `crate::core::version` (mais nova ou igual ⇒ ok; mais velha ⇒ `NoEffect` com nota); a nova tentativa com `winget source update` da 136 vale também para `Install` com o mesmo critério de código já usado. Resumo de sucesso `instalado via winget`.
5. **CLI:** `install_cmd::choose` aceita candidatos winget (o `target.rs` deixa de marcar winget como não selecionável; `WINGET_NOTE` sai). `install` com candidato winget chama `cli::winget_cmd::install_bridge(ctx, ui, &cand, scope, flags, stop)` (arquivo novo `src/cli/winget_cmd.rs`): lê o manifesto pelo `winget_hash` do catálogo (`details::merged_manifest`, `CDN_BASE`, versão = a do catálogo), imprime o resumo (`<nome> <versão> (winget) via winget`, linhas `Licença: <label> — <url>` de cada `Agreement`), `--dry-run` imprime a linha de comando (`bridge::command_text`) e sai; confirma com `proceed` (sem `--yes` e sem terminal ⇒ o erro de hoje); com `--yes` imprime `Licença aceita: <label> — <url>` por agreement antes de rodar; roda pelo `ElevatedBridge` (`SystemTransport::new(current_exe)`) com `tui::jobs::run_bridge_elevated` e `bridge::spawn_win::run`; manda `shutdown(5 s)` no fim, em todos os caminhos (guard). `Command::Install` ganha `--scope <user|machine>` (`value_parser` com mensagem em português); `--scope` com alvo do Scoop ⇒ erro `--scope só vale para pacotes do winget`.
6. `job_log.rs`: `pub fn start_accepting(info: JobInfo, agreements: &[String]) -> JobLog` que escreve o mesmo `job.start` com o campo extra `agreements` (`"<label> <url>"` unidos por `; `, ou `-`); `start` fica como está.
7. **TUI:** `search_actions`/`install_one` para entrada winget: sem a nota da 2d; `Cmd::PlanWingetInstall { id }` ⇒ `run.rs` lê o manifesto (como `applicability`) e responde `Msg::WingetPlanned(Result<(BridgeJob, Vec<String> /*agreements*/), String>)`; a confirmação mostra o item (`manager: "winget"`) e uma nota por agreement `Licença: <label> — <url>`; confirmar ⇒ `Action::Bridge(job)` com `op: Install`. `Action::label` para `Install` ⇒ `Instalando <nome> via winget`.

**Escopo vindo do Ajustes (resposta do orquestrador, 2026-10-18):** quando o usuário mudou o padrão para `machine` no Ajustes/`config.toml` e não passou `--scope`, a ponte passa `--scope machine`; se o winget responder que não há instalador aplicável para esse escopo (`0x8A150010` ou a mensagem equivalente — confira no `ShellExecuteInstallerHandler`/códigos do winget-cli e cite), tenta uma vez sem `--scope` e o progresso diz `"sem instalador para máquina; instalando para o usuário"`. Teste com exec falso nas duas ordens.

**Files:**
- Create: `src/cli/winget_cmd.rs` (já existe com uma linha `//!` e `pub mod winget_cmd;` em `src/cli/mod.rs`)
- Modify: `src/bridge/mod.rs`, `src/bridge/worker.rs`, `src/bridge/elevated.rs`, `src/tui/jobs.rs`, `src/tui/app.rs`, `src/tui/run.rs`, `src/install/target.rs`, `src/cli/install_cmd.rs`, `src/cli/mod.rs`, `src/job_log.rs`, testes com literal `BridgeJob` (`tests/it/bridge.rs`, `elevated_worker.rs`, `job_log.rs`, `tui_jobs.rs`, `tui_state.rs` — só a linha `op:`), testes de `target` que esperavam winget não selecionável (`tests/it/install_target.rs`, se houver; diga quais)
- Test: `tests/it/wi_bridge_install.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_bridge_install.rs`, exec falso, transporte falso do `ElevatedBridge` como em `elevated_worker.rs`):
1. `install_command_exact`: `Install { scope: None }` para `Git.Git` ⇒ `("winget", ["install","--id","Git.Git","--exact","--source","winget","--silent","--disable-interactivity","--accept-package-agreements","--accept-source-agreements"])`; `scope: Some(Machine)` ⇒ mesmo + `["--scope","machine"]`; `Some(User)` ⇒ `["--scope","user"]`.
2. `serve_runs_install_only_with_valid_target`: `{"op":"install","target":"Git.Git","scope":"machine"}` ⇒ o exec recebe o comando do teste 1 com `--scope machine`; alvos `a b`, `--override`, `x&y` ⇒ `error`, exec nunca chamado.
3. `install_request_refuses_bad_scope_and_extra_fields`: `"scope":"admin"` e campo `"cmd":"calc"` ⇒ `serve` para sem executar.
4. `request_for_maps_install`: `request_for` do comando do teste 1 ⇒ `Request::Install { target, scope }`; um comando de instalação com `--override x` acrescentado ⇒ `PermissionDenied`.
5. `machine_only_manifest_goes_elevated_user_never`: manifesto com dois instaladores `x64 machine` e `x86 machine` ⇒ job `scope == Some(Machine)` ⇒ `exec_for` elevado; manifesto `[x64 machine, x64 user]` ⇒ `None`, não eleva; `--scope user` ⇒ `Some(User)`, não eleva.
6. `install_success_and_older_version`: exec devolve 0 com a linha que nomeia a versão `2.0`: `available "2.0"` ⇒ `Ok`; `available "2.0.0"` ⇒ `Ok` (mesma versão com zeros); exec escolhe `1.9` com `available "2.0"` ⇒ `NoEffect`.
7. `job_start_lists_agreements`: `JobLog::start_accepting(info, ["EULA https://x/eula", "Privacy https://x/p"])` com assinante de teste (modelo de `tests/it/job_log.rs`) ⇒ a linha `job.start` tem `agreements=EULA https://x/eula; Privacy https://x/p`; `start` normal não tem o campo.
8. `cli_yes_prints_each_agreement`: `winget_cmd::agreement_lines(&[a1, a2], accepted = true)` ⇒ `["Licença aceita: EULA — https://x/eula", "Licença aceita: Privacy — https://x/p"]`; `accepted = false` ⇒ `Licença: …`; agreement sem label usa `Licença` e sem URL usa o texto `Agreement` truncado em 200 caracteres.
9. `scope_flag_parses_and_rejects`: `wingetrs install Git.Git --scope machine` ⇒ `scope: Some(Machine)`; `--scope admin` ⇒ erro de parse com `user` e `machine` na mensagem; `install --help` lista `--scope` (help do subcomando).
10. `tui_search_install_winget_enabled`: `search_actions` de uma entrada winget ⇒ `Instalar` sem nota; `install_one` ⇒ `Cmd::PlanWingetInstall { id }`; `Msg::WingetPlanned(Ok(..))` com 1 agreement ⇒ modal com a nota `Licença: EULA — https://x/eula`.

#### Checks beyond the steps

1. Mutação: `serve` sem `valid_target` no braço `Install` ⇒ teste 2 **PEGA**.
2. Mutação: `--scope` acrescentado mesmo com `scope: None` ⇒ teste 1 **PEGA**.
3. Mutação: elevar quando algum (não todos) instalador é `machine` ⇒ teste 5 **PEGA**.
4. Os testes da 137 (`elevated_worker`) e da 134/136 (`tui_jobs`) continuam verdes.
5. Binário real: `target/release/wingetrs.exe install Git.Git --dry-run` e `install Microsoft.PowerToys --dry-run` (só imprime; nada roda); cole a saída. Não rode sem `--dry-run`.
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): winget install through the bridge (CLI and TUI), --scope, accepted agreements in the log`

---

### Task 177: Download do instalador e arquivo conferido (P5)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/install/download.rs:263` `fetch_all(http, data, &[Source], progress)`: baixa em streaming para `tmp\dl-*.part` (`http.get_to_file`), confere o hash em streaming (`hash_file`, 64 KiB) e renomeia para `blobs\<sha256>`; cache conferido em `:156` `cached`. **O defeito da spec (`download.rs:92` carregando tudo na memória) já não existe:** o download e o hash já são em streaming. Esta tarefa só reaproveita.
- `src/install/manifest.rs:85` `Source { url, file_name, hash: HashSpec, extract_dir, extract_to }`, `:52` `HashSpec { algo, hex }`, `:44` `HashAlgo::Sha256` (só leitura; arquivo da 2c, não edite).
- `src/http.rs:36` `redirect_allowed` (https→http recusado salvo ajuste) — já vale para este download.
- `src/cache/mod.rs` `DataDir::root()`/`blobs_dir()`/`tmp_dir()`; `src/install/safepath.rs:6` `safe_components`; `src/bridge/mod.rs:225` `valid_target`.
- `src/sources/winget/manifest.rs`: `EffectiveInstaller { url, sha256, kind, .. }`, `InstallerKind` (Task 170).

#### What binds this task

1. `src/winget_install/fetch.rs`: `pub async fn fetch_installer(http: &Http, data: &DataDir, id: &str, version: &str, inst: &EffectiveInstaller, progress: Progress<'_>) -> Result<Materialized, FetchError>`; `pub enum FetchError { NotHttps(String), BadSha, BadId(String), Download(DownloadError), Io(String), Integrity }` (`thiserror`, mensagens em português).
2. Antes de qualquer requisição: URL começa com `https://` (sem caixa) senão `NotHttps`; `sha256` com 64 hex senão `BadSha`; `id` passa em `valid_target(Tool::Winget, id)` senão `BadId`.
3. Baixa com `download::fetch_all` (um `Source` com `HashSpec { Sha256, hex minúsculo }`, `file_name` = nome final). Materializa em `<root>\installers\<id>\<ver>\<id>-<ver>.<ext>`: `ver` com todo caractere fora de `[A-Za-z0-9._+-]` trocado por `_`, e o segmento final aceito por `safe_components` (um componente só); `ext` por `pub fn extension(kind) -> &'static str` (msi/wix ⇒ `msi`, msix ⇒ `msix`, appx ⇒ `appx`, demais ⇒ `exe`). Remove um arquivo antigo no destino e cria **hardlink** do blob (`std::fs::hard_link`); se falhar, copia.
4. `pub struct Materialized { path: PathBuf, sha256: String }` com `Drop` que apaga o arquivo materializado (o blob fica no cache) e a pasta `<ver>` se ficou vazia; erro de remoção só vai para o log.
5. `src/winget_install/verified_win.rs`: `pub struct VerifiedFile` com `pub fn open(path: &Path, sha256: &str) -> Result<VerifiedFile, FetchError>`: abre com `OpenOptionsExt::share_mode(FILE_SHARE_READ)` (nega escrita, exclusão e renome por outros), calcula o SHA-256 **pelo próprio handle** (streaming, 64 KiB) e compara; divergiu ⇒ `Integrity`. O handle fica aberto enquanto o `VerifiedFile` vive. `pub fn path(&self) -> &Path`.

**Files:**
- Create: `src/winget_install/fetch.rs`, `src/winget_install/verified_win.rs` (já existem com uma linha `//!`)
- Test: `tests/it/wi_fetch.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_fetch.rs`, wiremock servindo os bytes, SHA calculado no teste, `DataDir` em tempdir; o manifesto do instalador montado por YAML + `effective()`):
1. `downloads_verifies_and_materializes`: corpo de 8 MiB ⇒ `path` termina em `installers\Pub.App\1.0\Pub.App-1.0.exe`, bytes iguais ao corpo, `VerifiedFile::open(path, sha)` ok.
2. `last_byte_wrong_leaves_nothing`: o último byte trocado em relação ao SHA ⇒ `Err(Download(HashMismatch))`; não existem `<root>\installers\Pub.App`, `<root>\blobs\<sha>` nem nenhum `*.part` em `<root>\tmp` (o `tmp` existe: prova que o download começou).
3. `http_url_refused_without_request`: URL `http://…` ⇒ `NotHttps`; mock com `.expect(0)`.
4. `cached_blob_no_second_request`: duas chamadas, mock `.expect(1)` ⇒ as duas `Ok`, o segundo `Materialized` com os mesmos bytes.
5. `bad_sha_and_bad_id_refused`: sha `xyz` ⇒ `BadSha`; id `../x` ⇒ `BadId`; mock `.expect(0)`.
6. `version_is_sanitized`: versão `1.0/../x` ⇒ o caminho começa com `<root>\installers\Pub.App\` e o nome não contém `..` como componente nem `/`.
7. `extension_by_kind`: Msi/Wix ⇒ `msi`, Msix ⇒ `msix`, Appx ⇒ `appx`, Inno/Nullsoft/Burn/Exe ⇒ `exe`.
8. `verified_open_detects_change`: materializa, sobrescreve o arquivo com outros bytes de mesmo tamanho, `VerifiedFile::open` ⇒ `Integrity`.
9. `verified_handle_blocks_writers_and_rename` (`#[cfg(windows)]`): com o `VerifiedFile` aberto ⇒ abrir para escrita falha com `raw_os_error() == Some(32)`, `remove_file` falha, `rename` falha; depois do `drop` as três funcionam.
10. `materialized_drop_removes_link_keeps_blob`: depois do `drop` do `Materialized`, o arquivo em `installers\…` sumiu e `blobs\<sha>` continua.

#### Checks beyond the steps

1. Mutação: `share_mode(FILE_SHARE_READ)` ⇒ `share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)` ⇒ teste 9 **PEGA**.
2. Mutação: tirar a checagem de `https://` ⇒ teste 3 **PEGA**.
3. Mutação: `VerifiedFile::open` hasheia pelo caminho (reabrindo) em vez do handle — diga no relatório por que a mutação é ou não detectável e, se não for, por que a forma pelo handle continua obrigatória.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): installer download, materialization and share-deny verified file`

---

### Task 178: Lançador sem elevação, regra de elevação e `is_elevated` (P6)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/bridge/elevate_win.rs:278` `launch` (modelo de `ShellExecuteExW` com `CoInitializeEx` STA, `SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC`, `ERROR_CANCELLED`), `:581` `program_files` (`SHGetKnownFolderPath`, nunca variável de ambiente).
- `src/installed/mod.rs:21` `Hive`; `src/sources/winget/manifest.rs` `InstallerKind` (Task 170).
- ARMADILHAS "API Win32 receitada no brief sem sonda": confira o comportamento real de cada chamada com um teste local antes de confiar.
- S2: `elevatesSelf` em 8%, `elevationRequired` em 2%, 48% sem `Scope`.

#### What binds this task

1. `src/winget_install/launch.rs` (puro): `pub enum Show { Normal, Hidden }`; `pub struct LaunchSpec { pub program: PathBuf, pub params: String, pub dir: Option<PathBuf>, pub show: Show }`; `pub trait Launcher: Send + Sync { fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32>; }` (espera o fim, devolve o código); `pub enum Elevation { Never, Helper }`.
2. `pub fn install_elevation(kind: &InstallerKind, scope: Option<&str>, requirement: Option<&str>) -> Elevation`, nesta ordem: msix/appx ⇒ `Never`; `elevationProhibited` ⇒ `Never`; `elevationRequired` ⇒ `Helper`; escopo `user` ⇒ `Never`; escopo `machine` ⇒ `Helper`; escopo ausente e tipo msi/wix/burn ⇒ `Helper`; demais ⇒ `Never` (o instalador pede o UAC sozinho). Comparações sem caixa.
3. `pub fn uninstall_elevation(hive: Hive, msiexec: bool, program_in_admin_folder: bool) -> Elevation`: `User` ⇒ `Never`; máquina com `msiexec` ou programa em pasta de administrador ⇒ `Helper`; senão `Never`.
4. `src/winget_install/launch_win.rs`: `pub struct ShellLauncher;` com `Launcher`: numa thread nova (STA, `CoInitializeEx`/`CoUninitialize`), `ShellExecuteExW` verbo `open`, `SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI`, `nShow` de `Show`, `lpDirectory` de `dir`; `WaitForSingleObject(INFINITE)`, `GetExitCodeProcess`; a thread é juntada sempre e o pânico propagado com `resume_unwind`. Arquivo inexistente ⇒ `ErrorKind::NotFound`; `ERROR_CANCELLED` ⇒ `ErrorKind::PermissionDenied`. `hProcess` nulo (o shell não criou processo) ⇒ erro, nunca código 0 inventado.
5. `pub fn is_elevated() -> std::io::Result<bool>` (`OpenProcessToken` + `GetTokenInformation(TokenElevation)`); `pub fn msiexec_path() -> std::io::Result<PathBuf>` = `SHGetKnownFolderPath(FOLDERID_System)` + `msiexec.exe` (nunca PATH, nunca `%SystemRoot%`).
6. Sem features novas do `windows` (as necessárias já estão habilitadas; se faltar alguma, diga qual e por quê).

**Decisão do orquestrador (revisão da 178, 2026-10-18):** escopo `user` nunca vai ao auxiliar, nem com `ElevationRequirement: elevationRequired` (o instalador pede UAC sozinho se precisar); escopo desconhecido conta como ausente.

**Files:**
- Create: `src/winget_install/launch.rs`, `src/winget_install/launch_win.rs` (já existem com uma linha `//!`)
- Test: `tests/it/wi_launch.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_launch.rs`):
1. `install_elevation_truth_table`: tabela com pelo menos — `(Msi, None, None) ⇒ Helper`; `(Msi, Some("user"), None) ⇒ Never`; `(Inno, None, None) ⇒ Never`; `(Inno, Some("machine"), None) ⇒ Helper`; `(Nullsoft, Some("Machine"), Some("elevatesSelf")) ⇒ Helper`; `(Exe, None, Some("elevatesSelf")) ⇒ Never`; `(Exe, Some("user"), Some("elevationRequired")) ⇒ Helper`; `(Msi, Some("machine"), Some("elevationProhibited")) ⇒ Never`; `(Msix, Some("machine"), Some("elevationRequired")) ⇒ Never`; `(Burn, None, None) ⇒ Helper`; `(Wix, Some("user"), None) ⇒ Never`.
2. `uninstall_elevation_rules`: `(User, true, true) ⇒ Never`; `(MachineX86, true, false) ⇒ Helper`; `(MachineX64, false, true) ⇒ Helper`; `(MachineX64, false, false) ⇒ Never`.
3. `cmd_exit_code_is_returned` (`#[cfg(windows)]`): `t.cmd` em tempdir com `@exit /b 7`, `ShellLauncher.run(LaunchSpec { program: t.cmd, params: "", dir: None, show: Hidden })` ⇒ `Ok(7)`.
4. `params_reach_the_program` (`#[cfg(windows)]`): `t.cmd` com `@if "%~1"=="ok" (exit /b 3) else (exit /b 4)`, params `ok` ⇒ 3; params `no` ⇒ 4.
5. `dir_is_the_working_directory` (`#[cfg(windows)]`): `t.cmd` com `@if exist marker.txt (exit /b 5) else (exit /b 6)`, `marker.txt` só numa segunda pasta passada em `dir` ⇒ 5.
6. `missing_file_is_not_found` (`#[cfg(windows)]`) ⇒ `ErrorKind::NotFound`.
7. `msiexec_path_is_system32` (`#[cfg(windows)]`): absoluto, termina em `System32\msiexec.exe` (sem caixa), existe.
8. `is_elevated_answers` (`#[cfg(windows)]`): `Ok(_)` (não afirma o valor; o terminal pode estar elevado).
9. Teste unitário em `launch_win.rs` (`#[cfg(test)]`): o executor de STA com um closure que entra em pânico ⇒ `std::panic::catch_unwind` vê o pânico (propagado, não engolido).

#### Checks beyond the steps

1. Mutação: escopo `user` vira `Helper` ⇒ teste 1 **PEGA**.
2. Mutação: `elevationRequired` checado depois do escopo `user` ⇒ teste 1 **PEGA**.
3. Mutação: tirar `SEE_MASK_NOCLOSEPROCESS` ⇒ teste 3 **PEGA** (deve falhar com erro, não com 0).
4. Mutação: `join` do STA com `.ok()` — o `check_rules` barra; diga isso no relatório em vez de provar.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): unelevated ShellExecute launcher, elevation rule and is_elevated`

---

### Task 179: MSIX instalar e remover (P7)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/installed/appx_win.rs:6` usa `windows::Management::Deployment::PackageManager` (`:79` `PackageManager::new`, `FindPackagesByUserSecurityId`).
- `src/winget_install/codes.rs` `classify_msix(hresult) -> Outcome` (Task 175).
- Porte: `docs/reference/winget-cli/AppInstallerCommonCore_Deployment.cpp` (opções de implantação, remoção por full name).
- Feature `Foundation_Collections` (traz `Foundation`, com `Uri`) e `Management_Deployment` já habilitadas.

#### What binds this task

1. `src/winget_install/msix.rs` (puro): `pub fn file_uri(path: &Path) -> Option<String>` ⇒ `file:///C:/a%20b/x.msix` (barras normais; percent-encoding de espaço, `#`, `%`, `?` e de todo byte fora de ASCII imprimível; caminho relativo ⇒ `None`).
2. `src/winget_install/msix_win.rs`: `pub fn add_package(file: &Path) -> Result<(), MsixError>` com `AddPackageAsync(Uri, None, DeploymentOptions::None)` (decisão: **não** fechar o app à força; `PackageInUse` vira mensagem para o usuário fechar o app — registre no relatório o que o `MSIXInstallFlow.cpp` faz, para o orquestrador levar ao usuário se divergir); `pub fn remove_package(family: &str) -> Result<usize, MsixError>` acha os full names por `FindPackagesByUserSecurityIdPackageFamilyName("", family)` e chama `RemovePackageAsync` em cada um (devolve quantos); `pub struct MsixError { pub hresult: i32, pub text: String }` (código e `ErrorText` do resultado). A espera do `IAsyncOperationWithProgress` é bloqueante (`get()`); quem chama roda em `spawn_blocking` aguardado.
3. `pub trait MsixOps: Send + Sync { fn add(&self, file: &Path) -> Result<(), MsixError>; fn remove(&self, family: &str) -> Result<usize, MsixError>; }` em `msix.rs`, implementado por `pub struct SystemMsix` em `msix_win.rs`; `pub fn outcome(e: &MsixError) -> codes::Outcome` = `classify_msix(e.hresult)`.
4. Só escopo de usuário (nada de `ProvisionPackageForAllUsers`).

**Files:**
- Create: `src/winget_install/msix.rs`, `src/winget_install/msix_win.rs` (já existem com uma linha `//!`)
- Test: `tests/it/wi_msix.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_msix.rs`):
1. `file_uri_encodes`: `C:\a b\x.msix` ⇒ `file:///C:/a%20b/x.msix`; `C:\a#b\x.msix` ⇒ `file:///C:/a%23b/x.msix`; `C:\100%\x.msix` ⇒ `file:///C:/100%25/x.msix`; `C:\ção\x.msix` ⇒ bytes UTF-8 em `%XX`; `x.msix` ⇒ `None`.
2. `outcome_uses_msix_table`: `outcome(&MsixError { hresult: 0x80073D02_u32 as i32, .. })` ⇒ `Failed { PackageInUse, .. }`.
3. `live_add_and_remove` `#[ignore = "live: implanta e remove um MSIX"]`: implanta um `.msix` que o usuário indicar por `WINGETRS_TEST_MSIX` e o remove pela família.

#### Checks beyond the steps

1. Mutação: não codificar espaço em `file_uri` ⇒ teste 1 **PEGA**.
2. Mutação: `remove_package` remove só o primeiro full name — diga no relatório como provou ou por que só a prova ao vivo alcança.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): MSIX add and remove through PackageManager`

---

### Task 180: Auxiliar elevado — operação "rodar instalador conferido" (D3) com sonda de TOCTOU

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/bridge/worker.rs:16` `Request` (`deny_unknown_fields`, variantes struct), `:30` `Reply { Line, End, Error }`, `:65` `read_line` (teto 64 KiB), `:92` `serve(reader, writer, exec: BridgeExec)`.
- `src/bridge/elevated.rs:16` `Helper`, `:26` `Transport`, `:37` `ElevatedBridge`, `:120` `ensure_started` (estado `NotStarted | Running | Refused`; recusa vale até reiniciar), `:239` `ask` (envia e repassa linhas até `End`/`Error`).
- `src/bridge/elevate_win.rs:482` `worker_main` (pipe, PID do servidor, `ParentWatch`, `serve(.., &mut run_winget)`), `:529` `run_winget`, `:581` `program_files` (known folder).
- `src/cli/mod.rs:919`: o `bridge-worker` roda antes de `DataDir`/log (nada em pasta do usuário).
- `src/winget_install/switches.rs` `validate`, `fill_log`, `msiexec_params`, `LOG_TOKEN` (Task 174); `launch_win::ShellLauncher`, `msiexec_path` (Task 178); `verified_win::VerifiedFile` (Task 177); `install::msiexec_log::tail_text` (`src/install/msiexec_log.rs:52`).
- Desenho do plano (TOCTOU): cópia para `<FOLDERID_ProgramFiles>\wingetrs\staging\<128 bits hex>\` com DACL protegida `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`, SHA da cópia por handle `FILE_SHARE_READ`, execução de lá. Motivo: DLL plantada ao lado do instalador numa pasta do usuário seria carregada como admin.

#### What binds this task

0. **Sonda antes do código** (registre a saída no relatório; nada elevado — isso fica no roteiro ao vivo):
   - `icacls "C:\Program Files"` ⇒ confirme que `BUILTIN\Users` só tem `(RX)`/`(GR,GE)` herdáveis e nenhum direito de escrita/criação; se tiver, **BLOCKED**.
   - Confirme no fonte do std (`library/std/src/os/windows/fs.rs`) que `share_mode` passa direto ao `CreateFileW`.
   - Confirme com um teste local (sem elevar) que um arquivo aberto com `FILE_SHARE_READ` pode ser executado por `ShellExecuteExW` (`.cmd` aberto pelo `VerifiedFile` e lançado pelo `ShellLauncher` ⇒ código certo). Se falhar, **BLOCKED** (o desenho precisa do handle aberto durante a execução).
1. Protocolo: `Request::RunInstaller { source: String, sha256: String, file_name: String, kind: ElevatedKind, params: String }` com `pub enum ElevatedKind { Msi, Wix, Burn, Inno, Nullsoft, Exe }` (`lowercase`; msix fora). Resposta: linhas (`Reply::Line`, no máximo 40 do fim do log do instalador, cada uma ≤ `LINE_MAX`), depois `End { code }` ou `Error`.
2. Validação pura (em `worker.rs`, antes de tocar o disco), cada uma com recusa ⇒ `Reply::Error`, nada executado: `source` absoluto com letra de unidade (`X:\`), sem componente `..`, sem `\\?\`/UNC, ≤ 32 767; `sha256` 64 hex; `file_name` casa `^[A-Za-z0-9][A-Za-z0-9._+-]{0,127}\.(exe|msi)$` com `.msi` exatamente para msi/wix e `.exe` para os demais; `params` passa em `switches::validate`; para msi/wix, `pub fn msi_params_allowed(params: &str) -> bool`: tokens separados por espaço fora de aspas, cada um sendo `/quiet`, `/passive`, `/qn`, `/qb`, `/norestart`, `/promptrestart`, `/log` seguido de um token entre aspas igual a `"<LOGPATH>"`, ou `PROPRIEDADE=valor` com nome `[A-Z_][A-Z0-9_.]*` (sem caixa) e valor sem aspas internas não balanceadas; qualquer outro switch (`/i`, `/x`, `/a`, `/p`, `/f*`, `/j*`, `/package`, `/uninstall`, `/update`, `/l*`…) ⇒ recusa.
3. `serve` passa a receber `ops: &mut dyn WorkerOps` com `pub trait WorkerOps { fn run(&mut self, program: &str, args: &[String], on_line: &mut dyn FnMut(&str)) -> std::io::Result<(i32, Vec<String>)>; fn run_installer(&mut self, req: &InstallerRequest, on_line: &mut dyn FnMut(&str)) -> std::io::Result<i32>; }` (`InstallerRequest` = os campos validados). `Upgrade`/`Install`/`SourceUpdate` continuam iguais pelo `run`. Adapte `tests/it/elevated_worker.rs` à nova assinatura sem mudar o que cada teste prova.
4. `src/bridge/staging_win.rs` (lado do auxiliar): raiz `program_files()?.join("wingetrs").join("staging")`, criada se faltar; recusa se a raiz ou `wingetrs` for ponto de reanálise (`FILE_ATTRIBUTE_REPARSE_POINT`); pasta da execução `<raiz>\<32 hex de BCryptGenRandom>` criada com a DACL protegida acima (`ConvertStringSecurityDescriptorToSecurityDescriptorW`). Cópia: lê `source` em streaming, escreve em `<run>\<file_name>`, teto 8 GiB (passou ⇒ erro), SHA-256 durante a cópia; divergiu ⇒ apaga a pasta e `Error("o instalador não confere com o hash")`. Depois `VerifiedFile::open(cópia, sha256)` (handle mantido até o fim). Programa: msi/wix ⇒ `msiexec_path()` com `msiexec_params(cópia, params_preenchidos)`; demais ⇒ a cópia com `params_preenchidos`; `<LOGPATH>` ⇒ `<run>\install.log` via `fill_log`. Lança com `ShellLauncher` (`Show::Normal`, `dir = <run>`), espera, lê o fim do log com `tail_text` (janela 8 KiB) e manda as linhas; apaga a pasta da execução num guard (`Drop`) em todos os caminhos. Na partida do auxiliar: apaga pastas da raiz com mais de 24 h (só as que casam 32 hex).
5. Cliente (`elevated.rs`): `pub enum ElevatedError { Refused, Lost(String), Remote(String) }`; `pub trait ElevatedRunner: Send + Sync { fn run_installer(&self, req: &InstallerRequest, on_line: &mut dyn FnMut(&str)) -> Result<i32, ElevatedError>; }` implementado por `ElevatedBridge` (inicia o auxiliar se preciso, como `exec_for`; estado `Refused` ⇒ `Refused` sem novo UAC). O `job.end` desses jobs leva `elevated=true`.
6. O auxiliar continua sem `DataDir` e sem log em arquivo: o que precisa registrar volta pelo pipe (linhas) e o cliente registra.

**Ambiente do instalador elevado (resposta do orquestrador, 2026-10-18):** o auxiliar **não** repassa `TEMP`/`TMP` do usuário: define os dois para `<pasta de execução>	mp` (criada com a mesma DACL, apagada no fim junto com a pasta). Motivo: instalador elevado que extrai em `%TEMP%` do usuário e roda de lá carrega DLL plantada, o mesmo problema da cópia conferida. O resto do ambiente segue herdado. Teste puro do ambiente montado (sem `TEMP`/`TMP` do usuário) e item no roteiro ao vivo da 188.

**Files:**
- Create: `src/bridge/staging_win.rs` (já existe com uma linha `//!` e o `mod` com `#[cfg(windows)]` em `src/bridge/mod.rs`)
- Modify: `src/bridge/worker.rs`, `src/bridge/elevated.rs`, `src/bridge/elevate_win.rs`, `tests/it/elevated_worker.rs` (só a adaptação de assinatura)
- Test: `tests/it/elevated_install.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/elevated_install.rs`, transporte em memória e `WorkerOps` falso que registra chamadas):
1. `run_installer_reaches_ops_with_fields`: requisição válida (msi, `C:\U\blobs\a`, sha válido, `App-1.0.msi`, `/quiet /norestart /log "<LOGPATH>" ALLUSERS=1`) ⇒ `run_installer` chamado uma vez com esses campos; linhas e `End { code: 0 }` chegam na ordem.
2. `bad_fields_never_reach_ops`: um caso por recusa — `source` relativo, com `..`, UNC `\\h\s\a`, `\\?\C:\a`; sha de 63 chars e com `g`; `file_name` `..\a.msi`, `a.msi` para kind `exe`, `a.exe` para `msi`, `a b.exe`, 129 chars; `params` com `\n` ⇒ cada um `Reply::Error`, `ops` nunca chamado.
3. `msi_params_allowlist`: aceitos — `/quiet /norestart /log "<LOGPATH>"`, `ALLUSERS=1 ADDLOCAL=ALL`, `/passive`; recusados — `/i "C:\x.msi"`, `/x {G}`, `/a x`, `/p p.msp`, `/fa`, `/package x`, `/l*v "C:\t.log"`, `/log "C:\t.log"`, `TRANSFORMS="a`.
4. `unknown_kind_and_extra_field_close`: `"kind":"msix"` e campo extra `"cmd":"calc"` ⇒ `serve` para sem executar.
5. `refused_runner_does_not_retry`: transporte falso que recusa ⇒ `run_installer` ⇒ `Err(Refused)`; uma segunda chamada ⇒ `Err(Refused)` sem chamar `start` de novo (contador de partidas == 1).
6. `upgrade_and_install_still_work`: `Upgrade` e `Install` pelo `run` como antes (um caso cada).
7. `copy_then_verify_rejects_tampered_source` (`#[cfg(windows)]`, sem elevar: chama a função de cópia+conferência de `staging_win` com uma raiz de teste em tempdir passada por parâmetro — a raiz de produção vem de `program_files`, a de teste do teste): fonte com SHA errado ⇒ erro e a pasta da execução não existe depois; fonte certa ⇒ cópia conferida e aberta.
8. `staging_dacl_has_no_users` (`#[cfg(windows)]`, mesma função de criação com raiz em tempdir): o SDDL lido da pasta criada (`GetNamedSecurityInfoW` + `ConvertSecurityDescriptorToStringSecurityDescriptorW`) contém `D:P` e só ACEs de `SY` e `BA`.
9. `live_run_installer_once` `#[ignore = "live: pede UAC e instala um MSI de teste"]`: pelo `ElevatedBridge` real, o MSI pequeno indicado em `WINGETRS_TEST_MSI` (o usuário escolhe), kind `msi`, params `/quiet /norestart /log "<LOGPATH>"` ⇒ código 0, linhas do log chegam, e `C:\Program Files\wingetrs\staging` não tem a pasta da execução no fim.

#### Checks beyond the steps

1. Mutação: `msi_params_allowed` aceita `/x` ⇒ teste 3 **PEGA**.
2. Mutação: não recusar `..` em `source` ⇒ teste 2 **PEGA**.
3. Mutação: rodar o arquivo de origem em vez da cópia — diga no relatório qual teste pega; se nenhum puder (exige elevação), deixe isso explícito e inclua a verificação no roteiro ao vivo.
4. Roteiro ao vivo para o usuário (no relatório): pedir UAC uma vez; instalar o MSI de teste; conferir que `C:\Program Files\wingetrs\staging` fica vazio depois; conferir com `icacls` a pasta durante a execução (pausa com um MSI lento).
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): elevated helper runs a hash-verified installer copied to an admin-only folder`

---

### Task 181: Fluxo de instalação nativa (P8)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/winget_install/select.rs` `choose(&[EffectiveInstaller], &Host, ScopeWanted) -> Result<Selection, Refusal>`, `Verdict` (Task 173); `switches.rs` `mode_for`, `install_args`, `fill_log`, `msiexec_params` (174); `codes.rs` `classify_install`, `UAC_REFUSED` (175); `fetch.rs` `fetch_installer`, `Materialized`, `verified_win::VerifiedFile` (177); `launch.rs` `Launcher`, `LaunchSpec`, `install_elevation`, `launch_win::msiexec_path` (178); `msix.rs` `MsixOps` (179); `bridge::elevated::ElevatedRunner`, `InstallerRequest`, `ElevatedKind` (180).
- `src/install/msi_queue.rs:79` `run_queued(policy, busy, launch, sleep, status)`, `:28` `POLICY`, `:122` `lock_msi`; `src/install/msiexec_win.rs:21` `installer_busy()`.
- `src/install/executor.rs:96` `LaunchGate` (`try_launch` antes de lançar; `try_cancel` no Ctrl+C) — só leitura.
- `src/install/progress.rs:22` `Phase` (enum exaustivo); consumidores: `src/cli/install_cmd.rs` (`LineProgress::update`, ~:323), `src/cli/output.rs`, `src/tui/jobs.rs:86` `progress_msg`, `src/tui/app.rs` (barra de status). Confira com `grep -rn "Phase::" src`.
- `src/installed/mod.rs:36` `InstalledApp` (`product_code`, `upgrade_code`, `family_name`, `name`, `publisher`), `:182` `scan_all`; `src/installed/normalize.rs` (normalização de nome/publisher usada na correlação).
- `src/install/layout.rs:43` `state_db()`, `:46` `install_lock()`, `:50` `msi_lock()`; `src/cache/mod.rs` `UpdateLock`, `DataDir::log_dir()`.

#### What binds this task

1. `src/winget_install/plan.rs`: `pub struct WingetPlan { id, name, version, publisher: Option<String>, index: usize, installer: EffectiveInstaller, mode: Mode, command: Option<CommandLine> /* None p/ msix */, elevation: Elevation, interactive: bool, agreements: Vec<Agreement>, upgrade_from: Option<String> }` e `pub fn plan(manifest: &MergedManifest, host: &Host, wanted: ScopeWanted, upgrade_from: Option<String>) -> Result<PlanOrBridge, Refusal>` com `pub enum PlanOrBridge { Native(WingetPlan), Bridge(String /*motivo*/) }`. `impl Display for WingetPlan`: a **mesma** função serve ao `--dry-run` e às confirmações (CLI e TUI): `<nome> <versão> (<tipo>, <escopo ou "escopo do instalador">)`, `Instalador: <url>`, `Linha de comando: <programa> <args com <LOGPATH>>`, `pede UAC` quando `Helper`, `abre a janela do instalador (sem modo silencioso)` quando interativo, uma linha `Licença: <label> — <url>` por agreement.
2. `src/winget_install/ops.rs`: `pub struct WingetCtx<'a> { data, http, wait: LockWait, progress: Progress<'a>, gate: LaunchGate, launcher: &'a dyn Launcher, elevated: &'a dyn ElevatedRunner, msix: &'a dyn MsixOps, scan: &'a (dyn Fn() -> Vec<InstalledApp> + Send + Sync), msiexec: PathBuf, busy: &'a (dyn Fn() -> bool + Send + Sync), sleep: &'a (dyn Fn(Duration) + Send + Sync) }` e `pub async fn install(ctx: &WingetCtx<'_>, plan: &WingetPlan) -> Result<InstallReport, WingetOpsError>` com `pub struct InstallReport { outcome: Outcome, found: Option<InstalledApp>, warning: Option<String>, log: Option<PathBuf> }`.
3. Ordem obrigatória: `install.lock` (`UpdateLock` com `wait`) → `Phase::Preparing` → `fetch_installer` → `VerifiedFile::open` (hash de novo) → `msi.lock` (todos os tipos com linha de comando; msix não) → `gate.try_launch()` (falso ⇒ `Cancelled`, nada lançado) → lançar: `Helper` ⇒ `elevated.run_installer` (`Phase::WaitingElevation` antes; `Refused` ⇒ erro com `UAC_REFUSED`; o auxiliar recebe `params` **com** `<LOGPATH>` literal); `Never` ⇒ `launcher.run` com `params` preenchidos (`<root>\log\installers\<id>-<ver>-<ts>.log`) e programa = `msiexec` (msi/wix, via `msiexec_params`) ou o arquivo materializado; msix ⇒ `msix.add` em `spawn_blocking` aguardado → `Phase::RunningInstaller { name }` enquanto roda → msi/wix/burn: `run_queued` com `busy` e retentativa de 1618 → `classify_install` → `Phase::Verifying` → `scan()` + `post_check::find_installed`.
4. `src/winget_install/post_check.rs` (puro): `pub fn find_installed<'a>(plan: &WingetPlan, apps: &'a [InstalledApp]) -> Option<&'a InstalledApp>` nesta ordem: `ProductCode`/`UpgradeCode` de `AppsAndFeaturesEntries` (sem caixa, com ou sem chaves), `ProductCode` do instalador, `PackageFamilyName` (só MSIX), por último nome + publisher normalizados (a normalização que a correlação já usa).
5. Saída `Ok`/`OkReboot` e app não encontrado ⇒ `warning = "o instalador terminou sem erro, mas o app não apareceu na lista de instalados"`, e o job registra `Outcome::NoEffect` (nunca "Instalado"). Saída `Failed` ⇒ erro com a mensagem da categoria e, para msiexec, `install::msiexec_log::failure_hint` quando houver.
6. Registro: tabela `winget_ops(id TEXT, version TEXT, installer_type TEXT, scope TEXT, sha256 TEXT, arp_key TEXT, at INTEGER)` no `state.db` (`Layout::state_db()`), criada com `CREATE TABLE IF NOT EXISTS` pelo próprio `ops.rs` via `rusqlite` (`busy_timeout` 5 s); **não edite `src/install/state.rs`**. Só pista: nada lê essa tabela para decidir.
7. `progress.rs`: `Phase::WaitingElevation` (título `Aguardando o UAC; se não aparecer, clique no escudo piscando na barra de tarefas`), `Phase::RunningInstaller { name: String }` (título `Instalador de <name> rodando (pode abrir janelas)`; `phase_status` acrescenta `não pode ser cancelado`), `Phase::Verifying` (`Conferindo a instalação`). Todos os consumidores tratam as três (o compilador exige; cada um tem teste).
8. `src/winget_install/prod_win.rs`: `pub fn build<'a>(..) -> WingetCtx<'a>` com `ShellLauncher`, `SystemMsix`, `msiexec_path()`, `installer_busy`, `std::thread::sleep`, `installed::scan_all` (só os apps).

**Files:**
- Create: `src/winget_install/plan.rs`, `ops.rs`, `post_check.rs`, `prod_win.rs` (já existem com uma linha `//!`)
- Modify: `src/install/progress.rs`, `src/cli/install_cmd.rs` (só o `match` de fases), `src/cli/output.rs` (se tiver `match` de fases), `src/tui/jobs.rs` (só `progress_msg`/fases), `src/tui/app.rs` (só a barra de status das fases)
- Test: `tests/it/wi_install_flow.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_install_flow.rs`; wiremock serve o "instalador" — bytes quaisquer com SHA calculado no teste; `Launcher`, `ElevatedRunner`, `MsixOps` falsos que registram chamadas e devolvem códigos programados; `scan` e `busy`/`sleep` falsos; `DataDir` em tempdir):
1. `hash_mismatch_never_launches`: bytes servidos ≠ SHA ⇒ `Err`; `launcher`, `elevated` e `msix` com 0 chamadas.
2. `exit_zero_found_is_installed`: inno `user`, código 0, `scan` com um app cujo `product_code` é o `ProductCode` do manifesto ⇒ `outcome Ok`, `found` Some, `warning` None.
3. `exit_zero_not_found_warns`: igual, `scan` vazio ⇒ `outcome Ok`, `warning` com `não apareceu`.
4. `machine_msi_goes_through_helper`: msi `machine` ⇒ `elevated` recebeu `kind Msi`, `file_name` terminando em `.msi`, `params` contendo `/quiet /norestart` e o texto literal `<LOGPATH>`, `sha256` igual; `launcher` 0 chamadas.
5. `user_inno_runs_unelevated_with_filled_log`: inno `user` ⇒ `launcher` com `program` = arquivo materializado, `params` contendo `/LOG="` e um caminho sob `<root>\log\installers\`, sem `<LOGPATH>`; `show Normal`; `elevated` 0 chamadas.
6. `unelevated_msi_uses_absolute_msiexec`: msi `user` ⇒ `program == ctx.msiexec`, `params` começando com `/i "` e contendo o arquivo materializado.
7. `refused_elevation_fails_with_message`: `elevated` devolve `Refused` ⇒ `Err` cujo texto é `UAC_REFUSED`; `launcher` 0.
8. `msi_1618_retries_then_succeeds`: `launcher` devolve 1618 e depois 0 (msi `user`) ⇒ `Ok`; `sleep` recebeu `[5 s]`; `launcher` 2 chamadas.
9. `plan_text_is_the_confirmation`: `plan(..).to_string()` de msi `machine` com um agreement contém `pede UAC` e `Licença: EULA — https://x/eula`; de inno `user` não contém `pede UAC`; de exe sem silencioso contém `abre a janela do instalador`.
10. `find_installed_order`: dois apps — um casando por nome+publisher, outro pelo `UpgradeCode` de `AppsAndFeaturesEntries` (listado depois) ⇒ escolhe o do `UpgradeCode`; sem `AppsAndFeaturesEntries` e com `ProductCode` ⇒ o do `ProductCode`.
11. `msix_goes_to_package_manager`: msix ⇒ `msix.add` com o arquivo materializado; `launcher` 0; pós-checagem pelo PFN.
12. `cancel_before_launch`: `gate.try_cancel()` antes ⇒ `Err` cancelado, `launcher` 0; num segundo teste o `launcher` falso chama `gate.try_cancel()` durante a execução ⇒ `false` (já lançado) e o fluxo termina normalmente.
13. `winget_ops_row_written`: depois do sucesso, `state.db` tem uma linha com `id`, `version`, `installer_type == "inno"`, `sha256` e `arp_key` do app encontrado.
14. `consumers_show_new_phases`: `phase_status(&Phase::RunningInstaller { name: "X".into() }, 0 s)` contém `não pode ser cancelado`; `tui::jobs::progress_msg` devolve `Some` para cada fase nova; `LineProgress::new(false, ..)` aceita `update` com cada fase sem pânico e o texto da última fase aparece na saída capturada (se o consumidor tiver saída testável; diga como provou).
15. `hold_msi_lock_while_running`: com um `UpdateLock` de `msi.lock` já tomado por outro "processo" (lock tomado no teste), o fluxo espera; o teste solta o lock de uma thread depois que o `launcher` ainda não foi chamado (contador 0) e então ele roda — prova por contagem, sem limiar de tempo (detector de travamento ≥ 60 s com `clock-ok`).

#### Checks beyond the steps

1. Mutação: lançar antes de `VerifiedFile::open` ⇒ teste 1 **PEGA**.
2. Mutação: sem o aviso de "não apareceu" ⇒ teste 3 **PEGA**.
3. Mutação: `Helper` para escopo `user` ⇒ teste 5 **PEGA**.
4. Mutação: `find_installed` com nome+publisher primeiro ⇒ teste 10 **PEGA**.
5. Binário real: ainda não há comando que use o fluxo (CLI na Task 186); rode só o gate e diga isso.
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): native install flow (download, verify, launch or elevate, classify, ARP/MSIX check)`

---

### Task 182: Auxiliar elevado — desinstalador do ARP HKLM e apagar chave HKLM (D3)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/bridge/worker.rs`: `Request` (com `RunInstaller` da Task 180), `serve(reader, writer, ops: &mut dyn WorkerOps)`, `WorkerOps { run, run_installer }`.
- `src/bridge/elevated.rs`: `ElevatedRunner`, `ElevatedError { Refused, Lost, Remote }` (180); `src/bridge/elevate_win.rs:581` `program_files`.
- `src/winget_install/uninstall_cmd.rs`: `ArpDetails`, `Rule`, `uninstall_command(&ArpDetails, nsis_hint) -> Option<UninstallCommand>`, `valid_arp_key` (Task 172); `arp_read_win::read_details(hive, key)` (172); `src/installed/arp.rs` `split_command`, `expand_vars`, `missing_uninstaller` (172).
- `src/winget_install/launch_win.rs` `ShellLauncher`, `msiexec_path` (178).
- `src/install/registry_win.rs:20` `open_nolink` e `:24` `is_link` (recusa link simbólico de registro) — modelo; não edite.
- S3: `USB-Driver-Installer` em HKLM 32 com desinstalador em `%LOCALAPPDATA%\Temp` (pasta do usuário); Kindle 2.8.3 HKLM 32 + 2.9.1 HKCU com o mesmo nome.

#### What binds this task

1. Protocolo: `Request::RunUninstaller { hive: MachineHive, key: String, rule: Rule, nsis_hint: bool }` e `Request::DeleteArpKey { hive: MachineHive, key: String, reason: DeleteReason }` com `pub enum MachineHive { Hklm64, Hklm32 }` (`lowercase`: `hklm64`/`hklm32`; HKCU **não existe** no protocolo) e `pub enum DeleteReason { Orphan, Duplicate }`. `WorkerOps` ganha `run_uninstaller` e `delete_arp_key`.
2. Regras puras em `src/winget_install/elevated_rules.rs`: `pub struct KnownFolders { program_files, program_files_x86, system_root, program_data: PathBuf }`; `pub fn expand_known(raw: &str, k: &KnownFolders) -> Option<String>` (só `%ProgramFiles%`, `%ProgramW6432%`, `%ProgramFiles(x86)%`, `%CommonProgramFiles%`, `%CommonProgramFiles(x86)%`, `%SystemRoot%`, `%windir%`, `%ProgramData%`, sem caixa; qualquer outra variável ⇒ `None`); `pub fn in_admin_folder(p: &Path, k: &KnownFolders) -> bool` (prefixo por componente, sem caixa, sem `..`, debaixo de `program_files`, `program_files_x86`, `system_root` ou `program_data\Package Cache`); `pub fn still_orphan(d: &ArpDetails, k: &KnownFolders, exists: impl Fn(&Path) -> bool) -> bool` (`missing_uninstaller` do `UninstallString` com `expand_known`); `pub fn is_duplicate(d: &ArpDetails, others: &[ArpDetails]) -> bool` (outro registro, chave ou hive diferente, com o mesmo `DisplayName` aparado e sem caixa).
3. Auxiliar, `run_uninstaller`: relê a chave (`read_details` com a view do `hive`), recalcula `uninstall_command(&d, nsis_hint)`; regra diferente da pedida ⇒ `Error("a chave mudou desde a confirmação")`; programa: `msiexec` ⇒ `msiexec_path()`; senão `expand_known` + absoluto + `in_admin_folder`, falhou ⇒ `Error("desinstalador fora de pasta de administrador")`; lança com `ShellLauncher` (`Show::Normal`) e devolve `End { code }`. Nada mais.
4. Auxiliar, `delete_arp_key`: `valid_arp_key`; abre a chave com a view do `hive` sem seguir link e recusa se for link; revalida a razão — `Orphan` ⇒ `still_orphan` (existência checada pelo próprio auxiliar); `Duplicate` ⇒ `is_duplicate` contra as chaves de HKLM 64, HKLM 32 e HKCU (o HKCU do token elevado; com outra conta ele não acha o par e recusa); razão não confirmada ⇒ `Error("a chave deixou de ser <sobra|duplicada>")`; senão apaga a árvore com a **mesma** view e responde `End { code: 0 }`.
5. `pub fn known_folders() -> std::io::Result<KnownFolders>` no lado Windows (`SHGetKnownFolderPath`: `ProgramFiles`, `ProgramFilesX86`, `Windows`, `ProgramData`), nunca variável de ambiente.
6. Cliente: `ElevatedRunner` ganha `run_uninstaller(..) -> Result<i32, ElevatedError>` e `delete_arp_key(..) -> Result<(), ElevatedError>`.

**Da revisão da Task 172 (2026-10-18): o parser do ARP (`uninstall_cmd::split_command`) não recusa nada; quem roda elevado recusa.** O auxiliar, antes de executar um desinstalador: (a) recusa linha de `Hive::User` (HKCU é gravável pelo usuário: rodar como admin seria escalada); (b) expande variáveis só por KnownFolder (`%ProgramFiles%`, `%ProgramFiles(x86)%`, `%SystemRoot%`, `%CommonProgramFiles%`), nunca pelo ambiente; (c) exige programa por caminho absoluto, existente, fora de pastas graváveis pelo usuário (sem relativo, sem PATH, sem aspas abertas, sem texto sem `.exe`); (d) `msiexec` vindo de Quiet/Uninstall só com `/x{GUID}` validado (nunca `/i`, `/f`, `/p`), e o programa é o `msiexec.exe` de `System32` por KnownFolder, não o texto da linha. Um teste por recusa.

**Files:**
- Create: `src/winget_install/elevated_rules.rs` (já existe com uma linha `//!`)
- Modify: `src/bridge/worker.rs`, `src/bridge/elevated.rs`, `src/bridge/elevate_win.rs`, `src/bridge/staging_win.rs` (se as funções Win32 de registro ficarem lá; ou um `src/bridge/arp_ops_win.rs` novo — diga qual)
- Test: `tests/it/elevated_uninstall.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/elevated_uninstall.rs`; `KnownFolders` de teste `C:\PF`, `C:\PF86`, `C:\W`, `C:\PD`):
1. `expand_known_only`: `%ProgramFiles%\A\u.exe` ⇒ `C:\PF\A\u.exe`; `%programfiles(x86)%\B\u.exe` ⇒ `C:\PF86\B\u.exe`; `%LOCALAPPDATA%\x.exe`, `%TEMP%\x.exe`, `%USERPROFILE%\x.exe` ⇒ `None`; `%ProgramFiles\x` (sem fechar) ⇒ `None`.
2. `admin_folder_rule`: `C:\PF\A\u.exe` ⇒ true; `C:\PFX\u.exe` ⇒ false; `C:\PF\..\Users\x.exe` ⇒ false; `C:\PD\Package Cache\{g}\s.exe` ⇒ true; `C:\PD\Other\s.exe` ⇒ false; `C:\Users\me\AppData\Local\Temp\uninst.exe` ⇒ false; `c:\pf\a\U.EXE` ⇒ true.
3. `run_uninstaller_recomputes_rule`: `WorkerOps` de teste cujo `read_details` devolve uma entrada com Quiet; pedido com `rule: msi` ⇒ `Error` com `a chave mudou`, nada lançado.
4. `protocol_has_no_user_hive`: `{"op":"run_uninstaller","hive":"hkcu",..}` e `{"op":"delete_arp_key","hive":"hkcu",..}` ⇒ `serve` para sem executar; campo extra ⇒ idem.
5. `delete_orphan_revalidates`: detalhes com o desinstalador agora existente ⇒ `Error`, delete 0 chamadas; ausente ⇒ delete chamado com `(Hklm32, key)` — e o mesmo com `Hklm64` (as duas no teste, para pegar troca de view).
6. `delete_duplicate_needs_other_record`: sem outro registro de mesmo nome ⇒ `Error`; com um em HKCU (`"Amazon Kindle"` vs `" amazon kindle "`) ⇒ delete.
7. `bad_key_refused`: `key` vazia, com `\`, 256 chars ⇒ `Error`, nada lido.
8. `user_folder_uninstaller_refused`: entrada HKLM32 Quiet com `%LOCALAPPDATA%\Temp\u.exe` ⇒ `Error` com `fora de pasta de administrador`.
9. `live_delete_test_key` `#[ignore = "live: pede UAC; cria e apaga uma chave de teste no HKLM"]`: o usuário cria antes, num terminal elevado (roteiro no relatório), `HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan` com `UninstallString` = `"C:\Program Files\wingetrs-test\u.exe"` (inexistente); o teste pede `DeleteArpKey { Hklm32, "wingetrs-test-orphan", Orphan }` pelo `ElevatedBridge` real ⇒ a chave some (o auxiliar não cria chaves: o protocolo não tem essa operação).

#### Checks beyond the steps

1. Mutação: o mapeamento `Hklm32` usa a view 64 ⇒ teste 5 **PEGA**.
2. Mutação: `in_admin_folder` com `starts_with` de texto (sem fronteira de componente) ⇒ teste 2 **PEGA**.
3. Mutação: `expand_known` aceita qualquer variável do ambiente ⇒ teste 1 **PEGA**.
4. Roteiro ao vivo para o usuário no relatório (chave de teste HKLM 32 e 64, `.reg` restaurado).
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): elevated helper runs revalidated HKLM uninstallers and deletes revalidated HKLM keys`

---

### Task 183: Órfãos — exportar `.reg`, conferir, apagar (P12, D5)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/installed/mod.rs:36` `InstalledApp.stale` (desinstalador que não existe; preenchido em `src/installed/arp_win.rs:64` `stale_uninstaller`), `InstallSource::Arp(Hive)`, `key`; `src/installed/overrides.rs:28` `app_key` (`arp/hklm64/<sub>`).
- `src/install/registry_win.rs:357` `export` (só HKCU, sem `/reg:`), `:384` `delete_key` (só HKCU, recusa link), `:402` `reg_exe()` (caminho absoluto) — modelo; não edite.
- `src/install/layout.rs:40` `backups()`.
- `src/bridge/elevated.rs` `ElevatedRunner::delete_arp_key(hive: MachineHive, key, reason: DeleteReason)` (Task 182).
- `src/winget_install/uninstall_cmd.rs` `valid_arp_key` (172); `elevated_rules::still_orphan` (182).
- S3: `reg export … /reg:32` e `/reg:64` funcionam sem admin; as 3 sobras desta máquina (dois `Steam App …` em HKLM 64, `USB-Driver-Installer` em HKLM 32).

#### What binds this task

1. `src/winget_install/orphans.rs` (puro + trait): `pub struct Orphan { hive: Hive, key: String, name: String, missing: String }`; `pub fn orphans(apps: &[InstalledApp]) -> Vec<Orphan>` (só `Arp(_)` com `stale`); `pub fn export_args(hive: Hive, key: &str, file: &Path) -> Vec<String>` ⇒ `["export", "<HKLM|HKCU>\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<key>", "<file>", "/y", "/reg:64"|"/reg:32"]` (HKCU sem `/reg:`; HKLM32 usa o caminho sem `WOW6432Node` e `/reg:32`); `pub fn backup_file(backups: &Path, ts: &str, o: &Orphan) -> PathBuf` ⇒ `backups\<ts>\<hklm64|hklm32|hkcu>-<key saneada>.reg` (caractere fora de `[A-Za-z0-9 ._{}+-]` ⇒ `_`); `pub fn check_export(file: &Path, key: &str) -> Result<(), String>`: existe, não vazio, BOM UTF-16LE, primeira linha `Windows Registry Editor Version 5.00`, e alguma linha `[...\Uninstall\<key>]` (sem caixa).
2. `pub trait ArpRegistry: Send + Sync { fn export(&self, args: &[String]) -> std::io::Result<()>; fn delete_user_key(&self, key: &str) -> std::io::Result<()>; fn still_orphan(&self, hive: Hive, key: &str) -> bool; }` e `pub fn remove_orphan(reg: &dyn ArpRegistry, elevated: &dyn ElevatedRunner, backups: &Path, ts: &str, o: &Orphan) -> Result<PathBuf, String>`: `valid_arp_key` ⇒ `still_orphan` (falso ⇒ `Err("a chave deixou de ser sobra")`, nada exportado) ⇒ `export` ⇒ `check_export` (falhou ⇒ `Err`, **nada apagado**) ⇒ HKCU: `delete_user_key`; HKLM: `elevated.delete_arp_key(hive, key, Orphan)` ⇒ `Ok(<arquivo .reg>)`.
3. `src/winget_install/orphans_win.rs`: `pub struct SystemArp` com `export` = `reg.exe` por caminho absoluto (`SHGetKnownFolderPath(FOLDERID_System)\reg.exe`), `delete_user_key` = `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<key>` com recusa de link (como `registry_win.rs:384`), `still_orphan` = `read_details` + `missing_uninstaller` com a expansão do ambiente do usuário.
4. Nunca pré-marcado e `--orphans` exigido: regras de UI ficam na CLI/TUI (Tasks 186/187); esta tarefa só garante que `remove_orphan` age sobre **um** órfão por chamada.

**Files:**
- Create: `src/winget_install/orphans.rs`, `src/winget_install/orphans_win.rs` (já existem com uma linha `//!`)
- Test: `tests/it/wi_orphans.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_orphans.rs`; `ArpRegistry` falso que escreve o `.reg` programado pelo teste; `ElevatedRunner` falso):
1. `empty_export_never_deletes`: export cria arquivo de 0 bytes ⇒ `Err`; `delete_user_key` 0, `elevated` 0.
2. `export_without_key_name_never_deletes`: cabeçalho certo, chave `[...\Uninstall\Outra]` ⇒ `Err`; nada apagado.
3. `export_without_header_never_deletes`: UTF-16LE sem a linha `Windows Registry Editor Version 5.00` ⇒ `Err`; e um arquivo UTF-8 com o texto certo (sem BOM) ⇒ `Err`.
4. `hkcu_deleted_in_process_after_export`: `.reg` válido ⇒ `delete_user_key("App")` uma vez; devolve o caminho sob `backups\<ts>\hkcu-App.reg`.
5. `hklm_goes_to_helper_with_view`: um órfão `MachineX86` e outro `MachineX64` ⇒ `elevated.delete_arp_key` com `(Hklm32, k1, Orphan)` e `(Hklm64, k2, Orphan)`.
6. `export_args_views`: `MachineX86` ⇒ contém `/reg:32` e `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\K`; `MachineX64` ⇒ `/reg:64`; `User` ⇒ `HKCU\…` sem `/reg:`.
7. `no_longer_orphan_is_skipped`: `still_orphan` falso ⇒ `Err` com `deixou de ser sobra`; `export` 0 chamadas.
8. `orphans_from_stale`: apps `[Arp(MachineX64) com stale, Arp(User) com stale, Arp(MachineX86) sem stale, Msix]` ⇒ dois órfãos com os hives certos.
9. `backup_name_sanitized`: `Steam App 255710` ⇒ `hklm64-Steam App 255710.reg`; `a:b*c` ⇒ `hklm64-a_b_c.reg`.
10. `live_export_hklm_without_admin` `#[ignore = "live: lê o registro real"]`: `SystemArp.export` de uma chave HKLM 32 existente (a primeira de `WOW6432Node\…\Uninstall`) para tempdir + `check_export` ⇒ `Ok`; o arquivo é apagado no fim.

#### Checks beyond the steps

1. Mutação: `export_args` troca `/reg:32` e `/reg:64` ⇒ teste 6 **PEGA**.
2. Mutação: pular `check_export` ⇒ teste 1 **PEGA**.
3. Mutação: `still_orphan` checado depois do export ⇒ teste 7 **PEGA**.
4. Real: `cargo test --release --quiet live_export_hklm_without_admin -- --ignored` (só leitura) e cole.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): orphan ARP records — verified .reg export, then delete (HKCU in process, HKLM via helper)`

---

### Task 184: Fluxo de desinstalação nativa e o caso Kindle (P11, D6)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/winget_install/uninstall_cmd.rs` `ArpDetails`, `uninstall_command`, `Rule` (172); `arp_read_win::read_details` (172); `launch.rs` `Launcher`, `uninstall_elevation` (178); `msix.rs` `MsixOps::remove` (179); `codes.rs` `classify_uninstall` (175); `bridge::elevated::ElevatedRunner::{run_uninstaller, delete_arp_key}` (180/182); `elevated_rules::{KnownFolders, expand_known, in_admin_folder}` (182); `orphans::{ArpRegistry, export_args, check_export, backup_file}` (183).
- `src/installed/correlate.rs:72` `ListRow` (`app_key`, `origin`, `others: Vec<OtherRecord>`, `scope`, `stale`), `:61` `OtherRecord`; `src/installed/overrides.rs:28` `app_key` (`arp/hklm32/<sub>`).
- `src/install/layout.rs:40` `backups()`.
- S3: Kindle 2.8.3 (HKLM 32, NSIS sem Quiet, desinstalador existe) e 2.9.1 (HKCU); NSIS se copia para `%TEMP%` e sai antes de terminar (a prova é a chave sumir).

#### What binds this task

1. `src/winget_install/uninstall.rs`: `pub enum UninstallTarget { Arp { hive: Hive, key: String }, Msix { family: String } }`; `pub fn target_of(app_key: &str) -> Option<UninstallTarget>` (`arp/hklm64|hklm32|hkcu/<sub>`, `msix/<família>`); `pub struct UninstallPlan { target, name, version: Option<String>, command: Option<UninstallCommand>, elevation: Elevation, shared: Option<SharedData> }` com `Display` (mesmo texto no dry-run e nas confirmações: comando, `pede UAC`, `abre a janela do desinstalador` quando interativo, aviso do Kindle).
2. `pub fn plan_uninstall(d: &ArpDetails, nsis_hint: bool, has_others: bool, is_main: bool, k: &KnownFolders, folders: &dyn Fn(&str) -> bool) -> Result<UninstallPlan, String>`: comando de `uninstall_command`; sem comando ⇒ `Err("sem desinstalador registrado; use \"Remover registro órfão\" se for sobra")`; elevação por `uninstall_elevation(hive, msiexec, in_admin_folder(expand_known(program)))`.
3. Kindle (D6 = A + C): com `has_others`, `SharedData { folders: Vec<PathBuf>, registry_only_allowed: bool }` — pastas que **existem** entre `%LOCALAPPDATA%\<Publisher>`, `%APPDATA%\<Publisher>`, `%LOCALAPPDATA%\<Nome>`, `%APPDATA%\<Nome>`, `Documents\<Nome>` (Publisher/Nome como um componente que passa em `safe_components`; senão a pasta é ignorada); `registry_only_allowed = !is_main`. Texto: `Outra instalação de <nome> usa as mesmas pastas: copie antes de remover.`
4. `pub fn uninstall(ctx: &UninstallCtx<'_>, plan: &UninstallPlan) -> Result<UninstallReport, String>` com `UninstallCtx { launcher, elevated, msix, key_exists: &dyn Fn(Hive, &str) -> bool, clock: &dyn Clock, user_expand: &dyn Fn(&str) -> Option<String> }` e `pub trait Clock { fn now(&self) -> Duration; fn sleep(&self, d: Duration); }`: HKCU ou `Never` ⇒ `launcher` (`Show::Normal`, programa expandido com o ambiente do usuário, msiexec por `msiexec_path` injetado); `Helper` ⇒ `elevated.run_uninstaller(hive, key, rule, nsis_hint)`; MSIX ⇒ `msix.remove(family)`. Código por `classify_uninstall`; `Failed` ⇒ erro. Depois: consulta `key_exists` a cada 500 ms pelo `clock`, até 120 s; sumiu ⇒ `Removed`; não sumiu ⇒ `warning = "o desinstalador terminou, mas o registro continua"` (job `NoEffect`).
5. `pub fn backup_folders(folders: &[PathBuf], dest: &Path) -> Vec<String>` copia cada pasta para `backups\<ts>\<app>\<n>-<nome da pasta>`, recursivo, **sem seguir pontos de reanálise** (pula junções e links, registra cada pulo), devolve os erros.
6. `pub fn remove_registry_only(..)`: só com `registry_only_allowed`; exporta e confere o `.reg` como os órfãos (`export_args`, `check_export`); HKCU ⇒ `delete_user_key`; HKLM ⇒ `elevated.delete_arp_key(hive, key, Duplicate)`.

**Da revisão de segurança da Task 182 (2026-10-18):** o auxiliar elevado só apaga uma chave HKLM como "duplicada" se outra chave HKLM com o mesmo nome tiver `DisplayVersion` estritamente maior; HKCU nunca autoriza apagar HKLM (o usuário controla HKCU). Consequência para o caso Kindle (HKLM 2.8.3 + HKCU 2.9.1): a opção C ("remover só o registro") não se aplica à chave HKLM; a tarefa oferece a opção A (rodar o desinstalador antigo, com aviso e cópia das pastas) e explica por que a C não está disponível. Teste do caso.

**Files:**
- Create: `src/winget_install/uninstall.rs` (já existe com uma linha `//!`)
- Test: `tests/it/wi_uninstall_flow.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_uninstall_flow.rs`; fakes de `Launcher`, `ElevatedRunner`, `MsixOps`, `Clock` que só avança o tempo, `key_exists` com sequência programada):
1. `key_gone_is_removed`: `launcher` 0, `key_exists` = `[true, true, false]` ⇒ `Removed`; o relógio falso recebeu exatamente 2 `sleep(500 ms)`.
2. `key_stays_warns_after_120s`: `key_exists` sempre `true` ⇒ `warning` com `registro continua`; o relógio avançou ≥ 120 s e o laço terminou (contagem de `sleep` == 240).
3. `hkcu_never_elevated`: HKCU com Quiet ⇒ `launcher` 1, `elevated` 0.
4. `hklm_msi_goes_helper`: HKLM32 MSI ⇒ `elevated.run_uninstaller(Hklm32, key, Msi, false)`; `launcher` 0.
5. `hklm_user_folder_uninstaller_runs_unelevated`: HKLM32 Quiet em `%LOCALAPPDATA%\Temp\u.exe` ⇒ `elevation Never`, `launcher` 1.
6. `msix_uses_remove_package`: `UninstallTarget::Msix { family: "A_b" }` ⇒ `msix.remove("A_b")`.
7. `kindle_folders_only_existing`: `has_others`, `folders` diz que só `%APPDATA%\Amazon` existe ⇒ `shared.folders == [%APPDATA%\Amazon]` (com `APPDATA` do teste).
8. `registry_only_offered_only_when_not_main`: `is_main = true` ⇒ `registry_only_allowed == false` e `remove_registry_only` ⇒ `Err`; `false` ⇒ permitido, HKLM32 ⇒ `elevated.delete_arp_key(Hklm32, key, Duplicate)` depois do `.reg` conferido.
9. `backup_copies_and_skips_reparse` (`#[cfg(windows)]`): árvore em tempdir com `a.txt` e uma junção (crate `junction`) para outra pasta com `secret.txt` ⇒ o destino tem `a.txt` e não tem `secret.txt`; o retorno registra o pulo.
10. `nonzero_exit_fails_without_polling`: `launcher` 1603 numa entrada Quiet ⇒ `Err`; `key_exists` nunca consultado.
11. `target_of_parses_app_keys`: `arp/hklm32/{G}` ⇒ `Arp { MachineX86, "{G}" }`; `arp/hkcu/App` ⇒ `User`; `msix/A_b` ⇒ `Msix`; `scoop/x` ⇒ `None`.
12. `plan_text_matches_confirmation`: `Display` de um plano HKLM MSI contém `pede UAC`; de um interativo contém `abre a janela do desinstalador`; com `shared` contém `Outra instalação`.

#### Checks beyond the steps

1. Mutação: a condição de parada do laço invertida (para quando a chave **existe**) ⇒ teste 1 **PEGA**.
2. Mutação: `registry_only_allowed = is_main` ⇒ teste 8 **PEGA**.
3. Mutação: seguir pontos de reanálise na cópia ⇒ teste 9 **PEGA**.
4. Mutação: `Hklm32` vira `Hklm64` no pedido ao auxiliar ⇒ teste 4 **PEGA**.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): native uninstall flow (wait for the key to go), shared-data warning and registry-only removal`

---

### Task 185: Atualização nativa e escolha do motor (P9, D1)

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/config.rs` `WingetEngine { Wingetrs, Winget }`, `DefaultScope` (Task 171).
- `src/winget_install/plan.rs` `plan(..) -> Result<PlanOrBridge, Refusal>`, `WingetPlan`; `ops.rs` `install` (181); `uninstall.rs` `plan_uninstall`, `uninstall` (184); `select.rs` `ScopeWanted` (173); `switches.rs` `install_args(.., upgrade: bool)` (174).
- `src/bridge/mod.rs:154` `winget_applicability(manifest, scope, arch) -> WingetFit { applicability: Ok|Denied|ScopeMismatch|Unknown, explicit_only }` (regras de `UpdateFlow.cpp`/`ManifestComparator.cpp`).
- `src/pins.rs` `is_pinned`; `src/core/version.rs`.
- S2: `UpgradeBehavior` `install` 137, ausente 134, `uninstallPrevious` 22, `deny` 1; `RequireExplicitUpgrade` 2.

#### What binds this task

1. `src/winget_install/engine.rs`: `pub enum Engine { Native(WingetPlan), Bridge(Option<String>) }`; `pub fn choose_engine(setting: WingetEngine, planned: Result<PlanOrBridge, Refusal>) -> Result<Engine, Refusal>`: `Winget` ⇒ `Bridge(None)` sempre (mesmo com recusa do motor: o winget decide); `Wingetrs` ⇒ `Native` / `Bridge(Some(motivo))` / a recusa.
2. `pub enum UpgradeRoute { InPlace, UninstallFirst, Denied }` e `pub fn upgrade_route(i: &EffectiveInstaller) -> UpgradeRoute` (`install` ou ausente ⇒ `InPlace`; `uninstallPrevious` ⇒ `UninstallFirst`; `deny` ⇒ `Denied`; sem caixa).
3. `pub fn upgrade_scope(installed: Option<InstallScope>, default: DefaultScope) -> ScopeWanted`: app instalado com escopo conhecido ⇒ `Only(esse)`; desconhecido ⇒ `Prefer(default)`.
4. `pub async fn upgrade(..)`: `Denied` ⇒ `Err("o pacote não permite atualizar por cima; remova e instale de novo")`; `InPlace` ⇒ `ops::install` com o plano montado com `upgrade_from = Some(instalada)` (o `Upgrade` switch entra); `UninstallFirst` ⇒ `uninstall` do registro da linha (P11) e só se `Removed` ⇒ `ops::install`; desinstalação falhou ou ficou com aviso ⇒ erro, instalação **não** roda.
5. `pub fn upgrade_all_targets<'a>(rows: &'a [ListRow], pins: &BTreeSet<String>, explicit_only: &dyn Fn(&ListRow) -> bool) -> Vec<&'a ListRow>`: só linhas com atualização (`UpdateState::Available`), sem pin, sem `RequireExplicitUpgrade`; alvo nomeado pelo usuário inclui o explícito.
6. Versões por `crate::core::version` (a "tem atualização?" já vem de `ListRow.state`; qualquer comparação nova usa `Version`).

**Files:**
- Create: `src/winget_install/engine.rs` (já existe com uma linha `//!`)
- Test: `tests/it/wi_upgrade.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_upgrade.rs`):
1. `engine_setting_winget_always_bridge`: `Winget` com `Ok(Native(..))` ⇒ `Bridge(None)`; com `Err(Refusal)` ⇒ `Bridge(None)`.
2. `native_bridge_reason_passes_through`: `Wingetrs` com `Ok(Bridge("tipo zip: …"))` ⇒ `Bridge(Some(m))`, `m` contém `tipo zip`.
3. `refusal_is_not_bridged_under_wingetrs`: `Wingetrs` com `Err(Refusal("exige Windows …"))` ⇒ `Err`.
4. `uninstall_previous_order`: fakes registram eventos ⇒ `["uninstall", "wait_key", "install"]`; desinstalação devolvendo aviso ⇒ eventos `["uninstall", "wait_key"]` e `Err`.
5. `deny_refuses`: `UpgradeBehavior: deny` ⇒ `Err` com `não permite atualizar`.
6. `in_place_uses_upgrade_switch`: msi com `Upgrade: REINSTALL=ALL` ⇒ o `params` lançado termina em `REINSTALL=ALL`; instalação nova (sem `upgrade_from`) ⇒ não contém.
7. `explicit_upgrade_excluded_from_all`: duas linhas com atualização, uma `explicit_only` ⇒ `upgrade_all_targets` devolve só a outra.
8. `pinned_excluded_from_all`.
9. `scope_follows_installed`: `upgrade_scope(Some(Machine), User) == Only(Machine)`; `(None, Machine) == Prefer(Machine)`.
10. `native_agrees_with_winget_applicability`: para manifestos `[deny]`, `[só machine]` com app `User`, `[user e machine]` com app `User`, `[sem escopo]` ⇒ o motor nativo recusa exatamente quando `winget_applicability` dá `Denied`/`ScopeMismatch` e aceita quando dá `Ok` (equivalência entre os dois caminhos).

#### Checks beyond the steps

1. Mutação: instalar antes de desinstalar em `UninstallFirst` ⇒ teste 4 **PEGA**.
2. Mutação: sem o filtro de `RequireExplicitUpgrade` ⇒ teste 7 **PEGA**.
3. Mutação: `Winget` respeita a recusa do motor ⇒ teste 1 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): native upgrade (in place, uninstall first, deny) and engine choice`

---

### Task 186: CLI — `install`/`uninstall`/`upgrade` de pacotes winget e `cleanup --orphans` (P13)

**Modelo:** sonnet (revisor opus — liga a limpeza de órfãos e a elevação).

#### Where this fits

- `src/cli/mod.rs:85` `Command` (`Install` com `--scope` da Task 176, `Uninstall`, `Upgrade`, `BridgeWorker`), `:838` despacho das operações (Windows ⇒ `run_win::run`).
- `src/cli/install_cmd.rs:451` `execute`, `:527` `choose`, `:604` `install` (desvio para `winget_cmd::install_bridge` da 176), `:653` `uninstall`, `:763` `upgrade`, `:431` `proceed`, `Ui::pick_many(items, initial: &[bool])` (`:25`), `:79` `until_interrupted` + `LaunchGate`.
- `src/cli/winget_cmd.rs` (176): `install_bridge`, `agreement_lines`.
- `src/winget_install/engine.rs` `choose_engine`, `upgrade`, `upgrade_scope` (185); `plan.rs` `plan`, `Display` (181); `ops.rs` `install` (181); `prod_win::build` (181); `uninstall.rs` `target_of`, `plan_uninstall`, `uninstall` (184); `orphans.rs` `orphans`, `remove_orphan`, `orphans_win::SystemArp` (183); `host_win::current` (173).
- `src/installed/mod.rs:182` `scan_all`, `correlate` (linhas com `app_key`, `correlation` `Matched { package_id }`).
- `src/job_log.rs` `JobLog::start_accepting` (176).

#### What binds this task

1. `install <ALVO>` winget: `choose_engine(config.behavior.winget_engine, plan(..))` ⇒ `Native` ⇒ imprime o `Display` do plano (`--dry-run` para aqui com `Simulação: nada foi alterado.`), `proceed`, com `--yes` imprime as linhas `Licença aceita: …` e registra com `start_accepting`; roda `ops::install` dentro de `until_interrupted` (Ctrl+C antes do lançamento cancela; depois, `UNCANCELLABLE`); resultado: `Instalado: <nome> <versão>`, ou o aviso de "não apareceu" em amarelo, ou o erro da categoria. `Bridge(motivo)` ⇒ imprime `Instalando pelo winget: <motivo>` e segue o `install_bridge` da 176. `ScopeWanted`: `--scope` ⇒ `Only`; senão `Prefer(config.behavior.default_scope)`.
2. `uninstall <ALVO>`: app do wingetrs primeiro (como hoje); senão procura nas linhas instaladas (ARP/MSIX) por id winget correlacionado igual (sem caixa) ou nome igual (sem caixa); várias ⇒ `pick` com terminal, erro de ambiguidade sem terminal ou com `--yes`; uma ⇒ `plan_uninstall` + `Display` + `proceed` + `uninstall`. `--purge` não se aplica a winget (erro `--purge só vale para apps do wingetrs`).
3. `upgrade <ALVO>`: app do wingetrs primeiro; senão linha instalada correlacionada com o winget ⇒ `engine::upgrade` (nativo) ou ponte (`BridgeOp::Upgrade`) conforme o motor. `upgrade --all` **continua só com apps do wingetrs** (a aplicabilidade em lote na CLI está no BACKLOG).
4. `cleanup --orphans [--dry-run] [--yes]` (subcomando novo `Cleanup`, help em português com exemplos): sem `--orphans` ⇒ erro `diga o que limpar: --orphans`; lista os órfãos (`Órfão: <nome> — <hive> — falta <caminho>` e `O app pode ter mudado de lugar (ex.: Steam em outra pasta).` e o destino do `.reg`); com terminal e sem `--yes` ⇒ `pick_many` com **todos desmarcados**; `--yes --orphans` ⇒ todos os listados; cada um por `remove_orphan`, imprimindo `Removido: <nome> (backup <arquivo>)` ou o erro; `--dry-run` não exporta nada.
5. Elevação na CLI: um `ElevatedBridge` por comando (como na 176), `shutdown(5 s)` em todos os caminhos.

**Da revisão da Task 184 (2026-10-18):** `UserFolders` (APPDATA, LOCALAPPDATA, Documents) é montado por `SHGetKnownFolderPath` (FOLDERID_RoamingAppData, FOLDERID_LocalAppData, FOLDERID_Documents), nunca por variável de ambiente; teste que planta `APPDATA` falso no ambiente e confere que não muda nada.

**Files:**
- Modify: `src/cli/mod.rs`, `src/cli/install_cmd.rs`, `src/cli/winget_cmd.rs`, `src/cli/run_win.rs` (se o contexto precisar das peças nativas)
- Test: `tests/it/wi_cli.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_cli.rs`; `Ui` falso que registra `initial` do `pick_many` e as respostas; engine/fluxos falsos injetados nas funções de `winget_cmd`):
1. `help_lists_cleanup_and_orphans`: help raiz lista `cleanup`; `cleanup --help` contém `--orphans` (help do subcomando, não o raiz).
2. `cleanup_requires_orphans_flag`: `cleanup --yes` ⇒ erro com `--orphans`.
3. `orphans_never_preselected`: três órfãos, terminal interativo ⇒ `pick_many` recebeu `initial == [false, false, false]`.
4. `yes_with_orphans_removes_each`: `--yes --orphans` com dois órfãos ⇒ `remove_orphan` 2 vezes; `--dry-run` ⇒ 0 vezes.
5. `native_install_prints_plan_and_logs_agreements`: plano nativo com um agreement e `--yes` ⇒ saída contém o `Display` do plano e `Licença aceita: EULA — https://x/eula`; `job.start` tem `agreements=` (assinante de teste).
6. `bridge_reason_is_printed`: engine `Bridge(Some("tipo zip: instalado pelo winget"))` ⇒ saída contém `Instalando pelo winget: tipo zip`.
7. `uninstall_falls_back_to_installed_rows`: alvo que não é app do wingetrs; linha `arp/hklm32/{G}` correlacionada com `Pub.App` ⇒ o fluxo de desinstalação recebe `Arp { MachineX86, "{G}" }`.
8. `purge_refused_for_winget`: `uninstall Pub.App --purge` resolvido para linha ARP ⇒ erro com `--purge só vale`.
9. `ambiguous_name_needs_terminal`: duas linhas com o mesmo nome e `--yes` ⇒ erro de ambiguidade; com terminal ⇒ `pick`.

#### Checks beyond the steps

1. Mutação: `initial` com `true` no `pick_many` dos órfãos ⇒ teste 3 **PEGA**.
2. Mutação: `cleanup --yes` sem `--orphans` limpa ⇒ teste 2 **PEGA**.
3. Mutação: não imprimir as licenças aceitas ⇒ teste 5 **PEGA**.
4. Binário real (só o que não altera nada): `target/release/wingetrs.exe install Notepad++.Notepad++ --dry-run`, `install Git.Git --dry-run --scope machine`, `uninstall "Amazon Kindle" --dry-run` e `cleanup --orphans --dry-run` (deve listar as 3 sobras da S3, se ainda existirem); cole as saídas. Nada sem `--dry-run`.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): native winget install/uninstall/upgrade and cleanup --orphans`

---

### Task 187: TUI — jobs nativos, "aguardando UAC", "Remover registro órfão" e motor (P14)

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/tui/app.rs:192` `Action` (`Apply`, `Remove`, `Bridge`, `ApplyMsiexec`), `:205` `label`, `:2122` `install_one` (com o `Cmd::PlanWingetInstall` da 176), `:2747` `search_actions`, `:2763` `installed_actions` (`Remover` com `NOT_OURS` para origem ≠ `wingetrs`), `:2300`–`:2345` itens/notas da confirmação (`ConfirmItem.checked`), `:3244` `ConfirmMode::Removals`.
- `src/tui/run.rs:125` `Ctx.elevated` (`ElevatedBridge`), `:1397` `spawn_bridge`, `:1415` `spawn_execute`, `:1108` `applicability` (modelo de leitura do manifesto).
- `src/tui/jobs.rs:86` `progress_msg`, `:323` `run_action`.
- `src/tui/view.rs:848`: texto `Sobra provável: o desinstalador {path} não existe. A limpeza fica para a fase 2d.`
- `src/winget_install/engine.rs` `choose_engine`, `upgrade` (185); `plan.rs` (181); `ops.rs`/`prod_win.rs` (181); `uninstall.rs` (184); `orphans.rs`/`orphans_win.rs` (183); `install::progress::Phase::{WaitingElevation, RunningInstaller, Verifying}` (181); `config::Behavior.winget_engine`/`default_scope` (171).

#### What binds this task

1. `Action::WingetInstall(WingetPlan)`, `Action::WingetUninstall(UninstallPlan)`, `Action::WingetUpgrade { plan: WingetPlan, row_key: String }`, `Action::RemoveOrphan(Orphan)`; `label` em português (`Instalando <nome>`, `Removendo <nome>`, `Atualizando <nome>`, `Removendo registro órfão de <nome>`).
2. Instalar (Search, winget): `Cmd::PlanWingetInstall` passa a montar `choose_engine` com o motor do Ajustes ⇒ `Native` ⇒ confirmação com o `Display` do plano como nota (inclui `pede UAC`, licenças, aviso interativo); `Bridge(motivo)` ⇒ o job da 176 com a nota `Pelo winget: <motivo>`.
3. Remover (Installed/Updates): linhas `arp`/`msix` ganham `Remover` habilitado (scoop continua `NOT_OURS`); `Cmd::PlanWingetUninstall { app_key }` ⇒ `read_details` + `plan_uninstall` ⇒ confirmação com o `Display`; no caso Kindle, itens extras `Copiar pastas para backups antes` (marcado) e, se permitido, `Remover só o registro` (desmarcado).
4. Atualizar: linha winget com o motor `wingetrs` e plano nativo ⇒ `Action::WingetUpgrade`; senão o `Action::Bridge` de hoje.
5. `Remover registro órfão` no menu de linhas com `stale` (e nas `others` com `stale`): confirmação com o caminho que falta, `O app pode ter mudado de lugar (ex.: Steam em outra pasta).` e o destino do `.reg`; o item da confirmação começa **desmarcado** (`checked: false`) e confirmar com nada marcado não faz nada. `ConfirmMode::Never` não pula essa confirmação.
6. Barra de status: as três fases novas com `phase_status`; `WaitingElevation` mostra o texto do UAC.
7. `view.rs`: o texto da sobra vira `Sobra provável: o desinstalador {path} não existe. Use "Remover registro órfão" no menu.`
8. Jobs em `run.rs` com `prod_win::build` (o `ElevatedRunner` é o `Ctx.elevated` da sessão: um UAC por sessão para tudo).

**Da revisão da Task 184 (2026-10-18):** `UserFolders` (APPDATA, LOCALAPPDATA, Documents) é montado por `SHGetKnownFolderPath` (FOLDERID_RoamingAppData, FOLDERID_LocalAppData, FOLDERID_Documents), nunca por variável de ambiente; teste que planta `APPDATA` falso no ambiente e confere que não muda nada.

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/jobs.rs`, `src/tui/view.rs`, snapshots afetados em `tests/it/snapshots/` (diga quais)
- Test: `tests/it/wi_tui.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_tui.rs`, `App` montado como em `tests/it/tui_state.rs`):
1. `confirm_shows_pede_uac_for_machine_plan`: `Msg` com plano nativo msi `machine` ⇒ o modal contém `pede UAC`; inno `user` ⇒ não contém.
2. `orphan_item_never_prechecked`: abrir `Remover registro órfão` ⇒ `ConfirmItem.checked == false`; Enter sem marcar ⇒ nenhum `Cmd` de execução; com `ConfirmMode::Never` o modal aparece mesmo assim.
3. `remove_enabled_for_arp_and_msix_rows`: `installed_actions` de linha `arp` e `msix` ⇒ `Remover` sem nota; `scoop` ⇒ com `NOT_OURS`.
4. `engine_winget_routes_bridge`: `behavior.winget_engine = Winget` ⇒ instalar winget gera `Action::Bridge` com `op: Install`; `Wingetrs` com plano nativo ⇒ `Action::WingetInstall`.
5. `status_bar_waiting_uac_text`: `progress_msg(Phase::WaitingElevation)` aplicado ⇒ a barra contém `Aguardando o UAC`.
6. `stale_text_points_to_action`: painel de detalhes de linha com `stale` contém `Remover registro órfão` e não contém `fase 2d`.
7. `kindle_items`: linha com `others` e não principal ⇒ itens `Copiar pastas…` (marcado) e `Remover só o registro` (desmarcado); principal ⇒ sem `Remover só o registro`.

#### Checks beyond the steps

1. Mutação: item do órfão com `checked: true` ⇒ teste 2 **PEGA**.
2. Mutação: `ConfirmMode::Never` pula a confirmação do órfão ⇒ teste 2 **PEGA**.
3. Mutação: `Remover só o registro` oferecido no principal ⇒ teste 7 **PEGA**.
4. Binário real: abrir a TUI só se o terminal permitir; senão cole os snapshots novos e diga "não verificado na TUI".
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(tui): native winget jobs, UAC status, orphan record removal and engine routing`

---

### Task 188: Validação ao vivo (S4 + pontos restantes da S1)

**Modelo:** orquestrador + usuário (sem implementador).

#### Where this fits

- Tudo das Tasks 170–187; roteiros ao vivo deixados nos relatórios das Tasks 180, 182 e 183.
- Testes `#[ignore = "live: ..."]`: `live_split_matches_shell` (172), `live_add_and_remove` (179), `live_run_installer_once` (180), `live_delete_test_key` (182), `live_export_hklm_without_admin` (183), `live_single_uac_for_two_jobs` (137).
- Relatório em `docs/superpowers/reports/2026-10-XX-fase2d-validacao.md` (data do dia).

#### What binds this task

1. **S1 restante** (o usuário clica): o UAC do auxiliar aparece ou só pisca no Windows Terminal e no conhost (se só piscar, o texto da barra de status ajuda?); recusar ⇒ `Permissão de administrador recusada.` e nenhum novo UAC na sessão; `is_elevated` falso na TUI e verdadeiro no auxiliar (log); um MSI de máquina pequeno com `/quiet` **sem** elevação (motor `wingetrs` com `--scope user` forçado num pacote só `machine` não serve; use `msiexec /i x.msi /quiet` manual) ⇒ anotar o código (confirma a hipótese "msi de máquina exige elevação").
2. **S4:** com pacotes escolhidos pelo usuário, instalar e remover pelo motor nativo um de cada tipo disponível (msi, wix, burn, inno, nullsoft, msix, exe com silencioso, exe interativo); um pela ponte (zip ou portable); um `upgrade` `install` e um `uninstallPrevious`; conferir um UAC por sessão, `C:\Program Files\wingetrs\staging` vazio no fim, `job.start`/`job.end` no log com `elevated=` e `agreements=`.
3. **Órfãos:** um HKCU e um HKLM criados pelo usuário para o teste, removidos e depois restaurados com o `.reg` (`reg import`); conferir que o `.reg` restaura a chave idêntica.
4. **Kindle:** só a confirmação (dry-run e modal); remoção real só se o usuário quiser.
5. Cada achado vira item do ledger e, se for armadilha, entrada em `docs/ARMADILHAS.md`; emendas finais da spec (o que mudou do desenho) feitas pelo orquestrador.

**Files:**
- Create: `docs/superpowers/reports/2026-10-XX-fase2d-validacao.md`
- Test: não se aplica (no mutation proof: validação manual com o usuário)

#### Checks beyond the steps

1. Cada item acima com a saída real colada (comando, código, trecho do log) ou "não feito: <motivo>".
2. `pwsh -File scripts/leftovers.ps1` antes e depois.
3. Gate: `pwsh -File scripts/verify.ps1` no commit do relatório.

**Commit:** `docs(report): phase 2d live validation`

---

### Task 189 (opcional — depende de decisão do usuário): `portable` e `zip`→`portable` nativos

**Modelo:** opus (revisor opus).

> **Depende de decisão do usuário.** S2: ~32% da amostra cai na ponte só por `portable` (29 pacotes) ou zip com `portable` aninhado (66). O desenho abaixo reaproveita a máquina da fase 2a (pasta do app, shims, PATH do usuário) e toca arquivos de `src/install/` que a 2c também edita: só rode depois da 2c fechar, ou com o usuário aceitando a coordenação. Alternativa sem tocar `src/install/`: ficar na ponte (o winget já faz isso bem).

#### Where this fits

- `src/sources/winget/manifest.rs`: `EffectiveInstaller { kind, nested_kind, nested_files: Vec<NestedInstallerFile { relative_file_path, portable_command_alias }> }` (Task 170).
- `src/winget_install/select.rs` veredito `Bridge("tipo portable…")`/`Bridge("tipo zip…")` (173); `fetch.rs` (177).
- Fase 2a: `src/install/layout.rs` (`apps\<app>\<ver>`, `current`, `shims`), `src/install/shim.rs`, `src/install/archive.rs` (zip), `src/install/plan.rs` `InstallPlan`, `src/install/state.rs` (registro `wingetrs`), `src/install/safepath.rs`.
- winget-cli: `PortableInstaller.cpp`/`PortableFlow.cpp` (o winget grava em `%LOCALAPPDATA%\Microsoft\WinGet\Packages` e cria uma entrada ARP em HKCU — **o wingetrs não faz isso**: usa a própria raiz e o próprio registro de apps).

#### What binds this task

1. `portable` (exe solto) e `zip` com `NestedInstallerType: portable` viram um `InstallPlan` da 2a com `bucket = "winget"` e `app = <id>`: arquivos em `apps\<id>\<ver>\`, um shim por `PortableCommandAlias` (ou pelo nome do exe sem alias), todo `RelativeFilePath` por `safepath`; zip extraído pelo `archive.rs`; hash do `InstallerSha256` conferido como hoje.
2. `select.rs`: esses dois casos passam a `Native { interactive: false }` **só** quando a decisão do usuário ligar a opção (constante ou ajuste — a decisão diz qual); zip com outro aninhado continua na ponte.
3. Desinstalar e atualizar seguem o caminho dos apps do wingetrs (2a); a linha aparece com origem `wingetrs`.

**Files:**
- Create: (definido na decisão; provável `src/winget_install/portable.rs`)
- Modify: `src/winget_install/select.rs` e os pontos da 2a que a decisão aprovar
- Test: `tests/it/wi_portable.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/wi_portable.rs`, wiremock + zip montado no teste):
1. `portable_exe_becomes_app_with_shim`: `portable` com `Commands`/alias `x` ⇒ plano com o exe em `apps\Pub.App\1.0\` e shim `x`.
2. `zip_nested_portable_files`: zip com `bin\x.exe` e `NestedInstallerFiles [{bin\x.exe, x}]` ⇒ shim `x` apontando para `bin\x.exe`.
3. `nested_path_escape_refused`: `RelativeFilePath: ..\..\x.exe` ⇒ recusa (fixture só dessa recusa).
4. `zip_with_msi_nested_stays_bridge`.
5. `hash_mismatch_installs_nothing`.

#### Checks beyond the steps

1. Mutação: aceitar `..` em `RelativeFilePath` ⇒ teste 3 **PEGA**.
2. Rodar contra 10 manifestos reais `portable`/`zip+portable` da amostra S2 (só planejar, `--dry-run`) e colar.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(winget-install): native portable and zip-portable packages as wingetrs apps`

## Questões em aberto — respondidas pelo orquestrador (2026-10-18)

1. Remover pela ponte cedo: **não**; a desinstalação fica nativa (D1), na 184.
2. UAC mostra "wingetrs": **aceito** como consequência da D3 = B (um UAC por sessão, pedido do usuário); a confirmação do wingetrs lista o que vai rodar elevado, com nome do instalador e SHA-256.
3. MSIX em uso: **como o winget** (revisão da 179): `AddPackageAsync` em uso ⇒ stage + `RegisterPackageByFullNameAsync`; registro em uso ⇒ adiado, conta como sucesso ("o app será atualizado quando for fechado"); nunca fechar o app à força.
4. Escopo na ponte: **resolvido na Task 176** (passa `--scope machine` quando o Ajustes pede; sem instalador para máquina ⇒ uma tentativa sem `--scope`, com aviso).
5. `upgrade --all`: continua só com apps do wingetrs (BACKLOG).
6. `--location`: fora desta fase.
7. Ambiente elevado: **lista branca na Task 180** (revisão de segurança): nada herdado do usuário; variáveis só de KnownFolder e constantes, PATH só de pastas do sistema, `TEMP`/`TMP` na pasta de execução protegida. Propriedades MSI por lista fechada; parâmetros de outros tipos sem caminho fora da pasta de execução.
8. Task 189: **aguarda o usuário** (proposta: fazer depois da 188, sempre ligada, com o motor nativo).


### Task 190: Correções da revisão de fase do lado elevado (2d)

**Modelo:** opus (revisor opus). Origem: revisão de fase inteira do `bridge-worker` em 2026-10-18 (orquestrador). O achado Critical da mesma revisão (SHA vindo do próprio pedido) é decisão do usuário e **não** entra aqui.

#### Where this fits

- `src/winget_install/switches.rs` (~:130 `fill_log`) e `src/bridge/worker.rs` (~:261–264 `is_log`): `<LOGPATH>` sem aspas é aceito e substituído sem aspas; a pasta de staging (`C:\Program Files\wingetrs\staging\…`) sempre tem espaço.
- `src/bridge/elevate_win.rs` (~:628 `run_winget`) → `src/bridge/spawn_win.rs` (~:191–192 `run_in`): o winget elevado herda o ambiente do worker (sem `env_clear`), que vem de `HKCU\Environment`.
- `src/winget_install/elevated_rules.rs` (~:413–418 `still_orphan`): decide "órfão" por um caminho que pode estar numa pasta que o usuário grava.
- `src/bridge/staging_win.rs` (~:242–247): `FILE_FLAG_OPEN_REPARSE_POINT` só protege o último componente de `source`.
- `src/bridge/arp_ops_win.rs` (~:289–301): desinstalador NSIS/Inno que se copia para TEMP e sai; `RunDir` é solto com o filho ainda rodando.
- Regra do `CLAUDE.md` "Código que roda elevado" (ambiente montado do zero, DACL provada, lista fechada).

#### What binds this task

1. **`<LOGPATH>` com aspas:** o worker só aceita o marcador entre aspas (`"<LOGPATH>"`, como já faz o caminho MSI) **ou** `fill_log` põe as aspas; escolha uma das duas e aplique a todos os tipos (Inno `/LOG=`, Burn/exe `/log`, `/key:`). Caminho com espaço nunca chega partido ao instalador.
2. **winget elevado com ambiente do zero:** `run_winget` usa `env_clear()` + o mesmo `installer_env(sys, <RunDir>\tmp)` dos instaladores, com `TEMP`/`TMP` dentro de um `RunDir` de staging (criado e apagado como nos instaladores). Nenhuma variável vem do ambiente do worker.
3. **Órfão só com pasta de admin:** `still_orphan` só devolve verdadeiro quando o ancestral existente mais próximo do caminho ausente passa por `sddl_admin_only` (o mesmo usado na DACL do staging/ARP); senão, não é órfão (motivo no log).
4. **`source` sem reparse no meio:** abrir cada componente de `source` sem seguir reparse point (ou exigir que `source` fique sob a pasta de dados conhecida do wingetrs, lista fechada) e recusar hard link (`nNumberOfLinks > 1`).
5. **Desinstalador que se copia:** esperar por um job object com os processos filhos antes de soltar o `RunDir`; o código de saída é o do último processo relevante do job (documente a escolha).

#### Tests (literais)

- `log_placeholder_with_space_stays_one_argument`: staging com espaço; para Inno, Burn e exe, a linha de comando montada tem o caminho do log inteiro entre aspas (um argumento só ao dividir com `split_command`).
- `bare_log_placeholder_refused_or_quoted` (conforme a escolha do item 1): teste que falha só nessa regra.
- `winget_env_is_built_from_scratch`: com uma variável plantada no ambiente do processo de teste (`WINGETRS_TEST_PLANTED=1`) e `TEMP` apontando para uma pasta qualquer, o ambiente montado para o winget não tem a variável e o `TEMP` fica dentro do `RunDir`.
- `orphan_in_user_folder_is_not_orphan`: caminho ausente cujo ancestral tem DACL com escrita para outro SID ⇒ não órfão (DACL simulada pelo mesmo ponto de injeção dos testes de `sddl_admin_only`).
- `source_with_junction_in_middle_refused` e `source_hard_link_refused` (tempdir; sem tocar `Program Files`).
- `job_waits_for_copied_uninstaller`: `.cmd` em tempdir que inicia um filho que dorme 1 s e sai antes; o `RunDir` só some depois do filho.

Mutações obrigatórias (cole `[OK] mutação PEGA`): (a) tirar o `env_clear()` do `run_winget`; (b) aceitar `<LOGPATH>` sem aspas; (c) `still_orphan` sem a checagem de DACL.

**Files:**
- Modify: `src/bridge/worker.rs`, `src/bridge/elevate_win.rs`, `src/bridge/spawn_win.rs`, `src/bridge/staging_win.rs`, `src/bridge/arp_ops_win.rs`, `src/winget_install/switches.rs`, `src/winget_install/elevated_rules.rs`
- Test: os módulos de teste existentes desses arquivos em `tests/it/` (não crie módulo novo sem necessidade; se criar, a linha `mod` entra por âncora).

#### Checks beyond the steps

1. Testes nunca rodam instalador real, `runas`, winget real, nem tocam ARP/`Program Files` reais.
2. Outro agente edita `src/install/{runner.rs,script_scan.rs,prelude.ps1}` e testes de scripts: não toque nesses arquivos.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(bridge): phase review of the elevated side (log quoting, winget env, orphan DACL, reparse, job wait)`


### Task 193: CLI do winget igual à TUI (revisão de fase da 2d)

**Modelo:** opus (revisor opus). Origem: revisão de fase dos fluxos nativos da 2d em 2026-10-18 (orquestrador), sobre o HEAD f71ac10.

#### Where this fits

- `src/cli/winget_cmd.rs` ~:282 com `src/winget_install/ops.rs` ~:218,298: `stop.run` faz poll de um future que bloqueia a thread (`lock_msi`, esperas do `run_queued` e retentativas de 1618, `launcher.run` são síncronos dentro de `install_locked`); enquanto bloqueia, `interrupts.next()` nunca é lido. Ctrl+C esperando outro instalador não faz nada nem imprime nada; Ctrl+C durante o instalador nunca mostra "não pode ser interrompido".
- `src/cli/run_win.rs` ~:256 × `src/tui/run.rs` ~:1867: o plano da remoção de um upgrade `uninstallPrevious` passa `nsis = kind == Nullsoft` na TUI e sempre `false` na CLI.
- `src/cli/run_win.rs` ~:246–250 × `src/tui/run.rs` ~:1229: em linhas MSIX, a CLI usa fatos padrão (sem filtro de tipo); a TUI põe `kind: Msix`.
- `src/cli/winget_cmd.rs` ~:464–479: Kindle (D6) incompleto na CLI: `plan.shared` imprime "copie antes de remover", mas a CLI nunca chama `backup_folders` nem oferece "só o registro"; a TUI faz as duas coisas (`src/tui/jobs.rs` ~:780–799, ~:703).
- `src/cli/winget_cmd.rs` ~:469–479 com `run_win.rs` ~:215: a desinstalação nativa na CLI não tem progresso (`UninstallCtx` sem progresso): nenhuma fase, spinner ou relógio enquanto o desinstalador roda, durante a espera do UAC para HKLM e durante a espera de até 120 s pela chave sumir; Ctrl+C é segurado em silêncio; não toma o `install.lock` (a TUI toma em `jobs.rs` ~:697).

#### What binds this task

1. **Ctrl+C na CLI:** a operação roda em `spawn_blocking` (como a TUI) e o laço da CLI continua lendo as interrupções: antes do lançamento cancela e diz isso; depois do lançamento imprime uma vez "o instalador já começou e não pode ser interrompido; aguarde" (texto igual ao da TUI) e espera. Handle do `spawn_blocking` sempre aguardado.
2. **Uma função só para os fatos e o plano de remoção do upgrade:** `previous_plan(...)` e a montagem de `InstalledFacts` vivem em `src/winget_install/` e são chamadas pela CLI e pela TUI (tire as cópias de `run_win.rs` e `tui/run.rs`). Teste de equivalência: mesmas entradas ⇒ mesmo plano na CLI e na TUI, para nullsoft e MSIX.
3. **Kindle na CLI:** se `plan.shared` não está vazio, a CLI pergunta como a TUI (copiar as pastas e remover / só o registro / cancelar) usando as mesmas funções (`backup_folders` etc.; se estiverem em `tui/`, mova para `src/winget_install/` sem mudar o comportamento da TUI). Com `--yes` e `shared` não vazio: recusa com a mensagem "este app compartilha dados com outros; rode sem --yes para escolher". `--yes` nunca escolhe pelo usuário.
4. **Progresso da desinstalação na CLI:** fases `Removing`/`WaitingElevation`/"conferindo a remoção" com spinner e relógio (mesma infraestrutura da Task 141), aviso de Ctrl+C como no item 1, e `lock_install` antes de rodar.

#### Tests (literais)

- `cli_ctrl_c_while_waiting_cancels` e `cli_ctrl_c_after_launch_prints_notice_once` (lançador falso que bloqueia; prazo curto com `tokio::time::timeout`).
- `upgrade_previous_plan_same_in_cli_and_tui` (nullsoft e MSIX).
- `cli_kindle_yes_refused` e `cli_kindle_backup_then_remove` (pastas em tempdir).
- `cli_uninstall_shows_phases` (saída capturada contém a fase e o relógio).

Mutações obrigatórias (cole `[OK] mutação PEGA`): (a) CLI volta a passar `nsis = false`; (b) `--yes` com `shared` não vazio prossegue.

**Files:**
- Modify: `src/cli/winget_cmd.rs`, `src/cli/run_win.rs`, `src/tui/run.rs`, `src/winget_install/` (só arquivos novos ou os que receberem as funções movidas; **não** `engine.rs`, `uninstall.rs`, `codes.rs`, `plan.rs`, `post_check.rs`, `launch_win.rs`, `ops.rs`, `select.rs`, `host_win.rs`, `switches.rs`, `elevated_rules.rs`, que são de outras tarefas em paralelo)
- Test: módulos existentes em `tests/it/` da CLI do winget.

#### Checks beyond the steps

1. `src/tui/jobs.rs` está sendo editado pela Task 191: se precisar mover algo de lá, faça só a remoção da função movida e a nova chamada, e avise no relatório.
2. Testes nunca rodam instalador real, `runas` nem tocam ARP real.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(cli): winget native flows match the TUI (Ctrl+C, previous plan, Kindle, progress)`

### Task 194: Motor nativo — ordem do uninstallPrevious, códigos de saída e conferência (revisão de fase da 2d)

**Modelo:** opus (revisor opus). Origem: a mesma revisão da Task 193.

#### Where this fits

- `src/winget_install/engine.rs` ~:256–265: no `uninstallPrevious`, o app antigo é removido **antes** de baixar o novo (`install_locked` chama `fetch_installer` depois de `uninstall`), e a remoção nunca chama `gate.try_launch()`. Download que falha ou Ctrl+C no meio ⇒ usuário sem app. `tests/it/wi_upgrade.rs` ~:514 trava a ordem `[uninstall, wait_key, install]`.
- `src/winget_install/uninstall.rs` ~:462 com `codes.rs` ~:150: `classify_uninstall` aceita 3010/1641 só para `Rule::Msi`; Burn (VC++ redist, .NET runtime) com 3010 vira "código desconhecido" antes de olhar a chave do ARP.
- `src/winget_install/uninstall.rs` ~:442–460: `msiexec /x` sem `msi.lock` e sem retentativa de 1618 (a instalação tem).
- `src/winget_install/plan.rs` ~:205–254: a confirmação nunca diz que o `uninstallPrevious` remove a versão instalada antes, nem que esse desinstalador pode ser interativo ou pedir UAC.
- `src/winget_install/post_check.rs` ~:25: depois de um upgrade, não compara a versão instalada; "Atualizado: X <nova>" aparece mesmo se o instalador saiu 0 sem mudar nada.
- `src/winget_install/launch_win.rs` ~:102 com `ops.rs` ~:370: UAC recusado de um exe que se eleva sozinho (ERROR_CANCELLED/1223) vira `WingetOpsError::Launch` (Failed); a spec diz Cancelled/`UacRefused`.

#### What binds this task

1. **Baixar e conferir antes de remover:** no `uninstallPrevious`, `fetch_installer` + hash vêm primeiro (como o winget), o arquivo fica aberto/conferido, depois `gate.try_launch()` e só então a remoção e a instalação. Falha no download ⇒ a versão antiga continua instalada. Ajuste o teste de ordem para `[fetch, launch_gate, uninstall, wait_key, install]`.
2. **Burn com reinício:** 3010/1641 de `Rule::Burn` (e de `QuietUninstallString` de bundle) seguem o caminho do msiexec: confere se a chave sumiu antes de declarar falha; sumiu ⇒ sucesso com "reinício necessário".
3. **`msiexec /x` na fila:** toma o `msi.lock` e repete em 1618 como a instalação (mesma função).
4. **Confirmação diz o que o upgrade faz:** com `upgrade_route == UninstallFirst`, uma linha "remove a versão instalada antes (o desinstalador pode abrir janela ou pedir permissão de administrador)".
5. **Conferência da versão:** depois do upgrade, `found.version` diferente de `plan.version` ⇒ aviso "o instalador terminou, mas a versão instalada continua <x>"; comparação por `crate::core::version`.
6. **UAC recusado:** `PermissionDenied`/1223 no lançamento ⇒ `UacRefused` (Cancelled), não `Launch`.

#### Tests (literais)

- `upgrade_previous_download_failure_keeps_old` (fetch falha ⇒ 0 chamadas de uninstall) e `upgrade_previous_order` (`[fetch, launch_gate, uninstall, wait_key, install]`).
- `burn_uninstall_3010_key_gone_is_success` e `burn_uninstall_3010_key_present_is_failure`.
- `msiexec_uninstall_retries_1618`.
- `upgrade_previous_confirmation_mentions_removal`.
- `upgrade_same_version_after_warns` (versões `1.10` × `1.9` e `1.0` × `1.0.0`).
- `self_elevating_uac_refused_is_cancelled`.

Mutações obrigatórias (cole `[OK] mutação PEGA`): (a) voltar a remover antes do fetch; (b) Burn fora do caminho do 3010.

**Files:**
- Modify: `src/winget_install/engine.rs`, `src/winget_install/uninstall.rs`, `src/winget_install/codes.rs`, `src/winget_install/plan.rs`, `src/winget_install/post_check.rs`, `src/winget_install/launch_win.rs`, `src/winget_install/ops.rs`
- Test: `tests/it/wi_upgrade.rs` e os módulos existentes desses arquivos.

#### Checks beyond the steps

1. Arquivos de outras tarefas em paralelo: `src/cli/*`, `src/tui/*` (Tasks 191/193), `src/winget_install/{switches,elevated_rules,select,host_win}.rs`, `src/bridge/*` (Tasks 190/195): não edite.
2. Testes nunca rodam instalador, msiexec nem `runas` reais.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(winget-install): download before removing, burn reboot codes, msi queue on uninstall, version check`

### Task 195: Escolha do instalador — idiomas preferidos e arquiteturas não suportadas (revisão de fase da 2d)

**Modelo:** sonnet (revisor opus). Origem: a mesma revisão da Task 193.

#### Where this fits

- `src/winget_install/host_win.rs` ~:25: o idioma vem de `Control Panel\International\LocaleName` (formato regional), um valor só; o winget usa a lista de idiomas de interface preferidos do usuário.
- `src/winget_install/select.rs` ~:109 (pontuação de locale) e ~:235–282 (filtro de arquitetura): `UnsupportedOSArchitectures` não é lido.
- Referência: `docs/reference/winget-cli/` (filtros de arquitetura e locale do winget; cite o arquivo de origem em comentário de uma linha).

#### What binds this task

1. `host_win` lê a lista `GetUserPreferredUILanguages` (MUI_LANGUAGE_NAME) e entrega `Vec<String>` em ordem; `select` pontua o locale do instalador contra a lista (primeiro idioma vale mais; correspondência só de idioma, `pt` × `pt-BR`, vale menos que exata), como o winget.
2. `select` descarta instaladores cujo `UnsupportedOSArchitectures` contém a arquitetura do sistema.

#### Tests (literais)

- `locale_prefers_ui_language_list` (lista `[en-US, pt-BR]`, instaladores `pt-BR` e `en-US` ⇒ `en-US`).
- `locale_language_only_match_ranks_below_exact`.
- `unsupported_os_architecture_excluded` (arm64 com x64 marcado não suportado ⇒ outro instalador ou nenhum, com motivo).

Mutação obrigatória: ignorar `UnsupportedOSArchitectures` ⇒ teste PEGA.

**Files:**
- Modify: `src/winget_install/host_win.rs`, `src/winget_install/select.rs`
- Test: módulos existentes de `select`.

#### Checks beyond the steps

1. Outras tarefas em paralelo editam `src/winget_install/*` restantes, `src/cli/*`, `src/tui/*`, `src/bridge/*`, `src/install/*`: não edite.
2. O teste de equivalência nativo × ponte (Task 185) continua passando.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(winget-install): preferred UI languages and unsupported architectures in installer selection`
