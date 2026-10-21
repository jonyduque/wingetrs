# wingetrs — depois da 2d: achados da prova, buckets e releases do GitHub (plano)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-19-wingetrs-pos-2d.md`.

**Origem:** pedido do usuário em 2026-10-19 ("Planeje e implemente solução dos achados. Planeje a inclusão e remoção de buckets do scoop. Também planeje a instalação de releases de repositórios do github"). Análise dos releases: `docs/superpowers/reports/2026-10-19-analise-github-releases.md` (item F do `BACKLOG.md`).

**Goal:** (1) contornar o defeito do winget 1.29 com `--scope`; (2) completar a gestão de buckets do Scoop (CLI endurecida e TUI); (3) `install gh:<dono>/<repo>[@tag]` a partir do release, com atualização e remoção.

**Decisões aplicadas (recomendações da análise, aceitas pelo usuário em 2026-10-19 com "como comentado acima"):**
- **Releases, 1ª versão:** só arquivos compactados (`zip`, `7z`, `tar.gz`, `tar.xz`) e `.exe` solto que o conteúdo classifica como programa. Instalador (`.msi`, `.msix`, `.exe` de setup) é recusado com o motivo ("instalador NSIS"). Sem token do GitHub (60 pedidos/h); versão nova conferida pelo redirecionamento `302` de `/releases/latest`, que não gasta a cota.
- **Releases:** se algum bucket configurado tem manifesto cujo `url` aponta para o mesmo `<dono>/<repo>`, o CLI oferece o manifesto do bucket primeiro (seletor), com o release direto como 2ª opção.
- **Releases:** "origem não revisada" sempre no plano; exige `--allow-unreviewed` na CLI (nunca implícito em `--yes`, como o `--allow-scripts`).
- **Achado do pipe preso por processo neto (Task 106):** já resolvido no código (`src/bridge/spawn_win.rs` `PIPE_GRACE` 2 s + `CancelIoEx`); nenhuma tarefa. Só um teste ao vivo opcional fica para a prova.

## Review Focus

1. Winget 1.29.380 com `--scope` dá `0x8000FFFF` para **qualquer** pacote (medido na prova, seção C, 2026-10-19): a nova tentativa sem escopo tem de dizer que o escopo pedido não foi garantido.
2. Bucket adicionado com repositório inexistente só falha no `update` seguinte: o `bucket add` deve conferir e baixar na hora.
3. `bucket add/rm` sem lock enquanto um `update` reescreve o catálogo e o `search.idx`: corrida.
4. Release cujo arquivo muda de padrão de nome entre versões: a atualização não pode chutar; volta ao seletor.
5. App de release com o mesmo nome de app do Scoop já instalado: recusa clara, nunca sobrescreve.

## Ordem

| Onda | Tarefas | Depende de |
|---|---|---|
| 1 | 206 (buckets: CLI endurecida) ‖ 208 (cliente de releases) ‖ 209 (escolha do arquivo e classificação do `.exe`) | — (paralelas com 189/202 em andamento) |
| 2 | 205 (contorno do `--scope`) ‖ 207 (buckets na TUI) ‖ 210 (`install gh:`) | 202 (205 toca `src/bridge/`), 206, 208+209 |
| 3 | 211 (atualizar e remover apps de release) | 210 |

Arquivos de teste novos (criados pelo orquestrador com uma linha `//!` e `mod` no `tests/it/main.rs`): `bucket_admin` (206), `tui_buckets` (207), `gh_release` (208), `gh_asset` (209), `gh_install` (210, 211), `bridge_scope` (205).

---

### Task 205: Contorno do `--scope` do winget 1.29

**Modelo:** sonnet (revisor sonnet). Origem: prova ao vivo, seção C (2026-10-19): `winget install … --scope user` e `--scope machine` saem com `0x8000FFFF` ("Argument.cpp(512)") no winget 1.29.380 para qualquer pacote; sem `--scope` instalam. Registrado em `docs/ARMADILHAS.md`.

#### Where this fits

- `--scope` sai de um lugar só: `bridge::command` em `src/bridge/mod.rs:380-388` (`BridgeOp::Install { scope: Some(_) }`). Chega por `install_job` (`mod.rs:172`, `default_scope` em `mod.rs:204`: Ajustes `Machine` ⇒ `Some(Machine)`), pela CLI (`src/cli/winget_cmd.rs:170/191/233`, `wanted_scope` em `:362`) e pelo auxiliar elevado (`src/bridge/worker.rs:720-750`).
- Nova tentativa existente (Task 176): `run_bridge_install` em `src/tui/jobs.rs:862-905` repete uma vez sem escopo quando o código é `NO_APPLICABLE_INSTALLER` (`0x8A150010`, `mod.rs:162`), só se o escopo veio do padrão dos Ajustes (`scope_fallback`), com `SCOPE_FALLBACK_NOTE` (`mod.rs:166`).
- Tabela de mensagens: `winget_message` em `src/bridge/mod.rs:574-600` (sem `0x8000FFFF`).

#### What binds this task

1. Constante `WINGET_SCOPE_CRASH: u32 = 0x8000FFFF` em `src/bridge/mod.rs`.
2. Em `run_bridge_install`: código `0x8000FFFF` com `op` levando `scope: Some(_)` ⇒ uma nova tentativa sem `--scope`, **qualquer que seja a origem do escopo** (Ajustes ou `--scope` explícito), com a nota nova `SCOPE_CRASH_NOTE` = "o winget falhou com --scope (defeito do winget 1.29); instalando sem escopo: o winget escolhe usuário ou máquina". A 2ª tentativa usa o mesmo caminho (elevado ou não) da 1ª; estado por tentativa zerado (ARMADILHAS "Laço de nova tentativa").
3. Sem `scope` no `op`, `0x8000FFFF` não repete.
4. `winget_message(0x8000FFFF)` = "o winget falhou internamente (0x8000FFFF)".
5. A nota aparece no log do job, na CLI (formatador de `cli/output.rs`) e no toast da TUI, como a nota da Task 176.

Testes (`tests/it/bridge_scope.rs`, launcher falso como nos testes da 176):
1. `scope_crash_retries_without_scope_from_settings`.
2. `scope_crash_retries_without_scope_when_explicit`.
3. `scope_crash_without_scope_does_not_retry`.
4. `second_attempt_fails_for_another_reason_reports_that_reason`.
5. `no_applicable_installer_rule_unchanged` (a regra da 176 continua só para o padrão dos Ajustes).

**Files:** `src/bridge/mod.rs`, `src/tui/jobs.rs`, `src/cli/output.rs` (só a nota), `tests/it/bridge_scope.rs`.

#### Checks beyond the steps

