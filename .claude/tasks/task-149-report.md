Status: DONE_WITH_CONCERNS

Commits:
c8132da feat(update): store bucket scripts/ folders atomically

Gate: NOT green because of other agents' files. verify.ps1 output:
[OK]   rules 7s
[OK]   rules-selftest 17s
[FAIL] fmt exit 1
  error: character constant must be escaped: `\t`
  Error writing files: failed to resolve mod `prelude_core`: cannot parse ...tests\it\prelude_core.rs
Own proof: rustfmt --check on my 4 src files = 0 diffs; clippy --all-targets filtered by my paths = no findings;
cargo test --test it -- bucket_scripts:: update:: buckets:: => 44 passed; cargo test --lib => 62 passed.

Mutation proofs:
[OK] mutação PEGA: 'let Some(parts) = safe_components(rel) else {' -> 'let Some(parts) = Some(rel.split('/').map(str::to_string).collect::<Vec<_>>()) else {' fez o teste falhar (exit 101).  (test 2)
[OK] mutação PEGA: 'if !marker.exists() {' -> 'if false {' fez o teste falhar (exit 101).  (test 7)

Decisions:
- Brief anchor `safe_components(rel)` appears twice (also in write_scripts, which re-validates); used the full line `let Some(parts) = safe_components(rel) else {`.
- write_scripts lives in sources/scoop/bucket.rs. On write failure the update drops `.scripts-ok` so the next update is unconditional; failure never fails the update.
- Extra tests: byte_limit (8 MiB), empty_set_still_writes_marker, unsafe_name_in_write_fails_and_keeps_old, present_marker_keeps_conditional_get.
- Marker only affects `prev`; the freshness `skip` is unchanged (a fresh bucket without marker is not refetched until stale or forced).

Concerns:
- Gate fmt failure is from tests/it/prelude_core.rs (not mine). Update time overhead not measured (span update.scripts in place).

## Review fix
Commit: see git log (fix(scoop): review of task 149). Gate: rules/selftest/fmt OK; clippy FAIL only in winget_install/launch_win.rs and fetch.rs (other agents). Own proof: it bucket_scripts::/update::/buckets:: 49 passed; clippy filtered by my paths clean. Also edited tests/it/update.rs (seed markers; fresh buckets without marker now refetch).
