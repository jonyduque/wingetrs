# Task 125 report

Status: DONE_WITH_CONCERNS

Commits:
3814000 feat(install): MSI extraction from embedded cabinets without running the installer

Gate (run with `CARGO_BUILD_JOBS=4`; two earlier runs failed in the test step with linker/mmap errors, `os error 1455` "arquivo de paginação é muito pequeno" and "crate required to be available in rlib format": machine memory, not code. Before those, the same tree went green with default jobs: 932 passed, 23 ignored):
```
[OK]   rules 5s
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 19s
[OK]   test (932 passed, 23 ignored) 246s
[OK]   gate verde
```

Mutation proofs:
1. Accept `Cabinet` without `#`:
   `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor "match c.strip_prefix('#') {" -Replacement "match Some(c.strip_prefix('#').unwrap_or(&c)) {" -TestCommand 'cargo test --quiet --test install_msi external_cabinet_unsupported'`
   `[OK] mutação PEGA: 'match c.strip_prefix('#') {' -> 'match Some(c.strip_prefix('#').unwrap_or(&c)) {' fez o teste falhar (exit 101).` (the test panics at its `unsupported()` helper, so it is a runtime failure, not a compile error)
2. Case-sensitive `extract_dir`:
   `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor '.all(|(a, b)| a.to_lowercase() == b.to_lowercase())' -Replacement '.all(|(a, b)| a == b)' -TestCommand 'cargo test --quiet --test install_msi extract_dir_case_and_slash'`
   `[OK] mutação PEGA: '.all(|(a, b)| a.to_lowercase() == b.to_lowercase())' -> '.all(|(a, b)| a == b)' fez o teste falhar (exit 101).`
3. Patch builds a new decoder per file (`let mut fr = self.read_folder(idx)?;` moved inside the `for` loop), in `vendor/cab-0.6.0/src/cabinet.rs`, test `sequential_reader_is_linear`:
   `[OK] mutação PEGA: '            let mut fr = self.read_folder(idx)?;` ... `' fez o teste falhar (exit 101).` (multi-line anchor; the test panics at the `counter.get() <= 2 * len` assertion)
4. Gate: green, shown above.

Fixtures (`tests/fixtures/msi/`): made once with `makecab /F <name>.ddf` (Windows 11 makecab). Each DDF sets `CabinetNameTemplate=<name>`, `DiskDirectoryTemplate=out`, `Cabinet=on`, `Compress=on`, `MaxDiskSize=0`, `RptFileName=nul`, `InfFileName=nul`, `SourceDir=src`, plus:
- `three.cab`: `CompressionType=LZX`, `CompressionMemory=21`; f_a=`alpha\r\n`, f_b=xorshift `pseudo_random(40960)` (the same as in tests/install_7z.rs), f_c=`gamma file\r\n`
- `one.cab` / `two.cab`: MSZIP; f_one=`one\r\n`, f_two=`two\r\n`
- `many.cab`: MSZIP, one folder; m000..m199, each `"file NNN\n"` × 50

Real MSI check (temporary example, deleted afterwards): 008-pkgconf, 071-ncftpclient, 074-isocreator and 089-copy-dialog-lunar-lander all produced trees identical to `target/probe-2b/msi-admin/<id>/SourceDir` (paths plus SHA-256, ignoring the admin `.msi` at the root): 5/5, 18/18, 5/5 and 116/116 files.

Decisions:
- "Transform obrigatório" = a non-empty `TRANSFORMS` property in `Property`. The brief and spec do not define it, and the S3 corpus had no case.
- A `File` row with `msidbFileAttributesNoncompressed` (0x2000) also returns `Unsupported`. The spec puts "arquivo não comprimido" out of scope, and without this check it would become a `MsiLayout` "sem dados" error that never falls back to msiexec. It has its own test (`noncompressed_file_unsupported`). I also added tests `quantum_cabinet_unsupported` (patches `typeCompress` in memory), `transform_required_unsupported`, `not_an_msi_is_unsupported` and `file_without_data_is_layout_error`.
- Before writing anything, every cabinet is opened once to check Quantum, the entry count and that every placed `File` has data; then a second pass decodes. The whole extraction after `Package::open` runs inside `catch_unwind` (like `extract_7z`), not only `for_each_file`. That way `Cabinet::new` is covered too.
- `sequential_reader_is_linear` uses `cab::Cabinet` directly on a counting reader. The public extractor API cannot take a custom reader, and the rule under test lives in the patch.
- The package is opened through `BufReader<File>` (no `Write`), so `msi` cannot write to it.

