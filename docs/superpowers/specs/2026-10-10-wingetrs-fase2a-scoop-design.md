# wingetrs — Fase 2a: instalação de apps do Scoop (design)

Data: 2026-10-10
Status: aguardando revisão

## 1. Objetivo e contexto

A fase 1 (somente leitura) está concluída: catálogo winget + scoop, `search`, `show`, `list`, TUI. A fase 2 traz instalação e foi fatiada:

| Fatia | Escopo |
|---|---|
| **2a (esta spec)** | Scoop, manifestos **simples**: `install`, `uninstall`, `upgrade`, `--dry-run`, buckets do GitHub, limpeza de resíduos |
| 2b | Scoop: downloads `.7z` e `.msi` (extração) sem script |
| 2c | Scoop: manifestos com scripts PowerShell e innosetup |
| 2d | winget: instaladores silenciosos (MSI, MSIX, Inno, NSIS, exe), UAC, códigos de saída |

Motivação (do usuário): aprendizado + velocidade. Restrição de ambiente: Windows **Home** (sem Windows Sandbox) — instalações de teste rodam no sistema real, por isso a 2a começa pelo que não exige admin e é reversível.

### Cobertura medida (catálogo real, 2026-10-09)

| Nível | Main (1.671) | Extras (2.419) |
|---|---|---|
| **Simples** (zip, exe solto, tar.gz; sem script/7z/msi/innosetup) — **2a** | **77,8%** | **46,7%** |
| + 7z/msi sem script — 2b | 82,6% | 54,4% |
| + scripts/innosetup — 2c | ~100% | ~100% |

Recursos que o nível simples exige (fração de manifestos que os usam): `shortcuts` (Main 3,4% / Extras 81,1%), `persist` (9,3% / 29,3%), `env_add_path`/`env_set` (9,6% / 2,7%), `depends` (3,3% / 2,8%), `bin` (maioria), `extract_dir`, renomear via `#/`.

### Fora do escopo da 2a

Manifestos com `pre_install`/`post_install`/`installer.script`/`uninstaller.script`/`psmodule`/`innosetup`/`installer.file`; downloads `.7z`, `.msi`, `.rar`, `.nupkg`, `.tar.xz`, `.tar.bz2`; instalação de pacotes **winget**; versões antigas (bucket Versions entra como bucket comum, sem seleção de versão); buckets fora do GitHub; instalação no `~/scoop` do usuário; escopo de máquina/admin.

## 2. Decisões fechadas

| Decisão | Escolha | Motivo |
|---|---|---|
| Onde instalar | Raiz própria `%LOCALAPPDATA%\wingetrs\apps` com layout do scoop | Sem risco ao `~/scoop` do usuário; reversível; reaproveita o leitor de instalados da fase 1 |
| Nível de manifesto | Só "simples" | 78% do Main sem dependência nova nem execução de código de terceiros |
| Operações | `install`, `uninstall`, `upgrade` (+ `--all`), `--dry-run`, CLI **e** TUI | O plano puro alimenta CLI, TUI e dry-run com a mesma lógica |
| Arquitetura | Plano puro (`InstallPlan`) + executor com staging e rollback | Testável sem disco; dry-run e confirmação nunca divergem da execução |
| Identificação | ID ou nome, com ou sem bucket; ambiguidade ⇒ seleção com setas (CLI e TUI) | Pedido do usuário |
| Buckets | `bucket add/list/rm` para repositórios do GitHub; 10 conhecidos por nome | Generaliza o download por codeload que já existe |
| Resíduos | Varredura em dois níveis de confiança, sempre com confirmação, remoção recuperável | Limpa o certo, nunca apaga o duvidoso sem o usuário ver |

## 3. Layout em disco

```
%LOCALAPPDATA%\wingetrs\
  apps\<app>\<versão>\          arquivos extraídos
  apps\<app>\current            junction → versão ativa
  apps\<app>\current\install.json   {"bucket","architecture","version","wingetrs":1} (formato do scoop + marca)
  persist\<app>\                dados persistentes (junctions/hardlinks a partir da versão)
  shims\                        <bin>.exe + <bin>.shim (ou .cmd/.ps1 wrappers)
  backups\<AAAAMMDD-HHMMSS>\<app>.reg   exportações antes de apagar chaves de resíduo
  state.db                      estado de instalação (SQLite próprio, separado do catalog.db)
  install.lock                  lock de instalação (separado do update.lock)
  catalog.db, search.idx, http.json, blobs\, log\   (fase 1, inalterados)
```

