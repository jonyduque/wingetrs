Status: DONE

Commits:
a9f853f feat(install): .tar.xz and .nupkg downloads

Gate:
[OK]   fmt 3s
[OK]   clippy 32s
[OK]   test (935 passed, 23 ignored) 480s
[OK]   gate verde

Mutation proofs:
mutate.ps1 -Path src/install/archive.rs -Anchor '| ArchiveKind::TarXz' -Replacement '| ArchiveKind::Zip' -TestCommand 'cargo test --quiet --test install_archive manifest_and_extractor_agree'
[OK] mutação PEGA: '| ArchiveKind::TarXz' -> '| ArchiveKind::Zip' fez o teste falhar (exit 101).

Decisions:
- Test archive built in-test with lzma_rust2::XzWriter (no fixture); the writer worked via feature unification with the dev-dependency sevenz-rust2 "compress".
- extract_tar takes the already-wrapped reader; the file is opened in extract_one with io_at errors.
- supported(): SevenZip and Msi stay false (brief lists only TarXz/Nupkg).
- Ran rustfmt only on my files (cargo fmt would touch other agents' files).

Concerns:
- XzReader::new has no memory limit (u32::MAX); a hostile xz dictionary could allocate a lot before the byte budget applies. new_mem_limit exists if wanted.
- cargo tree -d showed no lzma duplicate.

## Follow-up: xz memory limit
- XzReader::new_mem_limit(f, true, XZ_MEM_LIMIT_KB) with (MAX_7Z_DICT >> 10) KiB; OutOfMemory io error -> ExtractError::Archive("dicionário xz maior que o limite de 1 GiB") (crate does not expose the size, so not DictionaryTooLarge).
- Test tar_xz_with_a_4_gib_dictionary_is_refused_before_allocating (dict byte patched to 40, block-header CRC32 refixed).
- Mutation: [OK] mutação PEGA: 'lzma_rust2::XzReader::new_mem_limit(f, true, XZ_MEM_LIMIT_KB)' -> 'lzma_rust2::XzReader::new(f, true)' fez o teste falhar (exit 101).
- Gate: [OK]   test (941 passed, 23 ignored) 356s / [OK]   gate verde

## Follow-up 2: review fixes
- xz path now runs under catch_unwind (extract_tar_guarded, D4-1; panic -> ExtractError::Decoder). Message reworded to "dicionário xz grande demais (limite de memória de 1 GiB)".
- Tests: damaged_tar_xz_is_a_typed_error (truncated + flipped byte), module test a_panicking_tar_decoder_becomes_a_decoder_error.
- Mutation catch_unwind removed (applied by hand, mutate.ps1 multi-line anchor gave no output): a_panicking_tar_decoder_becomes_a_decoder_error FAILED (panicked), file restored.
- Mutation: [OK] mutação PEGA: '| ArchiveKind::Nupkg' -> '| ArchiveKind::Zip' fez o teste falhar (exit 101).
- Gate: NOT green here: fmt fails on src/cli/style.rs (Task 132, not mine); rules OK, my files fmt/clippy clean, install_extract + install_archive tests pass (5 + 30). Orchestrator should run the full gate.
