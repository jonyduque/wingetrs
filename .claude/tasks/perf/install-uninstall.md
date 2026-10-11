# Desempenho de `install` / `uninstall` (Scoop, raiz `%LOCALAPPDATA%\wingetrs`)

Revisão somente-leitura. Nada foi compilado, executado ou medido por mim. Todo ganho abaixo é
hipótese "não medido"; os números citados vêm de `docs/superpowers/reports/2026-10-10-fase2a-validacao.md`
(install com cache e efeitos reais ~1000 ms; broadcast 438–568 ms; resíduos mediana 105 ms;
dry-run 114 ms; uninstall ~80 ms em raiz temporária; install em raiz temporária 522 ms, delta 497 ms;
divisão interna dos ~410 ms entre dry-run e install: "não medido, sem spans").

Pré-requisito comum a quase tudo: **não há spans `tracing` no install** (só `reg_values_containing`
em `registry_win.rs:92`). Ver ideia 0.

## 1. Mapa do caminho quente

### Install (`wingetrs install <alvo> --yes`, 1 pacote, sem dependências)

| # | Etapa | Onde | I/O / CPU / syscalls |
|---|---|---|---|
| 1 | `ProdEffects::new`: `CoInitializeEx`? (não, só no uso), `SHGetKnownFolderPath(Programs)`, lê env | `cli/run_win.rs:19`, `shortcut_win.rs:27,75` | 1 chamada shell32 por processo, mesmo sem atalho no plano |
| 2 | Catálogo abre read-only (`ensure_catalog`) | `cli/mod.rs:394-405` | 11–15 ms SQLite (armadilhas) |
| 3 | `resolve_target` | `ops.rs:327`, `target.rs` | consultas no catálogo |
| 4 | `plan_for`: `manifest_raw` (`catalog.get` traz a linha toda), `manifest::resolve` (parse JSON), `read_state` (abre `state.db`), `DepWalk`, SHA-256 do manifesto cru, `layout.launcher().exists()`, `planner::plan_install` (13 µs medido) | `ops.rs:461-520` | 1 query catálogo, 1 open SQLite + `get` por nó, 1 SHA-256 de ~KB, 1 stat |
| 5 | `print_plans`, `proceed` (`--yes` pula) | `install_cmd.rs:240-266` | stdout |
| 6 | `apply`: `lock_async` (spawn_blocking + `LockFileEx`), `open_state` (2ª conexão: `PRAGMA journal_mode=WAL` + `execute_batch(SCHEMA)`), `revalidate` (1 `get` por plano), `env_snapshot` (abre HKCU\Environment, 1 `get` por nome) | `ops.rs:546-577,214,523` | 1 open+DDL SQLite, ~3 syscalls de registro |
| 7 | `execute_install`: validações, `state.get` de novo, partição de steps | `executor.rs:312-384` | 1 query |
| 8 | `EnsureLauncher` (só 1ª vez; `exists()` de novo; baixa zip **sem usar o cache de blobs**, `ensure_launcher_from` ignora `_data`) | `executor.rs:420`, `shim.rs:90-131` | stat; na 1ª vez rede + zip + extração |
| 9 | `Download` (um `fetch_all` por step, 1 fonte cada): `read_index` (JSON), `cached` = **leitura integral do blob + SHA-256 (BlobStore::get) + `verify` = 2º SHA-256 do mesmo buffer** | `executor.rs:433`, `download.rs:131-137,87-97`, `cache/mod.rs:166` | leitura inteira em RAM (`Vec<u8>`), 2 passadas de hash (para sha256 as duas são iguais) |
| 10 | `Extract`: remove staging velho, `ensure_dirs`, `extract_all` -> pasta temporária irmã (`.wingetrs-extract-*`) -> `merge_into` (dest inexistente => `rename`) | `executor.rs:449-482`, `extract.rs:91-127,213-231` | zip: 1 passada só de nomes + 1 passada de descompressão; por arquivo: `create_dir_all` (stats) + `File::create` + `io::copy` (buffer de 8 KiB, não bufferizado) |
| 11 | `Promote`: `symlink_metadata`, `rename(staging, version)`, `write_install_json` | `executor.rs:483-494` | 1 rename + 1 escrita |
| 12 | `Persist` (por item): 2 `symlink_metadata`, rename/`hard_link`/junção, `ensure_dirs` | `executor.rs:681-736` | ~6–8 syscalls por item |
| 13 | `LinkCurrent`: `symlink_metadata`, junção (`DeviceIoControl` FSCTL_SET_REPARSE_POINT) | `executor.rs:496-514`, `fs_win.rs:45-49` | 3–4 syscalls |
| 14 | `Shim` (por comando): **`owner_of` = `state.apps()` + um `state.get` por app instalado**; `fs::read` de 4 extensões (salva para undo); `create_shim` (ler 4 KiB do alvo para o subsistema PE, `hard_link` do launcher, escrever `.shim`) | `executor.rs:651-679,627-649`, `shim.rs:162-205` | N+1 queries SQLite (2 `prepare` por app) por shim; ~8 syscalls |
| 15 | `Shortcut` (por atalho): `owner_of` de novo, `CoInitializeEx` + `CoCreateInstance(ShellLink)` + `Save` | `executor.rs:516-544`, `shortcut_win.rs:130-160` | COM STA init/uninit por atalho |
| 16 | `EnsureShimsOnPath`/`EnvAddPath`: `get` do registro, `set` (que faz outro `get_raw_value` para o tipo) | `executor.rs:573,601-625`, `env_win.rs:112-146` | 3–5 syscalls de registro, só se mudar |
| 17 | `RecordState`: transação com DELETE + INSERT por artefato, `commit` em WAL com `synchronous=FULL` (padrão) | `executor.rs:576-589`, `state.rs:116-139` | 1 fsync no `-wal` |
| 18 | `remove_upgrade_leftovers` / `RemoveOldVersion` (só upgrade) | `executor.rs:386-413` | `remove_dir_all` |
| 19 | `broadcast_if_changed`: reler o ambiente e comparar; `SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, ..., 2000 ms)` | `ops.rs:293-304`, `env_win.rs:166-190` | **438–568 ms medido** (síncrono, janela por janela, em série) |

