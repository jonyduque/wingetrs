Status: DONE_WITH_CONCERNS

Commits:
5b5ca04 fix(buckets): rm saves config under the lock, add rolls back a late failure, download phase with progress

Gate:
Gate NOT green because of another agent's file (Task 220, `src/install/executor.rs`, work in progress), not mine:
```
[OK]   rules 12s
[OK]   rules-selftest 47s
[OK]   fmt 5s            (fmt log shows a diff only in src\install\executor.rs:3018)
[OK]   licenses 7s
[FAIL] clippy exit 101
  src\install\executor.rs:3014:14: unused variable `staging_name`
  src\install\executor.rs:3006:4: function `is_own_run_leftover` is never used
```
My files proved apart: `cargo clippy --all-targets -- -D warnings` filtered for buckets/http/scoop/bucket/cli/mod/bucket_admin/cli_install = no hits; `cargo test --quiet --test it bucket` = 60 passed, 2 ignored; `cargo test --quiet --lib` = 64 passed. The orchestrator must run the gate after 220 lands.

Mutation proofs:
1. undo removed (tests 2 and 3): `mutate.ps1 -Path src/buckets.rs -Anchor 'undo_add(data, &mut catalog, name, &url, dir.filter(|_| !dir_existed));' -Replacement 'let _ = (&mut catalog, &dir, dir_existed);' -TestCommand 'cargo test --quiet --test it bucket_admin::add_'`
   `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
2. rm `cfg.save` removed from the lock (AnchorFile with the 3-line tail of `remove_and_cleanup_confirmed`; equivalent to moving it to the caller) with `bucket_admin::rm_saves`:
   `[OK] mutação PEGA: '    cfg.save(&data.config_path())` ... (exit 101)
3. no Download event: `mutate.ps1 -Path src/http.rs -Anchor 'progress(body.len() as u64, total);' -Replacement 'let _ = (&progress, total);' -TestCommand 'cargo test --quiet --test it bucket_admin::add_reports'`
   `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`

Decisions:
- The brief said `update` already streams tarball progress; it does not (only phases). Added `Http::get_conditional_progress` (streams the body, `get_conditional` delegates with a no-op), `fetch_bucket_progress` (extra `downloaded` callback to emit `Phase::Extracting` after the body and before parsing) and `buckets::add_downloaded_progress`; `add_downloaded` and `fetch_bucket` keep their signatures. `src/http.rs` is outside the Files list: needed for streaming.
- Download phase shown before the first byte is `Phase::FetchingCatalog { sources: 1 }` ("Baixando o catálogo"), the one `update` uses.
- `undo_add` also removes the URL from `http.json` (otherwise a later `update` would get 304 and skip a bucket missing from the catalog); covered by test 3.
- Test 1 saves the config with the bucket once before the lock check (setup saved it without `mine`), then proves `rm` alone removes it.
- Test 6 is a new test next to the existing `bucket_add_list_rm_round_trip`.

Concerns:
- `rustfmt src/cli/mod.rs` can format child modules; `git diff` of `src/cli/mod.rs` shows only my hunks, but `winget_cmd.rs` is modified by Task 220 and was not touched by me on purpose.
- Prova no binário (check 4) fica para o orquestrador.
