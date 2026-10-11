Status: DONE_WITH_CONCERNS

Commits:
9cf0d5e perf(ops): download every source of a batch in parallel before applying
(`git log 4f1babe..HEAD` also lists 9b5deaa and 79b5522, which belong to the Task 43 work, not to this task.)

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 12s
[OK]   test (421 passed, 16 ignored) 80s
[OK]   gate verde

Mutation proofs:
1. Paralelismo (teste 2):
   pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'let results = download::fetch_each(ctx.http, ctx.data, &sources, ctx.progress).await;' -Replacement 'let mut results = Vec::new(); for s in &sources { results.extend(download::fetch_each(ctx.http, ctx.data, std::slice::from_ref(s), ctx.progress).await); }' -TestCommand 'cargo test --quiet --test install_ops apply_downloads_the_batch_in_parallel'
   [OK] mutação PEGA: 'let results = download::fetch_each(ctx.http, ctx.data, &sources, ctx.progress).await;' -> 'let mut results = Vec::new(); for s in &sources { results.extend(download::fetch_each(ctx.http, ctx.data, std::slice::from_ref(s), ctx.progress).await); }' fez o teste falhar (exit 101).
2. Falha antes de mudar (teste 3):
   pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'if let Err(error) = r {' -Replacement 'if let (true, Err(error)) = (false, r) {' -TestCommand 'cargo test --quiet --test install_ops apply_fails_before_any_change_when_a_download_fails'
   [OK] mutação PEGA: 'if let Err(error) = r {' -> 'if let (true, Err(error)) = (false, r) {' fez o teste falhar (exit 101).
3. Ordem (teste 1):
   pwsh -File scripts/mutate.ps1 -Path src/install/download.rs -Anchor 'let results = futures_util::future::join_all(tasks).await;' -Replacement 'let results: Vec<_> = { use futures_util::StreamExt; tasks.collect::<futures_util::stream::FuturesUnordered<_>>().collect().await };' -TestCommand 'cargo test --quiet --test install_download fetch_each_keeps_order_and_reports_each_failure'
   [OK] mutação PEGA: 'let results = futures_util::future::join_all(tasks).await;' -> 'let results: Vec<_> = { use futures_util::StreamExt; tasks.collect::<futures_util::stream::FuturesUnordered<_>>().collect().await };' fez o teste falhar (exit 101).
4. Bench: the orchestrator's job. Times of test 2 (three 400 ms zips): 605, 503, 496, 501 ms (sequential would be at least 1200 ms).

Decisions:
- The pre-download is a new `prefetch(ctx, &plans)` in ops.rs. It is called inside `apply_with_targets` right after `revalidate`, so it covers both `apply` and `apply_groups`, under the lock and before the env snapshot or any executor step. The executor did not change: it calls `fetch_all`, finds each blob in the cache and checks its hash again (the file is read one more time).
- In test 1, `ok1` is delayed 300 ms so a result in order of completion would come out visibly wrong. `ok1` and `ok2` have different contents, so their paths can be told apart.
- Test 2 got a `delay_ms` field on `App` in tests/install_ops.rs (defaults to 0).
- Progress needed no change: `LineProgress` adds up per URL behind a Mutex, and `ProgressThrottle` is atomic and always lets the final message of each URL through. Concurrent downloads already happened before this task, within one plan (`MAX_PARALLEL` 4).

Concerns:
- I changed an existing test outside the brief's list: `failure_after_a_dependency_names_what_was_installed`. It forced a wrong hash on `top`, so with this task it now fails at download time with `OpsError::Download` instead of `PartlyApplied`. The brief's test 3 asks for exactly that new behavior. To keep checking `PartlyApplied`, the test now serves `top` as a non-zip file with the right hash, so it fails at extraction after `leaf` is installed. This is the same approach as test 3 of Task 41.

## Review follow-up (Minors 1 and 2)

Commit: 6fc74d8 fix(ops): fetch a shared source once and only for revalidated plans

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 8s
[OK]   test (423 passed, 16 ignored) 127s
[OK]   gate verde

Mutation proofs:
- Deduplication (`prefetch_downloads_a_shared_source_once`):
  pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor '.filter(|(_, src)| seen.insert(download::algo_key(&src.hash)))' -Replacement '.filter(|(_, src)| seen.insert(download::algo_key(&src.hash)) || true)' -TestCommand 'cargo test --quiet --test install_ops prefetch_downloads_a_shared_source_once'
  [OK] mutação PEGA: '.filter(|(_, src)| seen.insert(download::algo_key(&src.hash)))' -> '.filter(|(_, src)| seen.insert(download::algo_key(&src.hash)) || true)' fez o teste falhar (exit 101).
- Prefetch only the revalidated plans (`prefetch_skips_dependencies_installed_meanwhile`). `plans` is shadowed after `revalidate`, so the mutant prefetches all plans before revalidating:
  pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -Anchor 'let plans = revalidate(&state, plans, targets)?;' -Replacement 'let all: Vec<&InstallPlan> = plans.iter().collect(); prefetch(ctx, &all).await?; let plans = revalidate(&state, plans, targets)?;' -TestCommand 'cargo test --quiet --test install_ops prefetch_skips_dependencies_installed_meanwhile'
  [OK] mutação PEGA: 'let plans = revalidate(&state, plans, targets)?;' -> 'let all: Vec<&InstallPlan> = plans.iter().collect(); prefetch(ctx, &all).await?; let plans = revalidate(&state, plans, targets)?;' fez o teste falhar (exit 101).

Decisions:
- Deduplication happens in `prefetch` and is keyed by `download::algo_key`, which is now `pub(crate)`. The first owner is kept, so an error names the first app. For each plan, the executor then finds the shared blob in the cache.
- In the skip test, `blobs/` is deleted after the outside install of `leaf`. Without that, the cache would serve the file and hide a second fetch.