O lock `install.lock` é mantido de 6 até 19, **inclusive durante o download** (etapa 9 corre dentro de `apply`).

### Uninstall (`wingetrs uninstall <app> --yes`)

| # | Etapa | Onde | I/O / CPU |
|---|---|---|---|
| 1 | `plan_uninstall`: `read_state`, `UninstallPlan::from_state`, `refuse_if_needed` -> `dependents` | `ops.rs:630-636,595-618` | **para cada app instalado**: `catalog.get` (linha inteira) + `serde_json::from_str` do manifesto inteiro só para ler `depends` (`raw_depends`, `ops.rs:580`) |
| 2 | `proceed` / impressão do plano | `install_cmd.rs:268-293` | stdout |
| 3 | `uninstall` sob lock: `lock_blocking`, `open_state`, `state.get`, `homepage_of` (outro `catalog.get` + parse JSON completo), **`refuse_if_needed` de novo (repete o item 1)** | `ops.rs:641-666` | repete O(apps) leituras de catálogo + parses |
| 4 | `execute_uninstall`: `files_in_use(app_dir)` abre **cada .exe/.dll** com `share_mode(0)`, em série | `executor.rs:802`, `fs_win.rs:10-34` | 1 `CreateFile` por binário (candidato a varredura por antivírus) |
| 5 | `remove_artifact` por artefato: shim (`remove_shim`, até 4 `remove_file`), atalho (`remove_file` + poda de pastas), env, PATH (`path_users` = query + `path_remove_entry`) | `executor.rs:738-790,804-811` | ~2–6 syscalls por artefato |
| 6 | `remove_any(app_dir)` (e `persist_dir` com `--purge`) | `executor.rs:812-818` | `remove_dir_all` síncrono |
| 7 | `state.remove` (transação, fsync) | `executor.rs:819-821` | 1 fsync |
| 8 | `broadcast_if_changed` | `ops.rs:660-662` | só se o ambiente mudou (ver ideia 3) |
| 9 | `scan_residues`: varredura de registro (até 8 threads, `registry_win.rs:91-204`), `shortcut_dirs` recursivo (Desktop + Start Menu inteiro, profundidade 4) com **`link_target` = COM `CoInitializeEx`+`CoCreateInstance`+`Load`+`GetPath` por `.lnk` e buffer de 64 KiB na pilha**, `subdirs` de `%APPDATA%` e `%LOCALAPPDATA%`, `installed_by_scoop` | `ops.rs:707-750`, `shortcut_win.rs:104-120` | **mediana 105 ms medido** no total; divisão interna não medida |

## 2. Ideias numeradas

Legenda de segurança: **RB** = preserva rollback; **RV** = preserva revalidação sob lock; **SP** = preserva `safepath` e "nada fora da raiz exceto o autorizado".

### 0. Spans `tracing` por etapa (pré-requisito, não é otimização)
- **O quê / onde:** `info_span!` em `ops::{plan_for,apply,uninstall,scan_residues}`, em cada `Step` de `run_step` (`executor.rs:415`, usando `step_label`), em `fetch_all`/`cached`, `extract_all`, `broadcast`, `files_in_use`, `state.record`.
- **Mecanismo:** custo ~zero com `tracing` desligado. Resolve o "não medido" do relatório (§3, "Divisão interna dos ~410 ms").
- **Como medir:** `install ripgrep --yes -v` com raiz temporária e `WINGETRS_TEST_ENV_KEY`, 15 execuções; somar span por etapa; depois `bench.ps1 ... -Baseline noop.exe` para confirmar que os spans não custam.
- **Risco:** nenhum para RB/RV/SP. **Prioridade: P0** (sem isso, as ideias 4–12 são palpite; armadilhas diz "spans primeiro").

