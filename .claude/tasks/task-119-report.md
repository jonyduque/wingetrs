# Task 119 - Sonda S3: corpus MSI, layout e `msiexec /a`

Status: DONE_WITH_CONCERNS (ver Concerns: leitura sequencial do `cab` exigiu patch na sonda; maquina nao estava ociosa)

Commits: base e9b9234; commit unico deste relatorio (SHA na mensagem final ao orquestrador; `git log --format='%h %s' e9b9234..HEAD`).

Gate: nao aplicavel (sonda; nada em `src/`, `Cargo.toml` ou `Cargo.lock` do projeto; `git status` desta tarefa mostra so este relatorio).

Mutation proofs: no mutation proof (sonda). Checks: (1) so `msiexec.exe /a ... /qn TARGETDIR=... /l*v ...` foi lancado (78 execucoes: 73 + 5 repetidas por 1618), um por vez, via `subprocess.Popen` + `wait(timeout=180)`; nenhum `/i /x /f /p`. (2) No fim: `Get-Process msiexec` mostra so o PID 46424, sessao 0 (servico do Windows Installer, nao lancado por mim); `scripts/leftovers.ps1`: `[OK] nenhum processo de agente sobrando (> 2 min)`. (3) Tudo em `target/probe-2b/`: corpus `corpus-msi/`, `corpus-msi.json`, sonda Rust `s3/`, saidas estaticas `s3-static/`, arvores do msiexec `msi-admin/<id>/SourceDir` + `<id>.log` + `<id>.result.json`, `s3-compare.json`, snapshots `s3-snapshot-*.json`. Scripts Python no scratchpad. `catalog.db` aberto `file:...?immutable=1` (uri=True).

Ambiente: 2026-10-16 00:30-01:25, Windows 11 26220, usuario sem admin (token filtrado), msi =0.10.0, cab =0.6.0 (**copia com patch**, ver Concerns), rustc 1.99.0. **Maquina nao ociosa**: durante o msiexec outro processo criava `%TEMP%\eprocia_pdf_*.pdf` / `BenchmarkSinteticoocr=*` (~3.300 entradas transitorias) e havia builds Go/gcc (`go-build*`, `cc*.o`). Todos os tempos abaixo sao **sob carga**.

## Corpus
4115 manifestos scoop; **119** sem script com URL (64bit, senao raiz) terminando em `.msi` (bate com a spec). Ordem por SHA-256 de `source/id`. Baixados: **73**, 3.999.390.020 B, hash do manifesto conferido em todos. Fora: 8 por tamanho > 300 MB (schemacrawler, dangerzone, rust-msvc, rust-gnu, llvm, camo-studio, rust, github-copilot), 33 pelo teto de 4 GB (vagrant, freelens, nextcloud, ... hoppscotch), 5 erros de rede (clustalx/clustalw/mysql-shell 403, webpicmd 404, sonic-pi timeout).
**Assinatura real: 73/73 sao OLE/CFB `D0 CF 11 E0 A1 B1 1A E1`** (diferente da S2: nenhum `.msi` disfarcado). 0 com mais de uma URL no manifesto.

