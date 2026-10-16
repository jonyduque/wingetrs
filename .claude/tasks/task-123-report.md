Status: DONE_WITH_CONCERNS (review fixes applied: see "Review round 1" at the end)

Commits:
95ef55a feat(install): 7z extraction with content sniffing, pre-decode validation and limits

Gate:
[OK]   rules 5s
[OK]   rules-selftest 18s
[OK]   fmt 2s
[OK]   clippy 60s
[OK]   test (904 passed, 23 ignored) 329s
[OK]   gate verde

Dependencies (from the published sevenz-rust2 0.24.0 Cargo.toml):
- `[dependencies] sevenz-rust2 = { version = "=0.24.0", default-features = false, features = ["bzip2", "deflate", "ppmd"] }`. LZMA, LZMA2, BCJ (all), BCJ2 and Delta have no feature (always built). Left out: `aes256`, `compress`, `util`, `brotli`, `lz4`, `zstd`, `nt-time`.
- `[dev-dependencies]` the same plus `"compress"`. `rust-version = "1.93"`.
- New crates in the lock: sevenz-rust2 0.24.0 (Apache-2.0), lzma-rust2 0.22.0 (Apache-2.0), ppmd-rust 1.5.0 (CC0-1.0 OR MIT-0), bzip2 0.6.1 (MIT OR Apache-2.0), libbz2-rs-sys 0.2.5 (bzip2-1.0.6). crc32fast and flate2 were already there.
- `cargo tree -d`: no new duplicate. The new crates appear only under `miniz_oxide v0.9.1 <- flate2`, and that pair was already in the tree.

Mutation proofs:
1. safe_components moved into the loop (precheck removed):
   `mutate.ps1 -Path src/install/extract_7z.rs -Anchor 'safe_components(&f.name)?;' -Replacement '' -TestCommand 'cargo test --quiet --offline --test install_7z unsafe_second_entry_writes_nothing'`
   `[OK] mutação PEGA: 'safe_components(&f.name)?;' -> '' fez o teste falhar (exit 101).`
2. Skipped entries not charged (`budget.charge(n as u64)?;` removed from `drain`):
   - against test 3 (`extract_dir_on_solid_7z_charges_skipped`): `[X] mutação SOBREVIVEU: ... A regra não está coberta.` (see Concerns)
   - against the module test `decode_charges_entries_outside_extract_dir`: `[OK] mutação PEGA: '        budget.charge(n as u64)?;` (test `install::extract_7z::tests::decode_charges_entries_outside_extract_dir --- FAILED`)
3. catch_unwind removed (the whole `let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {...}));` replaced by `Ok::<_, Box<dyn Any + Send>>((|| {...})());`), module test `decoder_panic_becomes_an_error` with a reader that panics on the first read of the packed data:
   `[OK] mutação PEGA: '    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {` (the panic escaped at `extract_7z.rs:327`, the injected `panic!`)
4. `MAX_7Z_DICT: u64 = 1 << 30;` -> `MAX_7Z_DICT: u64 = u64::MAX;`, test `huge_dictionary_refused_before_decoding`:
   `[OK] mutação PEGA: 'MAX_7Z_DICT: u64 = 1 << 30;' -> 'MAX_7Z_DICT: u64 = u64::MAX;' fez o teste falhar (exit 101).`
Extra (checks I added):
- `if counted.n == entry.size {` -> `if true {` / `short_decoded_stream_is_an_error`: `[OK] mutação PEGA: 'if counted.n == entry.size {' -> 'if true {' fez o teste falhar (exit 101).`
- `ExtractError::Io(m) if counted.failed =>` -> `... if false =>` / `corrupt_data_is_an_archive_error`: `[OK] mutação PEGA: 'ExtractError::Io(m) if counted.failed =>' -> 'ExtractError::Io(m) if false =>' fez o teste falhar (exit 101).`
- SFX candidate validation `Ok(matches!(opened, Ok(Ok(_))))` -> `Ok(true)` / `sfx_7z_extracts`: `[OK] mutação PEGA: 'Ok(matches!(opened, Ok(Ok(_))))' -> 'Ok(true)' fez o teste falhar (exit 101).`

Test 5 (attributes): no byte editing was needed. `ArchiveEntry.has_windows_attributes` and `windows_attributes` are pub fields, and the writer serializes them. Test 6 edits the raw header: it finds the single `01 21 21 01` (1 coder, LZMA2, 1 property byte), sets the property to 40, and recomputes the next-header CRC and the start-header CRC with a CRC-32 written in the test.

