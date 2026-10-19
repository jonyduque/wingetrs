Status: DONE

Commits:
acb6c7d feat(cli): every output line goes through the formatter
d4d5ba3 feat(cli): dry-run plans and winget lines styled by output.rs

Gate:
```
[OK]   rules 10s
[OK]   rules-selftest 29s
[OK]   fmt 5s
[OK]   clippy 23s
[OK]   test (1887 passed, 87 ignored) 68s
[OK]   gate verde
```

Mutation proofs:
- Largura real nas tabelas: `pwsh -File scripts/mutate.ps1 -Path src/cli/output.rs -Anchor '    ratatui::text::Span::raw(text).width()' -Replacement '    text.chars().count()' -TestCommand 'cargo test --quiet --test it cli_color::wide_chars_align_table'`
  `[OK] mutação PEGA: '    ratatui::text::Span::raw(text).width()' -> '    text.chars().count()' fez o teste falhar (exit 101).`
- Linha "→ o que fazer": `... -Path src/cli/output.rs -Anchor '        s.push_str(&action(todo));' -Replacement '        let _ = todo;' -TestCommand 'cargo test --quiet --test it cli_color::error_has_action_line'`
  `[OK] mutação PEGA: '        s.push_str(&action(todo));' -> '        let _ = todo;' fez o teste falhar (exit 101).`
- Cadeia de causas (só as causas, não o topo): `... -Anchor '    let causes: Vec<String> = e.chain().skip(1)' -Replacement '    let causes: Vec<String> = e.chain().skip(0)' -TestCommand 'cargo test --quiet --test it cli_color::error_has_action_line'`
  `[OK] mutação PEGA: '    let causes: Vec<String> = e.chain().skip(1)' -> '    let causes: Vec<String> = e.chain().skip(0)' fez o teste falhar (exit 101).`
- Causas só com `-v`: `... -Anchor '    if verbose {' -Replacement '    if true {' -TestCommand 'cargo test --quiet --test it cli_color::error_has_action_line'`
  `[OK] mutação PEGA: '    if verbose {' -> '    if true {' fez o teste falhar (exit 101).`
- Sem ANSI com `NO_COLOR` (mesmo com `CLICOLOR_FORCE`): `... -Path src/cli/style.rs -Anchor '        ColorArg::Auto if no_color => ColorChoice::Never,' -Replacement '' -TestCommand 'cargo test --quiet --test it cli_color::no_ansi_when_not_tty'`
  `[OK] mutação PEGA: '        ColorArg::Auto if no_color => ColorChoice::Never,' -> '' fez o teste falhar (exit 101).`
- Regra mecânica (gate), violação plantada em `install_cmd::stdout_line` (`emit::out(line)` -> `anstream::println!("{line}")`), `-AnchorFile`/`-ReplacementFile`, `-TestCommand 'pwsh -File scripts/verify.ps1 -NoTests'`:
  `[OK] mutação PEGA: 'fn stdout_line(line: &str) {\n    emit::out(line);\n}' -> 'fn stdout_line(line: &str) {\n    anstream::println!("{line}");\n}' fez o teste falhar (exit 1).`
  Plantada à mão de novo para ver o motivo (e restaurada, `cmp` igual ao backup):
  ```
  [FAIL] rules exit 1
      src\cli\install_cmd.rs:651: [saida do CLI fora de cli/emit.rs] anstream::println!("{line}");
  ```
  Além disso, `scripts/check_rules_selftest.ps1` ganhou 5 casos plantados da regra (println/eprint/stderr travado em `src/cli/x.rs` falham; println em `src/cli/emit.rs` e `stdout()` sem trava passam): `[OK] check_rules: 29 casos plantados`.

- Rodada 2 (pendências do orquestrador):
  `[OK] mutação PEGA: '        let n = format!("{:>width$}", i + 1);\n        s.push_str(…' -> '        let n = (i + 1).to_string();…' fez o teste falhar` (`cli_color::snapshots_tty_color_and_plain`, números alinhados à direita no `--dry-run`)
  `[OK] mutação PEGA: '            emit::out_block(&output::install_plan_text(p, ui.glyphs()));' -> '            emit::out(&p.to_string());' fez o teste falhar (exit 101).` (`cli_color::snapshots_not_tty_dry_run`)
  `[OK] mutação PEGA: '            format!("{}{}", paint(HEADER, l), &body[l.len()..])' -> '            body.to_string()' fez o teste falhar (exit 101).` (`snapshots_tty_color_and_plain`, rótulos dos planos winget)
  `[OK] mutação PEGA: '                say(&output::cancelled());' -> '                say("Cancelado.");' fez o teste falhar (exit 101).` (`cli_scripts::`, `bucket add` cancelado)

