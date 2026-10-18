1. Status: DONE_WITH_CONCERNS (gate vermelho so por testes de outro agente: tui_state / tui_view settings, Task 171; todos os testes de install_*/manifest_2c passam)
2. Commits:
d4b584a feat(install): typed phase 2c manifest model (hooks, installer, psmodule, innosetup, helpers)
3. Gate (ultimas linhas reais):
  test result: FAILED. 1047 passed; 2 failed; 27 ignored; 0 measured; 0 filtered out
  falhas: tui_view::settings_screen_snapshot e tui_state (glyphs/icons) - arquivos da Task 171 nao commitados. rules/fmt/clippy verdes.
4. Mutation proofs:
[OK] mutação PEGA: 'field(root, variant, "installer")' -> 'root.get("installer").filter(|v| !v.is_null())' fez o teste falhar (exit 101).
[OK] mutação PEGA: '.or_else(|| root.get(key))' -> '.xor(root.get(key))' fez o teste falhar (exit 101).
[OK] mutação PEGA: '("expand-innoarchive", Helper::Innounp)' -> '("expand-innoarchivex", Helper::Innounp)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'return Err(PlanError::Unsupported("usa innosetup: suportado na fase 2c".into()));' -> 'let _unused = 0;' fez o teste falhar (exit 101). (com '{}' o mutante nao compila por causa do #[rustfmt::skip])
5. Decisions:
- A linha do innosetup tem #[rustfmt::skip] para continuar numa linha so (rustfmt a quebrava e a ancora do brief nao casava).
- `refuse_2c` e pub; ops.rs a chama em DepWalk::visit para dependencia manter DependencyUnsupported (teste install_ops existente).
- Testes antigos em tests/it/install_manifest.rs adaptados (recusa agora e do planejador; psmodule/innosetup so da raiz).
- ManifestError::Unsupported mantido (ainda usado em formatos/hash).
6. Concerns: toquei ops.rs e tests/it/install_manifest.rs (fora da lista Files) pelos motivos acima.

## Revisão (fix commit)
- rustfmt::skip removido; mutação multi-linha com -AnchorFile/{}: [OK] mutação PEGA (innosetup).
- Fixtures de recusa (installer não objeto, keep, lista com item não texto, psmodule não objeto), field() com valor vazio na variante, refuse_2c do alvo em ops.rs: mutações PEGAM ([OK] para .filter(|v| !blank(v)), .xor, refuse_2c(&target)).
- Gate: falhou só em clippy de arquivos de outros agentes; meus módulos: manifest_2c+install_ 341 passed, clippy filtrado sem avisos nos meus caminhos.
