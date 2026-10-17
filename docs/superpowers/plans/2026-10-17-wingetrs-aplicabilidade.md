# wingetrs — aplicabilidade antes de oferecer a ponte do winget (Task 135)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 135 -Plan docs/superpowers/plans/2026-10-17-wingetrs-aplicabilidade.md`.

**Goal:** a TUI não oferece "atualizar via winget" para um app que o winget vai recusar, e diz o motivo antes de o usuário tentar.

**Origem (sessão do usuário em 2026-10-17, logs do winget):**
- `Google.AndroidStudio` falhou com `0x8A150114`. O manifesto da 2026.2.1.8 (cache do wingetrs em `blobs/`) tem `UpgradeBehavior: deny`, `RequireExplicitUpgrade: true`, `Scope: machine`.
- `Anki.Anki` falhou com `0x8A15002B`. O instalado é de escopo máquina (ARP em HKLM, `location` "máquina 64-bit"), e a 26.09.3 só tem instaladores `Scope: user` (x64 e arm64, wix). O winget não troca o escopo numa atualização.

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- O manifesto vem de `sources::winget::details::details` (rede + `BlobStore` verificado por hash). Nenhum acesso novo a arquivos do winget.
- Testes sem rede: fixtures YAML pequenas escritas no teste (ou `wiremock`, como nos testes atuais de `details`).
- Sem crate nova. Mensagens ao usuário em português.
- Cada regra portada do winget-cli cita o arquivo de origem em comentário de uma linha (referência em `docs/reference/winget-cli/`; se o arquivo não estiver lá, cite o caminho no repositório do winget-cli, commit de `SOURCE_COMMIT.txt`).

## Review Focus

1. Sem rede ou manifesto que não baixa: o comportamento é o de hoje (ponte oferecida), nunca um bloqueio.
2. Instalador sem `Scope` (nem no instalador nem na raiz) serve para qualquer escopo.
3. `UpgradeBehavior` na raiz e sobrescrito no instalador (o do instalador vence).
4. Lista com dezenas de apps atualizáveis: a busca dos manifestos não trava o desenho da TUI e tem limite de concorrência.

---

### Task 135: Aplicabilidade do winget (UpgradeBehavior, escopo, RequireExplicitUpgrade)

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/sources/winget/manifest.rs`: `MergedManifest` (~:5–25, campos PascalCase; tem `scope` e `installer_type` na raiz) e `Installer` (~:63; `architecture`, `installer_type`, `scope`, `installer_url`, `installer_sha256`); `parse` (~:25 do bloco `impl`) e `into_details`.
- `src/sources/winget/details.rs`: `details(...)` (~:10) baixa versionData ⇒ manifesto, com cache em `BlobStore`.
- `src/installed/correlate.rs`: `ListRow` (~:65; tem `origin`, `correlation`, `state`, `location`); `location()` (~:234) mapeia `InstallSource::Arp(Hive::MachineX64|MachineX86|User)`.
- `src/bridge/mod.rs`: `bridge_target(r, pins) -> Result<BridgeJob, &'static str>` (~:48); o `Err` é a nota do item de menu desabilitado.
- `src/tui/app.rs` (~:2075–2100): escolha entre `Target::Ours` e `Target::Bridge` e as notas do menu; `Cmd::PlanUpgradeAll` (~:442, ~:2367).

#### What binds this task

1. **Manifesto:** `MergedManifest` e `Installer` ganham `upgrade_behavior: Option<String>` e `require_explicit_upgrade: Option<bool>` (o do instalador vence o da raiz, como `scope` já faz).
2. **Escopo instalado tipado:** `ListRow` ganha `scope: Option<InstallScope>` (`Machine`/`User`), vindo da colmeia do ARP (HKLM ⇒ `Machine`, HKCU ⇒ `User`; MSIX/scoop/wingetrs ⇒ `None`).
3. **Função pura** `bridge::winget_applicability(manifest: &MergedManifest, scope: Option<InstallScope>, arch: &str) -> Applicability` com `Applicability::{Ok, Denied, ScopeMismatch { installed, offered }, Unknown}`:
   - instaladores da arquitetura da máquina (mesma regra de arquitetura que `install`/`show` já usam; se nenhum casa, `Unknown`);
   - algum com `upgrade_behavior` = `deny` (sem caixa) e nenhum aplicável sem `deny` ⇒ `Denied`;
   - `scope` conhecido e todos os candidatos com escopo explícito diferente ⇒ `ScopeMismatch`;
   - instalador sem escopo serve para qualquer um;
   - senão `Ok`. `require_explicit_upgrade` vira um campo à parte (`explicit_only: bool`) no resultado, não muda o `Applicability`.
