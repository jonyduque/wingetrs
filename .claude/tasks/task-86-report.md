Status: DONE

Commits:
1dd1387 feat(tui): icons setting

Gate:
[OK]   fmt 1s
[OK]   clippy 15s
[OK]   test (682 passed, 16 ignored) 158s
[OK]   gate verde

Mutation proofs:
- always send icons: [OK] mutação PEGA: 'icons: s.icons_touched.then(|| s.icons.clone()),' -> 'icons: Some(s.icons.clone()),' fez o teste falhar (exit 101).
- do not clear cache: [OK] mutação PEGA: '    app.icons = IconCache::default();' -> '    let _ = IconCache::default();' fez o teste falhar (exit 101).

Decisions:
- Real names are options()/description() (brief said values/help). Ctx.icon_picker became Mutex<Option<Picker>>; run loop swaps it (and loads the icon map if needed) when app.icon_protocol changes, with no new Cmd.
- apply_icons clears icons cache, icon_in_flight and icon_none; change_setting also returns icon_cmd() to re-encode the selected icon. IconReady is ignored when protocol is Off (stale results).
- Failed save reverts icons (saved_icons) like the other settings; icons_in_flight tracks the value sent.
- Edited outside Files list: src/tui/jobs.rs (save_settings gets icons param), tests/tui_jobs.rs, existing tests (Theme now 6 Downs; Cmd literal gets icons: None), snapshot (only new Ícones line, one blank row less).

Concerns: none (auto uses env at runtime; live terminal swap not tested).

## Review round 1
- Stale IconReady: `App::icon_gen` (bumped in `apply_icons`) travels in `Cmd::EncodeIcon`/`EncodeCachedIcon` and `Msg::IconReady { generation, .. }` (`gen` is a reserved word in edition 2024, so the field is `generation`). A mismatching result returns early: not cached, not in `icon_none`, in-flight untouched. Test `icon_encoded_for_an_old_protocol_is_discarded` (tests/tui_state.rs).
- Details pane check added to `turning_icons_off_clears_the_cache_and_the_indent` (details icon cached; frame differs while on, equals the no-icon frame after the switch).
- Gate: [OK]   test (683 passed, 16 ignored) / [OK]   gate verde
- [OK] mutação PEGA: 'if generation != app.icon_gen {' -> 'if false {' fez o teste falhar (exit 101).