Concerns:
- `vendor/cab-0.6.0` is not formatted or linted by the gate (`cargo fmt --all` and clippy `-D warnings` passed with it). Rustc still prints 5 `mismatched_lifetime_syntaxes` warnings from the vendored crate into `target/verify.log`. They are noise, not failures; the vendored source was left unchanged apart from the patch.
- Cabinet entry sizes are keyed by name per cabinet. A broken cabinet with two entries of the same name but different sizes would raise a size-mismatch `Archive` error.
- The commit trailer uses the session's attribution line (`Claude Opus 5.5`) instead of the brief's `Co-Authored-By: Claude <noreply@anthropic.com>`.
- `vendor/cab-0.6.0/Cargo.lock` is not committed: the crate's own `.gitignore` excludes it. Cargo does not use it.

## Review round 1 (CHANGES_REQUESTED → fixes)

Status: DONE

Commits:
f5a8f8a fix(install): MSI cabinet overlap/gap budget, open inside catch_unwind, continued cabinets

Gate (`CARGO_BUILD_JOBS=4`, with task 126's uncommitted files present in the checkout):
```
[OK]   rules 4s
[OK]   rules-selftest 17s
[OK]   fmt 3s
[OK]   clippy 3s
[OK]   test (941 passed, 23 ignored) 354s
[OK]   gate verde
```
`grep -c vendor target/verify.log` = 0: the vendored crate no longer prints lifetime warnings.

Mutation proofs:
- Drop the overlap check: `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor 'if offset < end {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test install_msi overlapping_files_refused'`
  `[OK] mutação PEGA: 'if offset < end {' -> 'if false {' fez o teste falhar (exit 101).` In debug builds the test fails through an underflow panic in `end - in_files`, which `catch_unwind` turns into `Decoder` instead of the `Archive` the test expects. In release the same mutation would leave a wrapped, huge gap.
- Size check: `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor 'if counted.n != size {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test install_msi short_folder_data_is_error'`
  `[OK] mutação PEGA: 'if counted.n != size {' -> 'if false {' fez o teste falhar (exit 101).`

Changes:
1. `folder_gaps`: before anything is decoded, each folder's entries are sorted by (offset, size). An overlap raises `Archive("gabinete com arquivos sobrepostos")`, and `max(offset+size) > cCFData × 32768` raises `Archive("gabinete com arquivos além dos dados da pasta")`. The gap bytes (`max_end − Σ sizes`) are charged to the budget up front. File bytes are still charged as they are read, so the total charged is the folder's whole decoded extent. The vendored patch gained `FileEntry::uncompressed_offset()`.
   Tests: `overlapping_files_refused`, and `gap_before_file_is_charged` (offset 30 000 with a 1000-byte budget ⇒ `TooLarge`).
2. `Package::open` (and every later use of the package) now runs inside `catch_unwind`.
3. `short_folder_data_is_error`: `one.cab` with CFFILE `cbFile` raised from 5 to 100 ⇒ `Archive` containing "terminou com".
4. `continued`: reads the raw CFHEADER flags (PREV/NEXT) and each CFFILE `iFolder` (≥ 0xFFFD) before `Cabinet::new` (which would reject them as out of bounds). Either case ⇒ `Unsupported("gabinete com arquivos continuados: …")`. Test: `continued_cabinet_unsupported`, with both variants.
5. In-crate test `extract_msi::tests::unsafe_layout_leaves_root_empty`.
6. Added `#![allow(mismatched_lifetime_syntaxes)]` to `vendor/cab-0.6.0/src/lib.rs`, noted in the patch header in cabinet.rs.

Decisions:
- The folder total is the upper bound `cCFData × 32768` (MS-CAB caps `cbUncomp` per block). I did not add a pre-pass that parses every CFDATA header: with an exact total, a short folder would be refused by the extent check, and the "terminou com" check would have no fixture. The `cCFData → 1` patch suggested in the review cannot reach "terminou com": `three.cab`'s files span 40 979 bytes, more than one block's 32 768, so the extent check refuses it first. The fixture raises a file's `cbFile` instead.
- The gap charge happens during the pre-scan, so it lands before any byte of any cabinet is decoded.

Concerns: none new. The data the decoder produces after the last file of a folder is not charged; it is bounded by the rest of one block.
