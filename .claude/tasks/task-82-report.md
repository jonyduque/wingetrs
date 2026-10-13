# Task 82 — Sonda de ícones (ratatui-image) — relatório

Status: DONE_WITH_CONCERNS

Commits:
- só este relatório: `chore(tasks): icon probe report for task 82` (SHA na mensagem ao orquestrador; base 60529be).

Gate: não executado. A tarefa não altera código do repositório (só este relatório), e a árvore está compartilhada com as Tasks 80/81 em andamento, então um `verify.ps1` aqui mediria o trabalho delas, não o meu.

Mutation proofs: no mutation proof. Não há código no repositório. As três checagens são observacionais e as provas estão coladas abaixo (git status, saídas de origem, sha256).

Decisions:
- Usei `ratatui-image` **11.1.0** (estável mais recente), não a 12.0.0-rc.2. As duas declaram `ratatui ^0.30.1`. Usei `default-features = false, features = ["crossterm"]`, porque o padrão traz `chafa-dyn`, que pede `pkg-config` e libchafa.
- O Cargo.lock do projeto foi copiado para a sonda, para que ratatui e crossterm resolvam iguais (0.30.2 / 0.29.0).
- Mapeei id → manifesto pelos blobs YAML de `blobs/` (campo `PackageIdentifier`). Contei os `Icons` do topo e os de `Localization[*]`.
- Acrescentei o modo `--smoke` ao exe de demonstração: sem terminal, saída para `io::sink`, viewport fixo de 100x40. Assim o agente mede o custo de CPU e os bytes por quadro. O custo de o terminal desenhar segue não medido.
- O ícone do Notepad++ foi baixado à parte, porque o Notepad++ não caiu na amostra de 200.

## (a) Versões e compatibilidade

```
crates.io ratatui-image: 12.0.0-rc.2 (2026-10-05), 12.0.0-rc.1, 12.0.0-rc.0, 11.1.0 (2026-09-17), 11.0.8 ...
11.1.0 deps: ratatui ^0.30.1, image ^0.25.6 ['png'], icy_sixel ^0.5.0, windows ^0.58.0, rustix ^0.38.4, rand ^0.8.5, thiserror ^1.0.59, flate2 ^1.0, base64-simd ^0.8, self_cell ^1.2.2
Cargo.lock resolvido: ratatui 0.30.2, crossterm 0.29.0, ratatui-image 11.1.0, image 0.25.10, icy_sixel 0.5.1, windows 0.58.0
```
A sonda compila sem nenhum warning (`cargo build --release` não imprimiu linhas `warning`/`error`).

Ela adiciona 61 crates em relação à base ratatui+crossterm (`cargo tree -e normal`: 64 → 125 linhas únicas). Entre eles há duplicatas do que o projeto já usa:
- **`windows 0.58`**: o projeto usa 0.62.2;
- `windows-targets 0.52`;
- `thiserror 1`;
- `rand 0.8` e `rand 0.10`;
- `miniz_oxide 0.8` e `0.9`.

Também entram `palette`, `quantette`, `wide`, `bitvec`, `moxcms`, `zune-jpeg`, `gif` e `png`.

## Tamanho do binário release (lto thin, codegen-units 1, strip, iguais ao projeto)

```
346112   icon-probe-base.exe   (só ratatui+crossterm)
1489920  icon-probe.exe sem o ícone embutido (asset vazio)
1677312  icon-probe.exe com o ícone do Notepad++ (186712 bytes) via include_bytes!
```
As dependências de imagem custam cerca de 1,14 MB: 1489920 − 346112 = 1143808 bytes.

## (d) Amostra de 200 ids (wingetrs da base 60529be, worktree próprio)

```
installed winget ids: 113; catalog winget ids: 15575; sample: 200 (50+150)
show runs: 200 in 590 s; failures: 2
  fail: ('GoLang.Go', 1, 'erro: sem conexão: error sending request ...versionData.mszyml)')
  fail: ('tingit.rc', 1, 'erro: tempo esgotado')
```
Repeti as duas falhas uma vez (falha de rede): `GoLang.Go exit=0`, `tingit.rc exit=0`, e nenhuma das duas tem Icons (`tingit.rc False None`, `GoLang.Go False None`). Com isso fica 50 de 200.

