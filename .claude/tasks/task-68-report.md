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

## Round 2 (review fixes)
Gate: [OK]   test (599 passed, 16 ignored) / [OK]   gate verde
Mutation proofs (all `[OK] mutação PEGA ... (exit 101)`, run with CARGO_TARGET_DIR=target/t68):
- glyphs always sent: `(s.glyphs_touched && !s.glyphs_forced)` -> `true` => forced_glyphs_are_not_written_when_another_setting_changes
- save_settings writes glyphs when None: `glyphs.or(Some("nerd"))` => save_settings_without_glyphs_keeps_the_file_value
- save on every Enter: `if app.save_in_flight.is_some()` -> `if false` => only_one_save_in_flight_and_the_newest_follows
Decisions:
- `Cmd::SaveSettings.glyphs: Option<String>`; `Some` only when the user changed Glifos this session (`SettingsState.glyphs_touched`) and not forced. This also protects an unrecognizable value in the file. Test 2 now expects `glyphs: None`.
- One save in flight (`App.save_in_flight`). Changes meanwhile only update state. At `SettingsSaved`: Ok records the sent values as `saved`; if current differs from what was sent, one new SaveSettings with the current values is emitted and no toast/revert yet (the final result decides). Err with a newer change pending does not revert: the retry carries the newest; Err with nothing newer reverts to `saved` and toasts the error.