## Estatico (73 MSI, sonda `s3` = msi + cab)
- **SummaryInfo**: plataforma x64 58, Intel 14, vazia 1 (isocreator). Word count 2 (comprimido, nomes longos) 67; 10 (comprimido + sem elevacao) 6 (alien, ipfilter-updater, kiro-cli, scene-builder, scholdoc, webex). **Nenhum** com bit 1 (nomes curtos) nem bit 4 (imagem administrativa). Codepage 1252: 71, UTF-8: 2. Gerador: WiX 61, "Windows Installer" 6, Python MSI 2, Advanced Installer 2, outros 2. Assinados 31.
- **Media**: 69 com 1 gabinete; 4 com varios (cyberduck 2, p4v 3, streamdeck 4, jami 58). **Todos internos (`#`) e com o stream presente. Gabinete externo: 0.** Compressao dos gabinetes (136): LZX:21 (`Lzx(MB2)`) 71, MSZIP 47, LZX:18 (`Lzx(KB256)`) 18. **Quantum: 0. Sem compressao: 0.**
- **Arquivos nao comprimidos** (atributo 0x2000 ou word count sem bit 2): **0** em 73.
- **`DefaultDir`** (n de MSI com a forma): `a|b` 58; `.` no destino 57; `.` na origem 54; `x:y` 9, todos com **origem != destino** (alien, copy-dialog-lunar-lander, expresso, isocreator, ncftpclient, pip-tool, quicksfv, sharpkeys, speedtest). Raiz: sempre uma so, `TARGETDIR` = `SourceDir`. Erros de diretorio (pai ausente, ciclo, profundidade > 64): 0. Profundidade maxima: 10 (zebar).
- **Acoes customizadas** nas sequencias Admin: 21 entradas em 13 MSI, **todas de tipo 51 (definir propriedade) ou 307 (51 + 0x100)**: `DIRCA_TARGETDIR` (cond. `TARGETDIR=""`, 6 MSI), `AI_SET_ADMIN` (AdminUISequence, 3), `ProgramFiles64Folder.<guid>`/`CommonFiles*Folder.<guid>` (streamdeck, tortoisehg). **Nenhuma DLL/EXE/script** na `AdminExecuteSequence` deste corpus. Fora delas, a AdminExecuteSequence e sempre `CostInitialize, FileCost, CostFinalize, InstallValidate, InstallInitialize, InstallAdminPackage, InstallFiles, InstallFinalize`.
- **Transform**: nao aplicavel (o Scoop nao passa `TRANSFORMS`; nenhum manifesto do corpus pede).
- **`extract_dir`**: 62 dos 73 tem; **62/62 existem** no layout proprio e no do msiexec. Mas **3 so casam sem diferenciar maiusculas** (`kalker` vs `Kalker`, `ncftpclient` "Windows folder" vs "Windows Folder", `scala` "PFiles\Scala" vs "PFiles\scala") e **4 usam `/`** (`PFiles/Horizon EDA`, `PFiles64/StGit`, ...). O mapper do P4 precisa comparar sem caixa e aceitar `/`.

