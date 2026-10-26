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

**Nota da revisão da 209 (2026-10-22):** o nome do arquivo do release vira caminho só depois de passar por `install::safepath` (teste com nome `..\x.exe` recusado). Num `Choose`, depois da escolha do usuário, o chamador roda `unsupported()` (`.msi`/`.msix` sozinhos já dão `None` no `pick`), com teste. A marca `SquirrelSetup` não foi provada (o nupkg pode estar no `.rsrc`).

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

**Emenda do usuário (2026-10-26) — substitui a recusa de instalador do item 7 e o teste 6.** Origem: `wingetrs install https://github.com/jonyduque/eprocia` caiu na busca do catálogo. O release v0.3.0 tem `eprocia-0.3.0-windows-amd64-instalador.exe`, `eprocia-0.3.0-windows-amd64.exe` e `SHA256SUMS.txt`.

- **URL aceita.** `https://github.com/<dono>/<repo>` (com ou sem `/` no fim, `.git`, `www.`) e `https://github.com/<dono>/<repo>/releases/tag/<tag>` viram `gh:<dono>/<repo>[@<tag>]`, reconhecidos no mesmo ponto do item 1. Outras URLs do GitHub (issues, blob) são recusadas com uma dica. Dono e repositório só com `[A-Za-z0-9._-]`.
- **Instalador é o padrão.** Quando o `pick` do mesmo build oferece um `.exe` classificado como instalador (`classify_pe`) e um programa, ou só o instalador, o plano usa o instalador sem perguntar. O plano mostra "instalador <gerador> (origem não revisada)" e "se falhar: instala só <programa>". `--asset` continua escolhendo pelo nome exato.
- **Como o instalador roda.** O arquivo é baixado no cache do wingetrs e conferido pelo `digest` SHA-256 da API (sem digest, recusa). Ele roda com os privilégios do usuário e o wingetrs nunca o eleva. Se o próprio instalador pedir UAC, antes aparece "Aguardando autorização para executar como administrador". Os argumentos silenciosos saem por gerador, por lista fechada: Inno `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-`, NSIS `/S`. Gerador desconhecido roda sem argumentos, com a fase "Instalador de <app> rodando (pode abrir janelas)". Vale a regra da tela que não fica parada: spinner, fase e relógio. É código de terceiro, então exige `--allow-unreviewed` como o resto (nunca implícito em `--yes`), e a confirmação diz que o instalador roda com os privilégios do usuário. O que ele grava é aceito, como na exceção 2c (`installer.file`).
- **Detecção.** O ARP (HKCU e HKLM 32/64) é lido antes e depois. A chave nova que aparecer é gravada no estado do app (`install.json`: modo `installer`, a chave ARP, o `UninstallString` lido) para a Task 211 remover e atualizar. O registro do usuário real só é tocado em produção; os testes usam o registro falso da 2d.
- **Reserva: só o executável.** O instalador "falha" quando não inicia, sai com código diferente de 0 e de 3010 (3010 = ok, pede reinício), o UAC dele é recusado (1223) ou nenhuma chave ARP nova aparece. Aí o CLI mostra "o instalador falhou (<motivo>); instalando só o executável" e segue com o asset de programa do mesmo build, pelo caminho portátil dos itens 4–6 (shim etc.). Sem asset de programa, o erro é o do instalador e nada fica instalado. O JobLog registra as duas tentativas.
- **`--scope machine` com release:** recusado com "releases do GitHub instalam só para o usuário; o instalador pode pedir administrador por conta própria".
- **Testes que substituem o 6** (instalador falso pelo `Launcher` falso da 2d e registro falso; nunca instalador real):
  - `gh_url_forms_parse` (tabela de URLs aceitas e recusadas);
  - `gh_installer_is_default` (instalador e programa ⇒ roda o instalador com os args do gerador e grava a chave ARP);
  - `gh_installer_failure_falls_back_to_exe` (código 1, UAC recusado e "nenhuma chave nova" ⇒ três casos; cada um termina com o programa instalado e a mensagem da reserva);
  - `gh_installer_without_program_fails_clean` (nada instalado);
  - `gh_installer_never_elevated` (o `Launcher` falso nunca recebe pedido de elevação do wingetrs);
  - `gh_scope_machine_refused`.
- **Mutações** que substituem a 2 do "Checks beyond":
  - a reserva não roda depois de código 1 ⇒ `gh_installer_failure_falls_back_to_exe` PEGA;
  - 3010 tratado como falha ⇒ um teste PEGA;
  - args do Inno trocados ⇒ `gh_installer_is_default` PEGA.
- **Files extras:** o executor de instalador e o registro falso da 2d por reuso (`grep` por `Launcher`), e `src/install/state.rs` (o modo e a chave ARP no `install.json`).
- **Ao vivo (orquestrador, com pedido do usuário):** `wingetrs install https://github.com/jonyduque/eprocia --allow-unreviewed --dry-run` e depois a instalação real.

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
5. **Decisão do usuário (2026-10-21): os 10 testes também rodam de verdade, como usuário comum.** A lista fechada mora num arquivo só, `tests/it/non_elevated.txt` (um nome de teste completo por linha, como o libtest imprime, ex.: `script_host_win::refuses_elevated`), lido pela regra do `check_rules.ps1` e pelo script novo `scripts/ci_non_elevated.ps1`. Esse script, chamado por um passo da CI depois do gate:
   - acha o executável de teste `it` (`cargo test --test it --no-run --locked --message-format=json`, campo `executable`; sem recompilar);
   - cria um usuário local padrão (não membro de Administradores) com nome `wingetrs-ci-<aleatório>` e senha aleatória de 32 caracteres gerada no próprio passo (`RandomNumberGenerator`), nunca impressa nem gravada em disco, mascarada com `::add-mask::`;
   - dá a esse usuário leitura e execução no checkout e em `target` (`icacls ... /grant <user>:(OI)(CI)RX`) e uma pasta de trabalho própria gravável (`TEMP`/`TMP` dele);
   - roda o executável como esse usuário (`Start-Process -Credential -Wait -PassThru`, `-RedirectStandardOutput`/`-RedirectStandardError` para arquivos na pasta dele) com `--exact --test-threads=1` e os 10 nomes, e `CI` **sem** valor (o filho não é elevado, então o helper não pula de qualquer jeito);
   - passa só se o código de saída for 0 **e** a saída tiver `test result: ok. 10 passed` (exatamente o tamanho da lista) e nenhuma linha `pulado:`;
   - sempre remove o usuário e o perfil dele no fim (`finally`), mesmo em falha.
   **Reserva (também decisão do usuário):** se a criação do usuário ou o logon secundário falhar (`New-LocalUser`/`Start-Process -Credential` lançam, serviço `seclogon` parado), o passo não reprova o job: emite `::warning::` com o motivo ("os 10 testes não elevados ficaram só pulados nesta execução") e escreve isso no resumo. Teste que **roda** e falha reprova o job.
