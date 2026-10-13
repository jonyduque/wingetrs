Status: DONE_WITH_CONCERNS

Commits:
594e8e8 feat(tui): cached package icons in the list

Gate:
[OK]   rules 4s
[OK]   rules-selftest 10s
[OK]   fmt 1s
[OK]   clippy 32s
[OK]   test (679 passed, 16 ignored) 206s
[OK]   gate verde

Mutation proofs:
- Alinhamento (recuo so com icone): [OK] mutação PEGA: 'let text_area = if icons_on {' -> 'let text_area = if false {' fez o teste falhar (exit 101).
- So visiveis: [OK] mutação PEGA: 'self.page(),' -> 'usize::MAX / 4,' fez o teste falhar (exit 101).
- Teto de 8: [OK] mutação PEGA: 'if self.icon_in_flight.len() >= MAX_ICON_JOBS {' -> 'if false {' fez o teste falhar (exit 101).

Frame measurement (release, TestBackend 120x40, Pesquisar, 200 frames/series, machine otherwise idle but noisy):
- Base (c54332a, existing "search" scenario = icons off), median us per run: 358 362 278 266 361 492 323 288 318 284 (pooled median ~320, p95 460-860).
- New, icons off (3 series x 4 runs), median us: 281 341 256 388 345 322 425 428 405 284 286 328 (pooled median ~334; min 256 vs base min 266).
- New, icons on (test protocol reserving the area, every visible row has a cached icon), median us: 382 292 327 302 344 273 384 386 292 296 302 307 (pooled ~302-307), p95 430-675.
- Off vs base: delta of pooled medians ~ +0.01 ms, inside the run-to-run spread (+-0.1 ms); no regression above +0.05 ms detected, but the noise is larger than the limit, so the proof is "not distinguishable from the base", not a measured zero.
- Real terminal (sixel/iTerm2 write cost): nao medido. User command: `cargo run --release` in a Sixel/iTerm2 terminal with icons cached.
- Example: `tui_frame.exe frames` prints the 3 off + 3 on series.

Decisions:
- update() is now a wrapper: apply() + list_icon_cmds(); keeps the "Cmd per update" model (no view-driven side effects). New Cmd::EncodeCachedIcon{source,id,sha,size} (blob read + encode in one spawn_blocking, no http; a test scans its body), Msg::IconMapLoaded (startup, spawn_job) and Msg::IconMapped (after remember).
- icon_in_flight is now keyed by (key, size) so a list encode (4x2) and a details encode (8x4) of the same package do not block each other; cap of 8 counts all in-flight encodes.
- The visible window is computed in app.rs with the same page formula as clamp (new App::page); view.rs draws with its own measured window.
- Existing test details_without_icon_keep_the_old_layout changed: with the protocol on the list now indents (required by the brief), so that case compares only the details pane.

Concerns:
- Window in app (size-based) can differ by an item from view's measured one on odd layouts (narrow terminal where details are hidden still uses same height); an icon missed then appears once the selection moves.
- Disk filled during work (C: 100%); I deleted my own target/t85/debug. Other agents' target/t79..t84, review, orch dirs are large and stale; not touched.
- Stray python/bash processes from a bad heredoc of mine were killed.

Review fix (page height, one formula):
- app::items_in_rows + app::list_page_rows(screen_height) are the only page formulas; the view calls items_in_rows on its real area, App::page calls list_page_rows. MIN_HEIGHT_FOR_CHROME is now pub.
- Test list_page_rows_matches_the_items_the_view_draws (heights 1..=60) compares list_page_rows with items drawn on a TestBackend.
- Mutation: [OK] mutação PEGA: 'if screen_height < crate::tui::view::MIN_HEIGHT_FOR_CHROME {' -> 'if false {' fez o teste falhar (exit 101); failing: list_page_rows_matches_the_items_the_view_draws ("height 4").
- Gate: [OK]   test (680 passed, 16 ignored) / [OK]   gate verde
