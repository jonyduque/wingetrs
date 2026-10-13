Status: DONE

Commits:
7f6dcd3 feat(cache): prune blobs unused for 30 days after update
d95a2ca fix(cache): unique blob temp names and cover the file-type filter

Gate:
[OK]   rules-selftest 67s
[OK]   fmt 4s
[OK]   clippy 24s
[OK]   test (651 passed, 16 ignored) 139s
[OK]   gate verde

Mutation proofs:
1 age inverted: [OK] mutação PEGA: '.is_some_and(|age| age > limit)' -> '.is_some_and(|age| age <= limit)' fez o teste falhar (exit 101).
2 64 hex: [OK] mutação PEGA: 'n.len() == 64 && ...' -> '!n.contains(".")' fez o teste falhar (exit 101).
3 touch: [OK] mutação PEGA: '.and_then(|f| f.set_modified(SystemTime::now()));' -> '.map(|_| ());' fez o teste falhar (exit 101).
4 lock: [OK] mutação PEGA: 'let _lock = match UpdateLock::try_acquire(&data.install_layout().install_lock()) {' -> '...try_acquire(&data.root().join("x.lock"))...' fez o teste falhar (exit 101).
5 index rewrite: [OK] mutação PEGA: 'if removed > 0 {' -> 'if true {' fez o teste falhar (exit 101).
7 is_file filter (review): [OK] mutação PEGA: 'entry.file_type().is_ok_and(|t| t.is_file())' -> 'entry.file_type().is_ok()' fez o teste falhar (exit 101).  (failing test: prune_removes_only_old_blobs_and_old_temps; caught by an aged symlink named like a blob, a directory alone is an equivalent mutant because remove_file fails on it)
6 mtime re-read before delete: src/cache/mod.rs, comment "Re-read right before deleting" and the `let fresh = std::fs::symlink_metadata(&path);` line in BlobStore::prune (no deterministic test).

Decisions:
- Files with is_file()==false (symlink/dir) count in kept; stale check uses symlink_metadata (does not follow links).
- Test 8 calls data.ensure() then replaces the empty blobs dir by a file (otherwise winget fails for lack of tmp dir, unrelated to prune).
- Directory mtime not aged in test 2 (File::open on a directory is denied on Windows); irrelevant, dirs are never deleted.
- rustfmt run only on my six files.

Decisions (review round): put_verified temp is now <hash>.<pid>-<seq>.tmp, removed on write/rename error; tests/winget_details.rs edited (breaks the write with a non-empty dir at the blob path, since the fixed .tmp name is gone).

Concerns: tui_jobs::reading_the_catalog_never_writes_it (Task 80's file) failed twice in the shared gate while the other agent was mid-edit; passed in the final run.