6. O job registra no resumo (`$GITHUB_STEP_SUMMARY`) quantos testes foram pulados por elevação no gate (contando as linhas `pulado: processo elevado na CI`) e o resultado da rodada como usuário comum (`10 passaram como usuário comum` ou o aviso da reserva).
7. Badge de CI no README, ao lado dos atuais.
8. Fora do escopo: workflow de release (build, `.sha256`, anexar `NOTICE`/`THIRD_PARTY_LICENSES.txt`), assinatura do executável, matriz de plataformas.

Testes:
1. `skip_helper_only_in_ci_and_elevated` (unitário do helper com as duas entradas injetadas: `ci=false, elevated=true` ⇒ não pula; `ci=true, elevated=false` ⇒ não pula; `ci=true, elevated=true` ⇒ pula).
2. Regra nova do `check_rules.ps1` com caso no `check_rules_selftest.ps1` (chamada fora da lista ⇒ violação; nome na lista que não existe mais como teste ⇒ violação, para a lista não apodrecer).
3. `scripts/ci_non_elevated.ps1 -DryRun` localmente (não cria usuário: imprime o executável achado, os 10 nomes e os comandos que rodaria, com a senha trocada por `***`) — saída colada no relatório. O implementador **nunca** roda o modo real localmente (cria usuário na máquina do usuário).
4. Prova real: um push numa branch de teste mostra o job verde, o log com os 10 "pulado" no gate e nenhum outro, e o passo do usuário comum com `10 passed`. **Push só pelo orquestrador**, com pedido do usuário; o implementador termina com commits locais e o YAML validado por `actionlint` se estiver disponível (se não, diga "não validado").

**Files:** `.github/workflows/ci.yml` (novo), `scripts/ci_non_elevated.ps1` (novo), `tests/it/non_elevated.txt` (novo), helper de teste (novo ou existente em `tests/it/`), os testes da lista (só a chamada do helper), `scripts/check_rules.ps1`, `scripts/check_rules_selftest.ps1`, `README.md` (badge).

#### Checks beyond the steps

1. Mutação: helper pula sem checar `CI` ⇒ teste 1 **PEGA**.
2. Mutação: chamada do helper num teste fora da lista ⇒ regra do `check_rules` **PEGA**.
3. `grep -n "uses:" .github/workflows/ci.yml` colado no relatório: toda linha com `@<40 hex>`.
4. Mutação: o script aceita `test result: ok` sem conferir `10 passed` ⇒ caso do `-DryRun` com saída falsa de 9 testes (função pura `Test-RunOutput` testada pelo próprio script com `-SelfTest`) **PEGA**.
5. `grep -n "password\|Password" scripts/ci_non_elevated.ps1`: nenhuma linha imprime ou grava a senha.
6. Gate local: `pwsh -File scripts/verify.ps1` (num terminal não elevado).

**Commit:** `ci: GitHub Actions running the verify gate on Windows`

### Task 216: Barra de progresso e spinner do CLI no estilo da TUI; fase e relógio em `update` e `list`

**Modelo:** opus (revisor opus).

**Depois de:** 212 (`src/cli/style.rs`) e 215 (`src/cli/output.rs`, `src/install/progress.rs`). Nenhum arquivo em comum com tarefa rodando em paralelo.

> **Decisões do usuário (2026-10-21):** spinner e barra na cor `accent` do tema (como o item 4 abaixo já fixa); barra de progresso na extração fica fora desta tarefa (BACKLOG, item I).

#### Where this fits

- Eventos: `ProgressEvent` (`src/install/progress.rs:22-33`: `Download { url, done, total }` e `Phase(Phase)`); `Phase` (`:36-81`) é casado de forma exaustiva em `phase_title` (`:101-137`), `phase_status` (`:159-182`), `Phases::log_to_read` (`src/cli/output.rs:1456`), `take_notice_at` (`:1521`), `parts_at` (`:1571`), `tui/jobs.rs::progress_msg` (`:98-123`) e `tui/app.rs:1953`. Variante nova quebra o build em todos esses lugares (é de propósito: ver ARMADILHAS "Rótulo especial no callback de progresso só tratado num consumidor").
- CLI, estado puro: `output::Phases` (`src/cli/output.rs:1404-1414`), `update_at` (`:1427-1453`: soma por URL, `samples` numa janela de 3 s, `last_data`), `parts_at` (`:1571-1676`): spinner `glyphs.spinner[frame]`, barra `[{#×n}{-×(20-n)}] {pct}%  {done:.1} / {total} MiB` (`:1647-1660`), ` · {x} MiB/s · ~N s` (`:1662-1666`), aviso ` · sem dados há N s` depois de `STALL_AFTER` = 10 s (`:1668-1674`); `speed_at` (`:1679`); `mib` (`:1334`). Sem TTY: `PhaseLog` (`:1722-1745`) imprime uma linha por mudança de fase (`Baixando…`, `Preparando…`), sem `\r` nem cor.
- CLI, desenho: `output::progress_line(main, warn)` (`:810-812`) só pinta o aviso (`style::WARN`). `LineProgress`/`ProgressState` (`src/cli/install_cmd.rs:265-475`): thread de animação a cada `TICK` = 120 ms (`:260`, `:429-450`), `draw` (`:292-327`) corta com `fit_cols` (`:278-284`, o aviso sai primeiro) e escreve por `emit::progress()` (`src/cli/emit.rs:61-92`, `StderrLock` com `\r` e espaços). O único construtor é `TtyUi::build` (`install_cmd.rs:148-168`), que já monta um `tui::theme::Theme` (dark/light do config + `detect_depth`) e os `Glyphs` do config.
- Cores do CLI: `src/cli/style.rs` só tem constantes `anstyle` de 16 cores (`:12-26`); `paint` (`:147-153`) respeita `--color`/`NO_COLOR`/TTY do stdout (`ENABLED`, `:44`, decidido em `init` `:173-190`). Nenhum mapeamento de `tui::theme` existe hoje.
- TUI: a barra é só `"{label} [####----] {pct}%"` ou `"{label} {done} bytes"` (`src/tui/view.rs:1470-1489`, `progress_text`), sem tamanho, velocidade nem "sem dados"; vai dentro da pílula `pill_msg` da barra de status (`view.rs:1066-1090`, via `running_text` `:1445-1466`). Spinner: `view.rs:320-325`. `RunProgress` (`src/tui/app.rs:346-352`) é **substituído inteiro** a cada `Msg::InstallProgress` (`app.rs:1911-1921`), então não guarda amostras para velocidade.
- Glifos: `Glyphs` (`src/tui/theme.rs:18-51`), spinners `BRAILLE`/`ASCII_SPINNER` (`:53-54`); não há glifos de barra. Tokens: `Theme::accent()`/`muted()`/`warn()`/`spinner()` (`:307-377`).
- `wingetrs update`: `src/cli/mod.rs:711-728` chama `update::run_update` (`src/update.rs:186-310`: lock `:193-201`, downloads em paralelo `join_all` `:221-242`, abre e grava o catálogo `:246-285`, `search.idx` `:305`) e só imprime os relatórios no fim: nada na tela durante (inventário `docs/cli-textos.md` §1). `--background` (`spawn_background_update`, `mod.rs:502-520`) não imprime nada e deve continuar assim. O mesmo `run_update` roda calado na 1ª busca com catálogo vazio (`ensure_catalog_ran`, `mod.rs:542-591`, chamada `:572`; inventário SRC-07). `run_update` tem 21 chamadas (testes incluídos) e uma na TUI (`src/tui/run.rs:808`).
- `wingetrs list`: `src/cli/mod.rs:834-905`: três threads (varredura, rastreamento, normalizador) `:845-861`, `ensure_catalog` `:863`, `correlate` `:884-887`, saída `:895-902`; nada na tela até a tabela (inventário LST-11).
- Testes que fixam o texto atual: `tests/it/install_progress.rs` `phase_line_has_spinner_and_clock` (`:386`), `download_line_speed_eta_and_stall` (`:445`, `Baixando [######--------------] 34%`), `not_a_tty_prints_one_line_per_phase` (`:663`), `tui_status_shows_phase_and_clock` (`:702`); `tests/it/cli_plan_summary.rs` `download_bar_has_twenty_columns` (`:264`, `[#####---------------] 25%`). Terminal falso: `cli::picker::fake::plain` (`src/cli/picker.rs:540`, tira CSI). Harness de CLI ponta a ponta com wiremock: `tests/it/cli_color.rs:12-60` (`Env::run(args, vars)`).
- Regras (CLAUDE.md): "Operação que pode demorar nunca deixa a tela parada" (spinner, fase e relógio desde o primeiro instante; aviso quando passa do normal); "Cores só via tokens de `tui/theme.rs`"; toda saída do CLI pelos formatadores de `cli/output.rs` e escrita só em `cli/emit.rs`; "Mais de um caminho para o mesmo resultado ⇒ teste de equivalência".