### 1. Fazer o broadcast sem bloquear: paralelo por janela, ou assíncrono, ou sobreposto
- **Já feito no código:** só transmite se o valor final de alguma variável difere do snapshot (`ops.rs:293-304`, `apply` em `ops.rs:546-577`, `uninstall` em `ops.rs:660`). Logo, 2ª instalação com `shims` já no PATH não transmite; rollback completo também não. O que sobra é a 1ª instalação e a desinstalação do último app (PATH muda). Também já há um broadcast por `apply` (por grupo de dependências); o que falta é um por lote em `upgrade --all` (`install_cmd.rs:431-436` chama `apply` por grupo) e na TUI — essa é a tarefa já planejada, não repito.
- **Mecanismo A (assíncrono):** trocar `SendMessageTimeoutW` por `SendNotifyMessageW` por janela de topo (enumeradas com `EnumWindows`): para janelas de outras threads retorna imediatamente. Atenção: o `lParam` é ponteiro para string; para mensagens de sistema (`WM_SETTINGCHANGE`) o Windows faz o marshaling no `SendNotifyMessage`, mas isso é **hipótese a provar com sonda** (não afirmo). Alternativa sem dúvida de marshaling: `SendMessageTimeoutW` em N threads (uma por faixa de HWNDs de `EnumWindows`), custo = max em vez de soma; as janelas lentas (a de 2,5 s medida) ainda bloqueiam o join.
- **Mecanismo B (sobrepor):** o broadcast só depende de a escrita no registro ter terminado. Disparar numa thread logo depois do último `env.set` (`add_path`, `executor.rs:601`) e juntar antes de sair de `apply` (guard com `Drop`, regra do projeto: thread antecipada juntada em todos os caminhos). Sobrepõe com `RecordState`, `remove_old_version`, e, num lote, com o download/extração do próximo pacote. Ganho limitado ao que há depois do último env set (pequeno no install de 1 pacote; grande só em lote). Para uninstall: sobrepor com `scan_residues` (105 ms), que não depende do broadcast.
- **Mecanismo C (política):** não retirar `shims` do PATH ao desinstalar o último app (o scoop também deixa). Elimina o broadcast do uninstall-do-último e o do próximo primeiro install. Muda comportamento documentado ("Desfazer entrada no PATH"); decisão de produto, não só desempenho.
- **Recurso economizado:** wall-clock até ~500 ms (A/B/C) por operação que muda o PATH; CPU de todos os processos com janela (cada um processa a mensagem).
- **Como medir antes:** sonda `examples/` (outro agente mede; eu não rodo) com as variantes A (NotifyMessage), A' (N threads), baseline atual; 15 execuções intercaladas; registrar também o tempo até o Explorer enxergar o novo PATH num `cmd` novo (correção funcional: o novo PATH tem de aparecer em processos abertos depois).
- **Risco/custo:** A pode deixar o Explorer/terminais atualizando depois do exit do wingetrs: um terminal aberto logo em seguida pode ver PATH velho por ms (o valor no registro já está certo, só o cache do Explorer atrasa). B exige guard `Drop`. RB: o rollback do env (`apply_undo`, `executor.rs:228`) continua antes do broadcast pois `broadcast_if_changed` relê o estado final. RV/SP: não tocam. **Prioridade: P1** (maior item isolado medido; A é o candidato mais barato se a sonda provar o marshaling).

### 2. Cache hit de download: um só passo de hash, sem duplicar, sem ler duas vezes
- **Onde:** `download.rs:87-97` (`cached`) chama `BlobStore::get` (`cache/mod.rs:166`: `fs::read` + SHA-256) e depois `verify(spec, &bytes)` (`download.rs:19-27`), que para `Sha256` recalcula o mesmo hash dos mesmos bytes. Em `download_one` (`download.rs:112-114`) o arquivo recém-baixado também é relido inteiro (`fs::read`) e hasheado (verify + `Sha256::digest` de novo se o spec não é sha256).
- **Mecanismo:** (a) hash em streaming (`BufReader` de 256 KiB + `Digest::update`), uma passada, sem `Vec` do tamanho do arquivo; (b) quando o spec é sha256, o hash do nome do blob já é a verificação: pular o 2º `verify` (para sha1/sha512/md5, calcular os dois hashes na mesma passada); (c) no download, hashear dentro do laço de chunks de `http.get_to_file` (`http.rs:147-155`) em vez de reler o arquivo.
- **Recurso:** CPU (1 de 2 SHA-256 por cache hit), memória (pico = tamanho do arquivo, hoje; ex. instalador de 100+ MB vira ~256 KiB), leitura de disco (1 de 2 em download novo).
- **Como medir antes:** span em `cached` + `bench.ps1` com um manifesto cujo blob tem ~50–200 MB (fixture sintética em `target/`, não em `tests/fixtures`); `ripgrep` (pequeno) quase não vai mostrar ganho — só vale medir com arquivo grande.
- **Risco/custo:** baixo. **Preserva a validação**: continua a hashear todo o conteúdo (tem de manter a fixture "falha só na checagem do hash" da regra do projeto, e o teste de equivalência entre caminhos: cache hit vs download devem aceitar/recusar os mesmos bytes). Não use "confiar em tamanho+mtime" (ideia alternativa de pular o hash no hit): troca segurança por velocidade — o blob adulterado deixaria de ser detectado; só consideraria com decisão explícita do usuário. RB/RV/SP: sem efeito. **Prioridade: P1 para pacotes grandes, P3 para os pequenos.**

