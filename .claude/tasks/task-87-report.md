Status: DONE

Commits: (ver git log f2840ec..HEAD; um commit "feat(http): refuse https to http redirects unless allowed in settings")

Gate: [OK]   fmt / [OK]   clippy / [OK]   test (689 passed, 16 ignored) 42s / [OK]   gate verde

Mutation proofs (todas "[OK] mutação PEGA"):
- redirect_allowed -> false: PEGA (redirect_allowed_refuses_only_https_to_http)
- redirect_allowed -> true: PEGA (idem)
- attempt.follow() -> attempt.stop(): PEGA (http_redirect_is_still_followed)
- limite de 10 removido (`if false`): PEGA (redirect_loop_is_still_limited; falha por timeout de 20s do teste, não trava)
- InsecureRedirect em is_offline: PEGA (insecure_redirect_is_not_offline)
Live: `wingetrs show Notepad++.Notepad++ --source winget` -> "Nome: Notepad++"

Decisions:
- Testes existentes que navegam a aba Ajustes por contagem de Down (tui_state.rs, tui_view.rs) ganharam +1 por causa da linha nova antes de Glifos.
- Snapshot atualizado: só a linha nova entrou; a última linha em branco saiu para abrir espaço.
- Test 1 usa expected computado (allow || !(https->http)); espelha a regra mas cobre a tabela de 8 casos.

Concerns: none