#### What binds this task

1. **Uma formatação de download só, em `src/install/progress.rs`** (módulo comum ao CLI e à TUI, como `phase_text`):
   - `pub struct DownloadMeter` com o que hoje está em `Phases` (`per_url`, `samples`, `last_data`, `SPEED_WINDOW`, `speed_at`): `record_at(url, done, total, now)`, `rate_at(now) -> Option<f64>`, `quiet_at(now) -> Option<Duration>`, `done()`, `total() -> Option<u64>`, `finished()`. `Phases` passa a usar o `DownloadMeter`; nada de cálculo de velocidade duplicado.
   - `pub fn download_parts(meter: &DownloadMeter, now: Instant, glyphs: &Glyphs) -> DownloadParts` com `filled: String`, `empty: String` (20 células no total), `tail: String` e `warn: String`. `tail` = `"{pct}%  {done:.1} / {total:.1} MiB"` + `" · {x:.1} MiB/s"` + `" · ~N s"` (como hoje); total desconhecido: sem barra (`filled`/`empty` vazios) e `tail` = `"{done:.1} MiB · {x:.1} MiB/s"`. `warn` = `" · sem dados há N s"` a partir de `STALL_AFTER` (10 s; a constante muda para `progress.rs` e `output.rs` reexporta).
   - `pub fn download_text(label: &str, parts: &DownloadParts) -> String` = `"{label} {open}{filled}{empty}{close} {tail}{warn}"` (sem barra: `"{label} {tail}{warn}"`).
2. **Glifos de barra** em `Glyphs` (`tui/theme.rs`): `bar_full`, `bar_empty`, `bar_open`, `bar_close`. Nerd e Unicode: `"█"`, `"░"`, `""`, `""`. Ascii: `"#"`, `"-"`, `"["`, `"]"`. Com Ascii o texto fica igual ao de hoje (os testes de `:445` e `cli_plan_summary.rs:264` continuam valendo sem mudança).
3. **CLI**: `Phases::parts_at` monta a linha de download com `download_parts` + rótulo `Baixando` (texto sem cor igual ao de `download_text("Baixando", …)` com o spinner na frente). O texto sem cor de toda linha (`line_at`) não muda para Ascii.
4. **Cores do CLI pelos tokens do tema**: `style.rs` ganha `pub fn from_theme(s: ratatui::style::Style) -> anstyle::Style` (`Color::Rgb` ⇒ `RgbColor`, `Color::Indexed` ⇒ `Ansi256Color`, cores nomeadas ⇒ `AnsiColor`, `BOLD` ⇒ `Effects::BOLD`, sem cor ⇒ `Style::new()`) e `pub struct ProgressStyles { spinner, fill, empty }` com `ProgressStyles::of(&Theme)` = `accent`, `accent`, `muted`. Nenhum `anstyle` fora de `style.rs`. `LineProgress::new(tty, glyphs, styles)`: o `TtyUi::build` passa `ProgressStyles::of(&self.theme)`; `update`/`list` montam o mesmo `Theme` pelo config (função única `install_cmd::line_progress(cfg) -> Arc<LineProgress>`, usada também por `TtyUi::build`).
5. **Desenho pintado sem quebrar o corte**: `parts_at` devolve as partes separadas (spinner, rótulo, `filled`, `empty`, resto, aviso), e uma função pura `output::progress_frame(parts, cols, styles) -> (String, usize)` corta pelo texto **sem cor** (mesma prioridade do `fit_cols`: o aviso sai primeiro, depois o fim da linha) e só então pinta cada pedaço com `style::paint` (spinner e `filled` com `fill`/`spinner`, `empty` com `empty`, aviso com `WARN`). Devolve também a largura visível (para o `drawn`). `ProgressState::draw` só chama `progress_frame` e `emit::progress().redraw`. Com cor desligada (`--color never`, `NO_COLOR`, stdout redirecionado) a linha não tem `\x1b`; stderr fora de terminal continua no `PhaseLog` (uma linha por fase, sem `\r` e sem CSI), igual a hoje.
6. **TUI usa a mesma formatação**: `RunProgress` ganha `meter: DownloadMeter` (o `#[derive(Clone, PartialEq)]` continua); `Msg::InstallProgress` grava no medidor existente com `app.now` em vez de trocar o `RunProgress` (uma fase nova zera o medidor). `progress_text` (`view.rs:1470`) chama `download_parts` + `download_text(label, …)`: a TUI passa a mostrar tamanho, velocidade, estimativa e `sem dados há N s`. A pílula continua com o estilo `pill_msg` (sem cor por pedaço).
7. **`update` com spinner, fase e relógio**: `update::run_update_with(data, ep, http, force, wait, progress: download::Progress<'_>)`; `run_update` vira um atalho que passa progresso vazio (as 21 chamadas atuais não mudam). Fases novas em `Phase` (todas canceláveis: entram no grupo sem "não pode ser cancelado" de `phase_status`):
   - `WaitingUpdate` ⇒ "Aguardando outra atualização do catálogo", só quando a 1ª tentativa do lock sem espera falha e o `LockWait` permite esperar;
   - `FetchingCatalog { sources: usize }` ⇒ "Baixando o catálogo" (1 fonte) / "Baixando os catálogos ({n} fontes)", antes do `join_all`, com `n` = fontes que vão de fato baixar (sem as puladas por estarem em dia);
   - `WritingCatalog` ⇒ "Gravando o catálogo", antes de abrir o catálogo para gravar (só quando `needs_catalog`).
   A ordem é `Preparing` (antes do lock) ⇒ [`WaitingUpdate`] ⇒ `FetchingCatalog` ⇒ [`WritingCatalog`]. O comando `update` (sem `--background`) usa `line_progress(cfg)` e chama `finish()` antes de `print_reports`; `--background` passa progresso vazio (nenhuma saída, nem em stderr). `ensure_catalog_ran` (atualização bloqueante da 1ª busca/list/show) também mostra as fases e fecha a linha antes de devolver.
