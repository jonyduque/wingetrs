Status: DONE

Commits:
(see git log; fix(cli), fix(installed), fix(tui), refactor(search) on top of 6e4448d)

Gate:
[OK]   fmt 1s
[OK]   clippy 10s
[OK]   test (182 passed, 2 ignored) 79s
[OK]   gate verde

Mutation proofs:
- item 1: `mutate.ps1 -Path src/cli/mod.rs -Anchor 'Err(p) => std::panic::resume_unwind(p),' -Replacement 'Err(_p) => None,'`
  [OK] mutação PEGA: 'Err(p) => std::panic::resume_unwind(p),' -> 'Err(_p) => None,' fez o teste falhar (exit 101).
- item 2: `mutate.ps1 -Path src/installed/mod.rs -Anchor 'unwrap_or_else(|p| std::panic::resume_unwind(p))' -Replacement 'unwrap_or_else(|_| Err("collector panicked".to_string()))'`
  [OK] mutação PEGA: 'unwrap_or_else(|p| std::panic::resume_unwind(p))' -> 'unwrap_or_else(|_| Err("collector panicked".to_string()))' fez o teste falhar (exit 101).

Decisions:
- Item 1/5: thread now returns EarlyOutcome {result, index_warning}; warn logged after join. Drop only reaps (no propagate) while panicking. search_best delegates to search_early.
- Item 2: extracted scan_with(&[fn]) as test seam; no existing test depended on the old behavior.
- Item 3: spawn_scan joins tracking_job via catch_unwind on all paths; new Msg::InstalledFailed -> status "falha ao ler instalados: ..."; JoinError (panic) also reported. ratatui::init already installs a panic hook restoring the terminal (one-line comment added, no duplicate hook).
- Item 6: sort_ranked<T> in search.rs used by the 3 sites (empty query uses cached key by name; tests/index_file.rs equivalence unchanged and green).
- Item 7: tmp removed on fs::write failure (no dedicated test).

Concerns: none (items 3, 6 and 7 have no mutation proof, not requested).
