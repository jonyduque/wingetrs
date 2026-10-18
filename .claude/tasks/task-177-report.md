Status: DONE_WITH_CONCERNS

Commits:
ac4db3c fix(winget_install): plain http only for loopback
9fc6584 feat(winget-install): installer download, materialization and share-deny verified file

Gate (after ac4db3c):
```
Carregado em 262ms
[OK]   rules 5s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (test "it") due to 1 previous error
```
The clippy failure is `src\install\state.rs:252:5: error: unused variable: expected_sha256` (another agent's file). Filtered clippy finds nothing in my files. `wi_fetch::` -> `test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1239 filtered out; finished in 6.53s`.

Gate (first commit, kept for history):
```
Carregado em 267ms
[OK]   rules 6s
[OK]   rules-selftest 19s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\install_executor.rs:1628:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\install_written_manifest.rs:324:
```
The failure is only in other agents' files (2c, uncommitted). Proof for my files:
- `rustfmt --edition 2024 --check src/winget_install/fetch.rs src/winget_install/verified_win.rs tests/it/wi_fetch.rs` -> 0 diffs.
- `cargo clippy --all-targets -- -D warnings` filtered: the only finding is `tests\it\wi_launch.rs:66:9: error: used unwrap()` (another agent); nothing in fetch.rs / verified_win.rs / wi_fetch.rs.
- `cargo test --quiet --test it wi_fetch::` -> `test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1221 filtered out; finished in 2.06s`
- `scripts/check_rules.ps1` -> the gate's `[OK]   rules` line above (run with my files in place).
The orchestrator should rerun the full gate once the 2c files are formatted.

Mutation proofs:
1. share_mode (test 9):
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/verified_win.rs -Anchor '.share_mode(FILE_SHARE_READ.0)' -Replacement '.share_mode((FILE_SHARE_READ | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE).0)' -TestCommand 'cargo test --quiet --test it wi_fetch::verified_handle_blocks_writers_and_rename'`
   `[OK] mutação PEGA: '.share_mode(FILE_SHARE_READ.0)' -> '.share_mode((FILE_SHARE_READ | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE).0)' fez o teste falhar (exit 101).`
   (The anchor follows the real code: `share_mode` takes a `u32`, hence `.0`.)
2. https check (test 3):
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/fetch.rs -Anchor 'if require_https && !has_https {' -Replacement 'if false && require_https && !has_https {' -TestCommand 'cargo test --quiet --test it wi_fetch::http_url_refused_without_request'`
   `[OK] mutação PEGA: 'if require_https && !has_https {' -> 'if false && require_https && !has_https {' fez o teste falhar (exit 101).`
3. Hash through a reopened path instead of the handle (multi-line anchor: adds `let mut reopened = std::fs::File::open(path).map_err(io)?;` and reads from `reopened`, while keeping the share-deny handle in the struct):
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/verified_win.rs -AnchorFile <scratch>/a3.txt -ReplacementFile <scratch>/r3.txt -TestCommand 'cargo test --quiet --test it wi_fetch::'`
   `[X] mutação SOBREVIVEU: '        let mut sha = Sha256::new();` ... `e os testes passaram. A regra não está coberta.`
   Why no test can catch it: the share-deny handle is already open when the reopen happens, so nobody can write, delete or rename that file. The reopen asks only for read access, which our FILE_SHARE_READ allows, and the bytes it reads are the same. The only difference is *which file* the path resolves to at the moment of the reopen. A deterministic test cannot tell them apart without winning a race (for example, swapping a parent folder or junction between the open and the reopen). Why the handle form is still required: it is the only form where "what was hashed" and "what is held locked" are provably the same file object. With the path form, the verdict covers whatever the path points to at hash time, and the lock covers whatever it pointed to at open time. The guarantee then rests on the path not being redirected between the two, which is exactly the TOCTOU the share-deny handle exists to close.

4. Loopback-only plain http (orchestrator decision, ac4db3c):
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/fetch.rs -Anchor 'loopback && u.username().is_empty() && u.password().is_none()' -Replacement 'true' -TestCommand 'cargo test --quiet --test it wi_fetch::http_url_refused_without_request'`
   `[OK] mutação PEGA: 'loopback && u.username().is_empty() && u.password().is_none()' -> 'true' fez o teste falhar (exit 101).`
   Credentials check: `... -Replacement 'loopback' ...` -> `[OK] mutação PEGA: 'loopback && u.username().is_empty() && u.password().is_none()' -> 'loopback' fez o teste falhar (exit 101).`
   https branch: `-Anchor '"https" => true,' -Replacement '"https" => false,' -TestCommand 'cargo test --quiet --test it wi_fetch::https_prefix_is_case_insensitive'` -> `[OK] mutação PEGA: '"https" => true,' -> '"https" => false,' fez o teste falhar (exit 101).`
   Mutation 2 above was run against the first commit. In ac4db3c the check lives in `scheme_allowed`, and mutation 4 covers it.

Decisions:
- (Superseded by ac4db3c: the seam function was removed. Rule: `reqwest::Url` parse; `https`, or `http` only when the host is 127.0.0.1, ::1 or localhost (IPs compared as `IpAddr`) and the URL carries no username or password. Refused: 127.0.0.2, credentials, example.com, 127.0.0.1.evil.com, localhost@evil.com, ftp, non-URL. The 127.0.0.2 case runs first, so the accept-any-http mutant fails locally without DNS.) Original: wiremock only serves `http://`, but the brief requires refusing anything that is not `https://`. Added `#[doc(hidden)] pub fn fetch_installer_allowing_plain_http` (same code path, `require_https=false`, SHA still enforced). Tests 1,2,4-10 use it; test 3 uses `fetch_installer`. An env var was rejected because it races the parallel tests. Extra test `https_prefix_is_case_insensitive` covers `HTTPS://`.
- An unusable version (empty, `..`, trailing `.`, sanitized longer than 128) -> `BadId("<id> <ver>")` before any request. The enum has no dedicated variant. Test: `unusable_version_is_refused_without_request`.
- `Materialized` exposes `path()`/`sha256()` accessors instead of pub fields. The sha is stored lowercase, and uppercase manifest SHAs are accepted (test `uppercase_sha_is_accepted`).
- `materialize` runs in `spawn_blocking` (it may copy a large file) with `resume_unwind`. On failure it removes the partial target and an empty `<ver>`.
- TDD order: the lib did not compile at first because of another agent's `switches.rs`, so I wrote the implementation before seeing the red. The red is shown by mutations 1 and 2.

Concerns:
- Plain http to loopback is accepted in production as well (orchestrator decision). The SHA-256 still pins the bytes.
- The hard link makes the materialized file and `blobs\<sha>` the same file object. Writing to one changes the other: test 8 tampers the blob too, but `cached()` rehashes, so it is only re-downloaded.
- Two live `Materialized` for the same id and version share one path. The second removes and recreates the file, and dropping the first then deletes it. Callers must serialize per package (test 4 drops the first before the second call).
- The `<root>\installers\<id>` folder is not removed when empty (the brief only asks for `<ver>`).

## Review fixes (a254ab9 fix(winget_install): review of task 177)

Gate:
```
[OK]   rules 6s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (test "it") due to 1 previous error
```
The only clippy finding is `tests\it\wi_bridge_install.rs:410:37` (another agent's file). `wi_fetch::` passes 18/18, and `--test it http` gives `21 passed; 0 failed; 1 ignored`.

What changed:
1. Version cap: added `"1".repeat(129)` to the version-refusal test (mock `.expect(0)`) and a test that a 128-character version is accepted.
2. The id must also be one `safepath` component. `Pub.App.`, `NUL`, `CON` and `com1.App` are each refused with `BadId`.
3. New `Http::with_strict_download_redirects` (outside the Files list, at your request). It gives the installer download its own client whose redirect policy is `strict_redirect_allowed` (only https -> https), ignoring the global `allow_http_redirect`. A loopback-http origin therefore follows no redirect. Wiremock test: with the global setting on, a 302 to `localhost` or to the same host is refused as `InsecureRedirect`, and `/real.exe` is never requested. A pure test covers the four scheme pairs.
4. `http://localhost.:<port>` is now in the refusal loop.
5. Test 9: while the `VerifiedFile` is open, opening `blobs\<sha>` for writing also fails with error 32.
6. `Materialized::verify(&mut self)` (Windows only) keeps the `VerifiedFile`. `Drop` closes it before deleting the file, and the order is documented on the type. The test checks that neither the file nor the `<ver>` folder is left behind.

Mutation proofs:
- `[OK] mutação PEGA: 'ver.len() > MAX_VERSION_LEN || ' -> '' fez o teste falhar (exit 101).` (wi_fetch::unusable_version_is_refused_without_request)
- `[OK] mutação PEGA: ' || !one_component(id)' -> '' fez o teste falhar (exit 101).` (wi_fetch::bad_sha_and_bad_id_refused)
- `[OK] mutação PEGA: '.redirect(checked_policy(strict_redirect_allowed))' -> '.redirect(redirect_policy(self.allow_http_redirect.clone()))' fez o teste falhar (exit 101).` (src/http.rs; wi_fetch::loopback_http_follows_no_redirect_even_when_allowed_globally)
- `[OK] mutação PEGA: 'std::fs::hard_link(blob, target)' -> 'std::fs::hard_link(blob, target).and(Err::<(), _>(std::io::Error::other("forced")))' fez o teste falhar (exit 101).` (forces the copy fallback; wi_fetch::verified_handle_blocks_writers_and_rename)
- `[OK] mutação PEGA: 'drop(self.verified.take());' -> '' fez o teste falhar (exit 101).` (wi_fetch::materialized_closes_its_verified_handle_before_deleting)

Decisions: when building the strict client fails, the error comes back as `FetchError::Download(DownloadError::Http(_))`, so the enum keeps the variants the brief fixed. `Http` now stores `connect_timeout` so the strict client keeps the caller's timeouts; the chunk timeout carries over through the clone.
