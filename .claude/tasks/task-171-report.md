Status: DONE_WITH_CONCERNS

Commits:
b3b1825 feat(config): winget engine and default scope settings

Gate: full gate NOT green because of other agents' files (clippy/test errors in script_runner.rs, install_manifest, uninstall_cmd, classify_2c at the time). Per coordinator rule, proved own files instead:
- cargo test --quiet --test it tui_  -> test result: ok. 341 passed; 0 failed
- cargo test --quiet --test it wi_settings::  -> ok. 7 passed
- cargo test --quiet --test it config::  -> ok. 15 passed
- cargo clippy --all-targets filtered by config.rs/settings.rs/wi_settings/tui_state/tui_view: no diagnostics. rules/rules-selftest/fmt were [OK] in verify.

Mutation proofs:
[OK] mutação PEGA: 'self.behavior.winget_engine = [WingetEngine::Wingetrs, WingetEngine::Winget][next];' -> 'self.behavior.default_scope = [DefaultScope::User, DefaultScope::Machine][next];' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'DefaultScope::User => 0,' -> 'DefaultScope::User => 0, DefaultScope::Machine => 0,' fez o teste falhar (exit 101).
TUI real: não verificado na TUI. Snapshot novo: linhas "Motor do winget  ‹ wingetrs ›  winget" e "Escopo padrão (winget)  ‹ usuário ›  máquina".

Decisions:
- Snapshot changed: tests/it/snapshots/it__tui_view__settings_screen_snapshot.snap only (diff = 2 new rows + header metadata). Render height raised 24->26 in settings_screen_snapshot so the "Fontes" rows are not pushed off screen.
- Existing tests with hardcoded Down counts bumped by 2 (tui_state.rs: glyphs/icons/list_icons/theme tests; tui_view.rs turning_icons_off), config.rs roundtrip Behavior literal got the two new fields.
- wi_settings.rs got #![allow(clippy::unwrap_used, clippy::expect_used)] like config.rs.

Concerns: the 15 earlier tui_state/tui_view failures were from index shifts and are fixed; install_manifest::* failures seen in the full run belong to other agents. Files outside the Files list touched: tests/it/config.rs, tui_state.rs, tui_view.rs (needed by ORDER shift / new fields).

## Review fix
Commit: fix(tui): review of task 171 (see git log). Settings body now scrolls (regions::settings_scroll, shared by view and mouse rows; last setting reveals the Fontes section); new 24-line snapshot with selection last, 26-line one kept. Down loops replaced by select_setting/ORDER position. Invalid winget_engine/default_scope fall back to default with tracing::warn (wrong type still a parse error); tests for both fields. Gate: clippy failed only in another agent's file (see target/verify.log); own modules: tui_ 342 ok, wi_settings 9 ok, config 15 ok, clippy filtered clean.