Fora da raiz, o wingetrs escreve **somente**: `HKCU\Environment` (entrada `…\wingetrs\shims` no `Path` e variáveis de `env_set`/`env_add_path`), `%APPDATA%\Microsoft\Windows\Start Menu\Programs\wingetrs Apps\` (atalhos), a Lixeira (resíduos) e chaves `HKCU` de resíduo confirmadas pelo usuário. Nunca `HKLM`, `C:\Program Files`, nem nada fora do perfil do usuário. O `CLAUDE.md` passa a refletir esses limites.

## 4. Componentes

```
src/install/
  mod.rs          API pública: resolve, plan, execute, uninstall, upgrade
  target.rs       resolução do alvo (id/nome/bucket) e lista de candidatos
  manifest.rs     leitura tipada do manifesto scoop + checagem de suporte (com motivo)
  planner.rs      (puro) manifesto + arquitetura + estado → InstallPlan
  plan.rs         InstallPlan / Step / UninstallPlan (dados; Display para dry-run)
  executor.rs     aplica passos via traits, com staging e rollback em ordem inversa
  download.rs     downloads com cache por sha256 (blobs\), progresso, hash (sha256/sha1/sha512/md5)
  extract.rs      zip, tar.gz, exe/arquivo solto; extract_dir/extract_to; renomeação `#/`
  shim.rs         geração de shims e wrappers
  state.rs        state.db (apps, artefatos) + install.json
  residue.rs      (puro) classificação de resíduos em alta/baixa confiança
  traits.rs       Fs, EnvStore, Shortcuts, Trash, RegistryScan (+ fakes em memória para teste)
  fs_win.rs, env_win.rs, shortcut_win.rs, trash_win.rs, registry_win.rs   implementações reais
src/buckets.rs    config de buckets: conhecidos + adicionados (GitHub owner/repo)
src/cli/picker.rs seleção com setas inline (mesmo componente/estado usado pela TUI)
```

Dependências novas previstas: nenhuma além do que já existe (`zip`, `flate2`+`tar`, `sha2`, `windows` com features adicionais para COM `IShellLinkW`, `IFileOperation`, `SendMessageTimeoutW`). Hashes `sha1`/`md5` dos manifestos: confirmar no plano (crate ou implementação) — decisão com sondagem.

## 5. Resolução do alvo

Entrada: `<alvo>` = `id`, `nome`, `bucket/id`; ou `--bucket <b>`.

1. ID exato (case-insensitive) em buckets do scoop; com bucket informado, só nele.
2. Moniker/nome exato.
3. Nada exato ⇒ busca fuzzy (a mesma do `search`) para **sugerir** — nunca instala por fuzzy.

Resultado:
- 1 candidato instalável ⇒ segue.
- \>1 ⇒ **seleção com setas** (CLI inline e TUI, mesmo componente): `↑/↓`, `Enter` escolhe, `Esc` cancela. Linhas: nome, id, versão, bucket, descrição.
- Sem terminal interativo (stdin/stdout não-TTY) **ou** com `--yes` ⇒ erro listando candidatos e sugerindo `bucket/id`.
- Candidatos do **winget** aparecem na lista marcados "instalação na fase 2d", não selecionáveis.

## 6. Operações

### 6.1 `install <alvo> [--dry-run] [--yes]`

1. Resolver o alvo (§5).
2. Checar suporte do manifesto; não suportado ⇒ `NãoSuportado(motivo)` sem tocar no disco. Manifesto sem `hash` para alguma URL também é `NãoSuportado` ("sem hash para verificar o download").
3. Resolver `depends` recursivamente (detecção de ciclo); já instaladas são puladas; dependência não suportada ⇒ falha antes de qualquer download.
4. Planejar (puro): arquitetura (`64bit` → `32bit` → `arm64` conforme a máquina), URLs, hashes, extração, shims, atalhos, persist, env.
5. Mostrar o plano e confirmar (pulado com `--yes`; `--dry-run` encerra aqui sem tocar no disco).
6. Executar com staging: baixar (paralelo; cache por sha256) e verificar hash → extrair em `apps\<app>\.staging-<versão>` → renomear para `<versão>` → `current` → shims → atalhos → persist → env → estado. Falha em qualquer passo ⇒ desfazer os passos executados em ordem inversa; `current` só muda no fim.

### 6.2 `upgrade [<alvo> | --all] [--keep] [--dry-run] [--yes]`

Versão nova vem do catálogo (comparação de versões da fase 1). Instala lado a lado (passos 3–6), troca `current`, regera shims/atalhos, **preserva `persist`**, remove a versão antiga após sucesso (`--keep` mantém).

### 6.3 `uninstall <alvo> [--purge] [--dry-run] [--yes]`

1. Recusa se outro app instalado pelo wingetrs depende deste (lista quem).
2. Remove shims, atalhos e variáveis **registrados em `state.db`** (restaurando `valor_anterior` quando houver); remove a entrada `…\wingetrs\shims` do `Path` se não restar app.
3. Remove `apps\<app>`; `persist\<app>` é preservado salvo `--purge`.
4. Varredura de resíduos (§8) e confirmação.
5. App em uso (arquivos bloqueados) ⇒ para **antes** de remover qualquer coisa, indica o arquivo e sugere fechar o programa; nunca mata processo.

### 6.4 `bucket add <nome> [<url>]`, `bucket list`, `bucket rm <nome>`

Conhecidos por nome: main, extras, versions, nerd-fonts, java, games, nirsoft, sysinternals, php, nonportable (lista do `buckets.json` oficial do scoop). Outros: URL do GitHub `owner/repo` ou `https://github.com/owner/repo`; bucket não conhecido pede confirmação explícita ("bucket de terceiro: código/binários de terceiros"). O download usa o tarball do codeload com ETag (já existente). `bucket rm` recusa se houver app instalado vindo dele (lista quais).

