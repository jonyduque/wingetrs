# Task 70 report: `show` winget, versionData cached by the catalog hash

Status: DONE

Commits:
```
bb30232 perf(show): cache winget versionData by its catalog hash
2c1ff4d perf(show): spans for find_by_id, version_data and manifest
```
(base 26787c2)

Gate (on bb30232, the same output as on 2c1ff4d except for the test count, 622 there):
```
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 8s
[OK]   test (625 passed, 16 ignored) 171s
[OK]   gate verde
```

## Probe (gate before the optimization)
`python -I vd-probe.py <S>\la\wingetrs\catalog.db <S>\vd`, isolated root, catalog copied at 2026-10-12:
```
AtomicJar.Testcontainers bfa733d5a21f7f99c4cf6c5eb003485abc58fde0a58e7ddfc1b4d708c9307574 bfa733d5a21f7f99c4cf6c5eb003485abc58fde0a58e7ddfc1b4d708c9307574 OK
AtomixProductions.VirtualDJ 0619dc37022b8abf32cd0c2e9fd181c9c235d9574ef347a68739341b332a50b8 0619dc37022b8abf32cd0c2e9fd181c9c235d9574ef347a68739341b332a50b8 OK
Atrix.AtrixLauncher 7ab9ac7f8a881a7d072489cb2215415026ee9f15fb90763a0f3d27924d679a67 7ab9ac7f8a881a7d072489cb2215415026ee9f15fb90763a0f3d27924d679a67 OK
Atticus64.juice e62863dcb8ead07264a32e4b884828c51ff8fc4a329950b4c3f191ce56f846a2 e62863dcb8ead07264a32e4b884828c51ff8fc4a329950b4c3f191ce56f846a2 OK
Atticus64.news 9f4fce7ee50d7054419faba0d5b868cbfe00d091143978de115ec9d3a03d0a69 9f4fce7ee50d7054419faba0d5b868cbfe00d091143978de115ec9d3a03d0a69 OK
```
5/5 OK, 0 DIVERGE. `<S>\vd` deleted.

## Spans from commit 1 (base binary 2c1ff4d, 7 runs of `show Microsoft.PowerToys --source winget`, isolated root)
Last log lines (`Select-String ... 'show\.(find_by_id|version_data|manifest).*close time\.busy'`), excerpt from the last 3 runs:
```
DEBUG show.find_by_id: wingetrs::cli: close time.busy=13.0ms time.idle=13.7µs
DEBUG show.version_data: wingetrs::sources::winget::details: close time.busy=152ms time.idle=57.3ms
DEBUG show.manifest: wingetrs::sources::winget::details: close time.busy=945µs time.idle=1.70µs
DEBUG show.find_by_id: wingetrs::cli: close time.busy=13.1ms time.idle=14.9µs
DEBUG show.version_data: wingetrs::sources::winget::details: close time.busy=150ms time.idle=47.3ms
DEBUG show.manifest: wingetrs::sources::winget::details: close time.busy=885µs time.idle=1.50µs
```
busy values over the 7 runs:
- find_by_id: 13.1 10.8 11.6 12.6 12.9 13.0 13.1
- version_data: 137 153 139 137 204 152 150 (idle was another 36–121 ms)
- manifest: 3.21 81.9 1.17 0.84 1.04 0.945 0.885

Medians: **find_by_id 12.9 ms, version_data 150 ms (busy), manifest 1.04 ms** (the manifest is a cache hit from the 2nd run onward).

## A/B (base = 2c1ff4d, new = bb30232, same root, catalog refreshed right before)
`ab-cmd.ps1 -CmdArgs 'show Microsoft.PowerToys --source winget' -Span 'show\.version_data' -Runs 15`:
```
wall base med=   294.2 max=   534.0 | nova med=    84.9 max=   120.3 | delta pareado (nova-base) med=  -210.8 ms
piso noop med=31.2 ms | sobre o piso: base 263.0, nova 53.7 ms
span base med=   152.0 max=   211.0 | nova med=     2.0 max=     3.0 | delta pareado (nova-base) med=  -149.0 ms
brutos wall base: 363.1 294.8 534.0 275.2 270.8 287.9 297.3 304.1 294.2 271.5 302.4 316.3 290.7 293.6 281.2
brutos wall nova: 109.9 86.7 120.3 85.8 86.3 77.1 79.5 83.1 75.6 84.9 84.5 80.4 86.4 87.4 84.9
brutos span base: 211.0 148.0 153.0 142.0 142.0 146.0 158.0 141.0 150.0 143.0 183.0 165.0 158.0 152.0 154.0
brutos span nova: 3.0 2.2 1.8 1.7 2.5 1.7 2.0 2.0 2.3 2.2 1.8 1.7 1.9 3.0 2.1
```
No AVISO line, so no wingetrs was left running.

