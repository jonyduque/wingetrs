Status: DONE_WITH_CONCERNS

Commits:
4f08bd4 feat(github): releases client with asset digests and redirect tag

Gate: fmt, rules, rules-selftest, clippy OK; step `test` FAILED only in another agent's test (Task 189):
  wi_select::bridge_reasons_one_fixture_each (tests/it/wi_select.rs:144, "tipo portable: instalado pelo winget").
  My module: `cargo test --quiet --test it gh_release::` -> 9 passed, 0 failed. Full "[OK] gate verde" not obtained.

Mutation proofs:
[OK] mutação PEGA: 'let Some(sha256) = a.digest.as_deref().and_then(parse_digest) else {' -> '...unwrap_or([0u8; 32])...' fez o teste falhar (exit 101).  (asset_without_digest_dropped)
[OK] mutação PEGA: '.filter(|u| download_url_allowed(repo, u))' -> '.filter(|u| { let _ = download_url_allowed(repo, u); true })' fez o teste falhar (exit 101).  (foreign_download_host_dropped)
Source restored after each (verified by grep).

Decisions:
- Added Http::get_limited_raw (status+headers, needed for X-RateLimit-*) and RawReply; get_limited delegates to it; Http::final_url for latest_tag. http.rs change is slightly bigger than "only the final-URL method".
- Download allowlist is prefix match with trailing "/" (host cannot be extended); tests added for github.com.evil, userinfo "@evil" on both hosts, http://, other repo.
- Extra tests beyond the 7: draft_or_garbage_is_typed, repo_names_follow_bucket_rules.

Concerns: gate test step blocked by Task 189 test (above). No .join with external data written.