Rodada 2, saída real (release, não TTY):
```
$ wingetrs install main/fastfetch --dry-run
Instalar fastfetch 2.70.0  main · 64bit
  1. Baixar [1] https://github.com/fastfetch-cli/fastfetch/releases/download/2.70.0/fastfetch-windows-amd64.7z (fastfetch-windows-amd64.7z, sha256 00d3b9453749aae9cd61aa3ead2f101cd7c4bf5e74d1a79302f10edf3234c829)
  2. Extrair [1] fastfetch-windows-amd64.7z (7z) para C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\.staging-2.70.0
  3. Mover C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\.staging-2.70.0 para C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\2.70.0
  4. Apontar C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\current -> C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\2.70.0 (junção)
  5. Criar comando fastfetch em C:\Users\jonyd\AppData\Local\wingetrs\shims (fastfetch.exe + fastfetch.shim) -> C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\current\fastfetch.exe
  6. Criar comando flashfetch em C:\Users\jonyd\AppData\Local\wingetrs\shims (flashfetch.exe + flashfetch.shim) -> C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\current\flashfetch.exe
  7. Garantir C:\Users\jonyd\AppData\Local\wingetrs\shims no PATH do usuário
  8. Gravar C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\2.70.0\manifest.json
  9. Registrar fastfetch 2.70.0 (main) em C:\Users\jonyd\AppData\Local\wingetrs\state.db
Simulação: nada foi alterado.
[exit 0]
$ wingetrs install 7zip.7zip --dry-run
7-Zip 26.04 (exe, machine)
Instalador: https://www.7-zip.org/a/7z2604-x64.exe
Linha de comando: <instalador> /S
pede permissão de administrador para este instalador
o instalador roda numa pasta temporária do wingetrs
mostra o progresso do instalador
SHA-256: D54BF805F9F3704D1E8DB2FA3498AE7EF2DF0312B40B558E7C71C734430A665D
Simulação: nada foi alterado.
[exit 0]
```
Em TTY os dois ganham estilo (cabeçalho, nome em negrito, números em negrito, rótulos `Instalador:`/`Linha de comando:`/`SHA-256:`/`Licença:` em destaque, `(tipo, escopo)` dim): ver `tests/it/snapshots/it__cli_color__tty_color.snap`. Fora de TTY o texto do plano winget é idêntico ao de antes (de propósito: testes e TUI leem o mesmo `Display`).

Saída real da rodada 1 (release, não TTY: `| cat`, dados reais do usuário):
```
$ wingetrs search 7zip
Nome                                Id                         Versão           Fonte
7-Zip                               7zip.7zip                  26.04            winget
7zip                                7zip                       26.04            scoop:main
7-Zip ZS                            mcmilk.7zip-zstd           26.02-v1.5.7-R2  winget
7-Zip ZS Non Darkmode               mcmilk.7zip-zstd-ndm       26.02-v1.5.7-R2  winget
7zip19.00-helper                    7zip19.00-helper           19.00            scoop:main
7zr                                 7zip.7zr                   26.04            winget
Dark7zip                            ozone10.7zip.Dark          26.02-0.70.0     winget
Advanced Archive Password Recovery  Elcomsoft.ArchivePassword  4.66.266.6965    winget
Cram                                Nexalit.Cram               1.3.2            winget
NanaZip                             M2Team.NanaZip             7.0.1843.0       winget
NanaZip Preview                     M2Team.NanaZip.Preview     7.0.1800.0       winget
QNapi                               QNapi.QNapi                0.2.3            winget
[exit 0]
$ wingetrs show 7zip.7zip
Nome: 7-Zip
Id: 7zip.7zip
Versão: 26.04
Fonte: winget
Publicador: Igor Pavlov
Descrição: A file archiver with a high compression ratio.
Página: https://7-zip.org/download.html
Licença: LGPL-2.1
Tags: 7zfiles, archive, archiver, compress, compression, decompress, decompression, extract, extractor, rarfiles, targzfiles, unarchive, unarchiver, unzip, zip
Versões: 26.04, 26.03, 26.02, 26.01, 26.00, 25.01, 25.00, 24.09, 24.08, 24.07, 24.06, 24.05, 23.01, 22.01, 22.00, 21.07, 21.06, 19.00, 18.06, 18.05, 18.01, 16.04
Instalador: x86 exe machine https://www.7-zip.org/a/7z2604.exe
Instalador: x64 exe machine https://www.7-zip.org/a/7z2604-x64.exe
Instalador: arm exe machine https://www.7-zip.org/a/7z2604-arm.exe
Instalador: arm64 exe machine https://www.7-zip.org/a/7z2604-arm64.exe
Instalador: x86 wix machine https://www.7-zip.org/a/7z2604.msi
Instalador: x64 wix machine https://www.7-zip.org/a/7z2604-x64.msi
[exit 0]
$ wingetrs install main/fastfetch --dry-run
Instalar fastfetch 2.70.0 (bucket main, 64bit)
  1. Baixar [1] https://github.com/fastfetch-cli/fastfetch/releases/download/2.70.0/fastfetch-windows-amd64.7z (fastfetch-windows-amd64.7z, sha256 00d3b9453749aae9cd61aa3ead2f101cd7c4bf5e74d1a79302f10edf3234c829)
  2. Extrair [1] fastfetch-windows-amd64.7z (7z) para C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\.staging-2.70.0
  3. Mover C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\.staging-2.70.0 para C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\2.70.0
  4. Apontar C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\current -> C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\2.70.0 (junção)
  5. Criar comando fastfetch em C:\Users\jonyd\AppData\Local\wingetrs\shims (fastfetch.exe + fastfetch.shim) -> C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\current\fastfetch.exe
  6. Criar comando flashfetch em C:\Users\jonyd\AppData\Local\wingetrs\shims (flashfetch.exe + flashfetch.shim) -> C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\current\flashfetch.exe
  7. Garantir C:\Users\jonyd\AppData\Local\wingetrs\shims no PATH do usuário
  8. Gravar C:\Users\jonyd\AppData\Local\wingetrs\apps\fastfetch\2.70.0\manifest.json
  9. Registrar fastfetch 2.70.0 (main) em C:\Users\jonyd\AppData\Local\wingetrs\state.db

Simulação: nada foi alterado.
[exit 0]
```