```
with Icons: 50 of 198 (installed-50: 16/49, random-150: 34/149)
more than one icon: 0; icons per package: {1: 50}
total icon entries: 50
IconFileType: {'ico': 50}
IconResolution: {'custom': 50}
IconTheme: {'default': 50}
IconUrl hosts: {'cdn.winget.microsoft.com': 50}
```
- 25% dos pacotes têm ícone: 50/200, sendo 16/50 entre os instalados e 34/150 entre os sorteados.
- Todos os ícones são `.ico`, com resolução `custom` e tema `default`, servidos pelo CDN da Microsoft (`/icons/<sha256>.ico`).
- Não apareceu nenhum pacote com mais de um ícone, nenhum png/jpeg/svg e nenhum tema claro/escuro.

## (e) Download, sha256 e decodificação

```
downloaded 30 of 30; sha256 conferem: 30; divergem: 0; sem hash: 0
tamanhos (bytes): min 2445 med 73812 max 370070 soma 2802702
Notepad++: sha256 7f00c181277bddfe06f51e7712c713e24e29a167b20033c43829136cdadc655d (= IconSha256 do manifesto), 186712 bytes
```
Os downloads tiveram intervalo de 250 ms entre requisições.

Saída de `decode.exe` (image 0.25.10, features png+ico+jpeg+gif):
- 31/31 decodificaram (`Some(Ico) ok`).
- O `image` escolhe a maior entrada do .ico. As dimensões finais foram:
  - 256x256 em 22 ícones;
  - 48x48 em 2;
  - 128x128 em 2;
  - 64x64, 103x104 e 512x512 em 1 cada;
  - 256x268 em 2, onde o PNG embutido tem altura diferente da que o diretório do .ico declara.
- A estrutura interna dos .ico variou bastante. 25/30 têm uma entrada PNG de 256 px; os outros só têm BMP de 32 bpp, 24 bpp ou até 4 bpp (00.ico: `32x32b4 16x16b4 48x48b8`).

Tempo de decodificação por arquivo, máquina carregada por outros agentes (12 `rustc` vivos):
```
n=31 min=478us med=1401us max=50636us
n=31 min=416us med=1233us max=30428us
```
Nenhuma falha de decodificação. Não sei se o `image` decodifica svg: não apareceu nenhum na amostra (não medido).

## (b, c) Demonstração e custos

O exe fica em `C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\icon-probe\target\release\icon-probe.exe`. Comando para o usuário colar:

```
! & "C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\icon-probe\target\release\icon-probe.exe"
```
- Teclas: ↑/↓ rolam, `q` sai.
- Na saída, o exe imprime os números e grava `...\scratchpad\icon-probe\last-run.txt`.
- Variáveis de ambiente para forçar a escolha: `ICON_PROBE_PROTOCOL=sixel|halfblocks|kitty|iterm2` e `ICON_PROBE_FONT=WxH`.
- Variante forçando sixel: `$env:ICON_PROBE_PROTOCOL='sixel'; & "<exe>"`.

**(b) Se o terminal do usuário tem sixel: não medido (exige terminal interativo).** O ambiente indica **WezTerm** (`TERM_PROGRAM=WezTerm`, `WEZTERM_EXECUTABLE=...`). O código do `Picker` 11.1.0 faz o seguinte nesse caso:
- com WezTerm ou Konsole, **põe Sixel e Kitty na lista negra**;
- `iterm2_from_env` devolve **Iterm2** para WezTerm;
- se o ConPTY não entregar a resposta da consulta, o Windows não tem fonte de fallback (`font_size_fallback() -> None`), e `fallback_picker` devolve **Halfblocks** com fonte 10x20, ignorando a dica de iTerm2.

O resultado esperado no WezTerm é "Iterm2" se a consulta responder, ou "Halfblocks" se não responder. Sixel só sai forçado por `ICON_PROBE_PROTOCOL=sixel`. É preciso confirmar com o `last-run.txt`.

