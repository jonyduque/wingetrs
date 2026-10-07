Status: DONE_WITH_CONCERNS

Commits: see `git log --format='%h %s' e52ef17..HEAD`; this task's commit is the newest "feat(cache): data dir, http state, blob store, update lock and conditional http".

Gate (verify.ps1, run before commit, with other agents' work present):
[OK]   fmt 0s
[OK]   clippy 2s
[OK]   test (57 passed, 0 ignored) 7s
[OK]   gate verde

TDD red: `cargo test --test http --test cache` -> error[E0432]: unresolved imports `wingetrs::http::Fetch`... / `wingetrs::cache::BlobStore`...

Mutation proofs (outputs real):
1. IF_NONE_MATCH removed (src/http.rs, --test http): [OK] mutação PEGA: 'req = req.header(reqwest::header::IF_NONE_MATCH, etag);' -> 'let _ = etag;' fez o teste falhar (exit 101).
2. put_verified check disabled (src/cache/mod.rs, --test cache): [OK] mutação PEGA: 'if !actual.eq_ignore_ascii_case(expected_sha256_hex) {' -> 'if false {' fez o teste falhar (exit 101).
3. save writes directly, no tmp/rename: [OK] mutação PEGA: 'f.sync_all()...' -> '...std::fs::write(path, &json)...return Ok(());' fez o teste falhar (exit 101). (caught by save_leaves_no_tmp_file because the mutant leaves http.json.tmp behind; pinned rule is only absence of .tmp, as the brief allows.)

Decisions:
- tests/http.rs sends_validators_and_maps_304: brief literal `header("If-Modified-Since", "Wed, 07 Oct ...")` can never match, because wiremock's header matcher splits the request value on commas (wiremock 0.6.5 matchers.rs HeaderExactMatcher). Replaced with `headers("If-Modified-Since", vec!["Wed", "07 Oct 2026 03:02:43 GMT"])`, same exact-value assertion, comment added. Without it the test got 404.
- Added data_dir_paths_and_ensure and blob_roundtrip tests beyond the brief prose. Ran rustfmt on my 4 files.
- UpdateLock::try_acquire creates the parent dir; blob get/put compare hash case-insensitively but name file by lowercase hex.

Concerns: none (clippy needed no extra allows).
