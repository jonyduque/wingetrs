# wingetrs — saída colorida e formatada da CLI (Task 132)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 132 -Plan docs/superpowers/plans/2026-10-16-wingetrs-cli-cores.md`.

**Goal:** a CLI (`search`, `show`, `list`, `install`, `uninstall`, `upgrade`, `update`, erros, avisos e as ajudas do clap) sai colorida e formatada no terminal, e sem cor quando a saída vai para arquivo, pipe ou `--json`.

**Pedido do usuário (2026-10-16):** "deixar coloridas e formatadas as saídas quando usar o modo CLI, inclusive as ajudas".

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- ⚙ Nada escreve em stdout fora de `main.rs`/`cli/`. Cores da TUI só via `src/tui/theme.rs`; as da CLI só via o arquivo novo `src/cli/style.rs` (regra mecânica nova nesta tarefa).
- Sem crate nova no `Cargo.lock`: `anstyle` e `anstream` **já estão no lock** (vêm do `clap` 4.6.7 com a feature padrão `color`); entram como dependência direta com a versão exata que o lock já tem.
- `--json` nunca tem cor. Mensagens em português.

## Review Focus

1. Saída redirecionada (`> arquivo`, `| findstr`) sem nenhum byte `ESC`.
2. `NO_COLOR` definido desliga a cor mesmo num terminal; `--color always` liga mesmo em pipe.
3. Tabelas com cor continuam alinhadas: a largura das colunas ignora os códigos ANSI.
4. Console antigo do Windows (sem VT) não mostra lixo `←[32m`.

---

### Task 132: Saída colorida e formatada na CLI, inclusive as ajudas

**Modelo:** sonnet (revisor opus).

#### Where this fits

- `src/cli/mod.rs`: `#[derive(Parser)] struct Cli` com `#[command(...)]` (~:34–54), `enum Command` (~:79); 39 chamadas `print!`/`println!`/`eprintln!` em `src/cli/*.rs` e `src/main.rs`.
- `src/cli/output.rs`: as funções devolvem `String` sem cor e são testadas por texto: `search_table` (~:12), `details_text` (~:45), `report_line` (~:82), `list_text` (~:138), `ambiguous_text` (~:229), `not_found_text` (~:239), `uninstall_plan_text` (~:251), `upgrades_text` (~:285), `repair_text` (~:293), `progress_line` (~:315).
- Testes: `tests/cli_offline.rs`, `tests/cli_install.rs`, `tests/cli_parse.rs` (comparam texto; continuam comparando o texto **sem** cor).
- `scripts/check_rules.ps1` (~:25): regra `Color::` fora de `src/tui/theme.rs`.
- Paleta da TUI em `src/tui/theme.rs` (use os mesmos papéis: destaque, sucesso, aviso, erro, apagado).

#### What binds this task

