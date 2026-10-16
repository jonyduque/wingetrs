Status: DONE

Commits:
3674fff refactor(install): one ArchiveKind for manifest, plan and extractor
6961fe0 test(install): assert nothing is written when the extractor refuses a format

Gate:
[OK]   fmt 6s
[OK]   clippy 47s
[OK]   test (860 passed, 22 ignored) 61s
[OK]   gate verde

Mutation proofs:
1. scripts/mutate.ps1 -Path src/install/archive.rs -Anchor 'has(".7z")' -Replacement 'has(".7zz")' -TestCommand 'cargo test --quiet --test install_archive'
   [OK] mutação PEGA: 'has(".7z")' -> 'has(".7zz")' fez o teste falhar (exit 101).
2. scripts/mutate.ps1 -Path src/install/extract.rs -Anchor 'Ok(ArchiveKind::Plain) => {' -Replacement 'Ok(ArchiveKind::Plain | ArchiveKind::SevenZip) => {' -TestCommand 'cargo test --quiet --test install_archive manifest_and_extractor_agree'
   [OK] mutação PEGA: ... fez o teste falhar (exit 101).
3. install_planner: passed in the full gate; `git status tests/snapshots` clean.
4. grep UNSUPPORTED_EXTS / .tgz" in manifest.rs, plan.rs: no matches.

Decisions:
- Err(ext) in extractor yields "<file> (<ext>)" inside Unsupported.
- `of` implemented as an if-chain on a lowercased name (suffix checks; no overlap issues).

Concerns: none