Decisions:
- `find_7z_in_pe` validates each candidate with `sevenz_rust2::Archive::read` on an `OffsetReader`, inside `catch_unwind`. It refuses a candidate whose start-header CRC is 0, because `sevenz-rust2` then falls into `try_to_locale_end_header`, which guesses the header without checking it. `crc32fast` is not a direct dependency, so the library's own check is used. `pe_overlay_start(head)` returns 0 when the PE headers don't parse, so the brief's `MZ + 4 KiB` case is scanned from 0. `OffsetReader` lives in `sniff.rs`.
- The pre-decode check also covers PPMd memory (props[1..5]) under `MAX_7Z_DICT`, refuses AES coders (`Encrypted`) and any coder outside the readable list (`UnsupportedMethod`). Anti-items, reparse points and symlinks give `UnsafePath(name)`.
- Two hardenings the brief didn't ask for. 1) `Crc32VerifyingReader` in sevenz-rust2 returns `Ok(0)` without error when the stream ends short. Each entry is therefore counted and checked against the header's `size`, so a short stream gives `Archive("... terminou com 5 de 6 bytes")`. 2) A read error from the decoder inside `write_entry` (CRC mismatch) gives `Archive`, not `Io`. Each has its own fixture and mutation (above).
- I wrote `extract_7z.rs` and `sniff.rs` before the integration tests (not strict TDD). Each rule's tests were then proven by mutation.
- The commit uses the session's Co-Authored-By trailer (system attribution), not the brief's generic `Claude <noreply@anthropic.com>`.

Concerns:
- **Mutation 2 against test 3 cannot be caught as designed.** The brief requires the pre-decode check "soma de `size` acima do orçamento ⇒ TooLarge", which counts every entry (skipped ones included, the same conservative rule as zip). So test 3 is refused before decoding, and the charge inside the loop is never the one that fires. With honest sizes no black-box test can reach it: the library caps every entry at its declared size. The rule is proven by the module test `decode_charges_entries_outside_extract_dir`, which calls `decode` directly. If the plan meant the pre-check to count only mapped entries, tell me. I chose the stricter reading.
- Test 4 got a second case: the same archive (COPY method) with its packed data overwritten. Without it, mutation 1 survives, because `TempGuard` cleans up whatever the first entry wrote. The brief's assertions are unchanged.
- The brief lists `src/install/mod.rs` under Modify; per the orchestrator, it was not touched (stubs already declared). The `rust-version` bump needs "registro na spec": not done, specs belong to the orchestrator.
- Environment: disk C: was at 100% (4.6 GB free; `target/debug` 53G, `target/orch` 38G, `target/probe-2b` 17G). The first two gates failed with `os error 112` and invalid metadata from parallel builds. The third gate (above) passed with 21 GB free. Mutations ran on the shared `target/` (no separate `CARGO_TARGET_DIR`, for lack of disk). A gate from another agent running at the same time may have seen a 7z mutant.
- `ArchiveKind::SevenZip.supported()` is still `false` and the executor is unchanged (Task 128).

## Review round 1 (CHANGES_REQUESTED, 8 items)

Gate (after the fixes):
[OK]   rules 5s
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 14s
[OK]   test (918 passed, 23 ignored) 249s
[OK]   gate verde

Each item, its new test (tests/install_7z.rs) and its mutation (`T = cargo test --quiet --offline --test install_7z`):
1. Anti-item fixture: `anti_item_refused` (pub `is_anti_item`).
   `'f.is_anti_item || ' -> ''` / `$T anti_item_refused`: `[OK] mutação PEGA: 'f.is_anti_item || ' -> '' fez o teste falhar (exit 101).`
