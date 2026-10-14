Status: DONE_WITH_CONCERNS

Commits: b5f4c28 feat: scoop app icon from its winget twin

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 179s
[OK]   test (782 passed, 19 ignored) 286s
[OK]   gate verde

Mutation proofs:
- pick takes first of two: [OK] mutação PEGA: 'if same.next().is_some() {' -> 'if false {' fez o teste falhar (exit 101).
- site ignores github owner: [OK] mutação PEGA: 'if matches!(host.as_str(), "github.com" | "gitlab.com" | "codeberg.org") {' -> 'if false {' fez o teste falhar (exit 101).
- list_fetch_cmds winget-only (anchor: the `.filter(|(_, key)| ...)` line in list_fetch_cmds, replaced by one adding `key.starts_with("winget/") &&`): [OK] mutação PEGA ... fez o teste falhar (exit 101) (test scoop_rows_are_prefetched).

Live (release, copy of real catalog in a temp dir, blobs in the temp dir; removed after), tests/scoop_winget_match.rs live_winget_candidates_timing_and_twins:
- 7zip: twin 7zip.7zip, icon present (the case from the probe)
- git: 3 candidates, twin Git.Git (gitforwindows.org), but no icon in the winget manifest
- nodejs: twin OpenJS.NodeJS, no icon in the manifest
- docker: Docker.DockerDesktop NOT picked (docs.docker.com vs www.docker.com), as intended
- ripgrep: no candidates (winget id tail is MSVC, name "Ripgrep (MSVC)", moniker not "ripgrep")
- winget_candidates time per call: 19.0 to 28.5 ms (release, full scan of winget rows). Above the 5 ms limit.
The live test composes the public pieces (candidates, get, details, pick); the TUI path (twin_icon in run.rs) was not run end to end (no interactive terminal), only compiled/gated.

Decisions:
- Icon resolution lives in load_details (Scoop arm), so details and list prefetch share it; load_icon then remembers under the Scoop key unchanged. winget arm split into winget_details.
- winget_candidates scans id/name/moniker of winget rows ordered by id in Rust (norm cannot be done in SQL); stops at 5.
- site/URL parsing is hand-written (no url crate in Cargo.toml).
- Existing tests prefetch_asks_for_visible_rows_once / prefetch_skips_mapped_and_off (renamed helper winget_keys -> row_keys) updated: they encoded the winget-only rule being removed.

Concerns:
- winget_candidates 19-28 ms per call (over the 5 ms limit); runs on spawn_blocking per visible Scoop row (max 4 at once). Could be indexed in memory if it shows in use; not measured in the TUI.
- Every Scoop details/prefetch now fetches the manifests of up to 5 candidates (cached in blobs after first time).