Decision: **the optimization stays.** (1) The median wall delta is -210.8 ms, within the limit of L = -50 ms. (2) max(new) 120.3 ≤ max(base) 534.0. (3) Tests 1–3 pass. The new span median is 2.0 ms, below the expected 5 ms.

## Mutation proofs
Each run used `$CARGO_TARGET_DIR` = `target/t70` and `-TestCommand 'cargo test --quiet --test winget_details <name>'`:
1. `-Path src/sources/winget/details.rs -Anchor 'blobs.get(&vd_key)' -Replacement 'None::<Vec<u8>>'` (version_data_is_cached_by_catalog_hash)
   `[OK] mutação PEGA: 'blobs.get(&vd_key)' -> 'None::<Vec<u8>>' fez o teste falhar (exit 101).` The test that failed was `version_data_is_cached_by_catalog_hash`: the wiremock expect(1) check panicked in exposed_server.rs.
2. `-Path src/sources/winget/details.rs -Anchor 'hex::encode(hash)' -Replacement 'hex::encode(&hash[..4])'` (version_data_is_cached_by_catalog_hash)
   `[OK] mutação PEGA: 'hex::encode(hash)' -> 'hex::encode(&hash[..4])' fez o teste falhar (exit 101).` The test that failed was `version_data_is_cached_by_catalog_hash` at winget_details.rs:123.
3. `-Path src/cache/mod.rs -Anchor '.then_some(bytes)' -Replacement '.then_some(bytes).or(Some(Vec::new()))'` (tampered_cached_version_data_is_refetched)
   `[OK] mutação PEGA: '.then_some(bytes)' -> '.then_some(bytes).or(Some(Vec::new()))' fez o teste falhar (exit 101).` The test that failed was `tampered_cached_version_data_is_refetched` at :141.
4. `-Path src/sources/winget/details.rs -Anchor 'tracing::warn!(%expected, %actual, url = %vd_url, "versionData does not match the catalog hash; not cached");' -Replacement 'let _ = (expected, actual); return Err(WingetError::ManifestIntegrity);'` (version_data_with_another_hash_is_shown_but_not_cached)
   `[OK] mutação PEGA: '...' -> 'let _ = (expected, actual); return Err(WingetError::ManifestIntegrity);' fez o teste falhar (exit 101).` The test that failed was `version_data_with_another_hash_is_shown_but_not_cached` at :160.
5. The probe table is in the "Probe" section above.

TDD: before the implementation, tests 1 and 2 failed at the `blobs.get(HASH_HEX).is_some()` assertion (lines 123 and 144). Test 3 already passed, because the old code downloaded every time. It guards against a regression, and mutation 4 shows that it catches one.

## Decisions
- The new tests use their own helper, `server_expecting(n)`, so they can set `.expect(n)` on the versionData mock. The original `server()` helper is unchanged.
- In `show.manifest`, the span now covers `manifest::parse` too. To do that, the blob/GET and the parse moved into an async block. The behavior is the same.
- The `HashMismatch` warning logs `expected`, `actual` and the URL.
- Test 3 changes the last byte with `^= 0xff`.

## Concerns
- `target/t70` was a separate target dir for the debug builds. It is removed at the end.
- The cleanup removed only this task's artifacts from the scratchpad. The other files there came from earlier sessions and were left alone.
- The `HKCU\Software\wingetrs-test\perf` key does not exist (`reg query` returned "não pôde localizar a chave"). `leftovers.ps1` reports `[OK]`.

## Review fixes
Commit: `a2798e3` fix(show): a failed versionData cache write no longer fails show
1. `details.rs`: when `put_verified` for versionData fails with any error other than `HashMismatch`, the code now logs `tracing::warn!` and continues with the downloaded bytes. The manifest path is unchanged. New test `version_data_cache_write_failure_does_not_fail_show`: a directory named `<HASH_HEX>.tmp` inside the blobs dir makes only the versionData write fail (the manifest is still written). It checks that `show` returns `Ok` with the same `PackageDetails` as a run with a working cache, and that the `<HASH_HEX>` blob was not created.
   Mutation: `-Anchor 'tracing::warn!(error = %other, url = %vd_url, "could not cache versionData");' -Replacement 'return Err(WingetError::Cache(other));' -TestCommand 'cargo test --quiet --test winget_details version_data_cache_write_failure_does_not_fail_show'`
   `[OK] mutação PEGA: 'tracing::warn!(error = %other, url = %vd_url, "could not cache versionData");' -> 'return Err(WingetError::Cache(other));' fez o teste falhar (exit 101).` The test that failed was `version_data_cache_write_failure_does_not_fail_show` at :190.
   The first attempt made the blobs dir a file. That failed because the manifest write also fails in that setup, and the manifest path still returns an error by design.
2. Test 3 now checks that no file named after the altered hash exists in the blobs dir: `assert!(!dir.path().join(hex::encode(&hash)).exists())`.
Gate:
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 97s
[OK]   test (626 passed, 16 ignored) 158s
[OK]   gate verde
```