8. **`list` com spinner, fase e relógio**: `Phase::ReadingInstalled` ⇒ "Lendo os apps instalados" antes de abrir as threads; `Phase::Correlating` ⇒ "Comparando com o catálogo" antes do `correlate`; `finish()` antes de imprimir (também no caminho de erro: guard ou `finish` antes do `?`). `--json` mantém o stdout só com o JSON (o progresso vai em stderr).
9. Todos os consumidores exaustivos de `Phase` tratam as variantes novas (`tui/jobs.rs` manda para `Msg::InstallPhase`, como as demais). Nenhum `if cfg!`, nenhum `println!`, nenhuma cor fora de `style.rs`/`theme.rs`.

Testes (`tests/it/install_progress.rs`, acrescenta; nenhum teste usa relógio real: `Instant` base + `Duration`):
1. `bar_parts_unicode_and_ascii` (`download_parts` com 5 MiB de 20 MiB: Unicode ⇒ `filled == "█████"`, `empty == "░"×15`, `tail == "25%  5.0 / 20.0 MiB"`; Ascii ⇒ `download_text("Baixando", …) == "Baixando [#####---------------] 25%  5.0 / 20.0 MiB"`).
2. `cli_and_tui_share_download_text` (os mesmos 4 eventos de download nos mesmos instantes em `Phases::update_at` e em `update(&mut app, Msg::InstallProgress{..})` com `app.now`; o pedaço depois do rótulo, de `Baixando ` no CLI e de `rust.msi ` na TUI, é idêntico aos 3 s, com `" · 3.1 MiB/s · ~6 s"`, e aos 15 s, com `" · sem dados há 12 s"`).
3. `tui_download_shows_size_speed_and_stall` (`running_text` contém `9.3 / 27.0 MiB · 3.1 MiB/s` e depois termina em ` · sem dados há 12 s`; total desconhecido ⇒ `rust.msi 4.0 MiB · 2.0 MiB/s`, sem a palavra `bytes`).
4. `progress_frame_paints_with_theme_tokens` (`style::with_enabled(true)`, `ProgressStyles::of(&Theme::new(true, ColorDepth::Ansi256))`: a saída contém `"\x1b[38;5;99m"` (o accent) imediatamente antes do spinner; `fake::plain(saída) == line_at(...)`).
5. `progress_frame_plain_without_color` (`with_enabled(false)`: nenhum `\x1b` na saída; texto igual a `line_at`).
6. `progress_frame_cuts_before_painting` (linha de download com aviso, `cols = 30`, cor ligada: `fake::plain(saída)` tem exatamente 30 caracteres e é o prefixo da linha sem cor; toda sequência `\x1b[` da saída termina antes do fim da string).
7. `from_theme_maps_rgb_indexed_and_none` (`Rgb(1,2,3)` ⇒ `RgbColor(1,2,3)`; `Indexed(99)` ⇒ `Ansi256Color(99)`; `Style::new()` ⇒ `anstyle::Style::new()`; `BOLD` vira `Effects::BOLD`).
8. `update_reports_phases_in_order` (`run_update_with` contra o wiremock de `tests/it/update.rs` `setup()`, progresso gravando num `Mutex<Vec<Phase>>`: `[Preparing, FetchingCatalog { sources: 3 }, WritingCatalog]`; segunda rodada sem `force` com tudo em dia ⇒ `[Preparing, FetchingCatalog { sources: 0 }]`, sem `WritingCatalog`).
9. `update_waits_for_lock_with_phase` (lock segurado por um `UpdateLock` do teste, `LockWait::Block(…)`: o primeiro evento depois de `Preparing` é `WaitingUpdate`; solta o lock por um canal depois de receber o evento, sem `sleep` nem limiar de relógio).
10. `new_phase_titles` (`phase_text` de `WaitingUpdate`, `FetchingCatalog{1}`, `FetchingCatalog{3}`, `WritingCatalog`, `ReadingInstalled`, `Correlating` aos 2 s, texto exato com `· 00:02`; `phase_status` igual a `phase_text` para todas, sem "não pode ser cancelado").

Testes (`tests/it/cli_color.rs`, acrescenta, binário real com stderr redirecionado):
11. `update_progress_plain_off_a_terminal` (`update --force`: stderr tem as linhas `Preparando…`, `Baixando os catálogos (3 fontes)…`, `Gravando o catálogo…` nessa ordem, nenhum `\r` nem `\x1b`; stdout continua só com os relatórios).
12. `background_update_prints_nothing` (`update --background --force`: stdout e stderr vazios).
13. Ao vivo, `#[ignore = "live: varre o registro real"]` em `tests/it/cli_color.rs`: `list_progress_plain_off_a_terminal` (`list` com stderr redirecionado: `Lendo os apps instalados…` antes de `Comparando com o catálogo…`; stdout começa pelo cabeçalho da tabela).

Antes de mexer, rode `grep -rn "stderr" tests/it/cli_offline.rs tests/it/cli_color.rs tests/it/cli_install.rs` e ajuste só as asserções que exigem stderr vazio em `update`/`search` com catálogo vazio (cite cada uma no relatório).

**Files:** `src/install/progress.rs`, `src/cli/output.rs`, `src/cli/style.rs`, `src/cli/install_cmd.rs`, `src/cli/mod.rs`, `src/update.rs`, `src/tui/theme.rs`, `src/tui/app.rs` (`RunProgress`, `Msg::InstallProgress`, `phase_progress`, o `match` de `:1953`), `src/tui/view.rs` (`progress_text`), `src/tui/jobs.rs` (`progress_msg`), `tests/it/install_progress.rs`, `tests/it/cli_color.rs`, e os testes listados pelo grep acima se algum mudar.

#### Checks beyond the steps