### 3. Desinstalar sem pagar `dependents` duas vezes e sem parsear o manifesto inteiro
- **Onde:** `ops.rs:595-618` (`dependents`), chamado em `plan_uninstall` (`ops.rs:635`) e de novo sob lock (`ops.rs:653`); `raw_depends` (`ops.rs:580-592`) faz `serde_json::from_str::<Value>` do manifesto completo de cada app instalado; `manifest_raw`→`catalog.get` (`catalog/mod.rs:457`) traz todas as colunas (`PKG_COLS`) quando só `raw` é preciso; `homepage_of` (`ops.rs:668-678`) repete `catalog.get` + parse completo do app que está sendo removido.
- **Mecanismo:** (a) consulta dedicada `SELECT raw FROM packages WHERE source=?1 AND id=?2` (ou só o campo `depends` já extraído na gravação do catálogo/`state.db`); (b) pré-filtro textual: se `raw` não contém `"depends"`, nem parsear; (c) gravar `depends` na tabela `apps` no `RecordState` (coluna nova, migração) e consultar só o `state.db`: O(1) arquivos; (d) sob lock, reaproveitar o resultado do plano quando `state.apps()` (app, versão) é idêntico ao do plano — a **checagem sob lock continua existindo**, só compara a "impressão digital" do estado em vez de reler o catálogo.
- **Recurso:** CPU (parse JSON de N manifestos × 2), E/S SQLite, tudo proporcional ao nº de apps instalados (hoje poucos; cresce com o uso). Para 3 apps é pouco; é escalabilidade.
- **Como medir antes:** span em `dependents`; instalar 20–30 apps numa raiz temporária (fixtures) e medir `uninstall --dry-run` e `uninstall --yes` com `bench.ps1 -Baseline`.
- **Risco/custo:** (a)(b) baixo. (c) muda schema (migração; `state.rs:SCHEMA`) e é fonte de verdade duplicada com o catálogo: exige teste de equivalência (regra do projeto). (d) enfraqueceria RV se mal feito: manter a releitura do estado sob lock e só pular a releitura de catálogo quando o estado é idêntico. SP/RB: sem efeito. **Prioridade: P2 (a+b+d), P3 (c).**

### 4. `owner_of`: N+1 consultas por shim e por atalho
- **Onde:** `executor.rs:627-649`: `state.apps()` e depois `state.get(&rec.app)` (2 `prepare` + 2 queries) para cada outro app, chamado para **cada** shim (`executor.rs:652`) e atalho (`executor.rs:518`). `StateDb::get` usa `conn.prepare` (não `prepare_cached`) (`state.rs:141-152`).
- **Mecanismo:** uma consulta `SELECT app FROM artifacts WHERE kind=?1 AND lower(target)=lower(?2) AND app<>?3 LIMIT 1` (com índice `(kind,target COLLATE NOCASE)` ou aproveitando `artifacts_app`); ou `prepare_cached`. Idem `path_users` já é uma consulta só (`state.rs:194`).
- **Recurso:** E/S SQLite e CPU, O(shims × apps) → O(shims). Com 1 app instalado é desprezível; um pacote com 10 binários e 30 apps instalados = ~600 queries.
- **Como medir antes:** span no step `Shim`; fixture com `bin` múltiplo (`bin-alias__main__alist.json` ou similar) em raiz temporária com N apps no `state.db`.
- **Risco:** a regra "o comando já pertence a X" é de integridade (impede sobrescrever shim de outro app): precisa de teste que compare o resultado da consulta nova com o laço antigo (equivalência). O `LIKE`/`lower()` do SQLite é ASCII-only enquanto `eq_ignore_ascii_case` também é ASCII → coincide; ainda assim cobrir com teste. RB/RV/SP: preservados. **Prioridade: P2** (barato, mas ganho depende de N).

