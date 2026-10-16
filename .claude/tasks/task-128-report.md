Status: DONE_WITH_CONCERNS

Commits:
cc19bdc feat(install): install 7z, MSI, tar.xz and nupkg apps from Scoop

Gate:
[OK]   fmt 8s
[OK]   clippy 6s
[OK]   test (962 passed, 23 ignored) 107s
[OK]   gate verde

(An earlier gate run in the same session went red only on `cli_parse::help_snapshots` — the `--color` flag of Task 132, in progress in the same checkout; after Task 132 updated its snapshot, `cargo test --all-targets --no-fail-fast` exited 0 and the gate above is green.)

Mutation proofs:
1. Extraction without `spawn_blocking` => test 6 PEGA
   `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'let job = tokio::task::spawn_blocking(work);' -Replacement 'let job = std::future::ready(Ok::<_, tokio::task::JoinError>(work()));' -TestCommand 'cargo test --quiet --test install_executor extract_runs_off_the_runtime'`
   `[OK] mutação PEGA: 'let job = tokio::task::spawn_blocking(work);' -> 'let job = std::future::ready(Ok::<_, tokio::task::JoinError>(work()));' fez o teste falhar (exit 101).`
   Failing test: `extract_runs_off_the_runtime --- FAILED` ... `message: "arquivo inválido: runtime parado durante a extração"`
2. `.7z` back to `supported() == false` => test 1 PEGA
   `pwsh -File scripts/mutate.ps1 -Path src/install/archive.rs -Anchor '| ArchiveKind::SevenZip' -Replacement '| ArchiveKind::Zip' -TestCommand 'cargo test --quiet --test install_planner sevenzip_manifest_resolves'`
   `[OK] mutação PEGA: '| ArchiveKind::SevenZip' -> '| ArchiveKind::Zip' fez o teste falhar (exit 101).`
   Failing test: `sevenzip_manifest_resolves --- FAILED`
3. (extra, rule 6) TUI hook ignores the extraction flag => test 7 PEGA
   `pwsh -File scripts/mutate.ps1 -Path src/tui/run.rs -Anchor 'if crate::install::executor::in_decoder() {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test install_executor decoder_panic_rolls_back'`
   `[OK] mutação PEGA: 'if crate::install::executor::in_decoder() {' -> 'if false {' fez o teste falhar (exit 101).`
   Failing assertion: `o hook restaurou o terminal`
4. (extra, rule 6) executor does not set the flag on the extraction thread => test 7 PEGA
   `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'let _flag = DecoderFlag::set();' -Replacement 'let _flag = ();' -TestCommand 'cargo test --quiet --test install_executor decoder_panic_rolls_back'`
   `[OK] mutação PEGA: 'let _flag = DecoderFlag::set();' -> 'let _flag = ();' fez o teste falhar (exit 101).`
   Failing assertion: `o hook restaurou o terminal`
5. Snapshots: `git diff --stat tests/snapshots` printed nothing before the commit; the only snapshot changes are the two new untracked files `install_planner__sevenzip__main__cdrtools.snap` and `install_planner__msi__main__7zip19.00-helper.snap`.

Decisions:
- Injection seam: `#[doc(hidden)] pub async fn execute_install_with(plan, ctx, extract: ExtractFn)` (fn pointer, `'static`, fits `spawn_blocking`); `execute_install` passes `extract::extract`. I chose this over a new `ExecCtx` field so `ops.rs` is untouched. Test 7 injects a fake decoder that panics inside `catch_unwind` on the extraction thread, the same pattern the real decoders use. The real decoders' reader injection (`extract_7z`) is `pub(super)`, and exposing it would mean editing extract.rs (contested by Task 126).
- Flag: a `thread_local` `IN_DECODER` in executor.rs, set by a Drop guard for the whole extraction closure on the blocking thread. The new `tui::run::install_panic_hook(restore)` only logs (`tracing::error!`) when it is set, and otherwise calls `restore` (mouse off plus the ratatui and default hook, as before).
- Test 6 does not depend on how long extraction takes: the injected extractor runs the real `extract::extract` on a 32 MiB 7z (COPY), then waits up to 10 s for the current_thread runtime to tick twice. On the runtime thread the ticker can't run, so the test fails and does not hang.
- Engine label lives in `ArchiveKind::engine()` (one source of truth) and is shown as `Extrair [1] <file> (<engine>) para ...`.
- The fixtures `unsup-7z__main__cdrtools.json` / `unsup-msi__main__7zip19.00-helper.json` now resolve. I renamed them to `sevenzip__…`/`msi__…` (git mv), which produces the new insta snapshots through `display_snapshots_for_every_supported_fixture`.