1. **Dependências:** `anstyle` e `anstream` em `[dependencies]` com as versões exatas que o `Cargo.lock` já tem (`grep -A1 'name = "anstyle"' Cargo.lock`). `git diff Cargo.lock` não pode acrescentar pacote.
2. **`src/cli/style.rs`:** os únicos estilos da CLI, como constantes `anstyle::Style` com nome de papel: `HEADER`, `ID`, `NAME`, `VERSION`, `NEW_VERSION`, `SOURCE`, `OK`, `WARN`, `ERROR`, `DIM`, `KEY` (teclas e flags na ajuda), `PLACEHOLDER`. Cores ANSI básicas de 16 (funcionam em qualquer tema de terminal), alinhadas aos papéis de `tui/theme.rs`. Uma função `paint(style, text) -> String` (`format!("{style}{text}{style:#}")`).
3. **Regra mecânica nova** em `check_rules.ps1`: `anstyle::` (e `AnsiColor::`) só em `src/cli/style.rs`; prove plantando um uso em `src/cli/output.rs`, vendo o gate falhar e restaurando (cole a saída). Acrescente o caso ao `check_rules_selftest.ps1`.
4. **Escolha de cor:** flag global `--color <auto|always|never>` (padrão `auto`). `auto` = cor só se o stdout for terminal e `NO_COLOR` não estiver definido; `CLICOLOR_FORCE=1` liga. Tudo que vai para a tela sai por `anstream::stdout()`/`anstream::stderr()` com `ColorChoice` da flag, que **remove** os códigos quando não há cor e converte para a API do console no Windows antigo. `--json` imprime sempre por um caminho sem estilo.
5. **Saídas:** as funções de `output.rs` passam a pintar com `style::paint` (id, nome, versão atual × nova, fonte, estado "atualizável" em `NEW_VERSION`, erros em `ERROR`, avisos em `WARN`, cabeçalhos de tabela em `HEADER`). **Alinhamento:** a largura das colunas é calculada sobre o texto sem cor (`anstream::adapter::strip_str` ou largura antes de pintar). Mensagens de erro de `main.rs` (o `anyhow` final) com prefixo `erro:` em `ERROR`.
6. **Ajudas do clap:** `#[command(styles = style::CLAP_STYLES)]` com `clap::builder::Styles` (`header`, `usage`, `literal`, `placeholder`, `error`, `valid`, `invalid`) a partir dos mesmos papéis; ajuda raiz com `after_help` em português mostrando 4–6 exemplos (`wingetrs search vlc`, `wingetrs install 7zip`, `wingetrs list --upgradable`, `wingetrs upgrade --all`, `wingetrs update`), com os comandos pintados em `KEY`. O texto das ajudas existentes não muda além disso.
7. Nada muda no texto sem cor: os testes atuais passam sem alteração (eles rodam sem terminal, logo sem cor).

**Files:**
- Create: `src/cli/style.rs`, `tests/cli_color.rs`
- Modify: `Cargo.toml`, `src/cli/mod.rs`, `src/cli/output.rs`, `src/cli/install_cmd.rs`, `src/main.rs`, `scripts/check_rules.ps1`, `scripts/check_rules_selftest.ps1`

Testes (`tests/cli_color.rs`; o binário roda por `assert_cmd`/`std::process::Command` como nos testes de CLI existentes, contra o mesmo cenário offline):
1. `piped_output_has_no_escape`: `wingetrs search <termo>` com stdout capturado ⇒ nenhum byte `0x1B`.
2. `color_always_paints`: `--color always` ⇒ contém `0x1B` e, depois de `strip_str`, é **igual** à saída sem cor do teste 1.
3. `no_color_env_wins_over_auto`: `NO_COLOR=1` sem flag ⇒ sem `0x1B` (o teste força terminal? não dá: prove pela função pura de decisão, item abaixo).
4. `color_choice_table` (teste de módulo da função pura `choose(flag, is_tty, no_color, clicolor_force) -> ColorChoice`): as 8 combinações relevantes, com `--color` vencendo as variáveis e `NO_COLOR` vencendo `auto`.
5. `json_never_colored`: `search --json --color always` ⇒ JSON válido e sem `0x1B`.
6. `table_alignment_ignores_ansi`: `search_table` com cor, depois de `strip_str`, tem as mesmas colunas da versão sem cor (compare linha a linha).
7. `help_is_styled`: `Cli::command().render_help().ansi().to_string()` contém `0x1B` e o texto sem cor contém os exemplos do `after_help`.
8. `help_plain_when_piped`: `wingetrs --help` capturado ⇒ sem `0x1B`.

#### Checks beyond the steps

1. Mutação: `choose` ignorar `NO_COLOR` ⇒ teste 4 **PEGA**.
2. Mutação: largura calculada sobre o texto pintado ⇒ teste 6 **PEGA**.
3. Mutação: imprimir o JSON pelo caminho com estilo ⇒ teste 5 **PEGA**.
4. Prova da regra nova do `check_rules.ps1` (item 3).
5. Rode o binário real num terminal (`target/release/wingetrs.exe search vlc`, `list`, `--help`) e cole no relatório a saída com `--color always` passada por `strip_str` e a contagem de `ESC`; diga o que não conseguiu ver (cores reais só o usuário vê).
6. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): colored, formatted output and help with --color and NO_COLOR`
