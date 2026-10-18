Status: DONE_WITH_CONCERNS (gate not green only because of other agents' in-flight files)

Commits:
47b5635 feat(winget-install): installer selection (arch, scope, locale, refusals)

Gate: verify.ps1 final lines (other agents' files broke the `it` build, tests/it/wi_launch.rs unresolved imports from launch.rs):
[OK]   rules 5s
[OK]   rules-selftest 16s
[OK]   fmt 2s
[FAIL] clippy exit 101   (error: could not compile `wingetrs` (test "it") due to 1 previous error; 0 hits in target/verify.log for select.rs/host_win/wi_select)
Own proof: `cargo test --quiet --test it wi_select::` -> test result: ok. 13 passed; 0 failed; 1 ignored. Earlier `cargo clippy --all-targets` filtered by my paths: no output.

Mutation proofs (cargo test --quiet --test it wi_select::):
[OK] mutação PEGA: '"x64" => &["x64", "x86"],' -> '"x64" => &["x64"],' fez o teste falhar (exit 101).   (native_arch_beats_earlier_fallback)
[OK] mutação PEGA: '            scope_rank(&i.scope, pref),...' (arch before scope, -AnchorFile) fez o teste falhar   (scope_beats_arch fails)
[OK] mutação PEGA: 'Some(min) if min <= host.os => {}' -> text comparison of the version string, fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if let Some(d) = &i.dependencies' -> '&None::<..Dependencies>' fez o teste falhar (exit 101).

Live: cargo test --release --quiet --test it host_current_prints -- --ignored --nocapture
Host { arch: "x64", os: [10, 0, 26220, 9606], locale: "pt-BR" }
test result: ok. 1 passed

Decisions:
- Extra tests beyond the brief: no-arch refusal text, unspecified locale beats other language, bad/short MinimumOSVersion.
- Refusal for a platform-only filter (no Windows.Desktop installer) is "nenhum instalador para a plataforma Windows.Desktop" (brief listed no text); `exige Windows X` shows the lowest turned-away version.
- Neutral ranks right after native; fallbacks after it (x64 host: x64, neutral, x86).
- Dependencies count any of the four lists non-empty.

Concerns: gate not run green end-to-end due to others' in-flight work; orquestrador should rerun.

## Review fix (df61fd2)
- Order now ports ManifestComparator.cpp:766-775/831-895 (pairwise strong/weak; locale > scope > arch). None of the original 13 tests needed changing (scope_beats_arch still holds: opposite scope is strong). Added: winget_order_locale_then_scope_then_arch, neutral_*, min_os_equal..., platform_refusal..., unreadable_min_os..., each_dependency_list..., unknown_named_type...
- Own refusal "versão mínima do Windows inválida no manifesto"; Windows.Desktop case-insensitive; host arch via IsWow64Process2 (native machine), falls back to bridge::host_arch (left unchanged; tui/run.rs uses it).
- Cargo.toml: added windows feature Win32_System_SystemInformation (required by IsWow64Process2/IMAGE_FILE_MACHINE); Cargo.lock unchanged.
- Mutations [OK] PEGA: `return Some(1);`->`return None;`; `min <= host.os`->`<`; scope weak->strong. Before this fix: wi_select 20 passed, 1 ignored. Live: Host { arch: "x64", os: [10, 0, 26220, 9606], locale: "pt-BR" }.
- Gate: rules/selftest/fmt OK; clippy failed only on others' `it` build (0 hits in my files); later an E0277 from another agent's file blocks the `it` build.
