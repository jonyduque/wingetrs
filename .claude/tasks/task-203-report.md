# Task 203 report

Status: DONE

Commits:
```
1ce073c fix: plan texts from the final dry-run preflight
```

Gate:
```
[OK]   rules 11s
[OK]   rules-selftest 35s
[OK]   fmt 3s
[OK]   clippy 28s
[OK]   test (1901 passed, 87 ignored) 85s
[OK]   gate verde
```

## Check 1: os 10 comandos `--dry-run` com `target/release/wingetrs.exe` (depois da correção)

Nada instalado (só `--dry-run`). CrystalDiskMark só resolve como `"CrystalDiskMark 9.0.3"` nesta máquina (antes da correção, `uninstall CrystalDiskMark` também dava "não está instalado"; não muda nesta tarefa).

```
### wingetrs uninstall jqlang.jq --dry-run
Desinstalar jq 1.8.2
  1. Remove pelo winget (pode mostrar o progresso dele)
  Comando: winget uninstall --product-code jqlang.jq_Microsoft.Winget.Source_8wekyb3d8bbwe
Simulação: nada foi alterado.
[exit 0]
### wingetrs install PeterPawlowski.foobar2000 --dry-run
foobar2000 2.26.1 (nullsoft, máquina)
  1. ⚠ Pede permissão de administrador para este instalador
  2. Roda numa pasta temporária do wingetrs
  3. Sem janela
  Instalador: https://www.foobar2000.org/files/foobar2000-x64_v2.26.1.exe
  Linha de comando: <instalador> /S
  SHA-256: 72FE15A88B085EE4E49D68114E2BB9655000C2E0FDF40DCC128BBEF8BCAF3600
Simulação: nada foi alterado.
[exit 0]
### wingetrs install M2Team.NanaZip --dry-run
NanaZip 7.0.1843.0 (msix, escopo do instalador)
  1. Instala pelo Windows (pacote MSIX), sem janela
  2. Só para este usuário
  Instalador: https://github.com/M2Team/NanaZip/releases/download/7.0.1843.0/NanaZip_7.0.1843.0.msixbundle
  SHA-256: F5B013AFFF37ECA32ED7E318CB7D7D1B7A1E11F9765E4C2E9AFAFCDE15EB085D
Simulação: nada foi alterado.
[exit 0]
### wingetrs install sharkdp.hyperfine --dry-run
hyperfine 1.20.0 via winget
  1. Instala pelo winget (zip)
  Comando: winget install --id sharkdp.hyperfine --exact --source winget --silent --disable-interactivity --accept-package-agreements --accept-source-agreements
Simulação: nada foi alterado.
[exit 0]
### wingetrs uninstall Amazon Kindle --dry-run
Desinstalar Amazon Kindle 2.9.1.71006
  1. Roda o desinstalador registrado do app
  ⚠ Outra instalação de Amazon Kindle usa as mesmas pastas: copie antes de remover.
  ⚠ Pasta compartilhada: C:\Users\jonyd\AppData\Local\Amazon
  ⚠ Tamanho da cópia: 800,4 MiB em 498 itens
  Comando: C:\Users\jonyd\AppData\Local\Amazon\Kindle\application\uninstall.exe /S
Simulação: nada foi alterado.
[exit 0]
### wingetrs install JAMSoftware.TreeSize.Free --dry-run
TreeSize Free 4.8.1.610 (inno, usuário)
  1. ⚠ O instalador pode pedir permissão de administrador
  2. Mostra o progresso do instalador
  Instalador: https://downloads.jam-software.de/treesize_free/archive/v4.8.1.610/TreeSizeFreeSetup.exe
  Linha de comando: <instalador> /SILENT /SUPPRESSMSGBOXES /NORESTART /CurrentUser /LOG="<LOGPATH>"
  SHA-256: D800F1765539194A088CD06B2B8A73C40C4A1001425F2A892F495A53DB9DA2DE
Simulação: nada foi alterado.
[exit 0]
### wingetrs install main/python --dry-run
✗ o script uninstaller.script foi recusado: usa -Global (instalação para todos os usuários)
[exit 1]
### wingetrs install Python.Python.3.12 --scope machine --dry-run
Python 3.12 3.12.10 (burn, máquina)
  1. ⚠ Pede permissão de administrador para este instalador
  2. Roda numa pasta temporária do wingetrs
  3. Mostra o progresso do instalador
  Instalador: https://www.python.org/ftp/python/3.12.10/python-3.12.10-amd64.exe
  Linha de comando: <instalador> /passive /norestart InstallAllUsers=1 PrependPath=1
  SHA-256: 67B5635E80EA51072B87941312D00EC8927C4DB9BA18938F7AD2D27B328B95FB
Simulação: nada foi alterado.
[exit 0]
### wingetrs install tumic0.GPXSee --dry-run
GPXSee 16.17 (nullsoft, máquina)
  1. ⚠ Pede permissão de administrador para este instalador
  2. Roda numa pasta temporária do wingetrs
  3. Sem janela
  Instalador: https://sourceforge.net/projects/gpxsee/files/Windows/GPXSee-16.17_x64.exe/download
  Linha de comando: <instalador> /S /norestart
  SHA-256: DA5B3122A4375117FCA2607B2A1FC32D165960D36DDD467842F96CED3676EA8C
Simulação: nada foi alterado.
[exit 0]
### wingetrs uninstall Graphviz --dry-run
Desinstalar Graphviz 16.1.0
  1. ⚠ Pede permissão de administrador (UAC)
  2. Roda o desinstalador registrado do app
  Comando: C:\Program Files\Graphviz\Uninstall.exe /S
Simulação: nada foi alterado.
[exit 0]
### wingetrs uninstall CrystalDiskMark 9.0.3 --dry-run
Desinstalar CrystalDiskMark 9.0.3
  1. ⚠ Pede permissão de administrador (UAC)
  2. Roda o desinstalador registrado do app
  Comando: C:\Program Files\CrystalDiskMark9\unins000.exe /SILENT
Simulação: nada foi alterado.
[exit 0]
### wingetrs uninstall Notepad++ --dry-run
Desinstalar Notepad++ 1.0.0.0
  1. Remove o pacote MSIX NotepadPlusPlus_2247w0b46hfww (só deste usuário)
Simulação: nada foi alterado.
[exit 0]
```

