Status: DONE

Commits:
8080168 feat(tui): theme tokens, glyph sets and elm-style state

Gate:
[OK]   fmt 1s
[OK]   clippy 15s
[OK]   test (121 passed, 2 ignored) 73s
[OK]   gate verde

Mutation proofs (-TestCommand 'cargo test --quiet --test tui_state'):
[OK] mutação PEGA: 'KeyCode::Char('j') if empty' -> 'KeyCode::Char('j')' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !self.details.contains_key' -> 'if true || !self.details.contains_key' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'no_color.is_some_and(|v| !v.is_empty())' -> 'no_color.is_some()' fez o teste falhar (exit 101).

Decisions:
- Esc que limpa a query nao emite LoadDetails (o teste exige Vec vazio); digitar/Backspace/Enter/Ctrl+S emitem se nao ha cache.
- Tab para Installed/Updates com `installed` vazio emite ScanInstalled.
- RefreshFinished: offline = algum SourceReport Failed; status com a falha; index_age_secs = Some(0) se sem falhas.
- Ctrl+S/filtro em Installed: so linhas Matched da fonte; `#[allow(clippy::large_enum_variant)]` em Msg (interface fixa do brief).
- Tuplas de paleta com pares (rgb, indexed-256) escolhidos por mim; tema escuro/claro.

Concerns: apos Esc limpar a query, os detalhes da nova selecao so carregam ao mover/Enter. Sem rastreio de LoadDetails em voo (duplicatas possiveis).

Follow-up (revisao): +3 testes (no-color/truecolor, encolhimento da lista, filtro Available). Gate: test 125 passed, gate verde.
[OK] mutação PEGA: 'self.tab != Tab::Updates ||' -> 'true ||' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'app.installed = rows; app.clamp();' -> 'app.installed = rows;' (clamp removido em InstalledLoaded; anchor multilinha CRLF).
Flag `scanning` opcional: nao feita.
