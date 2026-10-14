Status: DONE

Commits:
7c98960 feat: one row per installed app name, records listed with location
56cfc4c docs: 2d warning also covers orphan ARP records

Gate:
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 1s
[OK]   clippy 4s
[OK]   test (790 passed, 19 ignored) 224s
[OK]   gate verde

Mutation proofs:
[OK] mutação PEGA: 'let key = r.name.trim().to_lowercase();' -> 'let key = r.name.trim().to_string();' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if rank(&g[i]) > rank(&g[best]) {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'hive_tag(*hive), app.key)' -> '"x", app.key)' fez o teste falhar (exit 101).

Real machine (target/release/wingetrs.exe list --json): 277 rows. Rows with others:
- Amazon Kindle 2.9.1.71006 (usuário) others: [máquina 32-bit 2.8.3.70995]
- Microsoft Edge 155.0.4283.45 (máquina 32-bit) others: [msix 155.0.4283.45]
- Python Launcher 3.14.7150.0 (máquina 32-bit) others: [máquina 32-bit 3.13.12150.0]
- Zed 1.23.3 (usuário) others: [msix 1.0.0.0]
- Adobe Acrobat Reader 26.0.0.1 (msix) others: [msix 26.0.0.1]

Decisions:
- Legacy override keys (arp/<subkey>) are resolved by new overrides::lookup (new key first, then legacy), not by rewriting in load; load/set stay untouched so old files keep round-tripping.
- Label is "+1 registro" / "+N registros" (extra_records_label) in CLI; TUI list shows just "+N", details show full text.
- Tests used key "Amazon Kindle" with a 7-Zip product code so both rows are Matched on the same id.
- Existing test override_resolves_ambiguous expected arp/dup; updated to arp/hklm64/dup. examples/tui_frame.rs also needed ..Default::default() (outside Files list).

Concerns: none. Details-section test is a buffer search, not a snapshot.

## Fix round (review)
Commits: 7c98960 and 2bd24c3 (fix). Gate in shared tree: fmt/clippy OK; test fails only in tests/scoop_winget_match.rs (details_are_sent_before_the_twin_search, impl-103 WIP in run.rs/app.rs), not in my files.
Mutation: [OK] mutação PEGA: 'r.origin == "wingetrs",' -> 'false,' fez o teste falhar (exit 101).
Real-machine: 277 rows (command output pasted in first round above, Kindle/Edge/Python Launcher/Zed/Acrobat).