Concerns:
- Files outside the list: `tests/install_manifest.rs` (removed the 7z/msi "unsupported" assertions; the renamed-file case now uses `.rar`), `tests/install_ops.rs` (the unsupported-dependency test now uses `unsup-script__main__7zip.json`, because cdrtools is now supported), and the two renamed fixtures. All are needed because 7z/MSI became supported. extract.rs was not edited.
- The flag covers the whole extraction closure, not only the decoder `catch_unwind` sites (those are in extract_7z.rs/extract_msi.rs/sniff.rs, outside the list). If our own extraction code panics outside a decoder, the hook only logs it, and `resume_unwind` then carries it to the TUI's outer `catch_unwind`, which restores the terminal. The panic message goes to the log, not to stderr.
- `manifest.rs`'s `Ok(k) if !k.supported()` arm ("suportado na fase 2b") is now unreachable, because every kind `of` accepts is supported. I left it in place because `supported()` stays the gate.
- If `execute_install`'s future were dropped mid-extraction, the blocking task would keep running and nothing would await it. I found no caller that cancels it, but I did not check this exhaustively.
- Extraction time is logged (`ms`), not measured: no benchmark was run.

## Review round 1

Commits:
9e714e1 fix(install): roll back before re-raising an extraction panic; decoder flag only inside decoders
(`git log --format='%h %s' 3b3c2f4..HEAD -- <files>` also lists 4278e5a, which is Task 126's commit on extract.rs and not mine.)

Gate:
[OK]   fmt 2s
[OK]   clippy 31s
[OK]   test (967 passed, 23 ignored) 363s
[OK]   gate verde

Fixes:
1. A worker panic is now stored in `Run::panic`, and the step returns an error. `execute_install` rolls back first and then calls `resume_unwind`. New test `extraction_worker_panic_rolls_back_then_propagates` uses an injected extractor that panics outside any guard: the panic reaches the test (`futures_util` `catch_unwind`) with its payload, and the root listing, the snapshot and the state equal what they were before the install.
2. `IN_DECODER` moved to `extract.rs`, with `pub fn in_decoder()` and `pub fn catch_decoder(f)` (RAII guard that restores the previous value). It is used at all 4 decoder sites: 7z, MSI, xz (`extract_tar_guarded`) and sniff. The executor no longer sets the flag, so a panic in our own code reaches stderr. New unit test `extract_7z::tests::decoder_panic_happens_with_the_flag_set`: a reader under the real 7z decoder records `in_decoder()` at the moment it panics (`Some(true)`), and the flag is cleared afterwards. The executor test's fake decoder now panics inside `catch_decoder`.
3. The `Ok(k) if !k.supported()` arm in manifest.rs stays, with a comment: it is the gate for a kind added before its extractor exists, and `manifest_and_extractor_agree` keeps the two in step. The stale "fase 2b" message is now "formato ainda não suportado".
4. Added a comment on the process-wide hook swap in `decoder_panic_rolls_back`. `extract_runs_off_the_runtime` now uses the small `fixture_7z()`; the tick wait is what proves the extraction left the runtime thread, so the payload size does not matter.

Mutation proofs:
- `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'run.panic = Some(p);' -Replacement 'std::panic::resume_unwind(p);' -TestCommand 'cargo test --quiet --test install_executor extraction_worker_panic_rolls_back_then_propagates'`
  `[OK] mutação PEGA: 'run.panic = Some(p);' -> 'std::panic::resume_unwind(p);' fez o teste falhar (exit 101).` (failed at the `root_list` assertion)
- `pwsh -File scripts/mutate.ps1 -Path src/install/extract.rs -Anchor 'let _restore = Restore(IN_DECODER.with(|c| c.replace(true)));' -Replacement 'let _restore = Restore(IN_DECODER.with(Cell::get));' -TestCommand 'cargo test --quiet --lib decoder_panic_happens_with_the_flag_set'`
  `[OK] mutação PEGA: ... fez o teste falhar (exit 101).` (`install::extract_7z::tests::decoder_panic_happens_with_the_flag_set --- FAILED`)
- The same mutation with `-TestCommand 'cargo test --quiet --test install_executor decoder_panic_rolls_back'`
  `[OK] mutação PEGA: ... fez o teste falhar (exit 101).` (`o hook restaurou o terminal`)

Concerns: none new. The Round-0 concern about the flag covering our own code is resolved by fix 2.
