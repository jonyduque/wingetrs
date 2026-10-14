# wingetrs — Fase 1: catálogo e leitura (design)

Data: 2026-10-07
Status: aguardando revisão

## 1. Objetivo

wingetrs é um gerenciador de pacotes para Windows escrito em Rust, com TUI em ratatui, que consome o catálogo do winget (winget-pkgs) e buckets do Scoop.

Motivações, em ordem:

1. **Aprendizado** — reimplementar as peças nativamente (sem delegar à API COM do winget).
2. **Velocidade** — o winget é lento em todos os comandos; wingetrs deve responder em milissegundos usando cache local.
3. **Portabilidade** — foco em Windows; funcionar onde o winget não existe (Server, LTSC, sandbox) é desejável, não obrigatório. Como tudo é HTTP + SQLite próprio, a fase 1 não depende do winget instalado.

### Decomposição do projeto

O projeto completo é grande demais para uma única spec. Fases:

- **Fase 1 (esta spec):** somente leitura — `update`, `search`, `show`, `list` (incluindo atualizações disponíveis), para winget + scoop, com TUI e CLI.
- **Fase 1.5:** overlay do GitHub para reduzir o atraso do índice (ver §10).
- **Fase 2:** `install` (MSI, MSIX, Inno, NSIS, portable/zip do scoop).
- **Fase 3:** `upgrade`, `uninstall`, estado próprio de pacotes instalados.
- **Fora de escopo por ora:** Chocolatey (ver §11), fonte msstore.

### Fora do escopo da fase 1

Instalar, atualizar ou desinstalar qualquer coisa; overlay do GitHub; Chocolatey; msstore; mirror git próprio do winget-pkgs; delta por blocos do MSIX.

## 2. Fatos verificados que embasam o design

Todos medidos em 2026-10-07 com requisições reais (detalhes no Apêndice A).

- `https://cdn.winget.microsoft.com/cache/source2.msix`: 3,7 MB (ZIP) com `Public/index.db` (SQLite 8,6 MB, 15.402 pacotes). `Cache-Control: max-age=900`, `ETag`, `Last-Modified`, `Accept-Ranges`, header `x-ms-meta-SourceVersion` (ex.: `2026.1007.459.0`). `If-None-Match` → 304 em ~60–200 ms. Download completo ~0,4 s.
- Republicação a cada ~70–90 min. Atraso merge→CDN: mín. 48 min, mediana 112 min, máx. 179 min.
- Índice v2 (`metadata.majorVersion=2`): tabela `packages(rowid, id, name, moniker, latest_version, arp_min_version, arp_max_version, hash BLOB)` + tabelas de mapeamento `tags2/tags2_map`, `commands2/commands2_map`, `pfns2`, `productcodes2`, `upgradecodes2`, `norm_names2`, `norm_publishers2` (todas por `package` = rowid). Não contém publisher nem descrição.
- Versões por pacote: `cache/packages/<Id>/<8 primeiros hex de lower(hex(hash))>/versionData.mszyml`. Formato Windows Compression API (MSZIP): header de 24 bytes (tamanho descomprimido u64 no offset 8), depois blocos `u32 tamanho` + `"CK"` + deflate cru, cada bloco usando o anterior como dicionário. Conteúdo YAML: `sV: 1.0`, `vD: [{v, rP, s256H}]`.
- Manifesto: `cache/<rP>` (ex.: `manifests/m/Microsoft/PowerToys/0.101.2362.0/509b`) — YAML único com `ManifestType: merged`; SHA-256 do arquivo = `s256H`.
- `AppxBlockMap.xml` contém SHA-256 por bloco de 64 KB de `Public/index.db` e um FileHash; recalculados e conferidos.
- Delta por blocos não compensa: entre duas publicações com 13 pacotes alterados, 84 de 132 blocos mudaram (2,7 de 3,7 MB). `delta.msix` retorna 404 (spec existe no winget-cli, cliente não implementado).
- Scoop: tarball `https://codeload.github.com/ScoopInstaller/<Bucket>/tar.gz/refs/heads/master` — Main 507 KB (1.669 manifestos), Extras 841 KB (2.405), Versions 215 KB (623); ~2 s cada; `ETag` forte; `If-None-Match` → 304 sem consumir rate limit da API do GitHub. Buckets mudam várias vezes por hora.
- Baseline nesta máquina: `winget search powertoys` 0,9 s; `winget list` 4,6 s.

