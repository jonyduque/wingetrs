Status: DONE_WITH_CONCERNS

Commits:
d7f15b7 perf(list): measure per-collector spans and cut the largest cost
(git log base..HEAD also shows 325941f, a docs commit by the orchestrator, not mine)

Gate:
[OK]   fmt 3s
[OK]   clippy 28s
[OK]   test (160 passed, 2 ignored) 79s
[OK]   gate verde

Mutation proofs: no mutation proof (performance task, declared in the brief). No new pure function was created.

Spans (ms, release, `list -v`, indicative of 3 runs each; parallel steps overlap, so they do not sum)
Step                 before(3 runs)      after final(3 runs)
open_catalog         4-40                ~17 (now overlapped)
scan_upgrade_codes   14-18               ~13
scan_arp x64/x86/usr 12-15/29-40/5       12/24/5
scan_scoop           27-29               ~33
scan_msix_enumerate  28-34               ~38
scan_msix_identify   11-17               ~15
scan_msix_props      180-260             171-203  (largest step)
scan_all             230-301             224-250
normalizer_new       57-64 (serial)      76-85 (parallel, off critical path)
correlate            40-50               ~45-60
output               <1                  ~1
Not a per-run median: I did not compute a median of 10 per step; the table gives ranges of 3 runs.

Bench (bench.ps1 list, 10 runs, 2 warmup, -Baseline noop.exe), delta median:
before: 451.9 ms, 558.1 ms (noisy; absolute median 487.7 / 599.7)
after step 1 (scan + normalizer started before catalog open): 379.4, 354.3
after step 2 (MSIX work-stealing over shared index): 280.0, 271.4 (absolute median 312.7 / 303.7) -> target < 350 met

Output identical: `list --json` before vs after: `cmp` IDENTICAL (byte-for-byte, 2960 lines); sorted diff SAME_ROWS.
Live test live_list_runs_under_budget (--release): passed 3 of 3 runs afterwards.

Decisions:
- Spans emitted as info_span plus an info event with ms (helper installed::timed), because the stderr layer has no FmtSpan::CLOSE and changing init_logging is outside the list path.
- Chosen by the span: scan_msix_props was the largest; the serial chain was catalog + scan + normalizer + correlate, so scan and normalizer now start before ensure_catalog.
- MSIX parallel() uses an atomic index instead of fixed chunks (imbalance); props dropped 260 -> ~190 ms under contention.

Concerns:
- First run of `cargo test --release --ignored` (all targets) showed the correlate live test FAILED once (0.62 s, cause not captured, likely right after a rebuild); 3 later runs passed.
- Edited src/installed/arp_win.rs, appx_win.rs, mod.rs, cli/mod.rs (list path only); all in the Files list.
- Machine noise: first runs vary +-50 ms; delta values are interleaved with noop baseline.

## Review fix (commit: see `git log -1`, fix(list): join early threads on every path and propagate worker panics)
- cli/mod.rs: ensure_catalog result held, both early threads joined, only then `?`. Test tests/cli_offline.rs list_without_catalog_and_without_network_fails_cleanly_and_fast (no "panicked", < 5 s).
- parallel moved to src/installed/par.rs (pure, no cfg), worker panic -> resume_unwind. Tests in tests/installed.rs (order of 1000 items, panic propagates).
- Gate: [OK] test (164 passed, 2 ignored) / [OK]   gate verde
- Mutation: [OK] mutação PEGA: 'items.get(i)' -> 'items.get(i + 1)' fez o teste falhar (exit 101).
- Also mutated `Err(p) => resume_unwind` -> `Err(_) => {}`: test failed (caught).
- Bench list (1 round, -Baseline): median 344.8 ms, delta median 309.0 ms (< 350, no regression; machine noisy).