2. `check_entries` on 7z: `too_many_entries_refused` (`max_entries: 1` on 2 entries ⇒ `TooManyEntries{limit:1}`; 2 passes).
   `'check_entries(archive.files.len() as u64, limits)?;' -> ''`: `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
3. LZMA and PPMd dictionary branch: `lzma_and_ppmd_dictionaries_refused`. Both are written with their own method; the u32 LE at props[1..5] is patched to 0x80000000 and the CRCs are recomputed ⇒ `DictionaryTooLarge{dict:0x8000_0000}`.
   `'id == EncoderMethod::ID_LZMA || id == EncoderMethod::ID_PPMD' -> 'id == EncoderMethod::ID_PPMD'`: `[OK] mutação PEGA`
   `... -> 'id == EncoderMethod::ID_LZMA'`: `[OK] mutação PEGA`
4. `encrypted_unknown_method_and_bad_lzma2_property_refused`:
   - AES: the coder is spliced to flags 0x24 + `06 F1 07 01`, with the header size and CRCs fixed ⇒ `Encrypted`.
   - Id `7F` ⇒ `UnsupportedMethod("[7F]")`.
   - LZMA2 property 41 ⇒ `Archive("propriedade LZMA2 inválida: 41")`.
   `'if id == EncoderMethod::ID_AES256_SHA256 {' -> 'if false {'`: `[OK] mutação PEGA`
   `'if !READABLE.contains(&id) {' -> 'if false {'`: `[OK] mutação PEGA`
   `'if p > 40 {' -> 'if false {'`: `[OK] mutação PEGA`
5. Header bomb: `check_header(r)` (extract_7z.rs) runs before `ArchiveReader::new` and in every SFX candidate. It works on raw bytes, as follows:
   - It refuses a zeroed start CRC: sevenz-rust2 then guesses the header without any CRC.
   - It refuses `next_header_size > 64 MiB`.
   - When the next header is kEncodedHeader (0x17), it walks PackInfo/UnpackInfo (port of 7-Zip ReadStreamsInfo, pack/unpack parts). Each coder goes through `check_method` (AES, unknown, dictionary). Any coder unpack size > 64 MiB ⇒ `Archive("cabeçalho 7z grande demais")`. Counts are bounded by the bytes left.
   Tests:
   - `encoded_header_with_huge_unpack_size_refused`: 80 long names force an encoded header. Untouched, it extracts. With the unpack size spliced to 2 GiB it is refused (< 5 s detector). In a PE, the SFX candidate is refused by the pre-check, which the captured debug log line `candidato 7z recusado offset=4098 error=arquivo inválido: cabeçalho 7z grande demais` proves.
   - `oversized_raw_next_header_refused`
   - `zeroed_start_header_refused`
   Mutations:
   - `'if r.number()? > MAX_7Z_HEADER {' -> 'if r.number()? > u64::MAX - 1 {'`: `[OK] mutação PEGA`
   - `'    if size > MAX_7Z_HEADER {' -> '    if false {'` / oversized_raw: `[OK] mutação PEGA`
   - `'    if hh[8..12] == [0, 0, 0, 0] {' -> '    if false {'` / zeroed: `[OK] mutação PEGA`
   - sniff.rs `'if let Err(e) = crate::install::extract_7z::check_header(&mut sub) {' -> 'if let Err(e) = Ok::<(), String>(()) {'` / encoded_header: `[OK] mutação PEGA`
     Note: against the result alone this mutation SURVIVED, because `Archive::read` also fails afterwards; the extra work the pre-check saves is invisible in the result. Hence the log assertion.
6. sniff.rs: a panic during candidate validation ⇒ `tracing::warn!(offset, panic = <message>)`, still `false`. `panic_message` is now `pub(crate)`. No test: no real header makes the library panic.
7. `.nsis` is tested before `mapper.map`: `extract_dir_holding_only_nsis_is_missing` (`d/x.nsis` + `other/y`, extract_dir `d` ⇒ `MissingExtractDir("d")`).
   Mutation, the old order (`let nsis = ...; match mapper.map(comps) { Some(rel) if !nsis => ..., Some(_) => drain ...`): `[OK] mutação PEGA`
8. `sum(block.get_unpack_size())` is in the pre-check: `declared > limit || unpacked > limit`.
   `' || unpacked > budget.limit' -> ''` against the whole install_7z suite: `[X] mutação SOBREVIVEU`. Equivalent mutation through sevenz-rust2:
   - `read_sub_streams_info` derives the last substream of each block as `unpack_size - sum(others)` and refuses `sum > unpack_size`. So the files' sizes always add up to the unpack size of every block that has files.
   - The only way to get `unpacked > declared` is a block with 0 substreams. `for_each_entries` never decodes such a block (no files ⇒ the BlockDecoder loop is empty).
   I kept the check because it was asked for, as defense in depth; no fixture can tell it apart.

Commits: see the final message (one commit for this round).
