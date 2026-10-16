1. Status: DONE
2. Commits:
cd29433 feat(bridge): tell the user when winget asks for a reboot
74f6667 test(bridge): every reboot marker matches, plain and wrapped in color codes
3. Gate:
[OK]   rules 6s / rules-selftest 17s / fmt 3s / clippy 57s
[OK]   test (863 passed, 22 ignored) 240s
[OK]   gate verde
4. Mutation proofs:
scripts/mutate.ps1 -Path src/bridge/mod.rs -Anchor 'markers.iter().any(|m| l.contains(m.as_str()))' -Replacement 'false' -TestCommand 'cargo test --quiet --test bridge reboot_marker'
[OK] mutação PEGA: 'markers.iter().any(|m| l.contains(m.as_str()))' -> 'false' fez o teste falhar (exit 101).
5. Decisions:
- Opção (A): sem reboot_win.rs/RebootSignals, teste 4 pulado. REBOOT_MARKERS = 11 textos do relatório 113.
- reboot_requested normaliza os dois lados (remove ESC[..letra, dobra acentos latinos); testado com VT e sem acento.
- No jobs.rs o ramo de reboot vem antes de winget_updated; frase vai ao log via tracing::info.
6. Concerns: none (git status mostra mudanças de outros agentes em outros arquivos; não toquei).

Mutation (review): Russian marker -> 'Перезапустите компьютер ZZZ' with `cargo test --quiet --test bridge every_reboot_marker`:
[OK] mutação PEGA: '"Перезапустите компьютер, чтобы завершить установку.",' -> '"Перезапустите компьютер ZZZ",' fez o teste falhar (exit 101).