Mutation proofs:

"Checks beyond the steps" lista só a reexecução dos dry-runs e o gate (sem regra mutável); fiz uma prova por item mesmo assim. Todas com `pwsh -File scripts/mutate.ps1 -Path <arq> -Anchor/-AnchorFile ... -TestCommand 'cargo test --quiet --test it <teste>'` (driver em `scratchpad/t203-mutate.ps1`):

1. `uninstall.rs` (item 1), teste `wi_cli::product_code_line_is_removed_by_winget`:
   `[OK] mutação PEGA: '|| (!p.starts_with('"') && p.split_whitespace().next().is_some_and(stem_is_winget))' -> '|| false' fez o teste falhar (exit 101).`
2. `switches.rs` (item 2), `wi_install_flow::mode_step_comes_from_the_switch_that_runs`:
   `[OK] mutação PEGA: '        if same(' -> '        if false && same(' fez o teste falhar (exit 101).`
3. `plan.rs` (item 3), `wi_install_flow::msix_plan_has_steps`:
   `[OK] mutação PEGA: '            out.push(MSIX_SCOPE_STEP.to_string());' -> '            // removed' fez o teste falhar (exit 101).`
4. `winget_cmd.rs` (item 4), `wi_cli::bridge_plan_has_numbered_steps`:
   `[OK] mutação PEGA: 'std::iter::once(bridge_step(why))' -> 'std::iter::empty()' fez o teste falhar (exit 101).`
   4b. `wi_cli::bridge_reason_is_printed` (o motivo chega ao `install_bridge`):
   `[OK] mutação PEGA: '.install_bridge(ui, cand, scope, why.as_deref(), f, stop)' -> '.install_bridge(ui, cand, scope, None, f, stop)' fez o teste falhar (exit 101).`
5. `output.rs` (item 5, bloco de aviso sem número), `wi_uninstall_flow::removal_plan_uac_first_and_warnings_apart`:
   `[OK] mutação PEGA: '            "  {} {}\n",' -> '            "  {}. {}\n",' fez o teste falhar (exit 101).`
6. `uninstall.rs` (item 6, ordem: UAC girado para o fim com `out.rotate_left(k)` antes do `out` final de `steps()`), mesmo teste, `-AnchorFile`/`-ReplacementFile`:
   `[OK] mutação PEGA: '        out` (âncora de várias linhas; a linha continua com o trecho)
