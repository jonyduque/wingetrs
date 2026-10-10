Status: DONE

Commits:
91fa4e7 feat(install): layout, effect traits, fakes, snapshot and pure PATH editing

Gate:
[OK]   rules 2s / [OK]   fmt 1s / [OK]   clippy 68s
[OK]   test (187 passed, 2 ignored) 109s
[OK]   gate verde

Mutation proofs:
1. [OK] mutação PEGA: '.unwrap_or(s).to_lowercase()' -> '.unwrap_or(s).to_string()' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: '.filter(|seg| norm(seg) != target)' -> '.filter(|seg| !seg.is_empty() && norm(seg) != target)' fez o teste falhar (exit 101).
3. [OK] mutação PEGA: 'out.insert(rel, "dir:".to_string());' -> '' fez o teste falhar (exit 101).

Decisions:
- Features do windows tiradas da linha Tech Stack do plano (Global Constraints não as lista).
- snapshot trata symlink/junction (reparse point) como link: hash do alvo.
- Fakes: dump() de FakeEnv inclui contagem de broadcast; FakeRegistry inclui exports.

Concerns: none

Review fix (090c970): teste de junction em snapshot (cfg(windows)); read_link agora propaga erro com ?. A mutação pedida (&0x400) sobrevivia porque is_reparse era redundante (std já reporta junction via is_symlink); removi is_reparse e mutei a condição real:
[OK] mutação PEGA: 'if ft.is_symlink() {' -> 'if ft.is_symlink() && false {' fez o teste falhar (exit 101).
Gate após o fix: clippy falhou em outros arquivos (tests/buckets, src/cli/picker.rs, de agentes paralelos); nada em src/install. Testes de install_foundation: 6 passed.