4. **TUI:** depois que a lista carrega, para cada linha com ponte winget possível (`bridge_target` Ok, ferramenta winget) e estado `Available`, buscar o manifesto da versão `latest` em segundo plano (no máximo 4 ao mesmo tempo, mesmo runtime dos outros jobs de rede, cancelável ao sair), guardar por `app_key`, e redesenhar quando chegar.
   - `Denied` ⇒ item "Atualizar via winget" desabilitado com a nota `"o editor não permite atualizar pelo winget"`.
   - `ScopeMismatch` ⇒ desabilitado com `"a versão nova só instala para {offered}; a instalada é para {installed}"` (`máquina`/`usuário`).
   - `explicit_only` ⇒ fica fora de "atualizar tudo" (e de qualquer seleção em lote automática), mas pode ser atualizado sozinho.
   - `Unknown`, erro de rede ou manifesto que não chegou ⇒ igual a hoje.
5. **Coluna/indicador:** a linha atualizável com `Denied`/`ScopeMismatch` mostra a versão disponível no estilo apagado (`theme`), para não parecer pendência acionável. Sem token de cor novo fora de `tui/theme.rs`.

**Files:**
- Modify: `src/sources/winget/manifest.rs`, `src/installed/correlate.rs`, `src/bridge/mod.rs`, `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/jobs.rs` (se a busca morar lá), `src/tui/view*.rs` (indicador)
- Test: `tests/it/winget_applicability.rs` (**já criado vazio e registrado pelo orquestrador**; não edite `tests/it/main.rs`)

Testes (`tests/it/winget_applicability.rs`):
1. `android_studio_denied`: manifesto com `UpgradeBehavior: deny`, `RequireExplicitUpgrade: true`, `Scope: machine`, instalador x64 ⇒ `Denied`, `explicit_only = true`.
2. `installer_overrides_root_upgrade_behavior`: raiz `deny`, instalador x64 com `UpgradeBehavior: install` ⇒ `Ok`; e o inverso ⇒ `Denied`.
3. `anki_scope_mismatch`: instalado `Machine`, instaladores x64 e arm64 só `Scope: user` ⇒ `ScopeMismatch { installed: Machine, offered: User }`.
4. `unscoped_installer_fits_any_scope`: instalado `Machine`, um instalador sem escopo ⇒ `Ok`.
5. `unknown_scope_is_ok`: `scope = None` com instaladores só `user` ⇒ `Ok`.
6. `no_installer_for_arch_is_unknown`: só arm64 numa máquina x64 ⇒ `Unknown`.
7. `row_scope_from_hive`: `ListRow` de HKLM x64, HKLM x86, HKCU, MSIX ⇒ `Machine`, `Machine`, `User`, `None`.
8. `menu_note_for_denied_and_mismatch` (TUI, estado sintético como nos testes de `tui_state`): com a aplicabilidade guardada, o item da ponte está desabilitado com a nota exata do item 4; sem aplicabilidade guardada, habilitado como hoje.
9. `explicit_only_left_out_of_upgrade_all`: duas linhas atualizáveis pela ponte, uma `explicit_only` ⇒ "atualizar tudo" só leva a outra.
10. `fetch_failure_keeps_today_behavior`: busca de manifesto com erro ⇒ item habilitado, sem nota.

#### Checks beyond the steps

1. Mutação: ignorar o `upgrade_behavior` do instalador (usar só o da raiz) ⇒ teste 2 **PEGA**.
2. Mutação: tratar instalador sem escopo como incompatível ⇒ teste 4 **PEGA**.
3. Mutação: não excluir `explicit_only` do lote ⇒ teste 9 **PEGA**.
4. Rode o binário real (`target/release/wingetrs.exe list --upgradable` e, se possível, a TUI) e diga se Android Studio e Anki aparecem com a nota; o que não conseguir ver, diga.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): do not offer winget upgrades that winget would refuse (UpgradeBehavior, scope)`