## 3. Arquitetura

Um único crate binário `wingetrs`, organizado em módulos com fronteiras claras.

```
src/
  main.rs            clap: sem subcomando → TUI; update | search | show | list
  core/              SourceKind, PackageEntry, PackageDetails, Version
  sources/
    mod.rs           trait Source
    winget/
      index.rs       baixa source2.msix, valida BlockMap, importa index.db
      mszip.rs       decodificador MSZIP
      version_data.rs
      manifest.rs    YAML merged → structs
    scoop/
      bucket.rs      tarball condicional, extração em streaming
      manifest.rs    JSON → structs
  catalog/           catalog.db (rusqlite, WAL) + carga em memória + busca fuzzy
  installed/
    arp.rs           registro Uninstall (HKLM, HKLM\WOW6432Node, HKCU)
    appx.rs          PackageManager.FindPackagesForUser
    scoop.rs         ~/scoop/apps
    normalize.rs     port do NameNormalizer do winget-cli
    correlate.rs     instalado ↔ catálogo
  cache/             diretório de dados, metadados HTTP, lock, blobs
  http.rs            cliente reqwest (rustls), GET condicional, downloads paralelos
  tui/
    app.rs           estado + enum Msg + update()
    view/            funções puras de renderização
    theme.rs         tokens semânticos de cor e conjuntos de glifos
    event.rs         loop tokio::select!
```

### 3.1 Trait `Source`

`async fn` nativo em trait (Rust ≥ 1.75); despacho estático por `enum AnySource { Winget(..), Scoop(..) }`, sem `dyn` nem crate `async-trait`.

```rust
trait Source {
    fn kind(&self) -> SourceKind;
    /// Busca dados novos (condicional). Retorna se algo mudou.
    async fn refresh(&self, ctx: &RefreshCtx) -> Result<RefreshOutcome>;
    /// Importa o dado baixado para o catalog.db numa transação.
    fn import(&self, tx: &rusqlite::Transaction) -> Result<ImportStats>;
    /// Detalhes completos de um pacote (pode usar rede + cache de blobs).
    async fn details(&self, id: &str) -> Result<PackageDetails>;
}
```

O overlay do GitHub (fase 1.5) entra como um `Source` adicional que sobrepõe `latest_version` de pacotes winget.

### 3.2 Dependências

| Crate | Uso |
|---|---|
| ratatui 0.30, crossterm 0.29 | TUI |
| tokio | runtime assíncrono |
| reqwest 0.13 (rustls) | HTTP |
| rusqlite 0.40 (`bundled`) | catalog.db e leitura do index.db |
| zip 8 | extrair source2.msix |
| flate2 + tar | tarballs do scoop; deflate cru do MSZIP |
| serde, serde_json, serde-saphyr | manifestos (serde_yaml está descontinuado) |
| sha2, base64 | integridade |
| winreg, windows 0.62 | registro e PackageManager |
| nucleo | busca fuzzy |
| clap | CLI |
| thiserror, anyhow, tracing | erros e logs |
| insta, wiremock (dev) | testes |

O crate `winget-types` (projeto Komac) é referência de modelagem, não dependência.

> **Emenda (2026-10-13, ícones; dependências aprovadas pelo usuário).** `ratatui-image` 11.1 (protocolos de imagem do terminal) e `image` (só as features `png`, `ico`, `jpeg`, `gif`). A compatibilidade com ratatui 0.30.2 e crossterm 0.29 foi provada na sonda da Task 82. O executável fica cerca de 1,14 MB maior, e o crate `windows` 0.58 entra duplicado ao lado da 0.62. A consulta automática ao terminal (`Picker::from_query_stdio`) não é usada: ela cria uma thread que nunca é juntada e trava com stdin em EOF.

## 4. Dados locais