### 5. Resíduos do uninstall: sobrepor o que é independente da remoção de arquivos
- **Onde:** `scan_residues` (`ops.rs:707-750`), chamado só depois de `execute_uninstall` e do broadcast (`ops.rs:660-665`). Mediana 105 ms vs. ~80 ms do uninstall inteiro: a varredura é maior que a remoção.
- **Mecanismo:** (a) **registro** (`values_containing` + `subkeys_named`) só depende de `app_dir` (string) e do nome: iniciar numa thread (`std::thread::scope`) antes de `execute_uninstall` e juntar depois (guard `Drop` + `resume_unwind`, regra do projeto). Já usa até 8 threads; sobrepor com remoção/`files_in_use` pode atrasar ambos se o disco/CPU saturar: medir. (b) **Atalhos/pastas** não podem começar antes: o uninstall remove atalhos do próprio app (`ArtifactKind::Shortcut`) e um scan adiantado listaria `.lnk` que sumirão (falso resíduo). Ficam depois. (c) `link_target` abre COM por `.lnk` (`shortcut_win.rs:104-120`: `CoInitializeEx`/`CoCreateInstance`/`Load`/`GetPath` com buffer `[0u16;32768]` = 64 KiB zerado por chamada): inicializar COM e criar o `IShellLinkW` **uma vez por thread** e reutilizar para todos os `.lnk`; reutilizar o buffer; filtrar antes por tamanho/prefixo do nome? (não: o `.lnk` pode ter qualquer nome). (d) paralelizar `link_target` por `.lnk` em threads (cada uma com seu apartamento COM).
- **Recurso:** wall-clock (até ~100 ms de 105 sobrepostos com ~80 ms de uninstall em (a); (c)(d) reduzem a parte de atalhos, cuja participação nos 105 ms é **não medida**); CPU: (c) elimina N inits de COM.
- **Como medir antes:** spans separados `scan.registry`, `scan.shortcuts` (`lnk_files` + `link_target`), `scan.folders`; contar `.lnk` em Start Menu+Desktop desta máquina; A/B sequencial vs sobreposto. `scripts/bench.ps1 uninstall ... ` com a varredura ligada (hoje o benchmark de raiz temporária usa `WINGETRS_TEST_ENV_KEY` e **`ResidueScan::none()`** — `prod_win.rs:62-66` — então os 80 ms de uninstall do relatório não incluem folders/atalhos, e o de 105 ms é só o registro; atenção ao comparar).
- **Risco:** (a) exige que `RESIDUE_REG_ROOTS` não inclua chaves que o próprio uninstall altere: hoje as raízes (`ops.rs:30-36`) são `Software\Classes`, `Run`, `RunOnce`, `App Paths`, `Uninstall`; env fica em `HKCU\Environment`, fora. OK, mas escrever teste fixando essa premissa. RB: o scan só roda após sucesso; se o uninstall falhar, descartar o resultado da thread (join mesmo assim). RV/SP: sem efeito. **Prioridade: P1 (a) / P2 (c).**

### 6. `files_in_use`: abrir só o que importa e em paralelo
- **Onde:** `fs_win.rs:10-34`: percorre `apps\<app>` inteiro e faz `CreateFile(share_mode 0)` em **todo** `.exe` e `.dll`, em série; a junção `current` não é seguida (symlink não é `is_dir`/`is_file`), então cada arquivo é visto uma vez (ok).
- **Mecanismo:** (a) paralelizar a abertura (rayon não está no `Cargo.toml`: usar `std::thread::scope` em N lotes); (b) parar no primeiro arquivo em uso (a função só é usada por `.first()` em `executor.rs:802`): hoje continua a varrer o resto e ordena; (c) priorizar `.exe` e abrir `.dll` só se nenhum `.exe` estiver em uso? Muda a cobertura (DLL carregada por outro processo sem o exe: raro mas possível) — não recomendo sem decisão.
- **Recurso:** E/S e CPU de antivírus em abrir binários grandes (apps Electron têm centenas de DLL/exe); **não medido** — pode ser desprezível aqui.
- **Como medir antes:** span em `files_in_use`; app com muitos binários (ex. um Electron/`cpu-z` do relatório).
- **Risco:** (b) é seguro (retorno só usado como "alguma em uso"; mas `files_in_use` é `pub` e pode aparecer na mensagem: manter `first()` determinístico = menor caminho: então não sair no primeiro achado se a mensagem deve citar o primeiro em ordem). RB/RV/SP: sem efeito. **Prioridade: P3.**

