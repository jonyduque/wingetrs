Status: DONE

Commits:
307d227 feat(cli): colored, formatted output and help with --color and NO_COLOR

Gate:
[OK]   rules 4s
[OK]   rules-selftest 14s
[OK]   fmt 5s
[OK]   clippy 27s
[OK]   test (962 passed, 23 ignored) 116s
[OK]   gate verde

Mutation proofs:
- choose ignora NO_COLOR (style.rs, anchor 'ColorArg::Auto if no_color => ColorChoice::Never,' -> ''), test color_choice_table:
  [OK] mutação PEGA: 'ColorArg::Auto if no_color => ColorChoice::Never,' -> '' fez o teste falhar (exit 101).
- largura sobre texto pintado (test table_alignment_ignores_ansi):
  [OK] mutação PEGA: 'width.saturating_sub(text.chars().count())' -> 'width.saturating_sub(paint(style, text).chars().count())' fez o teste falhar (exit 101).
- JSON pelo caminho com estilo (test json_never_colored):
  [OK] mutação PEGA: 'writeln!(std::io::stdout().lock(), "{text}")?;' -> 'anstream::println!("{}", super::style::paint_raw(ID, &text));' fez o teste falhar (exit 101).
- Regra nova check_rules (anstyle:: em output.rs):
  [OK] mutação PEGA: 'fn pad(style: Style' -> 'fn pad(style: anstyle::Style' fez o teste falhar (exit 1).
  Caso plantado novo no selftest: "[OK] check_rules: 10 casos plantados".

Binário real (target/debug, --color always, stdout redirecionado; saída sem ESC pelo sed equivalente a strip_str):
- search vlc -n 3: ESC=30, tabela alinhada (Nome/Id/Versão/Fonte, 3 linhas).
- list: ESC=1179, tabela alinhada, colunas Instalada/Disponível preenchidas.
- --help --color always: ESC=0 (ver Concerns).
Cores reais num terminal: não vistas (só o usuário vê).

Decisions:
- paint() só pinta quando a cor está ligada (flag global + override por thread `with_enabled` para testes); `paint_raw` pinta sempre (ajuda do clap é montada antes de a escolha existir). Assim os testes unitários existentes continuam comparando texto sem cor.
- choose() nunca devolve Auto; em terminal com flag auto, anstream recebe Auto (ele habilita VT/converte), nos demais casos Always/Never.
- print_json (output.rs) escreve em std::io::stdout direto: caminho sem estilo.
- --color com hide_possible_values (como --glyphs); snapshots de ajuda de todos os comandos ganharam a linha da flag global.
- Reexport `style::Style` para output.rs não citar anstyle (regra nova); AnsiColor::on_default em vez de anstyle::Color (regra Color::).

Concerns:
- (RESOLVIDO na revisão) `wingetrs --help --color always` não colorava: o clap decide a cor da ajuda ao parsear (Auto: tty + NO_COLOR), antes de run() ler a flag. Em terminal a ajuda sai colorida; só o forçar via flag não vale para a ajuda.
- Cargo.lock: só acrescentou anstream/anstyle à lista de dependências de wingetrs (nenhum pacote novo).
- Mensagens de progresso (stderr, install_cmd) e picker (crossterm) ficaram sem mudança.

Review fixes (commit 2):
- main.rs pre-scans argv for --color and sets Command::color; test help_color_flag_overrides_pipe.
  [OK] mutação PEGA: 'cmd = cmd.color(choice);' -> 'let _ = choice;' fez o teste falhar (exit 101).
- force_from(CLICOLOR_FORCE): unset/""/"0" não forçam; test clicolor_force_zero_and_empty_do_not_force.
  [OK] mutação PEGA: '!v.is_empty() && v != "0"' -> '!v.is_empty() && true' fez o teste falhar (exit 101).
- list_text_alignment_ignores_ansi adicionado.
- Nota: NO_COLOR vence CLICOLOR_FORCE de propósito.
Gate: [OK]   test (965 passed, 23 ignored) 358s / [OK]   gate verde