**(c) Custo de CPU (modo `--smoke`).** Medido sem terminal, com fonte 10x20 e uma tela de 100x40. Cada linha da lista mostra uma de 3 variantes do ícone (original, invertida, cinza), então a rolagem muda o ícone em cada posição. Saída real, cortada:
```
== sixel
codificação 2 linhas (new_protocol) por variante: 1.38 ms, 1.25 ms, 1.33 ms
primeiro desenho (inclui redimensionar+codificar o ícone grande): 7.29 ms, 103011 bytes
segundo desenho sem mudança: 0.30 ms, 25 bytes
rolagem automática 200 quadros: mediana 0.29 ms, p95 0.49 ms, máx 1.21 ms, total 61.8 ms
bytes por quadro na rolagem: mediana 55814, máx 56717
== iterm2
codificação 2 linhas (new_protocol) por variante: 0.45 ms, 0.27 ms, 0.30 ms
primeiro desenho (inclui redimensionar+codificar o ícone grande): 3.12 ms, 89288 bytes
segundo desenho sem mudança: 0.32 ms, 25 bytes
rolagem automática 200 quadros: mediana 0.24 ms, p95 0.44 ms, máx 0.75 ms, total 53.9 ms
bytes por quadro na rolagem: mediana 36951, máx 36975
== halfblocks
codificação 2 linhas (new_protocol) por variante: 0.39 ms, 0.29 ms, 0.44 ms
primeiro desenho (inclui redimensionar+codificar o ícone grande): 3.39 ms, 10903 bytes
segundo desenho sem mudança: 0.24 ms, 25 bytes
rolagem automática 200 quadros: mediana 0.27 ms, p95 0.54 ms, máx 1.64 ms, total 62.5 ms
bytes por quadro na rolagem: mediana 3553, máx 3573
== kitty
codificação 2 linhas (new_protocol) por variante: 1.11 ms, 0.22 ms, 0.31 ms
primeiro desenho (inclui redimensionar+codificar o ícone grande): 3.12 ms, 168029 bytes
segundo desenho sem mudança: 0.29 ms, 172 bytes
rolagem automática 200 quadros: mediana 0.24 ms, p95 0.47 ms, máx 1.80 ms, total 61.5 ms
bytes por quadro na rolagem: mediana 1435, máx 1457
```
Decodificar o .ico de 256 px embutido custou entre 1,19 e 1,66 ms.

O custo de CPU do lado do app é pequeno: menos de 1,5 ms para codificar um ícone de 2 linhas, e menos de 0,5 ms por quadro depois de codificado. O custo real está nos **bytes enviados ao terminal**:
- sixel manda cerca de 5,6 KB por ícone de 4x2 células, ou cerca de 56 KB por quadro de rolagem com 10 ícones;
- iTerm2 manda cerca de 3,7 KB por ícone, ou cerca de 37 KB por quadro;
- Kitty, que só manda placeholders depois da transmissão, fica em cerca de 1,4 KB por quadro.

Quanto o terminal leva para decodificar e desenhar esses bytes, e se pisca, é **não medido**. Esses números saem do `last-run.txt` do usuário (primeiro desenho, mediana e máximo de 200 quadros com terminal real).

## Riscos vistos (no código 11.1.0 e nas medições)

1. **Thread órfã e travamento na consulta.**
   - `query_with_timeout` cria uma thread com `thread::spawn` e nunca chama `join`. Se der timeout, ela fica bloqueada em `stdin.read`, competindo com o leitor de eventos do crossterm, que pode perder teclas. Isso fere a regra ⚙ de threads do projeto.
   - Cada leitura, inclusive uma que devolve 0 em EOF, envia `Busy`, que **reinicia o timeout**. Com stdin em EOF o laço não termina. Medido: `icon-probe.exe < /dev/null` ficou travado até o `timeout 20` matar (exit 124). A saída parou em `ESC[?1049h ESC[16t ESC[5n`.
   - Mitigação na sonda: recusar quando stdin não é TTY (`IsTerminal`).
   - No wingetrs: consultar uma vez só, antes do laço de eventos e só com TTY, e aceitar que a thread vaze. Uma alternativa é não usar `from_query_stdio` e montar o `Picker` por configuração/env.
2. **Rolagem.** O protocolo põe a sequência inteira no símbolo da 1ª célula e marca as demais como `Skip`. O diff do ratatui reenvia a imagem sempre que esse símbolo muda de posição.
   - Medido: o 2º desenho sem mudança enviou só 25 bytes.
   - Numa lista que rola, toda linha cujo ícone muda é reenviada: cerca de 56 KB por quadro com sixel.
   - Se o sixel antigo some, sobra ou pisca ao rolar, só o teste do usuário dirá (não medido).
3. **Imagem sobre diálogo.** O sixel e o iTerm2 são pixels desenhados por cima, fora do buffer de células.
   - Um popup do ratatui sobre a lista sobrescreve as células, mas o terminal pode manter os pixels antigos. A doc do `Resize::Crop` cita o Alacritty, que não desenha texto sobre célula com sixel.
   - Pela doc do crate, é preciso não renderizar ícones sob o diálogo ou limpar a área. Não medido.
