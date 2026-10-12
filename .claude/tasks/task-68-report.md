Status: DONE

Commits:
19117e4 feat(tui): settings tab persisted to config.toml

Gate:
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (595 passed, 16 ignored) 40s
[OK]   gate verde

Mutation proofs (all `[OK] mutação PEGA ... (exit 101)`):
1. Enter without SaveSettings: Enter arm -> `{ app.settings.cycle(true); Vec::new() }` -> enter_changes_confirm_and_requests_save
2. Ignore Err: `saved` restore replaced by current values -> failed_save_restores_the_last_saved_value
3. glyphs_forced ignored: `if false && self.glyphs_forced` -> forced_glyphs_cannot_be_changed
4. save_settings overwriting with default: `*c = Config::default(); c.behavior = ...` -> save_settings_keeps_buckets
5. Right cycles in Settings: `'h' | KeyCode::Right => change_setting(false)` -> h_cycles_backwards_and_arrows_leave_settings

Decisions:
- Existing tests tabs_cycle_and_filter_toggles, right_and_left_arrows_cycle_tabs, arrows_match_tab_and_backtab updated for the 4-tab cycle; 7 existing view snapshots regenerated (new "Ajustes" tab label only).
- set_tab no longer emits ScanInstalled for Settings. Settings tab swallows Home/End/PgUp/PgDn/Backspace/letters; Esc/Tab/BackTab/arrows fall through to common handling.
- Used CARGO_TARGET_DIR=target/t68 (debug exe locked); removed at the end.
- Option columns are not aligned across rows (the current value has ‹ › so widths differ); left as is.

Concerns: none