### 7. `remove_dir_all` fora do caminho crítico: renomear para lixo e apagar em paralelo com o resíduo
- **Onde:** `executor.rs:812-818` (`remove_any(app_dir)`), `executor.rs:108-123`.
- **Mecanismo:** `rename(apps\<app>, .trash\<app>-<n>)` (O(1) no mesmo volume), depois `state.remove`, e a deleção real numa thread que roda **durante** `scan_residues`, juntada antes de retornar (guard). Se algo falhar antes do `state.remove`, o rename é desfeito (é até mais atômico que apagar arquivo a arquivo: hoje um `remove_dir_all` que falha no meio deixa o app meio apagado com o registro intacto). Sobras de execuções interrompidas: varrer `.trash` no próximo `repair`/uninstall.
- **Recurso:** wall-clock do uninstall proporcional ao número de arquivos (Electron: milhares) — **não medido**; criação de entrada nova em disco (`.trash`) dentro da raiz, autorizado.
- **Como medir antes:** span em `remove_any`; app grande vs ripgrep (2 arquivos); A/B.
- **Risco/custo:** médio: novo estado em disco (`.trash`), `PartlyApplied`-like para a deleção em segundo plano, e a regra de threads (join em todos os caminhos). RB melhora; RV/SP: o destino da rename é fixo (`layout`), sem caminho de manifesto. **Prioridade: P2 (só vale se o span mostrar >20 ms em apps grandes).**

### 8. Extração: buffer maior, cache de diretórios, extrair direto no staging, paralelo por entrada
- **Onde:** `extract.rs:152-166` (`write_entry`: `create_dir_all(parent)` por arquivo + `File::create` + `io::copy` direto no `File` sem buffer, 8 KiB por `WriteFile`); `extract.rs:168-193` (`extract_zip`); `extract.rs:106-127` (pasta temporária + `merge_into`, que com `dest` inexistente é um `rename`, e logo `Promote` faz outro `rename`, `executor.rs:483-494`).
- **Mecanismo:** (a) `BufWriter` de 64–256 KiB ou `io::copy` com buffer próprio (menos syscalls `WriteFile`); (b) `HashSet<PathBuf>` dos diretórios já criados (evita `create_dir_all` = stat/`CreateDirectory` por arquivo); (c) no caminho do install, `dest` é o `staging` que já tem `Undo::RemovePath` (`executor.rs:471-478`): a pasta temporária extra é redundante, extrair direto elimina 1 rename e a poda recursiva do tmp; **mas** `extract_all` também é usado pelo launcher (`shim.rs:111`) e tem a garantia "tudo ou nada em `dest`" nos testes: manter a garantia só para quem pede; (d) zip: reabrir o arquivo por thread e dividir `0..zip.len()` entre threads (a descompressão deflate é CPU; `zlib-rs` só está habilitado em `flate2` para tar.gz — o zip usa `features=["deflate"]`, que em `zip` 8.x aponta o backend padrão do `flate2` (miniz_oxide); existe no `zip` uma feature `deflate-zlib-rs`? **não verifiquei**; trocar feature exige autorização do brief, regra de versões); (e) validar nomes na mesma passada de extração: a 1ª passada (`by_index_raw` só de nomes, `extract.rs:171-180`) existe para "nada escrito se houver entrada ruim" — manter a garantia, mas é barata (diretório central já em memória).
- **Recurso:** syscalls e CPU de descompressão; Defender/AV varrem cada arquivo criado (custo por arquivo, **não medido**; reduzir nº de operações ajuda pouco, o nº de arquivos não muda).
- **Como medir antes:** span em `extract`; `bench.ps1` com arquivo de ~500 arquivos / ~100 MB; `procmon`-style não disponível, então contar syscalls por amostragem de `Get-Process` I/O counters (`ReadOperationCount`/`WriteOperationCount` antes/depois) — ferramenta nativa, sem instalar.
- **Risco/custo:** (a)(b) baixo. (c) médio (muda semântica atômica). (d) médio: ordem de criação e colisões de caminhos (dois nomes que normalizam igual no NTFS case-insensitive: hoje serial, "o último ganha"; em paralelo vira corrida: **perigoso**, resolver antes com detecção de colisão). SP: a validação de `safe_components` por entrada (`extract.rs:35,171,184`) tem de continuar antes de qualquer escrita; (d) não pode contornar. **Prioridade: P2 (a+b), P3 (c,d).**