### 6.5 Concorrência

`install.lock` serializa install/upgrade/uninstall/bucket; independente do `update.lock` (refresh em segundo plano não bloqueia instalação e vice-versa). Threads seguem a regra do projeto (joined em todos os caminhos, pânico propagado).

## 7. Estado, shims, PATH e atalhos

- **`state.db`**: `apps(app, bucket, versão, arquitetura, instalado_em, manifesto_sha256)`; `artefatos(app, tipo, alvo, valor_anterior)` com tipo ∈ {shim, atalho, persist, path, env}. Única fonte de verdade do que o wingetrs criou.
- **`install.json`** replica o essencial no formato do scoop (para o leitor da fase 1 e para `repair`).
- **Shims** em `shims\`: lançador `.exe` + `.shim` (`path = …`, `args = …`) no formato do scoop; wrappers `.cmd`/`.ps1` para scripts. Origem do lançador (shim do scoop, MIT, baixado e verificado, ou lançador próprio em Rust) é decidida no plano após medir o tempo de abertura.
- **PATH**: uma entrada `…\wingetrs\shims` em `HKCU\Environment\Path`, adicionada na primeira instalação e removida após o último app. Preserva `REG_EXPAND_SZ` e `%VAR%` não expandidas. Após mudar, `WM_SETTINGCHANGE` ("Environment").
- **Atalhos**: `Start Menu\Programs\wingetrs Apps\<nome>.lnk` via `IShellLinkW`.
- **`wingetrs repair`**: reconstrói `state.db` a partir dos `install.json` e do conteúdo de `shims\`; app com pasta apagada aparece no `list` como "quebrado" e o `uninstall` limpa os artefatos restantes.

## 8. Resíduos (após `uninstall`)

- **Alta confiança (pré-marcados)**: valores sob `HKCU` cujo dado contém o caminho `…\wingetrs\apps\<app>\`, varridos só em locais conhecidos: `HKCU\Software\Classes` (associações, `shell`, ProgIDs), `…\CurrentVersion\Run` e `RunOnce`, `…\App Paths`, `…\Uninstall` (HKCU); e `.lnk` em Área de Trabalho, Menu Iniciar e Inicialização cujo alvo está nessa pasta.
- **Baixa confiança (desmarcados)**: pastas/chaves com o nome do app (ou do dono do repositório quando a homepage é do GitHub) em `%APPDATA%`, `%LOCALAPPDATA%`, `HKCU\Software`.
- **Conflito**: se o mesmo app aparece no `list` instalado por scoop/winget, o item é marcado "pode ser de outra instalação" e nunca pré-selecionado.
- **Recuperável**: pastas → Lixeira (`IFileOperation` com `FOFX_RECYCLEONDELETE`); chaves → `reg export` para `backups\…\<app>.reg` e só então apagadas.
- Seleção na mesma lista com setas (espaço marca/desmarca). Com `--yes`: remove só os de alta confiança.
- **Emenda (2026-10-11):** na TUI, o ajuste "Resíduos após remover" pode trocar a confirmação por "só alta confiança" (mesmo efeito do `--yes`) ou "manter" (não remove nada; avisa quantos ficaram). O padrão continua sendo confirmar.

## 9. Erros

`InstallError`: `NãoEncontrado{sugestões}`, `Ambíguo{candidatos}` (só em modo não interativo), `NãoSuportado{motivo}`, `HashDivergente{url}`, `Download(HttpError)`, `Extração{…}`, `EmUso{caminho}`, `DependênciaCíclica{cadeia}`, `DependênciaNãoSuportada{app,motivo}`, `Ocupado`, `Io{…}`. Mensagens em português. Toda falha após o início da execução dispara rollback e informa o que foi desfeito. Dado externo nunca causa pânico.

## 10. TUI

- Teclas: `i` instalar, `u` remover, `U` atualizar (selecionado; na aba Atualizações, `U` em lote com confirmação).
- Tela de confirmação mostra o `InstallPlan` (mesmo texto do `--dry-run`).
- Progresso de download/extração na barra de status; erro com estilo `err` e o que foi desfeito.
- Seleção de candidatos e de resíduos com o mesmo componente da CLI.

> **Emenda (2026-10-11, aprovada pelo usuário).**
> - Progresso continua na barra de status; **resultados e erros viram toasts** (fase 1 §7.1, emenda).
> - Aba **Ajustes**, gravada na seção `[behavior]` do `config.toml` (gravação lê-modifica-grava, para não apagar `bucket add` feito pela CLI com a TUI aberta): confirmar antes de aplicar (`sempre` padrão / `só remoções` / `nunca`), purge ao remover (`não` padrão), manter versão antiga ao atualizar (`não` padrão), resíduos (`perguntar` padrão / `só alta confiança` / `manter`), glifos e tema. Escolha de candidato ambíguo nunca é pulada.
> - Os ajustes valem **só na TUI**; a CLI continua com as flags de §6.

## 11. Testes e critérios de sucesso

Isolamento: todo efeito passa por `Fs`, `EnvStore`, `Shortcuts`, `Trash`, `RegistryScan`. Testes usam fakes em memória/`tempdir`; as implementações `*_win.rs` têm testes ao vivo opt-in que usam **`HKCU\Software\wingetrs-test`** no lugar do `Path` real e uma pasta temporária no lugar do Menu Iniciar (selecionados por variáveis de ambiente que só os testes definem). Nenhum teste toca o `Path` ou o Menu Iniciar reais.

1. **Planejador**: manifestos reais (fixtures do Main/Extras) cobrindo `extract_dir`, `#/rename`, `bin` com alias, `shortcuts`, `persist`, `env_*`, `depends` ⇒ snapshot `insta` do plano; manifestos com script/7z/msi/innosetup ⇒ `NãoSuportado` com o motivo certo.
2. **Rollback por injeção de falha**: para cada passo `k`, falhar em `k` ⇒ estado de `Fs`/`EnvStore` falsos **idêntico ao inicial**.
3. **Ciclo completo com bucket falso** (`wiremock` + zip real pequeno + raiz em `tempdir`): install → upgrade (persist sobrevive) → uninstall (estado idêntico ao inicial; `--purge` remove persist).
4. **`--dry-run`**: raiz idêntica byte a byte antes/depois.
5. **Resíduos**: registro falso com valores apontando para a pasta removida (alta), nomes semelhantes (baixa) e caso "mesmo app no scoop" (nunca pré-selecionado).
6. **Seleção com setas**: estado puro (teclas, Enter, Esc), como a TUI da fase 1.
7. **Resolução**: id, nome, `bucket/id`, `--bucket`, ambíguo interativo vs. `--yes`/não-TTY, candidato winget não selecionável.