7. `plan.rs` (item 7), `wi_install_flow::self_elevating_installer_warns`:
   `[OK] mutação PEGA: '} else if self.command.is_some() && self.elevates_itself() {' -> '} else if false && self.elevates_itself() {' fez o teste falhar (exit 101).`
   7b. `output.rs` (glifo ⚠ do passo novo, `SELF_ADMIN_PROMPT` tirado de `asks_admin`):
   `[OK] mutação PEGA: '        SELF_ADMIN_PROMPT,` (âncora de várias linhas)
8. `script_scan.rs` (item 8, deduplicação em `scan_inner`), `script_scan::same_reason_on_two_lines_is_one_flag`:
   `[OK] mutação PEGA: '    flags.sort_by_key(|f| f.line);` (âncora de várias linhas: remove `dedupe_reasons(&mut flags);`)
   8b. deduplicação em `scan_called_at` (arquivo chamado em duas linhas):
   `[OK] mutação PEGA: '    report.flags.sort_by_key(|f| f.line);` (âncora de várias linhas: remove `dedupe_reasons(&mut report.flags);`)
9. `worker.rs` (item 9), `elevated_install::prepend_path_passes_as_exact_literals`:
   `[OK] mutação PEGA: '["installallusers=1", "prependpath=1", "prependpath=0"]' -> '["installallusers=1", "prependpath=1", "prependpath=1"]' fez o teste falhar (exit 101).`
10. `plan.rs` (item 10), `wi_install_flow::dry_run_hash_is_upper_case`:
   `[OK] mutação PEGA: 'self.installer.sha256.trim().to_ascii_uppercase()' -> 'self.installer.sha256.trim()' fez o teste falhar (exit 101).`

Decisions:
- Item 2: `mode_for` devolve `Mode::Silent` quando o switch de progresso efetivo é igual ao silencioso (sem caixa, espaços normalizados). O passo sai do modo, e o modo sai do switch que roda; a linha de comando não muda (é o mesmo texto).
- Item 3: o segundo passo do MSIX é "Só para este usuário" (o `msix_win` só faz `AddPackageAsync` por usuário, igual ao "(só deste usuário)" da remoção). O título continua "(msix, escopo do instalador)".
- Item 4: a ponte do `install` vira plano: título `<nome versão> via winget`, passo 1 `Instala pelo winget (<motivo sem "tipo " e sem ": instalado pelo winget">)`, licenças numeradas, `Comando:` só no dry-run. O motivo agora chega ao `WingetFlows::install_bridge` (parâmetro `why` novo, por isso `src/cli/run_win.rs` e os dois fakes de teste mudaram; `run_win.rs` está fora da lista Files). A linha "Instalando pelo winget: …" sumiu do `install` em todos os modos (o plano já diz isso); o `upgrade` pela ponte ficou como estava.
- Item 5: novo `UninstallPlan::warnings()`; `steps()` não traz mais as linhas de pasta compartilhada. A CLI imprime os avisos sem número, com ⚠, depois dos passos; o `Display` (o que a TUI mostra) lista título, passos e depois avisos.
- Item 8: a deduplicação vale para todos os `Flag` com o mesmo (tipo, veredito, motivo), não só os de recusa, e fica com a primeira linha. Fica em `scan_inner` e depois da junção de `scan_called_at`.

Concerns:
- Testes existentes alterados porque o comportamento mudou de propósito: `bridge_reason_is_printed` (agora exige que o motivo chegue ao `install_bridge` e que não haja "Instalando"), `tui_confirmation_and_cli_uninstall_list_the_same_steps` (`steps() >= 3` virou `>= 2`, e agora compara também os avisos da CLI e da TUI e a ordem passos → avisos), `norestart_and_install_all_users_pass_as_exact_literals` (`PrependPath=1` saiu da lista de recusa e `PrependPath=2` entrou), `plan_text_is_the_confirmation` e `winget_dry_run_shows_url_and_sha256` (hash em maiúsculas), snapshots `tty_color`/`tty_plain` (passos de remoção e o plano novo da ponte).
- Item 1 só corrige o texto. A remoção nativa do jq de verdade continua tentando rodar `winget uninstall --product-code …` como programa, e `uninstall()` recusa com "desinstalador sem caminho absoluto" (`uninstall.rs` ~l.430). Não medi se o fluxo real manda pacotes instalados pelo winget para a ponte antes de chegar ali.
- O passo "O instalador pode pedir permissão de administrador" aparece em todo instalador não elevado com `elevatesSelf`/`elevationRequired`, inclusive `exe` sem escopo (`Elevation::Never`).