### 9. `state.db`: aberturas, DDL e fsync
- **Onde:** `ops.rs:214-225` (`open_state` = `Connection::open` + `PRAGMA journal_mode=WAL` + `execute_batch(SCHEMA)` a cada `apply`/`uninstall`; `plan_for` abre **outra** conexão em `read_state`), `state.rs:88-98`; `state.rs:116-139` (`record` sem `prepare_cached`); `state.rs:141-170` (`get`: 2 `prepare` por chamada, chamado em `plan_for` por nó, `revalidate`, `execute_install`, `owner_of`).
- **Mecanismo:** (a) `prepare_cached`; (b) `PRAGMA synchronous=NORMAL` com WAL: o commit deixa de fazer fsync (seguro contra corrupção em WAL; o risco é perder o último commit numa queda de energia, e o `repair`, `ops.rs:850`, reconstrói `state.db` a partir do disco — checar se cobre esse caso antes de aceitar); (c) pular o DDL quando `user_version` já indica schema atual (um `PRAGMA user_version` em vez de 3 `CREATE ... IF NOT EXISTS`); (d) reaproveitar a conexão do plano na aplicação (a conexão de `plan_for` é só leitura; sob lock é preciso uma de escrita: não reaproveitar).
- **Recurso:** ~11–15 ms por abertura SQLite (armadilhas, para o catálogo; `state.db` é pequeno, **não medido**) e um fsync (~ms, não medido).
- **Como medir antes:** spans `state.open`, `state.record`; A/B de `synchronous`.
- **Risco/custo:** (b) é troca durabilidade × latência: **decisão do usuário**, não aplico por padrão. (a)(c) baixo. RB/RV: o `record` continua sendo o último passo e uma transação; RV continua lendo o estado sob lock. **Prioridade: P3.**

### 10. Segurar o lock `install.lock` só pelo que precisa dele (baixar antes)
- **Onde:** `apply` (`ops.rs:546-577`) adquire o lock e só então executa `Step::Download` (`executor.rs:433`); em rede lenta o lock fica retido por todo o download e outro `wingetrs` (TUI + CLI) espera/falha.
- **Mecanismo:** antes de `lock_async`, rodar `fetch_all` para todos os `Source` de todos os planos (idempotente, endereçado por conteúdo; já verifica hash e usa o cache); sob lock, o step `Download` vira cache hit. Isso **também** habilita dentro do mesmo pacote o `MAX_PARALLEL=4` de `fetch_all`: hoje cada step chama `fetch_all(.., from_ref(source), ..)` (um por fonte, `executor.rs:434-440`), então um manifesto com várias `url` baixa em série (não confundir com o paralelismo entre pacotes, já planejado).
- **Recurso:** tempo de retenção do lock; wall-clock em manifestos com várias URLs.
- **Como medir antes:** span `lock.held`; manifesto com 2+ urls (procurar nas fixtures `tests/fixtures/scoop/manifests`; não abra as binárias).
- **Risco:** RV: a revalidação (`revalidate`) deve continuar sob lock — o download antes não toca o estado; mas se o plano ficar inválido após o download (outro processo instalou), o download foi desperdício (inofensivo, o blob fica no cache). RB: sem efeito (download não é desfeito hoje também; fica no cache). SP: hash e `safepath` seguem. A ordem do dry-run/confirm não muda (baixar só depois da confirmação). **Prioridade: P2.**

### 11. Registro: menos idas e vindas ao `HKCU\Environment`
- **Onde:** `env_snapshot` (`ops.rs:287`) abre a chave a cada nome (`RegEnvStore::get` abre/fecha, `env_win.rs:93-109`); `add_path` faz `get`; `set` faz `create_subkey` + `get_raw_value` (para o tipo) + `set_raw_value` (`env_win.rs:112-146`); `broadcast_if_changed` relê tudo.
- **Mecanismo:** manter a `RegKey` aberta durante a operação (uma abertura por `apply`), ou memorizar o tipo lido em `get` para o `set`. Ganho: algumas chamadas de registro (µs a poucos ms, **não medido**).
- **Risco:** o `get` por snapshot e o undo (`restore_env`, `executor.rs:125`) dependem de valores relidos frescos; manter semântica. **Prioridade: P3 (provavelmente abaixo do ruído).**

### 12. `ProdEffects::new` e COM preguiçosos
- **Onde:** `cli/run_win.rs:19`, `prod_win.rs:28-35`, `shortcut_win.rs:27-32,75`: `SHGetKnownFolderPath(FOLDERID_Programs)` roda em todo `install`/`uninstall`/`upgrade`, inclusive `--dry-run` e quando o pacote não tem atalho; `plan_for` ainda chama `shortcuts.root()` (`ops.rs:487`) para o planejador.
- **Mecanismo:** calcular o diretório Programas só quando o plano tiver atalho (ou via `OnceCell`); sem alteração se `WINGETRS_TEST_SHORTCUT_DIR`.
- **Recurso:** uma chamada de shell32 por processo (carregar a DLL já está no custo do processo); **não medido**, provavelmente <5 ms.
- **Risco:** o planejador usa `shortcut_root` para decidir o caminho dos `.lnk` e `safepath` os valida contra a raiz (`check_inside`, `shortcut_win.rs:49`); fazer lazy sem mudar o contrato. **Prioridade: P3.**

### 13. Lançador de shims no cache de blobs
- **Onde:** `shim.rs:90-131`: `ensure_launcher_from(http, _data, ...)` ignora `data`; baixa o zip na 1ª instalação de cada raiz e não o guarda em `blobs/`.
- **Mecanismo:** passar pelo `fetch_all` (hash conhecido: constante em `shim.rs:30-47`) → reinstalação em raiz nova/`repair` sem rede, e verificação reaproveitada.
- **Recurso:** rede e um zip em memória (`http.get` → `Vec<u8>`) só na 1ª vez. **Prioridade: P3** (evento raro; relevante para "instalar offline").

