Status: DONE_WITH_CONCERNS

Commits:
f048f21 feat: WINGETRS_TRANSCRIPT records CLI and TUI text for the live proof
(`git log --format='%h %s' cefa945..HEAD -- <Files>` also lists 5b5ca04, a Task 219 commit that touched `src/cli/mod.rs`; not mine.)

Gate:
[OK]   rules 9s
[OK]   rules-selftest 47s
[OK]   fmt 3s
[OK]   licenses 12s
[OK]   clippy 52s
[OK]   test (2040 passed, 89 ignored) 71s
[OK]   gate verde

Mutation proofs (all with `cargo test --quiet --test it transcript::`):
1. strip_ansi dropped: `[OK] mutação PEGA: 'let plain = strip_ansi(text);' -> 'let plain = text.to_string();' fez o teste falhar (exit 101).` (tests 1 and 5 failed)
2. Flush per line: NOT PROVED. `File` is unbuffered, so removing `flush()` changes nothing; the brief's mutant (a `BufWriter`) is a different design, not an anchor edit. Test 1 reads the file before `with_file` ends, so it would catch a buffered writer.
3. append -> truncate (`with_file`): `[OK] mutação PEGA: 'let file = OpenOptions::new().create(true).append(true).open(path);' -> '...write(true).truncate(true)...' fez o teste falhar (exit 101).`
4. transcript_path without BridgeWorker (after fmt, `-AnchorFile`): `[OK] mutação PEGA: '        Some(\n            Command::BridgeWorker { .. }\n            | Command::Update {' -> ... fez o teste falhar (exit 101).` Order check: the worker `return` is `src/cli/mod.rs:1209`, `open_transcript(&cli)` is `:1220`, and `HEAD:src/cli/mod.rs` has no `transcript` yet.
5. phase_text with clock instead of phase_title: `[OK] mutação PEGA: 'Some((phase, _)) => phase_title(phase),' -> 'Some((phase, since)) => ...phase_text(...)' fez o teste falhar (exit 101).`
6. Warning printed twice: `[OK] mutação PEGA: 'emit::err_with(|| output::warning(&output::transcript_failed(&path, &e)));' -> 'for _ in 0..2 { ... }' fez o teste falhar (exit 101).`
7. First lines of the test 5 transcript (`update --force --color always`; only 6 lines exist):
   Preparando…
   Baixando os catálogos (3 fontes)…
   Gravando o catálogo…
   ✓ winget: 20 pacotes (0 ignorados)
   ✓ scoop:main: 8 pacotes (1 ignorados)
   ✗ scoop:extras: falhou: HTTP 500

Decisions:
- `picker::yes_no` also writes its "{pergunta} Sim/Não" line to the transcript (test 10 requires it; the brief only cited the `select` summary at `picker.rs:389-393`). Both go through `transcript::line`.
- The CLI opens the transcript right after `style::init` (before `DataDir`), only when a command exists, so config and data-dir errors are also recorded; the warning therefore uses the glyphs of that moment (default set).
- `out_block("")` writes nothing to the transcript (otherwise it would add an empty line); one trailing `\n` of a block is not a line.
- Observer: a modal is "changed" when its generated lines change (so two consecutive dialogs with identical text show once); a job with no `progress` yet prints `[tui] job: {rótulo}` without a phase; item arrow only when `para` exists.
- `tests/it/main.rs`: added `mod transcript;` (allowed by the lead); `rustfmt --edition 2024` on cli/mod.rs recurses into modules, so I formatted that hunk by hand to the rustfmt output.

Concerns:
- Mutation 2 (flush) not provable as written, see above.
- No `.join` with external data written. The transcript path comes from the user's own environment variable and is only opened in append mode (not elevated: `bridge-worker` and `update --background` never open it, covered by test 9).
- The TUI branch (toast for a bad path at `tui::run::run`) has no automated test (needs a terminal); the CLI branch is test 4.

Revisão: commit de correção acima (teste OSC; mutação PEGA; apply-elevated; doc do VAR). Gate verde, 2041 passed.