1. Mutação: `progress_text` da TUI volta ao formato antigo (`"{} [{}{}] {pct}%"`) ⇒ teste 2 **PEGA**.
2. Mutação: a TUI troca o `RunProgress` inteiro a cada `InstallProgress` (perde o medidor) ⇒ teste 3 **PEGA** (sem velocidade).
3. Mutação: o spinner pintado com `style::WARN` em vez de `styles.spinner` ⇒ teste 4 **PEGA**.
4. Mutação: `progress_frame` usa `paint_raw` (ignora `--color never`) ⇒ teste 5 **PEGA**.
5. Mutação: cortar depois de pintar (`fit_cols` sobre o texto já com CSI) ⇒ teste 6 **PEGA**.
6. Mutação: tirar o `progress(Phase(WritingCatalog))` ⇒ teste 8 **PEGA**; contar todas as fontes em `FetchingCatalog` (sem descontar as puladas) ⇒ teste 8 **PEGA** na 2ª rodada.
7. Mutação: o `update --background` usa `line_progress` ⇒ teste 12 **PEGA**.
8. Colar no relatório: a linha do teste 4 com os escapes trocados por `<ESC>` e a saída `fake::plain` dos testes 1 e 2. Escapes nos testes: escreva `"\x1b"` direto no `.rs` com a ferramenta Edit/Write, nunca por heredoc (ARMADILHAS "Escape `\xNN` no texto de um comando vira byte bruto"); confira com `grep -c` por bytes `< 0x20`.
9. Prova manual (o orquestrador faz, não o implementador): `wingetrs update --force` e `wingetrs list` num terminal mostram spinner colorido, fase e relógio até a saída; `wingetrs install <app>` mostra a barra `█░` com tamanho e velocidade; com `--color never` sem cor.
10. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): themed progress bar and spinner shared with the TUI; phases in update and list`

---

### Task 217: Transcript do CLI e da TUI para a prova ao vivo

**Modelo:** sonnet (revisor sonnet).

**Depois de:** 212 (`src/cli/picker.rs`) e 216 (`src/cli/install_cmd.rs`, `src/cli/output.rs`, `src/tui/view.rs`, `src/tui/app.rs`). As mudanças em `scripts/live/` ficam com o orquestrador.

#### Where this fits

- Toda escrita do CLI passa por `src/cli/emit.rs`: `out` (`:18`), `out_block` (`:23`), `out_flushed` (`:28`), `err` (`:34`), `err_with` (`:39`), `json` (`:45`), `error` (`:52`, chamado por `main.rs:48`) e a linha de progresso `Progress` (`:61-92`: `erase`, `line_above` para avisos, `redraw` a cada 120 ms). `check_rules.ps1` recusa `print!`/`stdout()` fora de `emit.rs` em `src/cli/` (o terminal do seletor em `picker.rs` é a exceção).
- O seletor escreve no próprio `Term` (`src/cli/picker.rs`); a linha de resumo depois da escolha (`"{prompt} {rótulo}"`) sai em `picker.rs:389-393` (`summary`, `:184`). A Task 212 mantém essa linha.
- Linha de progresso com TTY (`LineProgress`, `src/cli/install_cmd.rs:358-475`): só redesenha; sem TTY, o `output::PhaseLog` (`src/cli/output.rs:1722-1745`) dá uma linha por fase.
- Processo elevado: `cli::run` (`src/cli/mod.rs:1037`) devolve antes de tudo para `Command::BridgeWorker` (`:1040-1043`, `bridge::elevated::run_worker`, `src/bridge/elevated.rs:577`), sem pasta de dados nem log (regra do CLAUDE.md: código elevado não grava em pasta do usuário). A TUI entra por `crate::tui::run::run` (`mod.rs:1083-1086`), depois do config. `update --background` é um processo filho destacado (`mod.rs:502-520`) que herda o ambiente.
- TUI: `update(app, msg)` é pura; efeitos ficam no laço `event_loop` (`src/tui/run.rs:287`), que desenha em `:539-543`. Toasts: `App::push_toast` (`src/tui/app.rs:986`), `Toast { kind, text, expires, count }` (`:719-724`), `ToastKind` (`:698-703`). Modais: `enum Modal` (`app.rs:~474`, variantes `Confirm`, `ConfirmQuit`, `Picking`, `Actions`, `Residues`, `Choosing`, `Queue`), abertos por `open_modal` (`:4007`, com fila `pending_modals`); os títulos estão espalhados no desenho (`src/tui/view.rs:1195-1275`: "Confirmar", "Escolha o pacote", nome do app, "Escolha a correspondência", "Resíduos de {app}", "Fila"; `draw_confirm` `:1312`). Itens do Confirm: `ConfirmItem { name, from, to, manager, .. }` (`app.rs:417`). Job em execução: `app.running` (`Running { label, progress }`, `app.rs:~366`), fase em `RunProgress.phase`; texto da barra de status em `view::running_text` (`view.rs:1445`).

#### What binds this task

1. Módulo novo `src/transcript.rs` (fora de `cli/` porque a TUI também usa): `pub const VAR: &str = "WINGETRS_TRANSCRIPT"`; `pub fn open(path: &Path) -> io::Result<()>` (cria ou abre em **append**; guarda o `File` num `Mutex<Option<File>>` global); `pub fn enabled() -> bool`; `pub fn line(text: &str)` (tira as sequências CSI/OSC com `strip_ansi`, divide em `\n`, sem `\r`, grava cada linha + `\n` com uma chamada `write_all` e `flush` por linha; sem transcript aberto não faz nada; erro de escrita depois de aberto fecha o transcript e vai para `tracing::warn!`, nunca para o terminal); `pub fn strip_ansi(text: &str) -> String`; `pub fn with_file<T>(path: &Path, f: impl FnOnce() -> T) -> T` (desvio por thread, para testes, no padrão de `style::with_enabled`). Nada aqui escreve em stdout/stderr.
2. Decisão de ligar: `pub fn transcript_path(cli: &Cli, var: Option<OsString>) -> Option<PathBuf>` em `cli/mod.rs`: `None` para `BridgeWorker` e para `Update { background: true }`, e para valor vazio; caso contrário, o caminho. `cli::run` chama `transcript_path` **depois** do `return` do `BridgeWorker` e antes do desvio para a TUI só nos comandos do CLI; falha de `open` ⇒ **um** aviso por `emit::err_with(|| output::warning(&output::transcript_failed(path, e)))` com o texto `não foi possível abrir o transcript {caminho}: {erro}; seguindo sem transcript` e o comando segue normalmente.
3. CLI (`emit.rs`): `out`, `out_block`, `out_flushed`, `err`, `err_with`, `json` e `error` mandam o mesmo texto para `transcript::line` (o texto já pronto, antes ou depois de escrever, tanto faz). `Progress::line_above` também (avisos acima da linha). `redraw`/`erase` **não** (seriam centenas de linhas): em vez disso, `LineProgress::update` com TTY passa o evento por um `PhaseLog` próprio e manda a linha da fase para o transcript, então o transcript tem as mesmas linhas de fase que um stderr redirecionado. O resumo do seletor (`picker.rs:389-393`) vai para o transcript.
4. TUI: `tui::run::run` abre o transcript com a mesma variável (aviso de caminho inválido vira **um** toast `ToastKind::Warning` com o mesmo texto do CLI, nunca stderr). Módulo novo `src/tui/transcript.rs` com `#[derive(Default)] pub struct Observer` e `pub fn observe(&mut self, app: &App) -> Vec<String>` puro, chamado no laço antes do desenho só quando `transcript::enabled()`; cada linha vai para `transcript::line`. Linhas, todas com prefixo `[tui] `:
   - toast novo, ou o mesmo com `count` maior: `[tui] toast {ok|info|aviso|erro}: {texto}`;
   - modal que passa a ser mostrado (`app.modal` muda): `[tui] modal: {título}`; para `Confirm`, uma linha por item `[tui]   {nome} {de} → {para} ({manager})` (sem `de`/`para` quando ausentes) e uma por nota `[tui]   nota: {nota}`; para `ConfirmQuit`, uma linha por linha do texto; modal fechado: `[tui] modal fechado`;
   - job: quando muda o rótulo do `running` ou o **título** da fase (`phase_title`, sem relógio; download conta como `baixando`): `[tui] job: {rótulo} · {fase}`; quando `running` volta a `None`: `[tui] job terminou: {rótulo}`.
   O título do modal vem de uma função só, `pub fn modal_title(m: &Modal) -> String` em `view.rs`, usada também pelo desenho (sem cópia do texto).
