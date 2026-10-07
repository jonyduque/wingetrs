Status: DONE

Commits:
ddded5a feat(winget): index acquisition with blockmap integrity and package details

Red (before impl): error[E0432]: unresolved import `wingetrs::sources::winget::details` (tests winget_index/winget_details did not compile).

Gate:
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (87 passed, 0 ignored) 5s
[OK]   gate verde

Mutation proofs:
1. [OK] mutação PEGA: 'if STANDARD.encode(Sha256::digest(chunk)) != *expected {' -> 'if false && ...' fez o teste falhar (exit 101).  (--test winget_index)
2. [OK] mutação PEGA: 'if major != "2" {' -> 'if false {' fez o teste falhar (exit 101).  (--test winget_index)
3. [OK] mutação PEGA: 'blobs.put_verified(&entry.sha256, &b)' -> 'blobs.put_verified(&crate::cache::sha256_hex(&b), &b)' fez o teste falhar (exit 101).  (--test winget_details; the s256H comparison lives in BlobStore::put_verified, so the mutation bypasses it by passing the actual hash)

Decisions:
- clippy collapsible_if: used let-chain in validate_msix.
- #[allow(clippy::large_enum_variant)] on WingetFetch (interface fixed by brief).
- Gate failed once on another agent's tests/normalize.rs (constant assertion); passed on retry, no action.
- Test bodies transcribed unchanged; wiremock up_to_n_times ordering worked as in brief.

Concerns: none

## Follow-up (commit above: test(winget): pin blockmap block count and file hash)
- Added missing_block_is_rejected (BlockMap) and bad_file_hash_is_rejected (FileHash).
- REAL BUG found: b4:FileHash is a child element <b4:FileHash Hash=".."/>, not an attribute; the FileHash check never ran. Fixed in index.rs.
- Mutations (--test winget_index): block count `if false` -> [OK] mutação PEGA; FileHash `&& false` -> [OK] mutação PEGA.
- Temp index.db now removed also when fs::write fails (mod.rs).
- Gate: winget_index 9 passed; verify.ps1 blocked by another agent's tests/installed.rs rustfmt diff (not my files); my files pass rustfmt/clippy.
