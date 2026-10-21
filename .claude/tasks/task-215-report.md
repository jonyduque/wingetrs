Status: DONE_WITH_CONCERNS

Commits:
b348950 feat(bridge): format winget output lines in the CLI and TUI

Gate: not green by my files. Final attempts of `scripts/verify.ps1`:
- 1st run: 1855 passed, 3 failed (2 in wi_cli, 1 install_progress::elevation_notice_same_text_cli_tui), all caused by my formatter; fixed (wingetrs' own ELEVATION_GRANTED/REFUSED lines stay raw; two wi_cli expectations updated).
- 2nd run: 1857 passed, 1 failed (`wi_portable::cli_portable_prints_agreements_and_hands_the_spec_over`); passes alone afterwards, wi_portable.rs is being edited by another agent.
- 3rd run: `[OK] rules`, `[OK] rules-selftest`, `[FAIL] fmt` only in `src/cli/winget_cmd.rs:517` (another agent's file). Not repeated per the brief.
- Mine proved separately: `cargo test --quiet --test it winget_out::` => `test result: ok. 8 passed`; rustfmt --edition 2024 clean on my files; clippy --all-targets with no warnings in my paths.

Mutation proofs:
- `scripts/mutate.ps1 -Path src/bridge/winget_out.rs -Anchor 'let text = plain(line);' -Replacement 'let text = line.to_string();' -TestCommand 'cargo test --quiet --test it winget_out::ansi_and_case_ignored'`
  `[OK] mutação PEGA: 'let text = plain(line);' -> 'let text = line.to_string();' fez o teste falhar (exit 101).`
- `scripts/mutate.ps1 -Path src/cli/output.rs -Anchor 'WingetLine::Downloading { .. } => format!("  {} Baixando o instalador", g.arrow),' -Replacement 'WingetLine::Downloading { url } => format!("  {} Baixando o instalador {url}", g.arrow),' -TestCommand 'cargo test --quiet --test it winget_out::cli_hides_download_url'`
  `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`

Decisions:
- `Starting { removing: bool }` (brief: `Starting`) so the CLI can say Instalando/Removendo.
- Downloading/Starting are printed as a dim-less stdout line "  → Baixando o instalador"/"Instalando", not a new `Phase` (would touch every exhaustive `Phase` match); no spinner phase for them.
- Lines `ELEVATION_GRANTED`/`ELEVATION_REFUSED` are wingetrs' own, shown raw (not `winget:`).
- `tui_label` lives in winget_out.rs (pure); `set_bridge_line` calls classify + tui_label. `main.rs` had no `mod winget_out`; I added it.
- Found shows `g.winget` glyph + name + version; Done prints the raw winget line after the ok glyph.

Concerns:
- Edited `tests/it/wi_cli.rs` (not in Files) for 2 expectations changed by this feature ("winget: Instalando o pacote...", "→ Baixando o instalador"); those edits were swept into the other agent's commit 88f0b4c.
- `plain()` folds accents, so a Found name loses them ("Visão" -> "Visao"); Portuguese texts beyond the table stay `Other` until the live proof.
- No `.join` with external data written.

## Correções da revisão (commit fix(bridge))
- Linhas do próprio wingetrs (`output::WINGETRS_LINES`: ELEVATION_GRANTED/REFUSED, SCOPE_FALLBACK_NOTE, WAITING_INSTALLER, STARTING_WINGET, `winget_out::UPDATING_SOURCE`) saem sem `winget:`; lista fechada. `tui/jobs.rs` não foi tocado: o literal "atualizando a fonte do winget" continua lá e deveria usar `winget_out::UPDATING_SOURCE` (pendente, após a 189). Teste verifica que as constantes existentes estão na lista.
- `Done` usa `winget_out::clean` (plain). Comentário do `found` corrigido; limitação: o nome do `Found` perde acentos.
- Mutação: `[OK] mutação PEGA: 'if WINGETRS_LINES.contains(&line) {' -> 'if WINGETRS_LINES.is_empty() {' fez o teste falhar (exit 101).`
- Mutação: `[OK] mutação PEGA: 'crate::bridge::winget_out::clean(line).trim()' -> 'line.trim()' fez o teste falhar (exit 101).`
- `cargo test --quiet --test it winget_out::` => 10 passed; clippy e rustfmt sem avisos nos meus arquivos.