4. **Redimensionamento e fonte.**
   - `Protocol` (widget `Image`) é codificado para um tamanho fixo em células e não reage a resize.
   - `StatefulProtocol` reencodifica quando a área muda.
   - Se o usuário trocar o zoom da fonte, o `FontSize` do `Picker` fica velho e a imagem sai com a proporção errada, até reconsultar.
   - A demo conta os eventos de resize; o efeito visual não foi medido.
5. **Troca de tema.** Os ícones vêm com fundo transparente. O `Picker` aceita `set_background_color`, e trocar o tema exige recodificar todos os protocolos guardados. Todos os 50 ícones da amostra são `IconTheme: default`, então não há variante clara/escura para escolher.
6. **Dependências.** O crate traz `windows 0.58`, que duplica a 0.62.2 do projeto, mais `thiserror 1`, `rand 0.8` e `rustix` (este só fora do Windows). São cerca de 1,14 MB a mais no exe.

## Recomendação para as fases 2 e 3

- **Fonte dos ícones.** Só o winget tem ícone, em cerca de 25% dos pacotes, sempre `.ico` no `cdn.winget.microsoft.com`, endereçado por sha256. O cache pode ser `blobs/icons/<sha256>.ico`, verificado pelo `IconSha256` (30/30 conferiram), e baixado só ao abrir o detalhe ou ao entrar na viewport, nunca no `update`. Basta `image` com a feature `ico`, mais `png`, que vem junto, para os .ico com entrada PNG. Não incluir jpeg nem gif enquanto não houver um caso real.
- **Onde desenhar.**
  - Fase 2 (detalhe do pacote): um `StatefulImage::default().resize(Resize::Fit(None))` com um `StatefulProtocol` (`picker.new_resize_protocol(img)`) guardado no estado da tela de detalhe e descartado ao sair dela.
  - Fase 3 (lista): o widget `Image::new(&protocol)` com `Protocol` pré-codificado por `picker.new_protocol(img, Size::new(4, 2), Resize::Fit(None))`, guardado num cache LRU `HashMap<sha256, Protocol>` no estado da TUI (não no buffer). A codificação custou menos de 1,5 ms por ícone e pode ir para uma thread de trabalho com o handle juntado.
  - Fallback: `Picker::halfblocks()`.
- **Detecção.** Fazer `Picker::from_query_stdio()` uma vez em `tui::run`, depois do alternate screen e antes do `EventStream`, só se stdin for TTY, por causa do risco 1. Permitir que a config ou o env forcem o protocolo, porque no Windows a falta de resposta cai em Halfblocks.
- **Pré-requisito.** Decidir se a fase 3 entra depois de o usuário rodar a demo no WezTerm: se a rolagem com iTerm2/sixel piscar ou deixar resto, a lista fica sem ícone e só o detalhe mostra.

## Checks beyond the steps

1. `git status --short` antes do commit. Só o relatório é desta tarefa; os demais `??` são do orquestrador/Task 81. Depois de `git worktree remove`, `git worktree list` mostra só a raiz:
```
?? .claude/tasks/task-81-base.txt
?? .claude/tasks/task-81-report.md
?? .claude/tasks/task-82-base.txt
?? .claude/tasks/task-82-report.md
```
2. Cada número acima vem acompanhado da saída de origem. O que depende do terminal real está escrito "não medido".
3. IconSha256: `sha256 conferem: 30; divergem: 0; sem hash: 0`. No Notepad++, o `sha256sum` bate com o manifesto.

## Limpeza

```
git worktree remove --force <S>/wt82; rm -rf <S>/wt82-target <S>/icon82-root <S>/icon82-dl
git worktree list -> C:/Users/jonyd/Projetos/wingetrs 648d196 [main]
leftovers.ps1 -> [OK] nenhum processo de agente sobrando (> 2 min)
```
Ficam no scratchpad, de propósito:
- `icon-probe/`, com o exe;
- `icon-probe-base/`;
- as saídas `icon82-sample.txt`, `icon82-decode.txt` e `icon82-out/downloads.tsv`.

Concerns:
- (b) e a parte do terminal em (c) dependem do usuário rodar o exe. É provável que o WezTerm escolha iTerm2, e não sixel.
- O `list` na cópia disparou o auto-refresh (`refresh.attempt` e spans `update.*` no log da raiz temporária), então a amostra usou um catálogo recém-baixado, e não o snapshot exato do real. Isso não muda a natureza dos números.
- O risco 1 (thread não juntada e laço em EOF no `Picker::from_query_stdio`) conflita com a regra ⚙ de threads. A Task que integrar o crate precisa decidir conscientemente: aceitar o vazamento documentado ou não consultar o terminal.
