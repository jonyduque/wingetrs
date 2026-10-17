Status: DONE_WITH_CONCERNS

Commits:
3bc9827 feat(bridge): do not offer winget upgrades that winget would refuse (UpgradeBehavior, scope)

Gate:
[OK]   rules 8s
[OK]   rules-selftest 17s
[OK]   fmt 2s
[OK]   clippy 11s
[OK]   test (1023 passed, 25 ignored) 53s
[OK]   gate verde
(rodado com src/install/extract_msi.rs e tests/it/install_msi.rs de outro agente modificados e não commitados no checkout)

Mutation proofs:
1. Ignorar o UpgradeBehavior do instalador (só o da raiz):
   pwsh -File scripts/mutate.ps1 -Path src/bridge/mod.rs -Anchor '!i.upgrade_behavior' -Replacement '!manifest.upgrade_behavior' -TestCommand "cargo test --quiet --test it winget_applicability::installer_overrides_root_upgrade_behavior"
   [OK] mutação PEGA: '!i.upgrade_behavior' -> '!manifest.upgrade_behavior' fez o teste falhar (exit 101).
2. Instalador sem escopo tratado como incompatível:
   pwsh -File scripts/mutate.ps1 -Path src/bridge/mod.rs -Anchor 'offered(i).is_none_or(|s| s == installed)' -Replacement 'offered(i).is_some_and(|s| s == installed)' -TestCommand "cargo test --quiet --test it winget_applicability::unscoped_installer_fits_any_scope"
   [OK] mutação PEGA: 'offered(i).is_none_or(|s| s == installed)' -> 'offered(i).is_some_and(|s| s == installed)' fez o teste falhar (exit 101).
   (Na 1ª versão ela SOBREVIVEU: o escopo oferecido vinha do 1º instalador, que sem escopo dava Ok. Agora vem do 1º instalador com escopo explícito (`find_map`), e o teste 4 ganhou o caso "sem escopo x64 + user x86, instalado máquina ⇒ Ok".)
3. Não excluir explicit_only do lote: NÃO RODADA. Não há código a mutar: "atualizar tudo" (U com a vista vazia ⇒ Cmd::PlanUpgradeAll ⇒ jobs::plan_upgrade_all ⇒ ops::upgradable) só planeja apps do wingetrs, e nenhuma linha da ponte entra em lote automático (o único lote com ponte é upgrade_marked, marcação manual = pedido explícito, como `winget upgrade <id>`). O orquestrador confirmou A, com a condição de não existir atalho que marque várias linhas automaticamente: conferido, não existe — a única escrita em `App.marks` é `toggle_mark` (src/tui/app.rs:1033, uma linha por vez), chamada só por Espaço, clique e item "Marcar" do menu (app.rs:1827, 1874, 1959, 1963, 3126). O teste 9 (`explicit_only_left_out_of_upgrade_all`) fixa o comportamento: com duas linhas da ponte (uma explicit_only) o U só dá `[PlanUpgradeAll]`, sem confirmação nem fila; e a linha explicit_only sozinha abre a confirmação via winget.

Check 4 (binário real): `target/release/wingetrs.exe list --upgradable` mostra Android Studio (2026.2 → 2026.2.1.8) e Anki (26.5.0 → 26.09.3) como hoje; a CLI não ganhou nota (o brief só pede a TUI). A TUI eu não consigo abrir/dirigir: NÃO VISTO. Em vez disso, o teste live `live_applicability_of_installed_winget_rows` (ignore, release) roda scan real + correlate + `merged_manifest(Some(latest))` + `winget_applicability` (as mesmas funções da busca da TUI) e imprimiu:
   Google.AndroidStudio 2026.2.1.8 scope=Some(Machine): Denied explicit_only=true note=Some("o editor não permite atualizar pelo winget")
   Anki.Anki 26.09.3 scope=Some(Machine): ScopeMismatch { installed: Machine, offered: User } explicit_only=false note=Some("a versão nova só instala para usuário; a instalada é para máquina")
   as outras 14 linhas da ponte (Git, PowerToys, Edge, VS Code...) deram Ok sem nota.

Decisions:
- `winget_applicability` devolve `WingetFit { applicability, explicit_only }` (o brief pede `-> Applicability` e `explicit_only` "à parte no resultado"; as duas coisas juntas pedem um struct).
- Ordem: arquitetura (vazio ⇒ Unknown) → sem `deny` (vazio ⇒ Denied) → escopo entre os permitidos (ScopeMismatch, `offered` do 1º com escopo explícito) → Ok. Escopo desconhecido no manifesto conta como "sem escopo". Arquiteturas: espelho de `install::manifest::Arch::preference` + `neutral` (`bridge::host_arch`).
- `upgrade_target` passou a devolver `Cow<'static, str>` (a nota de escopo é formatada). O veredito vale só para o `latest` com que foi buscado (`App.applicability: app_key -> (latest, fit)`); erro de busca apaga o veredito anterior.
- Busca em `run.rs::spawn_applicability`: semáforo de 4, JoinSet aguardado, pânico ⇒ `Msg::Panicked`, geração nova (rescan) para de iniciar; sair da TUI derruba o runtime. `details.rs` (fora da lista Files) ganhou `merged_manifest`, e `details` passou a chamá-la — motivo: a TUI precisa do `MergedManifest`, não do `PackageDetails`.
- `ListRow.scope` é serializado (`list --json` ganha `"scope"`); nenhum teste de JSON quebrou.

Concerns:
- Mutação 3 sem prova (ver acima): a regra de explicit_only não tem onde agir hoje; se a ponte entrar num "atualizar tudo" no futuro, o filtro precisa ser escrito e testado lá.
- `spawn_applicability` (rede real dentro da TUI) não tem teste unitário; coberto só pelo live acima, que chama as mesmas funções fora do runtime da TUI.
- A CLI `list --upgradable` continua listando Android Studio/Anki sem indicação.