5. Nunca escreve no terminal pela TUI; nunca abre transcript no `bridge-worker` nem no `update --background`.

Testes (`tests/it/transcript.rs`, novo; o `mod transcript;` em `tests/it/main.rs` é do orquestrador):
1. `emit_lines_are_plain` (`transcript::with_file(tmp)` + `style::with_enabled(true)`: `emit::out(&output::success("x", "y"))`, `emit::out_block("a\nb\n")`, `emit::err("e")` ⇒ o arquivo é exatamente `format!("{} x y\na\nb\ne\n", Glyphs::for_set(GlyphSet::Unicode).toast_ok)` (glifo unicode, padrão fora de terminal), sem `\x1b` nem `\r`; o conteúdo é lido **antes** de sair do `with_file`, provando o flush por linha).
2. `appends_to_existing_file` (arquivo já com `"antes\n"` ⇒ `"antes\nnovo\n"`).
3. `invalid_path_is_an_error_not_a_panic` (`transcript::open(tmp/"nao/existe/t.txt")` ⇒ `Err`; `transcript::line("x")` depois não entra em pânico e `enabled()` continua `false`).
4. `cli_warns_once_and_runs` (binário real, harness de `cli_color.rs`: `search ""` com `WINGETRS_TRANSCRIPT=<pasta inexistente>/t.txt` ⇒ sucesso, stdout igual ao da mesma busca sem a variável, stderr contém `não foi possível abrir o transcript ` exatamente uma vez).
5. `cli_transcript_has_stdout_stderr_and_phases` (binário real: `update --force --color always` com a variável ⇒ o arquivo contém a linha de relatório do `winget` (`winget: ` com o token inteiro), `Preparando…` e nenhum byte `0x1b`).
6. `tui_toast_goes_to_transcript` (`App::new`, `push_toast(Success, "instalado")`, `observe` ⇒ `["[tui] toast ok: instalado"]`; 2º `observe` ⇒ vazio; o mesmo toast de novo (`count` 2) ⇒ a linha outra vez).
7. `tui_confirm_modal_lines` (`Modal::Confirm` com dois itens, um com `from`/`to` e outro sem, e uma nota ⇒ `[tui] modal: Confirmar`, as duas linhas de item na ordem e `[tui]   nota: …`; fechar ⇒ `[tui] modal fechado`).
8. `tui_job_lines_ignore_the_clock` (`running` com `Phase::Preparing`; `observe`; `app.now += 5 s`; `observe` ⇒ vazio; `Msg::InstallPhase(Phase::Placing)` ⇒ `[tui] job: Instalar rust · Colocando os arquivos`; `running = None` ⇒ `[tui] job terminou: Instalar rust`).
9. `elevated_worker_and_background_never_transcribe` (`transcript_path` com `Cli::try_parse_from(["wingetrs", "bridge-worker", "--pipe", "p", "--parent", "1"])` e `Some("t.txt")` ⇒ `None`; `["wingetrs", "update", "--background"]` ⇒ `None`; `["wingetrs", "list"]` ⇒ `Some("t.txt")`; valor vazio ⇒ `None`).
10. `picker_summary_in_transcript` (`picker::yes_no` com `fake::ScriptedTerm` e `Enter`, dentro de `with_file` ⇒ o arquivo tem `Continuar? Não`).

**Files:** `src/transcript.rs` (novo), `src/lib.rs` (só o `mod`), `src/cli/emit.rs`, `src/cli/mod.rs` (`transcript_path` e a abertura), `src/cli/output.rs` (só `transcript_failed`), `src/cli/install_cmd.rs` (fases para o transcript com TTY), `src/cli/picker.rs` (só a linha de resumo), `src/tui/transcript.rs` (novo), `src/tui/mod.rs` (só o `mod`), `src/tui/view.rs` (`modal_title`), `src/tui/run.rs` (abertura e `Observer` no laço), `tests/it/transcript.rs` (novo).

#### Checks beyond the steps