### 14. Medir a parte do antivírus antes de otimizar E/S
- **Hipótese não medida:** parte dos ~410 ms entre dry-run e install real (relatório §3) é varredura de Defender sobre cada arquivo criado/aberto (extração, `install.json`, `.shim`, `.exe` do launcher, `files_in_use`). Experimento: repetir o `bench.ps1` do install com e sem exclusão do Defender para uma pasta de teste (feito pelo usuário; o wingetrs não deve alterar exclusões) e comparar os spans de ideia 0. Se a diferença for grande, as ideias 6–8 (menos arquivos tocados) ficam acima de ideias de CPU.

## 3. Desperdícios evidentes (já visíveis no código)

1. **Hash duplicado no cache hit** para sha256: `BlobStore::get` + `verify` (`download.rs:94-96`, `cache/mod.rs:166-169`) e leitura do arquivo inteiro para a RAM.
2. **`dependents` executado duas vezes** (plano e sob lock) com parse JSON do manifesto completo de cada app instalado, mais `homepage_of` fazendo `catalog.get` + parse de novo (`ops.rs:595-678`).
3. **`owner_of` N+1** por shim e por atalho (`executor.rs:627-649`), com `prepare` sem cache (`state.rs:141-152`).
4. **`CoInitializeEx`+`CoCreateInstance`+buffer de 64 KiB por `.lnk`** na varredura de resíduos (`shortcut_win.rs:104-120`) e COM por atalho criado (`shortcut_win.rs:130-136`).
5. **Download dentro do lock** e um `fetch_all` por fonte (sem usar `MAX_PARALLEL` dentro do pacote) (`executor.rs:433-447`, `ops.rs:546-551`).
6. **Duas conexões SQLite com DDL** no mesmo `apply` (`read_state` no plano e `open_state` sob lock, `ops.rs:214-225`), e o estado do mesmo app lido 3 vezes (`plan_for`, `revalidate`, `execute_install`) — a releitura sob lock é obrigatória (RV), as outras duas são redundantes só dentro de `apply`.
7. **Extração em pasta temporária + `rename` + `Promote` rename**, `create_dir_all` por arquivo, `io::copy` sem buffer (`extract.rs:152-166,106-127`).
8. **`ProdEffects::new` resolve a pasta Programas** mesmo sem atalho (`prod_win.rs:31`).
9. **Benchmark de uninstall não inclui atalhos/pastas** (`ResidueScan::none()` com `WINGETRS_TEST_ENV_KEY`, `prod_win.rs:62-66`): o "80 ms" e o "105 ms" medem coisas diferentes; qualquer ganho em atalhos/pastas só aparece num teste com a máquina real.
10. Lançador baixado sem passar pelo cache de blobs (`shim.rs:90-131`).

## 4. Top 3 recomendado

1. **Ideia 0 + 1: spans no install/uninstall e atacar o `WM_SETTINGCHANGE`.** Sem spans nada mais é confiável; o broadcast é ~metade do install real (438–568 ms), só ocorre quando o PATH muda (já garantido por `broadcast_if_changed`, `ops.rs:293-304`), e a rota mais barata (sonda de `SendNotifyMessageW` ou broadcast em threads por HWND, mais sobrepor numa thread com guard) preserva RB/RV/SP. Medir A/B com sonda e conferir que um `cmd` novo vê o PATH.
2. **Ideia 5(a)+(c): sobrepor a varredura de registro com a remoção e reutilizar COM/buffer na leitura dos `.lnk`.** A varredura (105 ms) é maior que o uninstall todo (~80 ms); a parte de registro não depende dos arquivos removidos, então pode rodar em paralelo (thread com guard `Drop`/`resume_unwind`); atalhos e pastas continuam depois. Medir primeiro com spans `scan.registry`/`scan.shortcuts`/`scan.folders` e com `ResidueScan::user_profile` ligado.
3. **Ideia 2 (+ 3 e 4 como cortes de custo de escala): um só passo de hash no cache de download e menos trabalho O(apps).** Remove o hash duplicado e a leitura integral em RAM (relevante para instaladores grandes), e baratas correções de N+1 em `owner_of`/`dependents`. Todas mantêm a verificação completa de conteúdo, a revalidação sob lock e os caminhos via `safepath`; cada uma precisa do teste de equivalência exigido pelas regras do projeto.

Fora do top 3 mas com bom custo/benefício se os spans confirmarem: ideia 10 (baixar antes do lock, reduz retenção do lock e ativa `MAX_PARALLEL` por pacote) e ideia 7 (deleção da pasta do app em paralelo ao scan de resíduos).
