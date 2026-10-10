Status: DONE_WITH_CONCERNS

Commits:
656df48 feat(install): target resolution and arrow-key picker
(git log base..HEAD also lists other agents' commits; only 656df48 is Task 35.)

Gate:
Full `verify.ps1` never reached green because of OTHER agents' in-progress files: last runs failed at fmt (tests/install_manifest.rs, tests/install_extract.rs, src/buckets.rs, src/install/manifest.rs) and clippy (tests/buckets: 3 errors). Last lines seen:
  [OK]   rules 2s
  [OK]   fmt 1s   (earlier run)
  [FAIL] clippy exit 101 / could not compile test "buckets"
For my own files: rules OK; `rustfmt --check` on my 4 files clean; `cargo clippy --lib --test install_target -- -D warnings` no findings in my files; `cargo test --test install_target`: 11 passed. The orchestrator must re-run the gate once the other tasks settle.

Mutation proofs:
1. Skip unselectable: mutate.ps1 -Path src/tui/picker.rs -Anchor '(self.selected + 1..self.items.len()).find(|&i| self.items[i].selectable)' -Replacement '... .find(|&i| i < self.items.len())' -TestCommand 'cargo test --quiet --test install_target arrows_skip'
   [OK] mutação PEGA: ... fez o teste falhar (exit 101).
2. Never install by fuzzy: mutate.ps1 -Path src/install/target.rs -Anchor '    Err(TargetError::NotFound {' -Replacement 'if let Some(e) = hits.first()... { return Ok(vec![candidate(e)]); } Err(...' -TestCommand 'cargo test --quiet --test install_target unknown_term'
   [OK] mutação PEGA: '    Err(TargetError::NotFound {' -> ... (test failed)

Decisions:
- API: `resolve(&Catalog, query, bucket: Option<&str>) -> Result<Vec<Candidate>, TargetError>`; `bucket/id` prefix overrides nothing: prefix wins over the flag. Bucket restriction excludes winget.
- `pick_inline(items, &Theme, &Glyphs) -> io::Result<Option<usize>>` (brief said Option only; theme/glyphs needed per brief, Err(Unsupported) if not a TTY; `is_interactive()` exposed for callers). Styles converted via ratatui `IntoCrossterm` (no `Color::` outside theme.rs).
- Suggestions: nucleo is subsequence-based so typos ("ripgrop") match nothing; fallback retries with progressively shorter prefixes (min 3 chars). Suggestions are "id (source)".
- Added `window()` (pure scroll window, max 15 rows) in tui::picker, tested.

Concerns:
- NOT verified: the inline drawing of `pick_inline` (raw mode, cursor movement, redraw, cleanup) needs a real TTY; only the pure `Picker` state and `window` are tested. Needs a manual run once a CLI command uses it.
- Exact-name match with multiple winget/scoop entries returns all; ordering follows catalog rowid.
- Gate not green end-to-end (see above), due to other agents' files.