1. Mutação: `line` grava o texto sem `strip_ansi` ⇒ testes 1 e 5 **PEGAM**.
2. Mutação: `BufWriter` sem `flush` por linha ⇒ teste 1 **PEGA** (lê antes do fim).
3. Mutação: `open` com `truncate(true)` em vez de `append` ⇒ teste 2 **PEGA**.
4. Mutação: `transcript_path` sem o caso `BridgeWorker` ⇒ teste 9 **PEGA**; o revisor confere também que a abertura em `cli::run` está depois do `return` do worker (`git show HEAD:src/cli/mod.rs | grep -n "run_worker\|transcript"`).
5. Mutação: `observe` usa `phase_text` (com relógio) em vez de `phase_title` ⇒ teste 8 **PEGA**.
6. Mutação: o aviso de caminho inválido impresso a cada linha ⇒ teste 4 **PEGA** (exatamente uma vez).
7. Colar no relatório o arquivo do teste 5 (as primeiras 10 linhas).
8. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat: WINGETRS_TRANSCRIPT records CLI and TUI text for the live proof`

### Task 219: Minors da revisão da 206 (buckets)

**Modelo:** sonnet (revisor sonnet).

**Depois de:** 216 (fase de download do `update` e o medidor de download do CLI; `src/cli/mod.rs`, `src/install/progress.rs`). Nenhum arquivo em comum com 220.

> Origem: revisão tardia da Task 206 (ledger, linha 206): APPROVE com quatro Minors, agora pedidos.

#### Where this fits

- `src/buckets.rs`: `lock` (`:196`, `UpdateLock::try_acquire(data.lock_path())`, ocupado ⇒ `BucketError::Busy`); `add_downloaded` (`:216-285`): lock `:224` ⇒ `fetch_bucket` `:232` ⇒ `catalog.replace_source("scoop:<nome>")` `:253` ⇒ `index_file::write` `:264` ⇒ `write_scripts` em `buckets\<nome>` `:268-274` ⇒ `http.json` `:276-281` ⇒ `cfg.save` `:282`. O comentário de `:214` promete "Any failure leaves config, catalog and index untouched", mas uma falha em `http.json` ou `cfg.save` deixa o catálogo e o índice com o bucket e o config sem ele. `remove_and_cleanup_confirmed` (`:351`): lock `:358` ⇒ `remove(cfg)` ⇒ `remove_source` ⇒ índice ⇒ apaga `buckets\<nome>` ⇒ `http.json`; devolve e **solta o lock**; o `cfg.save` é feito depois pelo chamador (`src/cli/mod.rs:1059`), fora do lock (um `update` ou `bucket add` que começa nessa janela lê o config antigo e reimporta o bucket).
- `src/cli/mod.rs`: `bucket_add` (`:969`, seletor `ui.confirm` `:990-997`), `run_bucket` (`:1004`): `add` manda `Phase::Extracting { file: "bucket <nome>", engine: "GitHub" }` (`:1024-1027`), mostrado como "Extraindo bucket X (GitHub)" durante o **download** (sem tamanho, sem "sem dados há N s"); `rm` usa `Phase::Removing` (`:1052`). `WINGETRS_SCOOP_BASE_URL` (`:1029`) troca a URL nos testes.
- `fetch_bucket(http, url, prev)` (`src/sources/scoop/bucket.rs:165`) chama `http.get_conditional`, sem progresso. A Task 216 deu ao `update` uma fase de download com o medidor do CLI (`DownloadMeter`/`download_parts` em `src/install/progress.rs`) e eventos `ProgressEvent::Download { url, done, total }`; use o mesmo caminho de progresso que o `update` usa para o tarball dos buckets (procure em `src/update.rs` como a 216 passa o `download::Progress` ao baixar os tarballs) em vez de criar outro.
- Testes existentes: `tests/it/bucket_admin.rs` (`add_downloads_and_indexes_now` `:105`, `add_404_writes_nothing` `:138`, `add_tarball_without_manifests_refused` `:153`, `add_and_rm_refuse_while_update_lock_held` `:169`, `rm_default_bucket_needs_confirmation` `:215`, `rm_drops_legacy_master_url` `:227`); `tests/it/buckets.rs` (`add_unknown_needs_confirmation` `:45`, `remove_and_cleanup_purges_catalog_index_and_http_state` `:223`); `tests/it/cli_install.rs:367` `bucket_add_list_rm_round_trip` (binário real, `--yes`, sem TTY). Nenhum teste passa pelo seletor. Seletor falso: `cli::picker::fake::ScriptedTerm` (Task 212) e o `Ui` falso de `tests/it/cli_install.rs`/`install_cmd` (procure `impl Ui for` nos testes).

#### What binds this task

1. **`rm` grava o config dentro do lock.** `remove_and_cleanup_confirmed` ganha o `config_path` (ou o `DataDir` já basta: `data.config_path()`) e chama `cfg.save` **antes** de soltar o lock, depois de `http.json`. O `cfg.save` sai de `run_bucket` (`cli/mod.rs:1059`). `remove_and_cleanup` (sem confirmação) também grava. Atualize o doc comment (`:338-340`, "The caller saves `cfg`" deixa de valer).
2. **`add` não deixa meio estado.** Uma falha depois do `replace_source` (índice é só aviso, como hoje; `http.json` e `cfg.save` são erros) desfaz: `catalog.remove_source("scoop:<nome>")`, regrava o índice (ou apaga, como hoje em falha), apaga `buckets\<nome>` se esta chamada o criou, e devolve o erro original. Desfazer que falha vai para `tracing::warn!` e não troca o erro. O comentário de `:214` passa a ser verdade.
3. **Fase de download no `bucket add`.** No lugar do `Phase::Extracting` durante o download: a mesma fase de download que o `update` usa para os tarballs (216), com `ProgressEvent::Download` alimentando o medidor (tamanho, velocidade e `sem dados há N s` depois de 10 s); depois do download, `Phase::Extracting { file: "bucket <nome>", .. }` só enquanto lê o tarball e grava o catálogo. `fetch_bucket` ganha o parâmetro de progresso se for preciso (as outras chamadas passam progresso vazio, sem mudar comportamento).
4. **Testes da CLI no caminho interativo e no `--yes`.**

Testes (em `tests/it/bucket_admin.rs`, salvo o 6):
1. `rm_saves_config_under_the_lock` (depois de `remove_and_cleanup_confirmed` devolver `Ok`, o `config.toml` em disco já não tem o bucket, sem nenhuma chamada a `cfg.save` pelo teste; e um `UpdateLock::try_acquire` tomado **antes** da chamada faz ela devolver `Busy` sem mudar o `config.toml`).
2. `add_late_failure_rolls_back_catalog` (servidor wiremock com tarball válido; `http.json` substituído por uma **pasta** com o mesmo nome para o `save` falhar ⇒ `Err(BucketError::State(_))`; o catálogo não tem linha com `source = 'scoop:<nome>'`, o `config.toml` não tem o bucket, `buckets\<nome>` não existe).
3. `add_config_failure_rolls_back_catalog` (o mesmo com `config.toml` substituído por uma pasta ⇒ catálogo sem a fonte).
4. `add_reports_download_progress` (progresso gravador: a sequência de eventos tem pelo menos um `ProgressEvent::Download` com `total = Some(tamanho do tarball)` antes do primeiro `Phase::Extracting`).
5. `add_interactive_yes_and_no` (Ui falsa interativa: `bucket_add` com bucket desconhecido e resposta "Não" ⇒ `Ok(false)`, `say` recebeu `Cancelado.`, `cfg` sem o bucket; resposta "Sim" ⇒ `Ok(true)` e `cfg` com o bucket; Ui não interativa sem `--yes` ⇒ erro que cita `--yes`).
6. `bucket_add_unknown_with_yes_end_to_end` (em `tests/it/cli_install.rs`, binário real com `WINGETRS_SCOOP_BASE_URL` do wiremock: `bucket add meu https://github.com/x/meu --yes` ⇒ sucesso, `bucket list` mostra `meu`, stdout tem `Bucket adicionado:`).

**Files:** `src/buckets.rs`, `src/cli/mod.rs` (só `run_bucket`), `src/sources/scoop/bucket.rs` (só se o progresso precisar), `tests/it/bucket_admin.rs`, `tests/it/cli_install.rs` (só o teste 6).