## `msiexec /a` (73 MSI, um por vez)
- Codigos de saida: **73 x 0** no final. Na primeira passada: gedit (15.327 arquivos) **estourou os 180 s**; matei so o meu `msiexec` cliente (PID registrado), mas **o servidor continuou a instalacao administrativa ate o fim** (log: 01:09:47 -> 01:14:02, 254 s, "Installation completed successfully") e segurou o mutex: os 5 seguintes (butterflow-ui, cockpit-tools, terminalpp, quickcpu, gauche) sairam com **1618** em 0,1-3,5 s. Repetidos depois: 0. **Matar o cliente nao cancela o `/a`.**
- **UAC: nao apareceu** em nenhum (usuario sem admin; os 73 terminaram com 0 e o log mostra `AlwaysInstallElevated` 0). O timeout do gedit nao foi UAC: o log crescia (10,3 MB no timeout, 35,5 MB no fim).
- Tempo (wall do cliente, sob carga): soma 1.019 s; mediana 3,2 s; maiores gedit 180+ (254 real), oci-cli 125,5 (23.246 arquivos), aws 60,7, puppet-agent 56,1, speedtest 54,5. A sonda `s3` (le tabelas + descomprime e faz SHA-256 de tudo, **sem gravar arquivos**): soma 189 s; razao mediana msiexec/s3 = 5,2. Nao e comparacao justa (a s3 nao escreve em disco): "nao medido" para extracao com escrita.
- **O que o `msiexec /a` gravou fora da pasta-alvo:**
  1. `C:\Windows\Installer\SourceHash{ProductCode}` (20.480 B), **persistente**, um por ProductCode: 193 antes -> 265 depois (+72; transmission e transmission-cli tem o mesmo ProductCode). Nao e limpo.
  2. `C:\Windows\Installer\<hex>.msi`, `MSI<hex>.tmp`, `inprogressinstallinfo.ipi`: **transitorios** (vistos por polling a cada 250 ms; apagados no fim). Ficaram 3 restos do gedit morto ate a proxima instalacao limpar.
  3. `C:\Config.Msi\<hex>.rbf` (backup de rollback) em 2 MSI (copy-dialog-lunar-lander, pip-tool), quando um arquivo sobrescreve outro no mesmo caminho; o log registra `Error 2911: Could not remove the folder C:\Config.Msi\` (nao fatal, saida 0). A pasta existe (ultima escrita 01:20:56); nao consigo listar (so SYSTEM).
  4. Registro HKLM `...\CurrentVersion\Installer` (chaves `InProgress`/`Rollback\Scripts`): transitorias; a contagem de subchaves so mudou (65557 -> 65559 -> 65557) com o gedit em andamento. HKCU `Software\Microsoft\Installer`, `HKLM\SOFTWARE\Classes\Installer` e as 3 chaves `Uninstall`: **contagem inalterada** antes/depois de cada MSI.
  5. **Log de eventos Application**: 4 eventos `MsiInstaller` por MSI (1040, 1042, 1033, 11707 "Installation completed successfully"): 292.
  6. `%TEMP%`: **nada atribuivel ao msiexec** (nenhum `MSI*.LOG`/`MSI*.tmp`; o log nao cita `AppData\Local\Temp`); o que mudou la era do outro processo (benchmark/builds).
  7. Na raiz da pasta-alvo: a copia administrativa do `.msi` (mesmo nome do arquivo de origem), em 73/73; ignorada na comparacao.
  System Restore: "SRSetRestorePoint skipped" / "No System Restore sequence number" (nao cria ponto).

## Layout proprio x `msiexec /a`
Regras: origem = depois do `:` em `DefaultDir`; `short|long` -> longo; `.` = pasta do pai; raiz `TARGETDIR` -> `SourceDir` (descartado); arquivo -> `Component.Directory_` + nome longo de `FileName`; gabinete = primeira `Media` com `LastSequence >= Sequence`; entrada do gabinete = chave `File`. Comparacao: conjunto de caminhos relativos (sem caixa) + tamanho + SHA-256, so arquivos (o msiexec nao criou nenhum diretorio vazio no corpus).
- **Identicos: 71/73 = 97,3%** com a regra ingenua (ultimo `File` por caminho vence). **73/73 = 100%** com a regra de versao abaixo. Meta da spec (>= 95%): atingida.
- **Diferentes (2), causa unica: dois `File` no mesmo caminho com conteudos diferentes.**
  - tortoisehg: `Common\TortoiseOverlays\TortoiseOverlays.dll` (2 componentes, 32/64 bit, mesma versao 1.1.3.21564): venceu o de **menor `Sequence`** (68, nao 69).
  - copy-dialog-lunar-lander: 6 DLLs `System.*.dll` com versoes 4.8.9032.0 e 4.6.x/4.7.x: venceu sempre a **maior versao**, mesmo quando tinha `Sequence` menor (ex.: 93 contra 104).
  - Regra conferida em todas as 23 colisoes do corpus (6 MSI: juliaup, tortoisehg, puppet-agent, ausweisapp2, pip-tool, copy-dialog-lunar-lander): **versionado > nao versionado; maior `Version`; empate -> menor `Sequence`** acerta 23/23 (as outras 15 tinham conteudo identico). Colisao de nao versionados com conteudo diferente: nao ocorreu (regra por data nao medida).
- Caixa dos caminhos: 0 diferencas de caixa entre os dois layouts.
- **Nao suportados (gabinete externo, sem compressao, transform, Quantum): 0/73.**
- Erros de leitura/panico das crates: 0 (`catch_unwind` na sonda; nenhum disparou). Tamanho lido == `FileSize` em todos.

## Vetores recomendados para o P3 (pequenos, identicos ao msiexec)
| id | URL | SHA-256 | bytes | por que |
|---|---|---|---|---|
| ncftpclient (extras) | https://www.ncftp.com/public_ftp/ncftp/older_binaries/Setup%20NcFTP%203.2.6.msi | b034c6c11cb2b888733e047ae9df494c8255e3d9ddfd0cbb2a6ccd98b02c79f2 | 848.896 | `.:WINDOW~1\|Windows Folder` (destino `.`, origem longa = "Windows Folder"); `DOC\|doc` > `HTML\|html` (2 niveis); LZX:21; `extract_dir` "Windows folder" so casa sem caixa. Ex.: `Windows Folder\ncftp.exe` 312.832 e8f6acad...; `doc\html\index.html` 2.203 1cdbb9d2... |
| isocreator (extras) | https://download.sourceforge.net/project/iso-creator-cs/iso-creator-cs/binary-installer/IsoCreator.msi | f01a0a824ddeeb46ade73e2bfc3141a0fee650089a6c4522ab38895d97351634 | 469.504 | `APPDIR:.` (destino APPDIR, origem `.` -> arquivos na raiz); `SHORTC~1\|SHORTCUTDIR`; plataforma vazia; MSZIP. Ex.: `IsoCreator.exe` 81.920 a7b26947... |
| pkgconf (main) | https://github.com/pkgconf/pkgconf/releases/download/pkgconf-3.0.8/pkgconf-x64-3.0.8.msi | a236479bbe403b202816745bef19afe2d6240606f28ab1f008ba0f33fbfae43e | 524.288 | `PFiles64` > `.` > `5lvxyizs\|pkgconf-3.0.8` (`.` no meio da cadeia); LZX:18; `extract_dir` `PFiles64\pkgconf-3.0.8`. Ex.: `PFiles64\pkgconf-3.0.8\pkgconf.exe` 274.432 8f7caa49... |
Extra para o P4 (colisao por versao): copy-dialog-lunar-lander, https://github.com/Sanakan8472/copy-dialog-lunar-lander/releases/download/v1.2/CopyDialogLunarLander.msi, 7c7f51589099d03c696128729ffcd9cd51ae32600701bb5f4ed4def8e43d1267, 2.049.024 B.
Linhas completas da tabela Directory e o layout com SHA-256 de cada um: `target/probe-2b/s3-static/<id>.json`.

## Decisions
- Teto de 4 GB cortou o corpus em 73/119 (33 por teto); mantive a ordem do brief em vez de escolher os menores.
- Comparacao sem caixa (Windows) e so de arquivos; o msiexec nao criou diretorio vazio, entao nao modelei `CreateFolder`.
- A sonda calcula SHA-256 em memoria em vez de montar a arvore em disco (mesma comparacao, sem gravar 2x).
- gedit: comparei a arvore que o servidor terminou depois do timeout (identica) e registrei o timeout.
- Repeti os 5 que deram 1618 depois do servidor terminar, sem mudar nada.

## Concerns
- **`cab` 0.6 `Cabinet::read_file` e quadratico por pasta**: cada chamada cria um `FolderReader` novo e descomprime do inicio da pasta ate o arquivo. webex (836 arquivos numa pasta LZX, 240 MB) passou de 577 s sem terminar (matei); com leitura sequencial: 16 s. Para medir, a sonda usa uma **copia da crate com patch** (`target/probe-2b/s3/cab-0.6.0`, `Cabinet::for_each_file`: um decodificador por pasta, arquivos em ordem de offset; `[patch.crates-io]` so na sonda). Conferido: mesma saida que `read_file` em rakudo-star (2.616 arquivos). O P4 precisa de leitura sequencial (fork/patch ou leitor proprio de pasta): **decisao para o orquestrador**.
- Arquivos no mesmo caminho: o extrator precisa da regra de versao (maior `Version`, empate -> menor `Sequence`) ou o layout difere do msiexec em 2/73.
- Reserva `msiexec /a`: matar o cliente nao cancela a instalacao (servidor segue e segura o 1618); grava fora da raiz `C:\Windows\Installer\SourceHash{...}` (persistente), transitorios em `C:\Windows\Installer`, `C:\Config.Msi\*.rbf` (rollback) e eventos `MsiInstaller`. Isso e o que iria para a excecao do `CLAUDE.md` se a reserva entrar.
- Tempos sob carga de outro processo (benchmark/builds de terceiros em `%TEMP%`); nao repeti medicoes.
- Acoes customizadas so propriedade/diretorio neste corpus; com o resto do corpus (46 nao baixados) isso nao foi medido.