Inventário (antes -> depois). Toda escrita de `src/cli/` sai agora por `src/cli/emit.rs` (`out`, `out_block`, `out_flushed`, `err`, `json`, `error`, e `erase`/`line_above`/`redraw`/`flush` da linha de progresso); o texto vem de `output.rs`.
- `update`: `mod.rs::print_reports` `println!(report_line)` -> `emit::out(report_line)`; linha ganhou glifo (✓/✗) e números em negrito. `--background` não escreve nada.
- `search`: `print!(search_table)` / `print_json` -> `emit::out_block` / `emit::json`; colunas pela largura do terminal (`output::columns`).
- `show`: `print!(details_text)` / `print_json` -> idem. Erros "pacote não encontrado" e "pacote ambíguo" viraram `Advice` (✗ + →).
- `list`: `print!(list_text)` / `print_json` -> idem; contagens em negrito. Avisos de coleta só no log (`tracing`), sem mudança.
- `install` (Scoop, `install_cmd.rs`): plano curto `print!(plan_summary)` (caminhos do PATH em `style::PATH`), `--dry-run` `println!("{p}")` (Display de `InstallPlan`, `src/install/plan.rs`) + dica MSI dim, bloco de código de terceiros, "Simulação: nada foi alterado." (`DRY_RUN_DONE`), "Cancelado." (`cancelled()`: ⚠ amarelo), "Instalado:" (`success()`: ✓ verde, nome em negrito), dica de terminal novo. Progresso (stderr): `ProgressState::draw`, `LineProgress::{update,notice,stdout_line,finish}` com `write!` em `stderr().lock()` -> `emit::{stderr,line_above,redraw,erase,err,out_flushed}` (comportamento igual). Recusas: `--yes` sem terminal, código de terceiros, alvo não encontrado viraram `Advice`.
- `install` (winget, `winget_cmd.rs`): ponte `println!` de "<nome> via winget", licenças, linha de comando (`--dry-run`, agora dim), Simulação, Cancelado -> `emit::out` com `output::{via_winget,dim,DRY_RUN_DONE,cancelled}`; linhas do winget/scoop continuam em stdout por `Ui::output` (Task 201); "Instalado:"/"Atualizado:"/"aviso:" por `say`/`ui.output` agora com `success()`/`warning()`. Aviso de pasta do auxiliar (`helper_notice`) -> `output::warn_text`.
- `uninstall`: plano (`uninstall_plan_text`, caminhos do PATH estilizados), Simulação, Cancelado, "Desinstalado:" (✓), notas `eprintln!("aviso: …")` -> `emit::err(warning())` (⚠ aviso:), resíduos sem terminal -> `output::residues_left` (⚠ + → use --yes), "Removido:" (✓). Winget/ARP: idem por `say`; "Removido: registro de …" com caminho estilizado; recusas de dados compartilhados viraram `Advice` (Display igual a `SHARED_YES_REFUSED`).
- `upgrade`: lista sem `--all` -> `output::upgrades_listing` (tabela alinhada `app  de  →  para`, "(fixado)" dim, "→ use `wingetrs upgrade --all` …" ou "Nada para atualizar."), avisos de pulados -> `emit::err(warning())`, "fixado, pulado: x" dim, "x já está na versão mais recente." com ✓, "Atualizado:" ✓; versão fixada virou `Advice` (✗ … fixada / → desafixe na TUI).
- `cleanup`: linhas por `say` (stdout): "Órfão: …", aviso do `.reg` (caminho estilizado), Simulação, "Removido: … (backup …)" ✓, "Erro: …" -> `failed_item()` (✗ Erro:); "diga o que limpar" virou `Advice` (→ use --orphans).
- `repair`: `print!(repair_text)` -> `emit::out_block`.
- `bucket list|add|rm`: `println!("{name}  {repo}")` -> `buckets_text` (tabela alinhada); "Bucket x adicionado. Rode …" -> "✓ Bucket adicionado: x" + "→ rode `wingetrs update` …"; "Bucket x removido." -> "✓ Bucket removido: x". `bucket add` desconhecido sem repo virou `Advice`.
- Erros do `anyhow` na borda: `main.rs` `eprintln!("erro: {e:#}")` (cadeia inteira) -> `cli::emit::error(&e, verbose)` = `output::error_report`: "✗ <topo>", "→ <ação>" quando o topo é `Advice`, causas ("  causa: …") só com `-v`; sem `-v` e com causas, "→ rode de novo com -v para ver a causa". Ctrl+C (130) continua sem linha.
- `--help`/erros de parse: clap (`CLAP_STYLES`, `e.exit()`), inalterados. Seletor (`picker.rs`, Task 200) escreve no `Term` com `write!`: fora da regra (não é `print!` nem stdout travado), inalterado. Logs `-v` vão por `tracing` para stderr, inalterados.