#### Checks beyond the steps

1. Mutação: tirar o desfazer do `add` ⇒ testes 2 e 3 **PEGAM**.
2. Mutação: `cfg.save` do `rm` de volta para depois do lock (no chamador) ⇒ teste 1 **PEGA**.
3. Mutação: não mandar `ProgressEvent::Download` ⇒ teste 4 **PEGA**.
4. Prova no binário (orquestrador): `wingetrs bucket add java` num terminal mostra a barra de download com tamanho e depois "Extraindo bucket java".
5. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(buckets): rm saves config under the lock, add rolls back a late failure, download phase with progress`

### Task 220: `JobLog.elevated` em todo caminho elevado; Ctrl+C não deixa `apps\<app>`

**Modelo:** sonnet (revisor sonnet).

**Depois de:** 216 (`src/cli/install_cmd.rs`). Nenhum arquivo em comum com 219.

> Origem: prova ao vivo de 2026-10-21 (`docs/superpowers/reports/2026-10-21-prova-ao-vivo.md`, achados D4 e A2).

#### Where this fits

- `JobLog` (`src/job_log.rs`): campo `elevated: bool` (`:104-105`, doc "Whether the tool ran through the elevated bridge helper"), padrão `false` (`:158`), escrito em `job.start`/`job.end` (`:183`, `:193`). Hoje só `run_bridge_as` (`src/tui/jobs.rs:1031`, valor `chosen.elevated()` de `run_elevated_as` `:1018-1028`, gravado em `:1061`) liga o campo: a ponte `winget` elevada (CLI e TUI). Ficam sempre `false`:
  - instalador nativo do winget de escopo máquina, que roda com o próprio UAC (`runas`, Task 196): CLI `src/cli/winget_cmd.rs:475` (`JobLog::start_accepting`), TUI `src/tui/jobs.rs:605`/`:742`; a decisão é `WingetPlan.elevation: Elevation` (`src/winget_install/plan.rs:50`; `enum Elevation { Never, Helper }` em `src/winget_install/launch.rs:58`, calculada por `install_elevation` `:68`);
  - remoção de pacote winget cujo desinstalador ou chave ARP vai pelo auxiliar elevado (HKLM): CLI `src/cli/winget_cmd.rs:834` (`JobLog::start` com `kind: "remove"`), TUI equivalente em `src/tui/jobs.rs`.
  O auxiliar (`bridge-worker`) não tem `JobLog` e continua sem (código elevado não grava em pasta do usuário).
- Ctrl+C na instalação de app do Scoop: o download (`Step::Download`, `src/install/executor.rs:2500-2513`) vem antes de qualquer pasta do app; o arquivo parcial é apagado pelo `TmpGuard` (`src/install/download.rs:202-207`). A pasta `apps\<app>` nasce no `Step::Extract` (`own_staging` `:914`, `run.ensure_dirs(layout.app_dir(app))` `:930`, staging `apps\<app>\.staging-<v>`). Cancelado ⇒ `AwaitWorker::drop` (`:2960-2993`): espera o passo, apaga o staging e tenta `fs::remove_dir(app_dir)`, que falha calado se sobrar algo, por exemplo a pasta de trabalho do `msiexec /a` ao lado (`.staging-<v>.msiexec`) ou um temporário de extração (`extract::TEMP_PREFIX`). `is_dead_run_leftover` (`:2999`) já reconhece esses nomes, e `remove_dead_run_leftovers` (`:3005`) só roda no **próximo** `own_staging` (`:929`). Na prova, `rust` (MSI pelo `msiexec /a`) cancelado deixou `apps\rust`. O diário de desfazer (`rollback`, `:573`) só roda em `Err` (`:740`), não no cancelamento.

#### What binds this task

1. **`elevated` diz se algo rodou como administrador**, por qualquer caminho: doc do campo passa a "Whether anything in the job ran elevated (bridge helper or the installer's own UAC prompt)". Instalação nativa do winget com `plan.elevation == Elevation::Helper` (o mesmo valor que já decide o `runas`) ⇒ `log.elevated = true` antes do `finish`, no CLI e na TUI. Remoção que passou pelo auxiliar elevado (desinstalador HKLM ou `delete_arp_key`) ⇒ `true`. Uma função só decide isso por plano (ex.: `fn plan_runs_elevated(plan: &WingetPlan) -> bool` em `winget_install`, e a equivalente da remoção), chamada pelos dois lados.
2. **Cancelamento limpa a pasta de um app novo.** No `AwaitWorker::drop`, depois de apagar o staging: apagar também os vizinhos que `is_dead_run_leftover` reconhece **e que esta execução criou** (`.staging-<v>.msiexec` da mesma versão, temporários de extração), e só então `remove_dir(app_dir)`. Nunca apaga nada de outra versão já instalada (`current`, `<versão>` antigas, `persist`). Falha ⇒ `tracing::warn!` com o caminho, como hoje.
3. Nada muda no caminho de erro (o `rollback` continua igual).

Testes:
1. `job_log_native_machine_install_is_elevated` (`tests/it/job_log.rs`: plano de instalador nativo com `elevation: Elevation::Helper` e `Launcher` falso de sucesso ⇒ a linha `job.end` tem `elevated=true`; com `Elevation::Never` ⇒ `elevated=false`).
2. `job_log_helper_removal_is_elevated` (remoção com o auxiliar falso ⇒ `elevated=true`; remoção HKCU sem auxiliar ⇒ `false`).
3. `cancel_during_extract_removes_new_app_dir` (`tests/it/` do executor, extrator falso que cria `apps\<app>\.staging-<v>.msiexec\x` e bloqueia até o cancelamento; cancelar ⇒ depois do drop, `apps\<app>` não existe).
4. `cancel_keeps_installed_versions` (o mesmo com `apps\<app>\1.0\` e `current` já presentes ⇒ continuam lá; só o staging e o `.msiexec` da versão nova somem).
5. `cancel_during_download_creates_no_app_dir` (wiremock que segura o corpo; cancelar no meio ⇒ `apps\<app>` nunca existiu e não sobra arquivo em `cache`).

**Files:** `src/job_log.rs` (doc), `src/cli/winget_cmd.rs`, `src/tui/jobs.rs`, `src/winget_install/` (a função de decisão), `src/install/executor.rs` (`AwaitWorker::drop`), `tests/it/job_log.rs`, testes do executor em `tests/it/` (arquivo existente de cancelamento, procure `AwaitWorker` ou `try_cancel` nos testes).

#### Checks beyond the steps

1. Mutação: `elevated` fixo em `false` na instalação nativa ⇒ teste 1 **PEGA**.
2. Mutação: `drop` sem apagar o `.msiexec` ⇒ teste 3 **PEGA**.
3. Mutação: `drop` apaga todo `is_dead_run_leftover` e também `1.0` (troca o filtro por "tudo") ⇒ teste 4 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix: job log marks every elevated run; cancelled install leaves no new app folder`
