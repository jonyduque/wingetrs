Status: DONE

Commits:
(see below, filled by git log)

Gate:
[OK]   rules 4s
[OK]   rules-selftest 11s
[OK]   fmt 2s
[OK]   clippy 14s
[OK]   test (867 passed, 22 ignored) 176s
[OK]   gate verde

Mutation proofs:
- [OK] mutação PEGA: 'let chunk = &buf[..n];' -> 'let chunk = &buf[..];' fez o teste falhar (exit 101).
- [OK] mutação PEGA: '(sha_hex == name && algo_hex' -> '(true && algo_hex' fez o teste falhar (exit 101).
- grep "fs::read(" download.rs: only read_index (:114) and prune_index (:137).

Decisions:
- Added a second cached test (sha1 spec, blob name != its sha256) because for a sha256 spec the sha256-name check is redundant with the manifest check, so the mutation survived with the tampered-blob test alone.
- Test covers Md5 too (HashAlgo has it). `cached` stays sync (hashes on the calling task), as the brief specified.

Concerns: none
eadeb6e fix(install): hash downloads in one streaming pass instead of reading them into memory
3fb7906 chore(tasks): ledger for task 122
fb15cfe feat(cli): spans for search startup, join wait and index stages
800bb93 fix(scripts): list oracle flags name mismatches, duplicate ids and version-format diffs
155f078 chore(harness): report contract asks for the agent's own SHAs, filtered by its files
cd29433 feat(bridge): tell the user when winget asks for a reboot
bac2434 feat(scripts): versioned list oracle against Get-WinGetPackage
3a3129a chore(tasks): ledger for task 117; .tar.xz enters phase 2b
37bf1a8 chore(tasks): ledger for task 116
c3c5759 docs(probe): 7z, msi and cab crates for phase 2b

## Fix round (review: cached blocked the runtime)
- `cached_blocking` runs `cached` in spawn_blocking with cloned DataDir/index/spec; panic via resume_unwind, cancellation -> DownloadError::Io; `fetch_each` awaits it.
- Test `cached_lookup_does_not_block_the_runtime` (current_thread, 64 MiB blob, 2 ms ticker needs >=2 ticks).
- [OK] mutação PEGA: 'let job = tokio::task::spawn_blocking({' -> sync call via std::future::ready (assertion failure "runtime was blocked", not compile error).
- Gate: [OK]   test (869 passed, 22 ignored) 199s / [OK]   gate verde
