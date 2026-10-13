Status: DONE_WITH_CONCERNS

Commits:
```
01f369b feat(tui): package icon in the details pane
```

Gate (`CARGO_TARGET_DIR=target/t84 pwsh -File scripts/verify.ps1`, after the last change):
```
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 9s
[OK]   test (670 passed, 16 ignored) 287s
[OK]   gate verde
```

Cargo.lock: `cargo build` with no `cargo update`. ratatui stays 0.30.2, crossterm stays 0.29.0, and no existing crate changed version (nothing was removed). Crates added: adler2 2.0.1, base64-simd 0.8.0, bitvec 1.1.1, bytemuck_derive 1.12.1, byteorder-lite 0.1.0, color_quant 1.1.0, fdeflate 0.3.7, funty 2.0.0, gif 0.14.2, icy_sixel 0.5.1, image 0.25.10, linux-raw-sys 0.4.15, miniz_oxide 0.8.9, miniz_oxide 0.9.1, moxcms 0.8.1, ordered-float 5.5.0, outref 0.5.2, png 0.18.1, ppv-lite86 0.2.21, pxfm 0.1.30, quantette 0.6.0, radium 0.7.0, rand_chacha 0.3.1, rand_xoshiro 0.8.1, ratatui-image 11.1.0, ref-cast 1.0.27, ref-cast-impl 1.0.27, rustix 0.38.44, safe_arch 1.2.0, self_cell 1.3.0, tap 1.0.1, vsimd 0.8.0, weezl 0.1.12, wide 1.7.1, windows 0.58.0 (+ windows-core/implement/interface 0.58.0, windows-result 0.2.0, windows-strings 0.1.0), wyz 0.5.1, zerocopy 0.8.62, zerocopy-derive 0.8.62, zune-core 0.5.3, zune-jpeg 0.5.15.
Features used: `ratatui-image = { version = "=11.1.0", default-features = false, features = ["crossterm"] }`. In 11.1.0, iterm2 and sixel have no feature flag of their own (sixel comes through the `icy_sixel` dependency, which is always on). `image = "0.25.10"` is the version that ratatui-image 11.1.0 resolves to (the probe's lock shows the same).

Mutation proofs (all run with `CARGO_TARGET_DIR=target/t84`):
1. Gap ignored in `split_icon`:
   `scripts/mutate.ps1 -Path src/tui/layout.rs -Anchor '        .saturating_add(gap)\n' -Replacement '' -TestCommand 'cargo test --quiet --offline --test tui_view -- split_icon_leaves_a_gap details_text_starts_after_the_icon_gap'`
   `[OK] mutação PEGA: '        .saturating_add(gap)' -> '' fez o teste falhar (exit 101).` (both failed: `split_icon_leaves_a_gap --- FAILED` and `details_text_starts_after_the_icon_gap --- FAILED`)
2. `auto` always `Sixel` in `choose`:
   `-Path src/tui/icons.rs -Anchor '            } else if wt_session {\n                IconProtocol::Sixel\n            } else {\n                IconProtocol::Off\n            }' -Replacement '            } else {\n                IconProtocol::Sixel\n            }' -TestCommand 'cargo test --quiet --offline --lib choose_protocol'` → `[OK] mutação PEGA: ...`
   A second variant replaced the whole `auto` branch with `IconProtocol::Sixel` → `[OK] mutação PEGA` (`tui::icons::tests::choose_protocol --- FAILED`, `auto Some("WezTerm") false`).
3. Icon drawn with a dialog open:
   `-Path src/tui/view.rs -Anchor 'app.modal.is_none() && !app.show_help && ' -Replacement '!app.show_help && ' -TestCommand 'cargo test --quiet --offline --test tui_view no_icon_over_a_dialog'`
   `[OK] mutação PEGA: 'app.modal.is_none() && !app.show_help && ' -> '!app.show_help && ' fez o teste falhar (exit 101).` (`assertion failed: !icon_visible(&m)`)
4. `grep -n "load_from_memory\|new_protocol" src/tui/view.rs`: no output (exit 1).
5. `grep -rn "from_query_stdio" src`: no output (exit 1). I reworded a doc comment in icons.rs that mentioned the name.

Extra mutation proofs, all with a `[OK] mutação PEGA` line:
- Toast check removed in `view` (`&& !toasts.iter().any(|t| t.intersects(rect))` → ''): `icon_crossing_a_toast_is_not_drawn --- FAILED`.
- Not remembering an undecodable icon (`app.icon_none.insert(key);` → ''): `undecodable_icon_is_remembered_as_none --- FAILED`.
- Cache evicting the newest (`pop_front` → `pop_back`): `icon_cache_keeps_the_newest_64 --- FAILED`.
- `Off` check removed from `icon_cmd`: `[OK] mutação PEGA` from `icon_encoding_is_requested_once_for_the_selected_package`.

Decisions:
- Encoding starts from `IconLoaded` and from every selection change (`details_cmd` → `icon_cmd`), and only for the selected key. A key gets one `Cmd::EncodeIcon` until it is cached, in flight, or in `icon_none`. `Msg::IconReady.proto` is `Result<Protocol, String>`; the worker logs the failure with `tracing::debug!`.
- Encoding happens in `spawn_icon` (run.rs): `spawn_blocking`, then `join_icon` awaits the handle. A panic becomes `Msg::Panicked`, which `jobs::propagate` re-raises with `resume_unwind`. I did not reuse `join_job` because it turns a cancellation into `Msg::Planned(Err)`.
- The icon is drawn last in `view`, after the toasts, and is skipped if it crosses any toast rectangle. The area stays reserved so the text does not jump when a toast shows up or goes away. `draw_toasts` now returns the rectangles it drew, so `src/tui/toast.rs` changed although it is not in **Files**. A modal or the help screen turns off both the reservation and the drawing (`icon_visible`).
- `Config::load` changes an unknown `icons` value to `"auto"` and logs `tracing::warn!`. `choose` also treats any unknown value as `auto`.
- The view tests use a red halfblocks `Protocol` (from `Picker::halfblocks()`) and find the icon by red background cells, because halfblocks writes ' ' for an opaque block. There is no `IconSlot` in production code.

Concerns:
- The "area narrower than `DETAIL_ICON_COLS + DETAIL_ICON_GAP + 10`" fallback cannot be reached with today's layout: the smallest terminal that shows details (60 columns) gives a 23-column details area. The fallback is in the code but no test covers it, and a mutation of that threshold would be equivalent.
- `font_size()` uses `crossterm::terminal::window_size()`. On Windows that probably fails and falls back to (10, 20). Not measured on a real terminal; no WezTerm or WT session was used in this task.
- `Picker::from_fontsize` is deprecated in 11.1.0 (`#[allow(deprecated)]` with a comment). It still reads `TERM`/`TERM_PROGRAM` to detect tmux, and inside tmux it runs `tmux set -p allow-passthrough on`. That only happens when TERM starts with "tmux".
- TDD order: the implementation of config/icons/layout was written before some of its tests ran red. Failure of each rule is shown by the mutations above instead.

## Review fixes

Commit: `1db3421 fix(tui): bound icon decoding and wrap the identity lines beside the icon`

Gate (`CARGO_TARGET_DIR=target/t84`):
```
[OK]   rules 6s
[OK]   rules-selftest 11s
[OK]   fmt 1s
[OK]   clippy 45s
[OK]   test (675 passed, 16 ignored) 227s
[OK]   gate verde
```

1. Bounded decode. `icons::decode` uses `ImageReader::new(Cursor).with_guessed_format()` and then `reader.limits(..)` (the image 0.25.10 name for set_limits), with width and height ≤ 1024 and `max_alloc` 64 MiB. Errors come back as `Err(String)`.
   - Test `oversized_icons_are_refused`:
     - a valid 1025x2 PNG and a 2x1025 PNG fail. Both decode without the limits, so the mutation is caught.
     - a hand-made IHDR declaring 20000x20000 fails.
     - a 256x256 PNG, a 1024x2 PNG and a 256x256 ICO decode `Ok`.
   - `[OK] mutação PEGA: '    reader.limits(limits);' -> '' fez o teste falhar (exit 101).` (`tui::icons::tests::oversized_icons_are_refused --- FAILED`)
2. Identity lines beside the icon.
   - `details_lines` now also returns how many of its first lines identify the package (name, ID, publisher, version). Those lines are wrapped in the text column. They stay there while their wrapped rows fit in `DETAIL_ICON_ROWS`; from the first one that does not fit, every remaining line goes below at full width, wrapped as before.
   - Rows are counted by `wrapped_rows`, which renders the line with the same `Paragraph` + `Wrap { trim: true }` into a scratch buffer. I did not use `Paragraph::line_count`, because it needs ratatui's `unstable-rendered-line-info` feature, a change to Cargo.toml.
   - Tests:
     - `long_id_beside_the_icon_wraps_in_full`: the cells of the text column, concatenated, contain the 49-character ID.
     - `description_without_publisher_and_version_goes_below_at_full_width`: Installed tab, no publisher, no installed version.
     - `icon_layout_draws_the_same_words`: the sorted words of the pane are equal with and without the icon, on the Search tab and on the Installed tab.
   - Mutations:
     - `[OK] mutação PEGA: '    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text);' -> '    frame.render_widget(Paragraph::new(lines), text);'` (`long_id_beside_the_icon_wraps_in_full --- FAILED`)
     - `[OK] mutação PEGA: '    let beside = lines[..identity]' -> '    let beside = lines[..lines.len().min(4)]'` (`description_without_publisher_and_version_goes_below_at_full_width --- FAILED`)
     - `[OK] mutação PEGA: '    let rest = lines.split_off(beside);' -> '    let rest = lines[beside.min(1)..].to_vec();'` (`icon_layout_draws_the_same_words --- FAILED`, duplicated words)
   - I changed my own `details_text_starts_after_the_icon_gap`: the description now starts at full width on row y+4, because it is no longer an identity line.
3. `icon_evicted_from_the_cache_is_encoded_again`: the icon is encoded, then pushed out by 64 others, and is requested again on reselection.
   - `[OK] mutação PEGA: '            app.icon_in_flight.remove(&key);' -> '' fez o teste falhar (exit 101).` (`icon_evicted_from_the_cache_is_encoded_again --- FAILED`)
4. The narrow-area fallback stays as a guard, with a one-line comment saying it cannot be reached today.