Decisions:
- `Advice { what, todo }` (erro em `output.rs`) com `Display` = "what; todo": logs, job log e testes antigos que comparam `to_string()` não mudam; só o CLI mostra em duas linhas. `Advice` sob `.context()` conta como causa (o topo manda).
- Regra mais estrita que o brief: `print!`/`println!`/`eprint!`/`eprintln!` e `stdout()/stderr().lock()` em `src/cli/` só em `emit.rs` (nem `output.rs`, que fica puro); o skip das duas regras de print passou a casar `src/` em qualquer profundidade para o selftest poder plantar em `<tmp>/src/cli/`.
- Glifos das linhas de status: o conjunto configurado (`--glyphs` > config) quando stdout é TTY; fora de TTY sempre unicode (✓ ⚠ ✗ →), para logs/pipes sem Nerd Font. Estilo `PATH` = ciano itálico.
- Snapshots: "TTY com cor" e "TTY sem cor" pelos formatadores (`with_enabled(true/false)`, ANSI visível como `␛`; teste exige texto idêntico sem ANSI); "não TTY" pelo binário (search/show/erro/bucket list sobre o fixture). `list` real não roda no teste (lê o registro real).
- Testes antigos ajustados por mudança intencional de formato: `cli_install.rs` (versão fixada agora em duas linhas; `bucket list` alinhado comparado por colunas).

Decisions (rodada 2):
- `--dry-run` do Scoop: `InstallPlan::step_lines()` (plan.rs só lista o texto de cada passo; o `Display` usa o mesmo método e não mudou, a TUI e os testes leem ele); `output::install_plan_text` monta o cabeçalho de `plan_summary`, os números em negrito alinhados à direita e as notas como `⚠ aviso:`. Caminhos dentro do texto do passo não ganharam estilo próprio (o texto vem pronto do plano).
- Planos winget (`WingetPlan`, `UninstallPlan`) e linhas de licença: `output::plan_text` estiliza o `Display` linha a linha sem mudar o texto (sem cor fica idêntico; teste exige). A URL do instalador continua na confirmação winget também fora do `--dry-run`, como antes (TUI igual); só estilo mudou.
- Testes antigos alterados de propósito: `cli_scripts.rs:1140` agora espera `"⚠ Cancelado."`; `cli_plan_summary.rs` só expôs `plan`/`source`/`shim` como `pub(crate)` para o snapshot.

Concerns:
- Fora da lista Files, editados com motivo: `src/main.rs` (borda do erro: `-v` e `emit::error`), `scripts/check_rules_selftest.ps1` (casos plantados da regra nova) e, na rodada 2 com autorização do orquestrador, `src/install/plan.rs` (`step_lines`).
- Mensagens com ação embutida em várias linhas (`ambiguous_text`, "corresponde a mais de um app instalado …") ficaram como estão (ação já na 1ª linha, lista abaixo).
- Nenhum `.join` com dado externo escrito nesta tarefa.
