Status: DONE

Commits:
1520540 test(cli): match exact span and event names in search_logs_join_wait
fb15cfe feat(cli): spans for search startup, join wait and index stages

Gate:
[OK]   rules 4s
[OK]   rules-selftest 12s
[OK]   fmt 2s
[OK]   clippy 16s
[OK]   test (866 passed, 22 ignored) 217s
[OK]   gate verde

Mutation proofs:
- Brief's rename "search.join_wait" -> "search.join_waitx" SURVIVES by construction (the new name still contains the old as a substring, and the test checks `contains`). Used "search.joinwait" instead:
[OK] mutação PEGA: '"search.join_wait"' -> '"search.joinwait"' fez o teste falhar (exit 101).

Decisions:
- Test `search_logs_join_wait` in tests/cli_offline.rs uses -vv (span CLOSE events on stderr) with the empty-catalog wiremock scenario.
- idx.read/parse/scan/sort all exist as separate stages in search_with; scan spans the parallel worker section, sort includes truncate.
- idx.* spans only appear when search_with runs after the subscriber (fallback/TUI); the early thread runs before logging, as accepted.
- startup.ensure/startup.log are debug events (target wingetrs::startup) with ms field, emitted after init_logging.

Concerns: none. Gate also ran with other agents' uncommitted files in the tree (src/install/download.rs); it was green.

Review fix mutation proofs (exact-name asserts, ANSI stripped):
[OK] mutação PEGA: '"search.join_wait"' -> '"search.join_waitx"' fez o teste falhar (exit 101).
[OK] mutação PEGA: '"startup.ensure"' -> '"startup.ensurex"' fez o teste falhar (exit 101).