Critérios:
- Testes 2, 3 e 4 verdes (prova de "nunca meio instalado", "nada sobra", "dry-run não toca").
- Prova ao vivo (com autorização do usuário): `wingetrs install ripgrep` → terminal novo `rg --version` → `wingetrs uninstall ripgrep` ⇒ nenhuma pasta, shim, entrada de PATH ou atalho restante.
- `install` de app com download já em cache < 1 s (`bench.ps1 -Baseline`, sem rede).
- Planejador em milissegundos (span medido).

## 12. Riscos e mitigação

| Risco | Mitigação |
|---|---|
| Estragar o `Path` do usuário | Leitura/escrita preservando `REG_EXPAND_SZ`; uma única entrada própria; testes só em chave de teste; backup do valor anterior em `state.db` |
| Apagar dado do usuário como "resíduo" | Só alta confiança pré-marcada; baixa confiança desmarcada; conflito com scoop/winget nunca pré-selecionado; Lixeira e `.reg` de backup |
| App duplicado (scoop e wingetrs) | `list` mostra a origem; a ordem no PATH decide qual roda (documentado na ajuda) |
| Manifesto de bucket de terceiro malicioso | Sem execução de scripts na 2a; confirmação explícita para bucket não conhecido; hash obrigatório (manifesto sem hash é recusado) |
| Junction/hardlink em `persist` mal criado | Testes de ciclo completo com `persist`; rollback por passo |