1. Mutação: tirar o braço `0x8000FFFF` ⇒ teste 1 **PEGA**.
2. Mutação: exigir `scope_fallback` também no braço novo ⇒ teste 2 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(bridge): retry without --scope when winget 1.29 crashes on it`

---

### Task 206: Buckets do Scoop: `add`/`rm` com lock, conferência e download na hora

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `src/buckets.rs` (230 linhas): `KNOWN` (10 conhecidos, `:7-18`), `parse_repo` (`:64-87`, só GitHub), `add` (`:110`, `NeedsConfirmation` para terceiro), `remove` (`:162`, recusa com apps instalados), `remove_and_cleanup` (`:184-230`: tira `scoop:<nome>` do catálogo, reescreve o `search.idx`, apaga `buckets\<nome>`, tira a URL do `http.json`).
- CLI: `run_bucket` em `src/cli/mod.rs:960-988`. **Não pega lock nenhum**; `update` pega `UpdateLock` (`src/update.rs:193-200`). `bucket add` só grava o `config.toml` e diz "rode `wingetrs update`" (`:967-978`): repositório errado só aparece no `update` seguinte.
- Download de um bucket: `fetch_bucket(http, url, prev)` + `read_tarball` (`src/sources/scoop/bucket.rs:165-177`), URL `repo_tarball_url` (`bucket.rs:46`, `tar.gz/HEAD`); gravação no catálogo por `replace_source` (`src/catalog/mod.rs:360`); scripts em `<raiz>\buckets\<b>\scripts` (`src/update.rs:313-330`).
- `bucket list` mostra só nome e repositório (`src/cli/output.rs:107-110`).
- Resto legado: `bucket_url` com `refs/heads/master` (`bucket.rs:31-40`) e entradas antigas no `http.json`.

#### What binds this task

1. `bucket add` e `bucket rm` pegam o `UpdateLock` (o mesmo do `update`); ocupado ⇒ "uma atualização está em andamento; tente de novo" (erro tipado, sem espera infinita).
2. `bucket add` baixa o bucket na hora com o mesmo `fetch_bucket`/`read_tarball` do `update`, grava só a fonte `scoop:<nome>` no catálogo, reescreve o `search.idx`, e **só então** grava o `config.toml`. Falha (404, offline, tarball sem `bucket/*.json`) ⇒ nada gravado, mensagem com o motivo ("repositório não encontrado: <dono>/<repo>"). Progresso com spinner, fase e relógio (regra do CLAUDE.md).
3. Tarball sem nenhum manifesto ⇒ recusa "não parece um bucket do Scoop (nenhum manifesto em bucket/)".
4. `bucket list` mostra nome, repositório, número de apps no catálogo e data da última atualização (do `http.json` ou do catálogo); "não baixado" quando faltar.
5. `bucket rm` de `main` ou `extras` pede confirmação ("bucket padrão"); com `--yes` passa.
6. `remove_and_cleanup` também tira do `http.json` as URLs legadas `refs/heads/master` do mesmo bucket.

Testes (`tests/it/bucket_admin.rs`, `wiremock` com tarball montado no teste, raiz em tempdir):
1. `add_downloads_and_indexes_now`.
2. `add_404_writes_nothing` (config, catálogo e `search.idx` iguais aos bytes de antes).
3. `add_tarball_without_manifests_refused`.
4. `add_and_rm_refuse_while_update_lock_held`.
5. `list_shows_app_count_and_date`.
6. `rm_default_bucket_needs_confirmation`.
7. `rm_drops_legacy_master_url`.

**Files:** `src/buckets.rs`, `src/cli/mod.rs` (`run_bucket`), `src/cli/output.rs` (lista e mensagens), `src/sources/scoop/bucket.rs` (só se precisar expor o download de um bucket), `src/update.rs` (só reaproveitar o download de um bucket, sem mudar o `update`), `tests/it/bucket_admin.rs`.

#### Checks beyond the steps

1. Mutação: gravar o config antes do download ⇒ teste 2 **PEGA**.
2. Mutação: tirar o lock do `add` ⇒ teste 4 **PEGA**.
3. Ao vivo (orquestrador, nunca o agente): `bucket add extras` com raiz de teste.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(buckets): lock, verify and download on bucket add; richer bucket list`

---

### Task 207: Buckets na TUI

**Modelo:** opus (revisor opus).

#### Where this fits

- Hoje a TUI só mostra a linha "Buckets do scoop  <nomes>" (`src/tui/view.rs:289-290`; `app.buckets_summary` em `src/tui/run.rs:220-223`, campo em `src/tui/app.rs:806-807`, região em `src/tui/regions.rs:57`). Não adiciona, não remove.
- Operações da Task 206 em `src/buckets.rs` (add com download, rm com limpeza, lock).
- Padrões da TUI: modais (`Modal::*`), fila de operações (Task 96), seletor com setas, gravação do `config.toml` lê-modifica-grava (Ajustes, Task 68), cores só de `tui/theme.rs`, glifos de `tui::theme::Glyphs`.

#### What binds this task

1. Ação "Buckets" (tecla `b` na aba Ajustes e item no menu de ações) abre o modal `Modal::Buckets` com a lista: nome, repositório, nº de apps, última atualização, marca "conhecido"/"de terceiro".
2. `a` adiciona: seletor com os conhecidos ainda não configurados + "Outro repositório do GitHub…" (campo de texto `dono/repo` ou URL; validação de `parse_repo` ao digitar). Terceiro ⇒ confirmação "bucket de terceiro: código e binários de terceiros". Roda como operação da fila, com progresso e relógio.
3. `d`/Delete remove: confirmação; com apps instalados do bucket, mostra quais e recusa (mesma regra da CLI). Bucket padrão pede confirmação extra.
4. Ao terminar, a linha "Buckets do scoop" e a busca refletem o catálogo novo sem reiniciar a TUI.
5. Lock ocupado (update em segundo plano) ⇒ a operação espera na fila (como `Busy` da Task 96), nunca falha calada.

Testes (`tests/it/tui_buckets.rs`, estado da TUI com operações falsas, como os testes da fila):
1. `buckets_modal_lists_known_and_third_party`.
2. `add_known_runs_as_queued_operation`.
3. `add_third_party_asks_confirmation`.
4. `remove_with_installed_apps_refused_with_names`.
5. `summary_line_updates_after_add`.
6. `busy_lock_requeues`.

**Files:** `src/tui/app.rs`, `src/tui/view.rs`, `src/tui/run.rs`, `src/tui/jobs.rs`, `src/tui/regions.rs`, `tests/it/tui_buckets.rs`.

#### Checks beyond the steps

1. Mutação: pular a confirmação de terceiro ⇒ teste 3 **PEGA**.
2. Retrato da tela do modal (`examples/tui_frame.rs` ou teste de buffer) colado no relatório.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(tui): manage scoop buckets`

---

### Task 208: Cliente de releases do GitHub

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- Medido em 2026-10-19 (relatório de análise): `GET https://api.github.com/repos/{o}/{r}/releases/latest` traz `tag_name`, `prerelease`, `draft`, `immutable`, `assets[] {name, size, digest: "sha256:<hex>", browser_download_url, content_type}`; 100% dos arquivos de 13 repositórios com `digest`. Sem token: `X-RateLimit-Limit: 60`. `https://github.com/{o}/{r}/releases/latest` responde `302` para `/releases/tag/<tag>` sem gastar a cota.
- HTTP: `src/http.rs` `Http` (reqwest 0.13.5 sem feature `json`; parse com `serde_json`); `get` (`:208`), `get_limited` (`:219`); política de redirecionamento própria (`redirect_allowed`, `:36`). Não há cabeçalho customizado.
- Teste: padrão de `tests/it/http.rs` (`MockServer::start().await`, `Mock::given(method("GET")).and(path(..))`).

#### What binds this task

1. Módulo novo `src/github/mod.rs` (`pub mod github;` em `src/lib.rs`): `GhRepo { owner, repo }` com `parse("dono/repo")` (mesmas regras de nome do `buckets::parse_repo`: letras, dígitos, `-`, `_`, `.`; sem `..`); `Release { tag, prerelease, immutable, assets: Vec<Asset { name, size, sha256: [u8; 32], url }> }`.
2. `latest(http, base, repo)` e `by_tag(http, base, repo, tag)` (`/releases/tags/<tag>`), com `base` injetável (wiremock). JSON lido com `get_limited` (teto 4 MiB). Arquivo sem `digest` ou com digest que não é `sha256:` + 64 hex ⇒ esse arquivo fica fora, com motivo no log (nunca sem hash).
3. `latest_tag(http, base_web, repo)`: segue o `302` e tira a tag do último segmento de `/releases/tag/<tag>` (decodificado de URL), sem gastar a cota; método novo em `Http` que devolve a URL final (`resp.url()`), sem baixar o corpo.
4. Erros tipados (`thiserror`): `Offline`, `NotFound` (repositório ou release), `RateLimited { reset: Option<u64> }` (403/429 com `X-RateLimit-Remaining: 0`, lendo `X-RateLimit-Reset`), `NoRelease`, `BadJson`.
5. URL de download só `https://github.com/<o>/<r>/releases/download/…` ou `https://objects.githubusercontent.com/…` (lista fechada); outra origem ⇒ arquivo fora.

Testes (`tests/it/gh_release.rs`, fixtures JSON com os nomes e digests reais de ripgrep, bat, jq, Notepad++ e PowerShell, tirados de `releases/latest` em 2026-10-19 e reduzidos aos campos usados):
1. `latest_parses_assets_with_digest`.
2. `asset_without_digest_dropped`.
3. `rate_limited_reports_reset`.
4. `not_found_is_typed`.
5. `latest_tag_from_redirect`.
6. `foreign_download_host_dropped`.
7. `by_tag_uses_tags_path`.

**Files:** `src/github/mod.rs` (novo), `src/lib.rs`, `src/http.rs` (só o método da URL final), `tests/it/gh_release.rs`, `tests/fixtures/github/*.json`.

#### Checks beyond the steps

1. Mutação: aceitar arquivo sem `digest` ⇒ teste 2 **PEGA**.
2. Mutação: aceitar host fora da lista ⇒ teste 6 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(github): releases client with asset digests and redirect tag`

---

### Task 209: Escolher o arquivo do release e classificar o `.exe`

**Modelo:** opus (revisor opus).

#### Where this fits

- Medido (relatório de análise, 12 repositórios): filtro por nome Windows + x64 dá escolha única em 3/12; com desempates, 8/12; 4/12 (hugo, PowerShell, Windows Terminal, Notepad++) precisam do usuário. Notepad++ não diz "win" nos nomes (repositório só de Windows).
- `.exe` instalador × programa (medido): `jq.exe` e `lazygit.exe` com overlay 0; instalador do Notepad++ com overlay de 6,7 MB de 7 MB, marca `NullsoftInst`, `requireAdministrator`. `src/install/sniff.rs`: `sniff(head)` (`:26`) ⇒ `Sniffed::{SevenZip, Zip, Pe, Cab, Other}`; `pe_overlay_start(head)` (`:43`).
- Tipos da Task 208: `Asset { name, size, sha256, url }`.

#### What binds this task

1. `src/github/pick.rs`: `pick(assets, host: Arch) -> Pick` com `Pick::{One(Asset), Choose(Vec<Asset>), None(motivo)}`. Ordem:
   - descarta: `.sha256`, `.sha512`, `.sig`, `.asc`, `.pem`, `.sbom`, `.pdb`, `checksums`, `.txt`, `.json`, `.deb`, `.rpm`, `.apk`, `.dmg`, `.pkg`, `.AppImage`, `.tar.bz2`, `.zst`, nomes com `linux`, `darwin`, `macos`, `apple`, `android`, `freebsd`, `source`/`src`;
   - arquitetura: tokens de `x86_64|amd64|x64|win64` = x64, `aarch64|arm64` = arm64, `i686|i386|x86|win32|386|32-bit` = x86; nome sem token de arquitetura vale para qualquer;
   - Windows: token `windows|win|pc-windows|msvc|mingw` **ou**, se nenhum arquivo do release tem token de SO, todos os restantes (repositório só de Windows, caso Notepad++);
   - desempates, nesta ordem: mesmo `sha256` = mesmo arquivo; `msvc` > `gnu`; compactado/`.exe` > `.msi`/`.msix`/`.msixbundle`/`.appx`; arquitetura exata > sem token;
   - sobrou 1 ⇒ `One`; mais ⇒ `Choose` (nunca chute).
2. `src/install/sniff.rs`: `classify_pe(bytes) -> PeKind::{Program, Installer(&'static str)}` (nome do gerador: "NSIS", "Inno Setup", "WiX Burn", "7-Zip SFX", "InstallShield", "Squirrel"; "instalador" quando só o overlay decide). Regras: marca do gerador ⇒ instalador; overlay **sem** a assinatura Authenticode (entrada 4 do diretório de dados, `IMAGE_DIRECTORY_ENTRY_SECURITY`) maior que 25% do arquivo e maior que 1 MiB ⇒ instalador; senão programa. Todo número lido do PE com teto antes de usar (ARMADILHAS).
3. `.msi`/`.msix`/`.appx` escolhidos ⇒ recusa nesta versão ("instalador MSI: fora do escopo da instalação por release").

Testes (`tests/it/gh_asset.rs`): tabela com as listas reais de nomes dos 12 repositórios (fixtures da 208 mais as listas de nomes coladas no teste), esperando `One` em bat, fzf, zoxide, ripgrep (msvc), lazygit (x86_64), jq (mesmo digest), neovim (zip), gh (zip); `Choose` em hugo, PowerShell, Terminal, Notepad++. PE sintéticos montados no teste: programa sem overlay; programa assinado (overlay = só a assinatura) ⇒ programa; NSIS; overlay grande sem marca ⇒ instalador; cabeçalho com `NumberOfSections` absurdo ⇒ erro, sem pânico.

**Files:** `src/github/pick.rs` (novo), `src/github/mod.rs`, `src/install/sniff.rs`, `tests/it/gh_asset.rs`.

#### Checks beyond the steps

1. Mutação: tirar o desconto da assinatura ⇒ teste do programa assinado **PEGA**.
2. Mutação: tirar o desempate por `sha256` ⇒ caso jq **PEGA**.
3. Mutação: tirar a regra "nenhum token de SO" ⇒ caso Notepad++ **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(github): pick the release asset and tell installers from programs`

---

### Task 210: `install gh:<dono>/<repo>[@tag]`

**Modelo:** opus (revisor opus).

#### Where this fits

- Planejador do Scoop: `manifest::resolve(raw_json, host)` (`src/install/manifest.rs:473`) aceita JSON cru (`version`, `url`, `hash` com prefixo `sha256:`, `extract_dir`, `bin`, `#/nome` para renomear); `plan_install(&PlanInput)` (`src/install/planner.rs:379`); a entrada real `ops::plan_for(ctx, &Candidate, keep_old)` (`src/install/ops.rs:564`) exige fonte `scoop:` e lê o manifesto do catálogo (`:396-401`), calcula `sha = sha256(raw)` (`:627`).
- CLI: `install()` em `src/cli/install_cmd.rs:751` (`choose` `:675`, `plan_for` `:775`, `print_plans`, `third_party_gate`, `--dry-run`, `ops::apply_with`). `target::split_bucket` (`src/install/target.rs:28`) leria `gh:dono` como bucket: o desvio vem antes.
- Estado: `apps(app, bucket, …, manifest_sha256)` (`src/install/state.rs:77`), `install.json` + `manifest.json` na pasta da versão (`state.rs:245`, `executor.rs:1319`).
- Nome do app: `layout::app_dir` não valida; `safepath::safe_components` com `len()==1` (como `buckets.rs:212`).
- Tipos das Tasks 208/209: `github::{GhRepo, Release, Asset, latest, by_tag}`, `github::pick::{pick, Pick}`, `sniff::{classify_pe, PeKind}`.

#### What binds this task

1. `gh:<dono>/<repo>[@<tag>]` reconhecido no topo de `install()` (antes de `choose`).
2. **Bucket primeiro:** se algum manifesto do catálogo `scoop:*` tem `url` (ou `architecture.*.url`) começando com `https://github.com/<dono>/<repo>/releases/download/`, seletor "manifesto do bucket <b>/<app> (revisado)" × "release direto (origem não revisada)". Com `--yes` sem TTY ⇒ bucket.
3. Release direto: `latest`/`by_tag`, `pick` (`Choose` ⇒ seletor; sem TTY ou `--yes` ⇒ erro listando os nomes e sugerindo `--asset <nome>`); `--asset <nome>` escolhe pelo nome exato.
4. Manifesto sintético em JSON (gravado como `manifest.json`, como os do Scoop): `version` = tag sem `v` inicial; `url` = URL do arquivo (com `#/<nome>` quando a extensão não diz o formato); `hash` = `sha256:<hex>`; `extract_dir` = pasta única do topo, quando houver (decidido depois da extração, no plano mostrado como "pasta do topo, se houver"); `bin` = `.exe` do topo classificados como programa (`classify_pe`), mostrados no plano; `.exe` solto ⇒ `bin` com ele mesmo.
5. `ops::plan_for_raw(ctx, app, bucket, raw_json, keep_old)`: irmã de `plan_for` sem catálogo nem dependências; `plan_for` passa a chamar o mesmo núcleo (um caminho só, sem duplicar o planejamento).
6. Estado: `bucket = "github:<dono>/<repo>"`; app = nome do repositório em minúsculas, passando por `safe_components` com um componente; app já instalado com outro `bucket` ⇒ recusa "já existe um app <nome> de <bucket>".
7. Confiança: o plano mostra "⚠ origem não revisada: <dono>/<repo>" e "release imutável: sim/não"; sem `--allow-unreviewed` ⇒ recusa com a dica (nunca implícito em `--yes`). `.exe` classificado como instalador ⇒ recusa "instalador <gerador>: fora do escopo da instalação por release".
8. `--dry-run` mostra URL, digest e arquivo escolhido (regra do CLI: URL e hash só no `--dry-run`).

Testes (`tests/it/gh_install.rs`, wiremock servindo a API e o zip montado no teste, raiz em tempdir, sem rede real):
1. `gh_zip_installs_with_shim`.
2. `gh_offers_bucket_manifest_first`.
3. `gh_ambiguous_without_tty_lists_assets`.
4. `gh_asset_flag_picks_by_name`.
5. `gh_requires_allow_unreviewed`.
6. `gh_installer_exe_refused`.
7. `gh_name_clash_refused`.
8. `gh_digest_mismatch_installs_nothing`.
9. `plan_for_and_plan_for_raw_same_plan` (equivalência: o mesmo JSON pelo catálogo e cru dá o mesmo `InstallPlan`).

**Files:** `src/cli/mod.rs` (flags `--allow-unreviewed`, `--asset`), `src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/install/ops.rs`, `src/github/install.rs` (novo), `src/github/mod.rs`, `tests/it/gh_install.rs`.

#### Checks beyond the steps

1. Mutação: `--yes` aceitar origem não revisada ⇒ teste 5 **PEGA**.
2. Mutação: aceitar `.exe` instalador ⇒ teste 6 **PEGA**.
3. Ao vivo (orquestrador, com pedido do usuário): `install gh:sharkdp/bat --allow-unreviewed --dry-run` e um install real numa raiz de teste.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): install from GitHub releases (gh:owner/repo)`

---

### Task 211: Atualizar, listar e remover apps de release

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `ops::upgradable` (`src/install/ops.rs:1147`) procura `catalog.get("scoop:"+bucket, app)`; app com `bucket = "github:…"` cai em `skipped` ("não está mais no catálogo"). `upgrade` (`src/cli/install_cmd.rs:934`, `rsplit('/')` em `:948`, `resolve_installed` em `ops.rs:407`) só pelo catálogo.
- Task 208: `github::latest_tag` (sem cota) e `latest`; Task 209: `pick`; Task 210: manifesto sintético, `plan_for_raw`, `manifest.json` gravado na versão (`state::installed_manifest`, `state.rs:269`).
- Remoção: o caminho dos apps do wingetrs já não depende do catálogo (conferir; ajustar só se depender).

#### What binds this task

1. `list --upgradable` e a TUI: apps `github:` conferem a versão por `latest_tag` (um pedido web por app, concorrência limitada como a TUI faz com manifestos); versão comparada por `crate::core::version` (nunca como texto).
2. `upgrade <app>` de release: `latest` + `pick` com a regra salva (nome do arquivo instalado, com a versão trocada pela nova); se o nome novo não bate ⇒ seletor (sem TTY ⇒ erro com os nomes). Mesma exigência de `--allow-unreviewed`.
3. Limite da API atingido no `upgrade` ⇒ erro tipado com a hora de liberação; no `list --upgradable` ⇒ a linha diz "não conferido (limite do GitHub até HH:MM)".
4. `uninstall` de app `github:` funciona como qualquer app do wingetrs.

Testes (`tests/it/gh_install.rs`, continuação):
1. `upgradable_uses_redirect_tag`.
2. `upgrade_keeps_asset_pattern`.
3. `upgrade_pattern_changed_asks`.
4. `rate_limited_upgrade_reports_reset`.
5. `uninstall_github_app`.
6. `version_compared_semantically` (`1.10.0` > `1.9.0`).

**Files:** `src/install/ops.rs`, `src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/github/install.rs`, `src/tui/jobs.rs` (só a conferência de versão), `tests/it/gh_install.rs`.

#### Checks beyond the steps

1. Mutação: comparar versão como texto ⇒ teste 6 **PEGA**.
2. Mutação: aceitar nome de arquivo diferente sem perguntar ⇒ teste 3 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): upgrade and list GitHub release apps`

---

## Extras de 2026-10-21 (pedido do usuário)

Pedido: "que o wingetrs tenha um comando update e que quando acesse a TUI seja feita a checagem de atualização disponível. Os prompts de sim ou não no CLI devem ser com botões. Os prompts que aparecem durante a instalação, desinstalação, atualização do CLI devem ser formatados, inclusive aqueles que são do winget, msiexec, etc." Decisões do usuário (2026-10-21): `wingetrs self update` (o `update` continua sendo o catálogo); na TUI, aviso na barra mais uma tecla que confirma e atualiza; saída do winget traduzida nas linhas conhecidas, as demais recuadas e esmaecidas. O `install.ps1` já instala para todos os usuários quando rodado elevado (`C:\Program Files\wingetrs`, PATH da máquina; commit e5a1e1b).

Sonda do orquestrador (2026-10-21, `winget download --id jqlang.jq` com stdout redirecionado, `--disable-interactivity`): **nenhuma barra de progresso nem spinner** com saída redirecionada; o texto sai no idioma do Windows (aqui pt-BR), em UTF-8 com `\r\n`. As 6 linhas, literais:

```
Encontrado jq [jqlang.jq] Versão 1.8.2
Este aplicativo é licenciado para você pelo proprietário.
A Microsoft não é responsável por, nem concede licenças a pacotes de terceiros.
Baixando https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-windows-amd64.exe
Hash do instalador verificado com êxito
Instalador baixado: C:\...\dl\jq_1.8.2_X64_portable_en-US.exe
```

| Onda | Tarefas | Depende de |
|---|---|---|
| 4 | 212 (botões Sim/Não) ‖ 215 (saída do winget) | trabalho em andamento de 189/208 commitado (tocam `wi_cli.rs`, `winget_cmd.rs`) |
| 5 | 213 (`self update`) | 208 (`github::latest_tag`, `Http::final_url`) |
| 6 | 214 (aviso na TUI) | 213 |

### Task 212: Sim/Não do CLI com botões

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- `src/cli/picker.rs`: `yes_no(term, question, theme, glyphs) -> io::Result<bool>` (`:450`) chama `select_one(..., Some(1), Letters::Pick)` e desenha a lista vertical (`s) Sim` / `n) Não`). Desenho: `draw()` (`:266`, `MoveUp(drawn)` + `Clear(FromCursorDown)`, linhas cortadas em `cols-1` com `\r\n`); `Session::enter`/`drop` (raw mode, cursor oculto, apaga o desenhado); depois da escolha, uma linha de resumo `"{prompt} {label}"` (`:389`). Sem TTY ⇒ erro `Unsupported` com `NO_TTY` (mantém).
- Único chamador de `picker::yes_no`: `TtyUi::confirm` (`src/cli/install_cmd.rs:225`); `ui.confirm(` em `install_cmd.rs:550`, `cli/mod.rs:952` e `:1005`.
- A TUI já tem botões: `CONFIRM_BUTTONS` (`src/tui/app.rs:428-439`), desenhados em `draw_confirm` (`src/tui/view.rs:1386`) como `" {label} "` com `theme.accent()` + `REVERSED` no focado.
- Testes que fixam o desenho vertical e mudam: `tests/it/cli_cancel.rs` `selector_yes_no_defaults_to_no` (`:725`), `tests/it/cli_scripts.rs` (`:1053`, `:1056`, `:1137`), `tests/it/wi_cli.rs` (`:1891-1915`). Terminal falso: `picker::fake::ScriptedTerm`, `fake::key`, `fake::ctrl_c`, `fake::plain`.

#### What binds this task

1. `yes_no` desenha **uma** linha: `"{question}  [ Sim ]  [ Não ]"`; o botão focado com o estilo de foco do CLI (via `src/cli/style.rs`, nunca `anstyle` fora dele), o outro esmaecido; uma linha de dica abaixo (`←/→ escolher · Enter confirmar · s/n · Esc = Não`). Foco inicial em **Não**.
2. Teclas: `←`/`→`/`Tab`/`BackTab` trocam o foco; `Enter` escolhe o focado; `s` ⇒ Sim e `n` ⇒ Não na hora; `Esc`, `q` e `Ctrl+C` ⇒ Não (cancelar continua sendo "não"). Eventos de soltar tecla e combinações Ctrl/Alt (exceto Ctrl+C) ignorados, como hoje.
3. Ao sair: apaga o desenho e escreve a mesma linha de resumo de hoje (`"{question} Sim"` / `"{question} Não"`), para o histórico do terminal não mudar.
4. `select_one`/`select_many` não mudam.

Testes (`tests/it/cli_cancel.rs`, troca o `selector_yes_no_defaults_to_no` e acrescenta):
1. `yes_no_buttons_one_line` (a tela depois do 1º desenho contém `Continuar?  [ Sim ]  [ Não ]` numa linha só).
2. `yes_no_enter_keeps_no`.
3. `yes_no_arrow_then_enter_is_yes` (`←` + `Enter` ⇒ `true`).
4. `yes_no_letters_pick_at_once` (`s` ⇒ `true`; `n` ⇒ `false`; uma leitura de tecla cada).
5. `yes_no_esc_and_ctrl_c_are_no`.
6. `yes_no_summary_line_unchanged` (termina com `"Continuar? Sim\r\n"`).
E ajustar as asserções de `cli_scripts.rs` e `wi_cli.rs` para o desenho novo (o resumo final não muda).

**Files:** `src/cli/picker.rs`, `src/cli/style.rs` (só se faltar estilo de botão), `tests/it/cli_cancel.rs`, `tests/it/cli_scripts.rs`, `tests/it/wi_cli.rs`.

#### Checks beyond the steps

1. Mutação: foco inicial em Sim ⇒ teste 2 **PEGA**.
2. Mutação: `Esc` ⇒ Sim ⇒ teste 5 **PEGA**.
3. Colar no relatório a tela do teste 1 (texto sem CSI, via `fake::plain`).
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): yes/no prompt as buttons`

### Task 213: `wingetrs self update`

**Modelo:** opus (revisor opus: código que roda elevado).

#### Where this fits

- Release: `github.com/jonyduque/wingetrs`, arquivos `wingetrs.exe` e `wingetrs.exe.sha256` (texto `<hex64>  wingetrs.exe`, servido como `application/octet-stream`). Tags `vX.Y.Z`. Versão atual: `env!("CARGO_PKG_VERSION")`.
- Task 208: `github::latest_tag(http, base_web, &GhRepo) -> Result<String, GhError>` (redirecionamento `302`, sem cota), `GhError::{Offline, NotFound, RateLimited, …}`, `Http::final_url`. `Http::get_to_file(url, &Path, progress)` (`src/http.rs:292`), `get_limited` (`:219`).
- Clap: `enum Command` (`src/cli/mod.rs:98`); grupo aninhado como `Bucket { #[command(subcommand)] action: BucketAction }` (`:292-308`, `subcommand_required`, `HELP_TEMPLATE`, `after_help` em português); despacho em `run_async` (`:698`). O snapshot da ajuda raiz (`tests/it/snapshots/it__cli_parse__help_*.snap`) muda.
- Escopo: o auxiliar elevado só sobe se o exe e cada pasta acima tiverem DACL só de administrador: `bridge::arp_ops_win::hold_admin_chain_with(program, sddl_of) -> io::Result<Vec<File>>` (`:270`; a versão de produção `hold_admin_chain` é privada, `:264`). Elevação: `winget_install::launch` `Launcher` + `LaunchSpec { program, params, dir, show, verb: Verb::RunAs }` (`launch.rs:15-51`, real em `launch_win.rs:22`). Aviso antes do UAC: o texto e o caminho que os outros pedidos de elevação do CLI já usam ("Aguardando autorização para executar como administrador").
- `install.ps1` faz a mesma troca: baixa para dentro da pasta de instalação, confere o SHA, renomeia o exe em uso para `.old`, move o novo.

#### What binds this task

1. Módulo `src/selfupdate/mod.rs` (lógica pura e testável) + `src/selfupdate/apply_win.rs` (só o que é Windows). Funções:
   - `newer(current: &str, tag: &str) -> Option<Version>`: compara `X.Y.Z` numericamente (`v` inicial opcional na tag); tag fora de `^v?\d+\.\d+\.\d+$` ⇒ `None` (nunca compara como texto).
   - `apply(http, base_dl, tag, exe: &Path) -> Result<(), SelfUpdateError>`: baixa `{base_dl}/{tag}/wingetrs.exe` (tag **fixa**, nunca `latest/download`, para o exe e o `.sha256` serem do mesmo release) para `<pasta do exe>\wingetrs.exe.download-<aleatório>`; confere o SHA-256 contra o `.sha256` do mesmo release; troca (`.old` antigo apagado se der, exe em uso renomeado para `.old`, novo movido); temporário apagado em todo caminho de erro.
2. Escopo pela pasta do exe (`std::env::current_exe`):
   - pasta que o usuário grava ⇒ `apply` direto, sem UAC;
   - cadeia só de administrador (`hold_admin_chain`) ⇒ aviso de elevação e `Launcher` com `Verb::RunAs` sobre o **próprio exe**, parâmetros literais `self apply-elevated` (subcomando oculto, sem nenhum argumento vindo do lado não elevado); o processo elevado busca a tag sozinho (`latest_tag`), valida o formato, roda `apply` e responde só pelo código de saída (`0` atualizado, `10` já na última, `20` sem internet, `21` SHA não confere, `22` outro erro), mapeado para português no lado não elevado. O lado elevado não grava nada fora da pasta do exe (nada em `%LOCALAPPDATA%`, nada de log).
   - exe de build de desenvolvimento (existe `Cargo.toml` duas pastas acima da pasta do exe) ⇒ recusa: "build de desenvolvimento: atualize com cargo".
3. CLI: `wingetrs self update [--check] [--yes]`. Sem versão nova ⇒ "o wingetrs já está na última versão (0.2.1)". Com versão nova ⇒ mostra `0.2.1 → 0.2.2` e confirma por `ui.confirm` (botões da Task 212), `--yes` pula; `--check` só informa (código de saída 0 com ou sem versão nova). Progresso com fases ("Conferindo versão", "Baixando", "Conferindo SHA-256", "Trocando o executável") pelo formatador de progresso existente. No fim: "wingetrs atualizado para 0.2.2".
4. Ao iniciar (`main.rs`), apagar `<exe>.old` se existir, em silêncio (escopo máquina sem permissão: ignora).
5. Erros tipados (`thiserror`): `Offline`, `RateLimited`, `HashMismatch`, `BadSumFile`, `DevBuild`, `Elevation(code)`, `Io`.

Testes (`tests/it/self_update.rs`, novo; wiremock para o release; exe falso num tempdir):
1. `newer_compares_numbers` (`0.10.0` > `0.9.9`; mesma versão ⇒ `None`; tag `latest`/`v1.2` ⇒ `None`).
2. `apply_replaces_exe` (exe falso trocado, conteúdo novo, nenhum `.download-*` sobrando).
3. `apply_hash_mismatch_keeps_exe` (exe antigo intacto, temporário apagado).
4. `apply_uses_pinned_tag_paths` (pedidos só em `/{tag}/wingetrs.exe` e `/{tag}/wingetrs.exe.sha256`).
5. `bad_sum_file_refused` (`.sha256` que não é 64 hex).
6. `dev_build_refused` (`Cargo.toml` duas pastas acima).
7. `elevated_scope_uses_runas_self_apply` (`Launcher` falso: um `LaunchSpec` com `Verb::RunAs`, programa = o exe, `params == "self apply-elevated"`; código `21` vira `HashMismatch`).
8. `cli_parse_self_update` (`self update --check`, `self update --yes`; `self apply-elevated` oculto na ajuda).

**Files:** `src/selfupdate/mod.rs` (novo), `src/selfupdate/apply_win.rs` (novo), `src/lib.rs`, `src/main.rs` (limpeza do `.old`), `src/cli/mod.rs`, `src/cli/output.rs`, `src/bridge/arp_ops_win.rs` (só tornar `hold_admin_chain` `pub(crate)`), `tests/it/self_update.rs` (novo; `mod` no `main.rs`), snapshot da ajuda raiz.

#### Checks beyond the steps

1. Mutação: baixar de `latest/download` em vez da tag ⇒ teste 4 **PEGA**.
2. Mutação: pular a conferência do SHA ⇒ teste 3 **PEGA**.
3. Mutação: passar a tag como argumento ao processo elevado ⇒ teste 7 **PEGA**.
4. Nada real: testes nunca usam `runas` nem tocam `Program Files`; a prova ao vivo fica para o usuário.
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): self update from the GitHub release`

### Task 214: Aviso de versão nova na TUI

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- Partida da TUI: `tui::run::run` (`src/tui/run.rs:201`) chama `spawn_boot(&ctx)` e `spawn_job(&ctx, |c, _| detect_tools(c))`; `spawn_job(ctx, work)` (`:1660`) manda o `Msg` resultante por `ctx.tx`. `Ctx` (`:104`) tem `http`, `data`, `cfg`. `enum Msg` (`src/tui/app.rs:73`), `enum Cmd` (`:580`), `update(app, msg) -> Vec<Cmd>` (`:1599`).
- Barra de status: `draw_status` (`src/tui/view.rs:1040`); à direita, pílulas (`theme.pill_a()`, `theme.alert()`, `muted`). Toasts: `app.push_toast(ToastKind, text)` (`app.rs:986`).
- Teclas: `on_key` (`app.rs:2101`); Ctrl usados: C, R, Espaço, S, E. **Ctrl+U livre.** Ajuda: `draw_help` (`view.rs:1104`) e `help::hints` (`src/tui/help.rs:34`).
- Botões de confirmação já existentes: `CONFIRM_BUTTONS` (`app.rs:428`), `draw_confirm` (`view.rs:1386`), `confirm_mouse` (`app.rs:2046`).
- Task 213: `selfupdate::newer`, a função que faz a atualização completa (mesma do CLI) e o mapeamento de erros para texto.

#### What binds this task

1. Ao abrir, um job de fundo confere a versão no máximo **uma vez a cada 24 h**: cache `self-update.json` em `DataDir` (`{ "checked_at": <unix>, "latest": "v0.2.2" }`); dentro das 24 h usa o cache sem rede. Sem internet, limite do GitHub ou erro ⇒ silêncio (nada na tela, só `tracing`).
2. Versão nova ⇒ `Msg::SelfUpdateAvailable { tag }` e uma pílula à direita na barra: `wingetrs 0.2.2 · ctrl+u` (token de tema existente; nada de `Color::` fora de `theme.rs`). Linha nova na ajuda.
3. `Ctrl+U` (só com a pílula visível) abre um modal com os botões existentes: "Atualizar o wingetrs de 0.2.1 para 0.2.2?". Continuar ⇒ job que roda a **mesma** função de atualização do CLI (escopo máquina ⇒ aviso "Aguardando autorização para executar como administrador" antes do UAC); sucesso ⇒ toast "wingetrs atualizado para 0.2.2; feche e abra de novo para usar a versão nova" e a pílula some; erro ⇒ toast de erro com o texto do erro tipado.
4. Uma operação de instalação rodando ⇒ `Ctrl+U` entra na fila como as outras (ou recusa com toast "espere a operação atual"; escolha uma e teste).

Testes (`tests/it/tui_self_update.rs`, novo; relógio e cache injetados, sem rede real):
1. `check_skipped_within_24h` (cache recente ⇒ nenhum pedido ao servidor falso).
2. `check_refreshes_after_24h`.
3. `offline_check_is_silent` (sem toast, sem pílula).
4. `pill_shows_new_version` (snapshot da barra).
5. `ctrl_u_without_update_does_nothing`.
6. `ctrl_u_opens_confirm_and_runs_update` (`Cmd` da atualização emitido só depois de Continuar).

**Files:** `src/tui/app.rs`, `src/tui/run.rs`, `src/tui/view.rs`, `src/tui/help.rs`, `src/selfupdate/mod.rs` (só o cache), `tests/it/tui_self_update.rs` (novo; `mod` no `main.rs`).

#### Checks beyond the steps

1. Mutação: ignorar o cache ⇒ teste 1 **PEGA**.
2. Mutação: toast no erro de rede ⇒ teste 3 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(tui): notice and Ctrl+U for a new wingetrs version`

### Task 215: Saída do winget formatada no CLI e na TUI

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- Sonda acima: com stdout redirecionado o winget não desenha barra nem spinner; as linhas saem no idioma do Windows. Linhas conhecidas hoje no código: "Found/Encontrado X [Id] Version/Versão v" (`bridge::winget_chosen_version`, `src/bridge/mod.rs:541`), "Successfully installed" / "Instalado com êxito" (`tests/it/job_log.rs:47-89`, `tests/it/bridge.rs:483`), textos de reinício (`REBOOT_MARKERS`, `:476`). Normalização: `bridge::plain()` (`:490`, tira ANSI e acentos).
- CLI: as linhas chegam por `output::bridge_progress` (`src/cli/output.rs:1697`), que imprime toda linha mostrada como `"  {line}"`; `install::progress::bridge_line` (`src/install/progress.rs:188`) só trata o aviso de elevação. TUI: `Msg::BridgeLine` ⇒ `on_bridge_line` (`app.rs:3119`) ⇒ `set_bridge_line` põe `"{ação} · {linha}"` no rótulo em execução. O log de jobs guarda as linhas cruas (fica assim).
- Regra do CLI (CLAUDE.md): URL, hash e caminhos de staging só no `--dry-run`; glifos de `tui::theme::Glyphs`; toda saída pelos formatadores de `cli/output.rs`.
- Instaladores nativos e `msiexec`: o resultado já vira texto em português por `winget_install::codes` (`classify_install`, `message(Category)`); o `msiexec` roda `/quiet` e não tem saída de texto. Janela própria de instalador que ignora o modo silencioso não é formatável (fora do escopo).

#### What binds this task

1. Módulo puro `src/bridge/winget_out.rs`: `classify(line: &str) -> WingetLine`, comparando por `plain()` (sem acento, minúsculas), com textos em **inglês e português**:
   - `Found { name, id, version }` ("Found … [id] Version v" / "Encontrado … [id] Versão v");
   - `License` ("This application is licensed to you by its owner." / "Este aplicativo é licenciado para você pelo proprietário."; "Microsoft is not responsible for, nor does it grant any licenses to, third-party packages." / "A Microsoft não é responsável por, nem concede licenças a pacotes de terceiros.");
   - `Downloading { url }` ("Downloading " / "Baixando ");
   - `HashOk` ("Successfully verified installer hash" / "Hash do instalador verificado com êxito");
   - `Starting` ("Starting package install..." / "Starting package uninstall..." / "Iniciando a instalação do pacote..." / "Iniciando a desinstalação do pacote...");
   - `Done` ("Successfully installed" / "Instalado com êxito" / "Successfully uninstalled" / "Desinstalado com êxito");
   - `NoUpgrade` ("No available upgrade found." / "No newer package versions are available from the configured sources."; textos em português que não estão nesta lista ficam como `Other` e a lista é completada na prova ao vivo);
   - `Other(text)`.
   Uma tabela só (`const`), um teste por entrada.
2. CLI (`bridge_progress`): `Found` ⇒ `{glifo} {name} {version}`; `License` ⇒ esmaecido, **uma** linha "licença de terceiros: aceita pelo --accept-package-agreements" no lugar das duas; `Downloading` ⇒ fase "Baixando o instalador" (URL só no `--dry-run`); `HashOk` ⇒ `{glifo ok} hash do instalador conferido`; `Starting` ⇒ fase "Instalando"/"Removendo"; `Done` ⇒ linha de sucesso com o glifo de ok; `NoUpgrade` ⇒ "nenhuma atualização encontrada pelo winget"; `Other` ⇒ recuada e esmaecida com prefixo `winget:`. Estilos só por `cli/style.rs` / formatadores de `output.rs`.
3. TUI: `set_bridge_line` usa o mesmo `classify` para o rótulo curto em português (ex.: `Instalar · baixando o instalador`); `Other` mostra o texto cru como hoje.
4. Equivalência: CLI e TUI nunca classificam por caminhos diferentes — os dois chamam `winget_out::classify`.

Testes (`tests/it/winget_out.rs`, novo):
1. `probe_lines_classified` (as 6 linhas literais da sonda ⇒ `Found`, `License`, `License`, `Downloading`, `HashOk`, `Other`).
2. `english_lines_classified` (as entradas em inglês da tabela).
3. `ansi_and_case_ignored`.
4. `cli_hides_download_url` (saída formatada de `Downloading` sem a URL).
5. `cli_license_lines_merged` (duas linhas ⇒ uma).
6. `other_lines_dimmed_with_prefix`.
7. `tui_label_uses_classify` (rótulo `· baixando o instalador`).

**Files:** `src/bridge/winget_out.rs` (novo), `src/bridge/mod.rs` (só o `mod`), `src/cli/output.rs`, `src/install/progress.rs`, `src/tui/app.rs` (só `set_bridge_line`), `tests/it/winget_out.rs` (novo; `mod` no `main.rs`).

#### Checks beyond the steps

1. Mutação: comparar sem `plain()` ⇒ teste 3 **PEGA**.
2. Mutação: mostrar a URL de `Downloading` ⇒ teste 4 **PEGA**.
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(bridge): format winget output lines in the CLI and TUI`

### Task 218: Terminal de administrador recusado já no plano

**Modelo:** opus (revisor opus).

> **Achado da prova ao vivo (2026-10-21), decisão do usuário (opção A):** num terminal de administrador, (1) `wingetrs install yori --allow-scripts` mostrou o plano, perguntou "Continuar?", extraiu e só no passo 16 recusou o `post_install` ("o wingetrs não roda scripts de manifesto com o processo elevado"), desfazendo tudo; (2) remover kondo-ui e hyperfine (registros HKCU do winget, desinstalador `winget uninstall --product-code`) falhou com `0x8A15007D: The package installed for user scope cannot be uninstalled when running with administrator privileges`. As duas recusas passam a sair no plano, antes de qualquer pergunta ou efeito. Rodar o winget sem elevação a partir do terminal admin é o item H do BACKLOG (não fazer aqui).

#### Where this fits

- Recusa de hoje, na execução (fica como defesa): `src/install/script_policy.rs:63` `refuse_if_elevated(elevated: bool) -> Result<(), String>`; chamada em `src/install/script_host_win.rs:89-91` (`PwshHost.elevated: fn() -> io::Result<bool>`), `src/install/installer_win.rs:23-30` e `src/install/inno_win.rs:22-29` (`run_with(..., elevated: fn() -> io::Result<bool>, launch)`). Token real: `src/winget_install/launch_win.rs:157` `is_elevated()`.
- Código de terceiro no plano do Scoop: `src/cli/output.rs:1146` `plan_third_party(plan: &InstallPlan) -> Vec<ThirdParty>` (CLI); `src/install/plan.rs:141` `InstallPlan.has_uninstall_scripts` (remoção); TUI monta o mesmo bloco em `src/tui/app.rs:2881`. Portão do CLI: `src/cli/install_cmd.rs:509` `third_party_gate(code, f)` (recusa sem `--allow-scripts` em `:522-525`, antes de `proceed`/"Continuar?" em `:541-550`); remoção Scoop: `install_cmd.rs:832` e `src/tui/jobs.rs:327` (`ops::plan_uninstall`).
- Rota winget do ARP: `src/winget_install/uninstall.rs:237` `winget_product_code(c) -> Option<String>`; plano `plan_uninstall` (`:350-373`, `UninstallPlan`, `:94-99`); hive em `ArpDetails.hive` (`Hive::User` / `MachineX64` / `MachineX86`); execução `uninstall_by_winget` (`:553-580`), desvio em `uninstall_queued` (`:669-670`). Plano no CLI: `src/cli/run_win.rs:258-275` (`previous_win::plain_removal_plan`); TUI: `Cmd::PlanWingetUninstall` (`src/tui/app.rs:623`, `:2415`; `src/tui/run.rs:465`).
- Testes de hoje das recusas: `tests/it/script_host_win.rs` (`elevated_refused` `:244`, `elevated_refusal_starts_nothing` `:516`), `tests/it/install_inno.rs:575`, `tests/it/install_file_installer.rs:673`; rota winget: `tests/it/wi_cli.rs:2129` `product_code_line_is_removed_by_winget`.

#### What binds this task

1. Função pura nova em `src/install/script_policy.rs`: `pub fn admin_terminal_refusal(elevated: bool, reason: AdminBlock) -> Option<String>` com `pub enum AdminBlock { ThirdPartyCode { what: String }, WingetUserScope }`. Textos (únicos, usados por CLI e TUI):
   - `ThirdPartyCode`: `"{what} é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita"` (`what` = o rótulo que o bloco "Script de terceiro (…)" já usa, ex. `script post_install`, `instalador (installer.file)`, `innounp`).
   - `WingetUserScope`: `"pacote instalado só para você: o winget não o remove de um terminal de administrador; abra um terminal sem administrador e repita"`.
2. **Scoop (install, upgrade, uninstall), CLI e TUI:** com o processo elevado e o plano trazendo código de terceiro (o que `plan_third_party` lista; na remoção, `has_uninstall_scripts`), o comando recusa **antes** do bloco de consentimento, antes de "Continuar?" e antes de baixar/extrair — vale com ou sem `--allow-scripts`, e com `--yes`. No `--dry-run` o plano aparece inteiro e a recusa sai como linha de aviso no fim (saída 0). Lote: o item recusado vira "pulado" com o motivo; os outros seguem.
3. **winget ARP:** registro com `winget_product_code(..).is_some()` e `hive == Hive::User` + processo elevado ⇒ mesma recusa no plano (`WingetUserScope`), CLI e TUI, antes de qualquer pergunta. Registro HKLM pela rota winget não muda.
4. O "está elevado?" entra por injeção (`fn() -> io::Result<bool>` como nos `run_with`; produção = `launch_win::is_elevated`), nunca chamado direto da lógica testável. Erro ao consultar o token ⇒ tratar como elevado (recusa; fail-safe).
5. As recusas da execução (`refuse_if_elevated`) continuam onde estão.
6. Upgrade de pacote winget pela ponte com escopo usuário num terminal admin: **fora** desta tarefa (não medido); só registrar no relatório se o código mostrar o mesmo risco.

Testes (`tests/it/admin_terminal.rs`, novo; `mod` no `main.rs`):
1. `refusal_texts_fixed` (os dois textos literais acima, e `None` com `elevated = false`).
2. `scoop_script_refused_before_confirm` (CLI, terminal falso: plano com `post_install`, elevado ⇒ erro com o texto, nenhuma pergunta lida do terminal, nada extraído na raiz temporária).
3. `scoop_script_allowed_when_not_elevated` (mesmo plano, não elevado ⇒ segue para o bloco de consentimento como hoje).
4. `dry_run_shows_plan_and_warning` (saída 0, plano + linha de aviso).
5. `batch_skips_only_the_blocked_item`.
6. `winget_user_scope_refused_in_plan` (registro HKCU com `winget uninstall --product-code`, elevado ⇒ recusa sem chamar a ponte; `FakeBridge` sem chamadas).
7. `winget_machine_scope_unchanged` (mesmo registro em HKLM ⇒ plano como hoje).
8. `cli_and_tui_same_refusal_text` (equivalência: o texto que a TUI mostra no toast/modal = o do CLI, para os dois casos).
9. `token_error_counts_as_elevated`.

**Files:** `src/install/script_policy.rs`, `src/cli/install_cmd.rs`, `src/cli/output.rs` (só a linha de aviso do `--dry-run`, se preciso), `src/cli/run_win.rs`, `src/cli/winget_cmd.rs`, `src/winget_install/uninstall.rs` (só o que o plano precisar), `src/tui/app.rs`, `src/tui/jobs.rs`, `tests/it/admin_terminal.rs` (novo), `tests/it/main.rs` (só o `mod`).

#### Checks beyond the steps

1. Mutação: tirar a recusa do plano do Scoop (deixar só a da execução) ⇒ teste 2 **PEGA**.
2. Mutação: recusar também `Hive::MachineX64` ⇒ teste 7 **PEGA**.
3. Mutação: erro do token ⇒ não elevado ⇒ teste 9 **PEGA**.
4. Colar no relatório a saída do teste 2 (texto sem CSI).
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat: refuse third-party code and user-scope winget removals in an admin terminal at plan time`

### Task 222: CI no GitHub Actions

**Modelo:** opus (revisor opus).

> **Pedido do usuário (2026-10-21).** O repositório é público e não tem `.github/`; hoje só o `scripts/verify.ps1` local protege o `main`. A CI roda o mesmo gate em Windows a cada push e pull request, sem segredo e sem publicar nada.

#### Where this fits

- Gate local: `scripts/verify.ps1` (passos `rules`, `rules-selftest`, `fmt`, `licenses`, `clippy`, `test`; `-NoTests`; resumo curto e log em `target/verify.log`; `INSTA_UPDATE=no`). Licenças: `scripts/third_party.ps1 -Check` (usa `cargo metadata --offline`: exige `cargo fetch` antes).
- Toolchain: `rust-version`/edição em `Cargo.toml` (README: Rust 1.93 ou mais novo); `Cargo.lock` travado (CI usa `--locked`).
- **Armadilha conhecida:** o runner `windows-latest` roda como administrador sem UAC (token elevado). Localmente, 10 testes falham elevados por desenho: os de `tests/it/script_host_win.rs`, `tests/it/install_file_installer.rs` e `staging_dacl_has_no_users` (recusa de script/instalador com processo elevado e DACL do staging). Eles testam justamente o comportamento não elevado.
- Testes `#[ignore = "live: ..."]` nunca rodam na CI (rede/máquina real).

#### What binds this task

1. `.github/workflows/ci.yml`: gatilhos `push` (todas as branches) e `pull_request`; `permissions: contents: read` no topo; `concurrency` por ref com `cancel-in-progress: true`; um job `windows-latest` com timeout de 60 min.
2. Toda action fixada por **SHA de commit** (com a versão em comentário), não por tag: `actions/checkout`, a de toolchain Rust e a de cache (`Swatinem/rust-cache` ou `actions/cache` com chave do `Cargo.lock`). Nada de `pull_request_target`; nenhum `secrets.*`; `persist-credentials: false` no checkout.
3. Passos: toolchain (com `rustfmt` e `clippy`) → `cargo fetch --locked` → `pwsh -File scripts/verify.ps1`. Se o gate falhar, o job imprime as linhas `[FAIL]` e o fim de `target/verify.log` (≤ 200 linhas) e sobe o log como artifact (retenção 7 dias).
4. **Testes que exigem processo não elevado:** cada um checa `launch_win::is_elevated()` e, **só quando `CI=true` e o processo está elevado**, termina cedo com `eprintln!("pulado: processo elevado na CI ({motivo})")`. Fora da CI, elevado continua falhando como hoje (o aviso de reabrir o Claude sem elevação continua valendo). Um helper único (`tests/it/support` ou equivalente já existente) em vez de cópia em cada teste. O `check_rules.ps1` ganha uma regra: o helper só pode ser chamado nos testes da lista fechada (os 10 de hoje), para a CI não virar desculpa para pular teste novo.
5. O job registra no resumo (`$GITHUB_STEP_SUMMARY`) quantos testes foram pulados por elevação (contando as linhas `pulado: processo elevado na CI`).
6. Badge de CI no README, ao lado dos atuais.
7. Fora do escopo: workflow de release (build, `.sha256`, anexar `NOTICE`/`THIRD_PARTY_LICENSES.txt`), assinatura do executável, matriz de plataformas.

Testes:
1. `skip_helper_only_in_ci_and_elevated` (unitário do helper com as duas entradas injetadas: `ci=false, elevated=true` ⇒ não pula; `ci=true, elevated=false` ⇒ não pula; `ci=true, elevated=true` ⇒ pula).
2. Regra nova do `check_rules.ps1` com caso no `check_rules_selftest.ps1` (chamada fora da lista ⇒ violação).
3. Prova real: um push numa branch de teste mostra o job verde, e o log mostra os 10 "pulado" e nenhum outro. **Push só pelo orquestrador**, com pedido do usuário; o implementador termina com commits locais e o YAML validado por `actionlint` se estiver disponível (se não, diga "não validado").

**Files:** `.github/workflows/ci.yml` (novo), helper de teste (novo ou existente em `tests/it/`), os testes da lista (só a chamada do helper), `scripts/check_rules.ps1`, `scripts/check_rules_selftest.ps1`, `README.md` (badge).

#### Checks beyond the steps

1. Mutação: helper pula sem checar `CI` ⇒ teste 1 **PEGA**.
2. Mutação: chamada do helper num teste fora da lista ⇒ regra do `check_rules` **PEGA**.
3. `grep -n "uses:" .github/workflows/ci.yml` colado no relatório: toda linha com `@<40 hex>`.
4. Gate local: `pwsh -File scripts/verify.ps1` (num terminal não elevado).

**Commit:** `ci: GitHub Actions running the verify gate on Windows`
