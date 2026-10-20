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
