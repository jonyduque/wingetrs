1. Status: DONE_WITH_CONCERNS

2. Commits:
5ed81ec feat(cli): short install plans and phase progress like the TUI

3. Gate:
[OK]   rules 4s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[OK]   clippy 26s
[OK]   test (1062 passed, 27 ignored) 59s
[OK]   gate verde

4. Mutation proofs:
- INSTALLER_RUNNING tratado como URL (`if label == INSTALLER_RUNNING` -> `if label == "__never__"`), `cargo test --quiet --test it cli_plan_summary::`:
  [OK] mutação PEGA: 'if label == crate::install::executor::INSTALLER_RUNNING {' -> 'if label == "__never__" {' fez o teste falhar (exit 101).  (testes 6 e 7)
- Esconder EnvAddPath (`Step::EnvAddPath { entry } if entry.as_os_str().is_empty() => ...`):
  [OK] mutação PEGA: 'Step::EnvAddPath { entry } => lines.push(format!(' -> 'Step::EnvAddPath { entry } if entry.as_os_str().is_empty() => lines.push(format!(' fez o teste falhar (exit 101).  (teste 2)
  (a 1ª tentativa de mutante não compilava e foi descartada como INCONCLUSIVO.)

Execução real (release, raiz real, main/fastfetch; desinstalado no fim; `list` não mostra mais fastfetch).
`install --dry-run`: plano completo (Display), 8 linhas, inalterado.
`install main/fastfetch --yes 2>&1 | cat -v` (stderr não é TTY aqui):
  Instalar fastfetch 2.70.0  main · 64bit
    1. Comandos: fastfetch, flashfetch
    2. PATH do usuário <seta nerd> C:\Users\jonyd\AppData\Local\wingetrs\shims
  Instalado: fastfetch 2.70.0
  Abra um terminal novo para usar: fastfetch, flashfetch
`uninstall fastfetch --yes`:
  Desinstalar fastfetch 2.70.0
    1. Apaga a pasta do app
    2. Comandos: fastfetch, flashfetch
    3. PATH do usuário <seta nerd> remove C:\Users\jonyd\AppData\Local\wingetrs\shims
  Desinstalado: fastfetch 2.70.0
Animação em TTY: não observada (o shell do agente não tem TTY); sem TTY nenhuma linha de progresso é escrita (verificado acima). A lógica de fases está coberta por testes puros (`Phases`).

5. Decisions:
- `Ui` ganhou `glyphs()` (padrão ascii) e `end_progress()` (padrão vazio); `TtyUi` guarda o `Arc<LineProgress>` e `run_win` o pega com `ui.progress()`. Necessário para parar a thread de animação antes de imprimir "Instalado:" (senão a animação redesenharia por cima).
- `uninstall_plan_text` ganhou o parâmetro `glyphs` (seta); testes de `install_state.rs` que fixavam o texto antigo foram atualizados. `progress_line` removida (substituída por `Phases::line`).
- Glifo novo `Glyphs::arrow` (nerd f061, unicode →, ascii ->). Número em negrito usa `style::NAME` (já negrito, sem estilo novo).
- Etapa "Instalando…" = todos os downloads com `done >= total` conhecido; download sem total nunca entra nessa fase. Cada `INSTALLER_RUNNING` reinicia o relógio mm:ss; um novo download volta à fase de download.
- `Abra um terminal novo…` só se o plano tem shim e alguma pasta (shims/EnvAddPath) falta no PATH do processo (comparação sem caixa e sem barra final).

6. Concerns:
- Animação/TTY não verificada visualmente.
- `tests/it/cli_cancel.rs` aparece modificado no checkout (não é meu; não incluído no commit).
- Arquivos do repo estão com CRLF no working copy (autocrlf); `cargo fmt` rodou limpo.