Diretório: `%LOCALAPPDATA%\wingetrs\`

- `catalog.db` — SQLite em modo WAL.
- `http.json` — por URL: `etag`, `last_modified`, `checked_at`.
- `blobs\<sha256 hex>` — versionData e manifests (conteúdo imutável, endereçado por hash).
- `update.lock` — lock exclusivo de arquivo durante `update`.
- `log\` — logs do tracing.
- `config.toml` — buckets do scoop, glifos, tema.

### 4.1 Schema do catalog.db

```sql
CREATE TABLE meta (
  source TEXT PRIMARY KEY,         -- 'winget' | 'scoop:main' | 'scoop:extras' ...
  source_version TEXT,             -- x-ms-meta-SourceVersion ou ETag do tarball
  fetched_at INTEGER NOT NULL      -- unix ts
);
CREATE TABLE packages (
  rowid INTEGER PRIMARY KEY,
  source TEXT NOT NULL,
  id TEXT NOT NULL,
  name TEXT NOT NULL,
  moniker TEXT,
  version TEXT NOT NULL,
  publisher TEXT,                  -- NULL para winget (não vem no índice)
  description TEXT,                -- NULL para winget
  tags TEXT,                       -- separadas por espaço
  commands TEXT,
  arp_min_version TEXT,
  arp_max_version TEXT,
  winget_hash BLOB,                -- para montar a URL do versionData
  raw TEXT,                        -- JSON completo (scoop)
  UNIQUE (source, id)
);
-- mapeamentos para correlação (somente winget)
CREATE TABLE map_productcode  (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE map_upgradecode  (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE map_pfn          (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE map_norm_name    (key TEXT NOT NULL, package INTEGER NOT NULL);
CREATE TABLE map_norm_publisher (key TEXT NOT NULL, package INTEGER NOT NULL);
-- índices em key para cada map_*
```

Chaves de ProductCode/UpgradeCode/PFN são gravadas em minúsculas; a comparação é case-insensitive.

## 5. Fluxos

### 5.1 `update`

1. Adquire `update.lock`; se ocupado, a CLI espera até 30 s e sai com mensagem; a TUI apenas mostra "atualização em andamento".
2. Em paralelo, por fonte:
   - **winget:** GET de `source2.msix` com `If-None-Match`/`If-Modified-Since`. 304 → só atualiza `checked_at`. 200 → grava em arquivo temporário, extrai `Public/index.db`, valida cada bloco e o FileHash contra `AppxBlockMap.xml`, verifica `metadata.majorVersion == 2`.
   - **scoop:** para cada bucket configurado (padrão `main`, `extras`), GET condicional do tarball no codeload. 200 → descompacta em streaming, parseia cada `bucket/*.json`; manifestos inválidos são contados e ignorados.
3. Importação: cada fonte alterada é substituída numa transação própria (`DELETE ... WHERE source = ?` + `INSERT`s). Com WAL, leitores continuam vendo o estado anterior até o commit.
4. Falha numa fonte não afeta as outras nem o dado antigo dela.
5. Resultado: por fonte, `inalterado | atualizado (N pacotes) | erro (motivo)`.

Refresh automático: ao abrir a TUI ou rodar um comando de leitura, se `checked_at` de alguma fonte tiver mais de 15 min, dispara `update` em segundo plano (na CLI: não bloqueia; o resultado vale para a próxima execução). Sem nenhum cache, o comando espera o primeiro `update`.

### 5.2 `search`

- O catálogo é carregado em memória: `(source, id, name, moniker, tags, commands, version)` de todas as fontes (~20 mil entradas).
- Busca fuzzy com `nucleo` sobre `name`, `id`, `moniker`; tags e commands entram com peso menor. Match exato de id ou moniker vai para o topo.
- CLI: `wingetrs search <termo> [--source winget|scoop] [--json] [-n N]`.

### 5.3 `show`

- **winget:**
  1. Calcula a URL do versionData a partir de `id` e `winget_hash`.
  2. Busca-o em `blobs/` ou na rede e decodifica o MSZIP.
  3. Escolhe a versão: a mais recente, ou `--version` se informado.
  4. Baixa `cache/<rP>`, confere `s256H`, grava em `blobs/` e parseia.
- **scoop:** usa `raw` do catálogo, sem rede.
- CLI: `wingetrs show <id> [--source ...] [--version V] [--json]`.

### 5.4 `list`

Coleta em paralelo (threads):

- **ARP:** subchaves de `Uninstall` em HKLM (64 bits), HKLM\WOW6432Node e HKCU. Campos: nome da subchave, `DisplayName`, `Publisher`, `DisplayVersion`, `WindowsInstaller`, `SystemComponent`, `ParentKeyName`, `ReleaseType`. Ignora entradas sem `DisplayName`, com `SystemComponent=1`, com `ParentKeyName` e as de `ReleaseType` de atualização/hotfix.
- **Appx/MSIX:** `PackageManager.FindPackagesForUser("")`; ignora frameworks, resource packages e pacotes de sistema (`SignatureKind::System`).
- **Scoop:** `%SCOOP%` ou `~/scoop`, `apps/<app>/current/manifest.json` + `install.json` (bucket, arquitetura).

Correlação, na ordem; a primeira regra que acerta decide:

1. Nome da subchave ARP (ProductCode) em `map_productcode`.
2. Para entradas MSI: UpgradeCode obtido em `HKLM\SOFTWARE\Classes\Installer\UpgradeCodes` (GUIDs comprimidos) em `map_upgradecode`.
3. PackageFamilyName em `map_pfn`.
4. `normalize(DisplayName)` em `map_norm_name` ∩ `normalize(Publisher)` em `map_norm_publisher`.
5. Scoop: `(bucket, app)` direto em `packages`.

Mais de um candidato após a regra que acertou → item marcado como **ambíguo**, com todos os candidatos; nunca se escolhe em silêncio. Sem candidatos → **não correlacionado** (listado à parte).

Atualização disponível:

- Compara-se `DisplayVersion` instalada com `packages.version` usando o algoritmo de versão do winget (§6.2).
- Se `arp_min_version`/`arp_max_version` existirem, a versão instalada é comparada com `arp_max_version` (instalada ≥ arp_max → atualizado).
- Versão instalada vazia ou não comparável → estado `desconhecido`; nunca aparece como atualizável.

CLI: `wingetrs list [--upgradable] [--source ...] [--json]`.

## 6. Algoritmos a portar do winget-cli

### 6.1 NameNormalizer

Fonte: `microsoft/winget-cli`, `src/AppInstallerCommonCore/NameNormalization.cpp` e testes correspondentes. Deve produzir exatamente as chaves presentes em `norm_names2`/`norm_publishers2` (ex.: `mozillafirefox(X64)`, `7zip(X64)`, `igorpavlov`). Porta fiel, incluindo remoção de versão, arquitetura, locale, sufixos legais e caracteres não alfanuméricos. Casos de teste portados dos testes do winget-cli.

### 6.2 Comparação de versões

Fonte: `Utility::Version` no winget-cli (`Versions.cpp`). Partes separadas por `.`, cada parte com prefixo numérico e sufixo textual. Inclui tratamento de prefixo `v`, zeros à direita e a versão especial `Unknown`. Casos de teste portados do winget-cli.

## 7. TUI

### 7.1 Layout

```
╭─  wingetrs ──  Buscar │  Instalados │ 󰚰 Atualizações 7 ─── 󰑓 12 min ─╮
│  powert▏                                                                 │
├────────────────────────────────────────┬───────────────────────────────────┤
│ ▶  PowerToys        0.100.1 → 0.101.2 │ PowerToys                         │
│    EverythingPT     0.91.1            │  Microsoft.PowerToys             │
│   󰮧 powertoys        0.101.2           │  Microsoft Corporation ·  MIT  │
│    CmdPal Bookmarks 0.2.3             │ Utilitários para power users...   │
╰────────────────────────────────────────┴───────────────────────────────────╯
 4 de 20.071 · ↑↓ mover · Tab aba · Enter detalhes · r refresh · ? ajuda
```

Abas:

- **Buscar:** catálogo inteiro.
- **Instalados:** resultado do `list`; seções para correlacionados, ambíguos e não correlacionados.
- **Atualizações:** só os atualizáveis; o contador aparece no título da aba.

> **Emenda (2026-10-11, redesenho no estilo Bubble Tea, aprovada pelo usuário).** O layout acima é substituído por:
> - Abas encaixadas no topo (caixas arredondadas; a ativa abre para a janela de conteúdo; badge ` wingetrs ` à direita; modo compacto de uma linha em terminal estreito). Quarta aba **Ajustes** (fase 2a §10).
> - Uma janela única sob as abas: linha da busca, uma linha em branco, lista | divisor | detalhes, com escala de espaçamento num lugar só (`src/tui/layout.rs`: recuo de 1 coluna dentro das caixas, 1 coluna de cada lado do divisor, diálogos com 2×1 de recuo).
> - Itens da lista em duas linhas (nome + versão; id · descrição), seleção por barra vertical `┃` à esquerda e cor rosa, sem fundo.
> - Rodapé com barra de status em "pepitas" (mensagem, atualizando, idade do índice) e, com altura ≥ 20, barra de ajuda contextual por aba (`tecla desc • …`, cortada em dicas inteiras com `…`).
> - Toasts no canto superior direito para **eventos** (sucesso 3 s, info 4 s, aviso 6 s, erro fixo até `Esc`; no máximo 3; repetidos somam `×N`); a barra de status mostra só **estado** atual. Terminal pequeno: o toast vai para a pepita da barra.

### 7.2 Teclas

| Tecla | Ação |
|---|---|
| texto | filtra a lista atual |
| `↑↓` / `j k` (fora do campo de busca) | mover |
| `PgUp PgDn Home End` | paginar |
| `Tab` / `Shift+Tab` | trocar aba |
| `Enter` | focar painel de detalhes |
| `Esc` | voltar foco / limpar busca / sair |
| `s` | alternar fonte: todas → winget → scoop |
| `r` | forçar refresh |
| `?` | ajuda |
| `q` / `Ctrl+C` | sair |
| `i` | instalar — exibido desabilitado ("fase 2") |

> **Emenda (2026-10-11).** `←` / `→` também trocam de aba (anterior/próxima, circular, incluindo Ajustes), em qualquer aba e com foco na lista ou nos detalhes; com um diálogo aberto as setas são do diálogo. Na aba Ajustes, `Enter`/Espaço avançam o valor do ajuste selecionado (as setas laterais continuam trocando de aba).

> **Emenda (2026-10-13, ação principal e ordenação, aprovada pelo usuário).** `Enter` num item faz a ação principal da aba: em Pesquisar, instala (como `i`); em Atualizações, atualiza (como `U`); em Instalados, abre um menu de ações do item (Atualizar, Remover; as opções que não se aplicam ficam desabilitadas, com o motivo). A confirmação segue o ajuste "Confirmar antes de aplicar". `Enter` não muda mais o foco para os detalhes; o foco nos detalhes deixa de existir. Lista vazia: `Enter` não faz nada (nunca dispara "atualizar tudo"). `o` alterna a ordem da lista em Instalados (nome / fonte / estado) e em Atualizações (nome / fonte); a ordem padrão é nome, vale por aba durante a sessão e aparece na barra de ajuda. Atualizações continua mostrando só os itens com atualização pendente. Pesquisar mantém a ordem por pontuação. Nos detalhes, com glifos `nerd`, o glifo e o texto ficam separados por 2 espaços (os glifos Nerd Font podem ocupar 2 colunas).

> **Emenda (2026-10-13, lote B: ícones dos itens visíveis, aprovada pelo usuário).** Substitui "A lista nunca baixa nada para mostrar um ícone" (§7.3, emenda dos ícones). Com o ajuste "Ícones da lista" em `visíveis` (padrão), a lista baixa o ícone dos itens do winget **visíveis na tela** que ainda não têm ícone no cache: 300 ms depois da última mudança da tela (rolar, trocar de aba, buscar, filtrar), no máximo 4 pacotes ao mesmo tempo, e um pedido que saiu da tela antes de começar é descartado. Cada pacote passa pelo mesmo caminho dos detalhes (versionData e manifesto do cache de blobs, depois o ícone verificado pelo SHA-256), sem mostrar os detalhes. Pacote sem ícone ou que falhou não é tentado de novo na mesma sessão. Com "Ícones da lista" em `só ao abrir`, vale a regra antiga (ícone só depois de abrir os detalhes). Com "Ícones" em `Desligado`, nada é baixado. Gravado em `config.toml` como `list_icons = "visible" | "opened"`.

> **Emenda (2026-10-14, lote D: fila, marcação e fixar versão, aprovada pelo usuário no roteiro).**
> - **Fila de operações.** Executar uma operação não abre mais um diálogo que trava a tela. A operação em andamento aparece na barra de status com o progresso, e a TUI continua navegável. Uma ação confirmada (instalar, atualizar, remover, um ou vários itens) entra numa fila e roda depois da atual, uma por vez, na ordem em que entrou. `f` abre a lista da fila: em andamento e pendentes. `Delete` (ou `x`) tira um pendente; a operação em andamento não é cancelada. Enquanto um diálogo de resíduos espera resposta, a fila pausa. A confirmação, quando o ajuste pede, acontece ao enfileirar, com o plano calculado naquele momento. Plano que ficou velho até a hora de rodar falha com erro em toast, sem afetar os seguintes.
> - **Marcação.** Nas abas de lista, `Espaço` marca/desmarca o item selecionado (glifo de marca na linha). Com itens marcados, a ação principal (`Enter`, `i`, `u`, `U` e as opções do menu de ações) vale para **todos os marcados**: um plano conjunto, uma confirmação, uma entrada na fila. Itens marcados que não se aplicam à ação (ex.: remover um app do winget) são pulados e listados no toast. A marcação é por aba e se perde ao trocar a lista (busca nova, novo scan não a perde: ela segue por identidade). `Esc` limpa as marcas antes de limpar a busca.
> - **Fixar versão.** O menu de ações de Instalados ganha "Fixar versão"/"Desafixar". Fixados ficam em `%LOCALAPPDATA%\wingetrs\pins.json` (chave = origem + nome do app na lista; vale para qualquer origem, porque para apps de fora do wingetrs só afeta o que o wingetrs mostra e faz). Um app fixado aparece com glifo de alfinete, continua listado em Atualizações, mas "atualizar" nele (`Enter`, `U`, menu) mostra "versão fixada" e não faz nada; "atualizar tudo" e marcados pulam fixados. Na CLI, `upgrade --all` pula fixados e avisa; `upgrade <app>` de um fixado recusa com a mensagem para desafixar na TUI. Não há comando novo de CLI.

> **Roteiro aprovado (2026-10-13).** Lotes, nesta ordem: A (esta emenda); B (ícones dos itens visíveis da lista, baixados com espera, teto de concorrência e ajuste "Ícones da lista"); C (mouse: abas, seleção, roda, clique duplo = `Enter`; ajuste "Mouse"); D (marcação com Espaço, fila de operações visível, fixar versão no estado do wingetrs); E (escolher o candidato de um pacote ambíguo, gravado em `correlations.json`; ícone de pacote do Scoop: primeiro pelo pacote do winget com mesmo domínio de homepage e nome normalizado, só com candidato único; reserva pela homepage do app — `apple-touch-icon`, depois `link rel=icon` ≥ 64 px, por último `og:image` só se quase quadrada; github.com fica de fora; limites de 1 MiB e 1024 px; reserva atrás de ajuste, ligado por padrão. O lote E começa com uma sonda nos buckets `main` e `extras` (amostra fixa) que mede cobertura e tempo mediano de cada caminho; se a homepage ganhar com folga, a ordem se inverte); F (Microsoft Store com instalação direta, precedido de sonda e spec própria). Popularidade/downloads mensais: descartado (nenhuma fonte tem esse dado).

### 7.3 Estilo visual

Todos os estilos vêm de `tui/theme.rs`; views não usam cores literais.

- **Tipografia:**
  - Negrito: nome do pacote, títulos de painel, versão nova.
  - Itálico: descrição, notas, "versão desconhecida", dicas da barra de status.
  - Dim: id, publisher, versão antiga.
  - Caracteres que casaram no fuzzy: sublinhados na cor `accent`.
- **Tokens de cor:**

  | Token | Cor | Uso |
  |---|---|---|
  | `accent` | azul/ciano | foco, seleção, match |
  | `ok` | verde | instalado e atualizado |
  | `warn` | amarelo | atualização disponível |
  | `muted` | cinza | versão desconhecida |
  | `alert` | magenta | correlação ambígua |
  | `err` | vermelho | falha de fonte |
  | `src_winget` | azul | badge da fonte winget |
  | `src_scoop` | laranja | badge da fonte scoop |

  Na linha de atualização, `antiga → nova`: a antiga em dim, a nova em `ok` e negrito.
- **Profundidade de cor:** truecolor quando `COLORTERM` é `truecolor`/`24bit`; caso contrário, 256 ou 16 cores. Tema escuro padrão, claro via config. `NO_COLOR` desativa cores, mantendo negrito e itálico.
- **Bordas:** arredondadas; o painel em foco tem borda `accent`; a linha selecionada tem fundo sutil e `▶`. Spinner braille durante refresh.
- **Glifos:** três conjuntos, escolhidos por `config.toml` ou `--glyphs`:
  - `nerd` (padrão): ícones Nerd Font.
  - `unicode`: `✓ ↑ ? ⚠ ⟳ ●`.
  - `ascii`.

> **Emenda (2026-10-11).** Paleta do Charm (Lip Gloss/Bubbles): `accent` roxo `#7D56F4` (escuro) / `#874BFD` (claro) no lugar de azul/ciano; tokens novos `sel_bar`, `sel_title`, `sel_desc` (seleção rosa), `badge`, `help_key`/`help_desc`/`help_sep` (texto de ajuda clareado para `#6C6C6C`), `pill_msg`/`pill_a`/`pill_b` (barra de status) e `spinner`; spinner `Dot` do Bubbles (`⣾⣽⣻⢿⡿⣟⣯⣷`) nos glifos `nerd`/`unicode`. A linha selecionada deixa de ter fundo e `▶` (barra `┃`; ver §7.1). `NO_COLOR`: pepitas e badge em vídeo reverso.

> **Emenda (2026-10-13, ícones dos pacotes, aprovada pelo usuário).** Os pacotes do winget cujo manifesto traz `Icons` (com `IconSha256`) mostram o ícone na TUI.
>
> - **Detalhes:** o ícone fica no canto superior esquerdo do painel, com 4 linhas × 8 colunas. Nome, versão e publisher ficam à direita dele, separados por 2 colunas em branco; o ícone nunca encosta no texto. O resto do texto continua abaixo.
> - **Lista:** só aparecem os ícones que já estão no cache. A lista nunca baixa nada para mostrar um ícone. Cada ícone ocupa 2 linhas × 4 colunas à esquerda do item, seguido de 1 coluna em branco. Quando os ícones estão ligados, itens sem ícone recebem o mesmo recuo, para o texto ficar alinhado.
> - **Download e cache:** o ícone é baixado junto com os detalhes, com no máximo 1 MiB. Ele é conferido pelo `IconSha256` e guardado em `blobs/`, sujeito à mesma limpeza de 30 dias. Ícone sem `IconSha256`, com hash divergente ou que não decodifica fica sem ícone, nunca dá erro.
> - **Qual ícone:** tema `default`, depois `dark`, depois qualquer outro.
> - **Protocolo:** vem do ajuste "Ícones" (config `icons`), com os valores `auto` (padrão), `iterm2`, `sixel` e `off`.
>   - `auto` escolhe iTerm2 no WezTerm (`TERM_PROGRAM=WezTerm`; foi o que gastou menos recursos na sonda) e sixel no Windows Terminal (`WT_SESSION`). Em qualquer outro terminal, `auto` desliga os ícones.
>   - O protocolo nunca é descoberto consultando o terminal.
>   - O tamanho da fonte em pixels vem de `crossterm::terminal::window_size()`; se ele não informar, usa-se 10×20.
> - **Sobreposições:** com um diálogo ou a ajuda abertos, nenhum ícone é desenhado. Um ícone que cruzaria um toast também não é desenhado.
> - **CLI:** o `show` não muda.

### 7.4 Modelo de eventos

Arquitetura estilo Elm: `App` (estado), `enum Msg`, `update(&mut App, Msg) -> Vec<Cmd>`, `view(&App, &mut Frame)`. `Cmd` são efeitos (refresh, carregar detalhes, varrer instalados) executados como tasks tokio que devolvem `Msg` por canal.

Loop único com `tokio::select!` sobre:

- `crossterm::event::EventStream`;
- canal de `Msg` das tasks;
- tick de 100 ms, ativo só com spinner.

Redesenha somente quando o estado muda. Detalhes do item selecionado são pré-carregados com debounce de 150 ms. Terminal restaurado via `ratatui::init()`/`ratatui::restore()` com hook de pânico.

## 8. Erros

`thiserror` por módulo; `anyhow` apenas em `main`. Dado externo nunca causa pânico.

| Situação | Comportamento |
|---|---|
| Offline/timeout | usa cache; status ` offline · índice de 3 h`; erro só sem cache algum |
| BlockMap/FileHash/`s256H` divergente | descarta download, mantém dado anterior, erro de integridade |
| `majorVersion` ≠ 2 | não importa; mensagem "formato de índice vN não suportado"; mantém anterior |
| Manifesto scoop inválido | ignorado e contado ("3 manifestos ignorados") |
| Acesso negado no registro | chave ignorada, `list` continua |
| `update` concorrente | lock de arquivo (§5.1) |

Logs: `tracing` para arquivo em `log\`; `-v`/`-vv` também no stderr na CLI.

## 9. Testes e critérios de sucesso

### 9.1 Testes

- **Unitários com fixtures reais** (salvos em `tests/fixtures/`):
  - decodificador MSZIP (versionData do PowerToys);
  - parser do manifest merged (PowerToys 0.101.2362.0);
  - manifestos scoop (com `architecture`, URL com `#/rename`, sem hash);
  - comparação de versões e NameNormalizer, com casos portados do winget-cli.
- **Integração:**
  - importação de um `index.db` reduzido (~20 pacotes extraídos do real, com mapeamentos);
  - HTTP com `wiremock`: 200, 304, corpo corrompido, timeout;
  - correlação com entradas ARP sintéticas.
- **TUI:** `ratatui::backend::TestBackend` + snapshots `insta` por tela e estado (vazio, carregando, offline, com atualizações, ambíguo).
- **Ao vivo** (`#[ignore]`, manuais):
  - CDN real;
  - oráculo `winget list` comparado com `wingetrs list --json`, listando divergências.

### 9.2 Critérios de sucesso

Medidos com `hyperfine` nesta máquina, comparando com o winget:

| Operação | Meta | winget |
|---|---|---|
| `search` com cache quente | < 100 ms | 0,9 s |
| `list` | < 500 ms | 4,6 s |
| primeiro frame da TUI com cache | < 50 ms | — |
| `update` sem mudanças (304) | < 300 ms | — |

Correlação: ≥ 90% dos pacotes que o `winget list` identifica nesta máquina também são correlacionados pelo wingetrs.

## 10. Evolução prevista: overlay do GitHub (fase 1.5)

Não implementado na fase 1; a arquitetura apenas o permite.

1. Usar `metadata.updateTrackingBase` do índice como corte.
2. Com token do GitHub (304 autenticado não consome rate limit), buscar `GET /repos/microsoft/winget-pkgs/commits?since=<corte>` e os arquivos de cada commit.
3. Baixar os YAML por `raw.githubusercontent.com/microsoft/winget-pkgs/<sha>/<path>`, que é imutável por SHA.
4. Sobrepor `latest_version` até o próximo `source2.msix`.

Isso reduz o atraso de ~2 h para minutos. Não usar a compare API (limite de 300 arquivos) nem `git/trees?recursive` (trunca).

## 11. Decisões descartadas

- **Delegar à API COM do winget:** contraria o objetivo de aprendizado e a portabilidade.
- **Mirror git próprio do winget-pkgs:**
  - custo: clone blobless de 40 MB/22 s, fetch de blobs lento (103 s para 3,2 mil), reimplementar o pipeline de índice;
  - adiado.
- **Delta por blocos do MSIX:** o SQLite muda a maior parte dos blocos a cada publicação.
- **Mirrors de terceiros** (winget.run, svrooij index, jsDelivr `@master`): congelados ou mais atrasados que o CDN oficial.
- **Índice v1 (`source.msix`, 21 MB):** maior e sem ganho para a fase 1.
- **Chocolatey:**
  - sem dump nem delta; OData pagina de 40 em 40 com teto de 10 mil;
  - enumeração completa leva ~5–6 min;
  - política oficial suporta apenas `choco.exe`;
  - instalação exige executar scripts PowerShell com os helpers do Chocolatey;
  - se entrar no futuro: busca sob demanda e instalação delegada ao `choco.exe`.

## Apêndice A — Endpoints verificados (2026-10-07)

| Endpoint | Resultado |
|---|---|
| `cdn.winget.microsoft.com/cache/source2.msix` | 200, 3.736.383 B, `max-age=900`, ETag, 304 condicional, Range 206 |
| `cdn.winget.microsoft.com/cache/source.msix` | 200, 21.354.042 B (v1) |
| `cdn.winget.microsoft.com/cache/delta.msix` | 404 |
| `cdn.winget.microsoft.com/cache/packages/Microsoft.PowerToys/d193b67a/versionData.mszyml` | 200, 5.461 B; com hash completo no path → 404 |
| `cdn.winget.microsoft.com/cache/manifests/m/Microsoft/PowerToys/0.101.2362.0/509b` | 200, 12.182 B, SHA-256 = `s256H` |
| `winget.azureedge.net` | DNS não resolve |
| `codeload.github.com/ScoopInstaller/{Main,Extras,Versions}/tar.gz/refs/heads/master` | 200, 507 KB / 841 KB / 215 KB, ETag, 304 condicional |
| `codeload.github.com/microsoft/winget-pkgs/tar.gz/refs/heads/master` | 200, ~128 MB, ~24 s |
| `api.github.com` sem token | 60 req/h; 304 condicional consome o limite |
| `community.chocolatey.org/api/v2` | OData, páginas de 40, `$skip` > 10.000 → 406, sem v3 |
| `api.winget.run` | ativo, mas dados congelados em 2023 |
