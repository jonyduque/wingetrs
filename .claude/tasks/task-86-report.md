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
