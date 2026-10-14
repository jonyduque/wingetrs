Status: DONE

Commits: ver `git log --format='%h %s' 4e0d808..HEAD` (SHA no commit final da mensagem ao orquestrador).

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 2s
[OK]   clippy 3s
[OK]   test (778 passed, 18 ignored) 171s
[OK]   gate verde

Mutation proofs:
1. correlate ignora overrides (teste override_resolves_ambiguous):
[OK] mutação PEGA: 'if let Some(r) = user_choice(app, catalog, overrides)? {' -> 'if let Some(r) = user_choice(app, catalog, &BTreeMap::new())? {' fez o teste falhar (exit 101).
2. override para id inexistente vira Matched (override_to_missing_package_falls_back):
[OK] mutação PEGA: 'if catalog.get(source, package_id)?.is_none() {' -> 'if false {' fez o teste falhar (exit 101).
3. app_key usa name em vez de key (override_resolves_ambiguous, assert app_key == "arp/dup" e o lookup):
[OK] mutação PEGA: 'origin(&app.source), app.key)' -> 'origin(&app.source), app.name)' fez o teste falhar (exit 101).
4. Testes usam DataDir em tempdir: `let data = DataDir::new(dir.path());` (tests/overrides.rs) e `wingetrs::cache::DataDir::new(dir.path())` (cli_list_uses_overrides, tests/correlate.rs).

Decisions:
- `Correlation::Ambiguous` ganhou `source` (o picker precisa da fonte para gravar `Choice::Package`).
- `ListRow` ganhou `not_in_catalog: bool` alem de `app_key`, porque uma linha Unmatched nao diz se veio de NotInCatalog (menu "Desfazer").
- "Desfazer" fica como ultimo item do menu (indice 3); "Escolher correspondencia" e o 1o e desloca os demais em 1.
- Detalhe do picker (nome + publisher) vem de `app.search.entries()` (ja em memoria); vazio se o id nao estiver la.
- `cli_list_uses_overrides` roda `overrides::load` + `correlate` + `list_text` sobre catalogo sintetico (o `list` real escaneia a maquina). Arquivos fora da lista Files: examples/tui_frame.rs (literal de ListRow), tests/cli_install.rs, tests/install_ops.rs, tests/tui_view.rs (assinaturas novas). `overrides::set` devolve io::Result como `pins::set`.

Concerns: none
