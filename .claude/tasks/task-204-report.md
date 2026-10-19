Status: DONE_WITH_CONCERNS

Commits:
4205df2 feat(install): extract cabinet archives detected by content

Gate:
[OK]   rules 11s
[OK]   rules-selftest 40s
[OK]   fmt 5s
[OK]   clippy 74s
[OK]   test (1910 passed, 87 ignored) 100s
[OK]   gate verde

Mutation proofs:
- sniff Cab arm: mutate.ps1 -Path src/install/sniff.rs -Anchor 'head.starts_with(&CAB_SIGNATURE)' -Replacement 'false' -TestCommand 'cargo test --quiet --test install_extract seven_z_named_cab_content_extracts'
  [OK] mutação PEGA: 'head.starts_with(&CAB_SIGNATURE)' -> 'false' fez o teste falhar (exit 101).
- multivolume refusal: -Path src/install/extract_cab.rs -Anchor 'if spanned {' -Replacement 'if false && spanned {' (cab_multivolume_refused)
  [OK] mutação PEGA: 'if spanned {' -> 'if false && spanned {' fez o teste falhar (exit 101).
- extra: safepath on names (cab_path_escape_refused)
  [OK] mutação PEGA: 'let comps = safe_components(&name)?;' -> 'let comps = vec![name.clone()];' fez o teste falhar (exit 101).

Decisions:
- New src/install/extract_cab.rs reuses place/Counted/folder_gaps/raw_continued from extract_msi.rs (made pub(super); MSI logic untouched).
- CAB fixtures are built in the test (hand-written uncompressed CFHEADER) in tests/install_extract.rs mod cab_files, not make_fixtures.py. Multivolume test covers PREV flag, NEXT flag and iFolder 0xFFFD/E/F, each separately. Extra test cab_byte_cap_refused.
- ArchiveKind::Cab engine() is None (suffix says it); progress::engine_of gives "CAB". Also edited src/install/progress.rs, examples/oracle_2a.rs (exhaustive matches) and tests/it/install_archive.rs table (outside Files list, needed to compile/cover).
- Entry cap checked incrementally while listing; names validated by safe_components before any write; Quantum refused.

Concerns:
- Not run against the real yori .cab (forbidden); orchestrator should verify with the real file / 9 manifests with .cab. Fixtures are synthetic uncompressed only; MSZIP/LZX paths are the cab crate's (same as MSI).
- Working tree has unrelated modified docs/ARMADILHAS.md and task-150-report.md (not mine, not committed). leftovers.ps1 lists an old conhost (pid 18592, 16:30), not started by me.

## Revisão (round 2)
- Quantum: o crate `cab` só falha na decodificação, então a recusa em extract_cab.rs fica. Teste `cab_quantum_refused` (typeCompress 0x1042 = Quantum, nível 4, memória 16). `[OK] mutação PEGA: 'if let CompressionType::Quantum(..) = folder.compression_type() {' -> 'if false {' fez o teste falhar (exit 101).`
- `cab_same_bytes_both_names` (x.cab e x.7z, árvores iguais) e `cab_mszip_extracts` (CK + deflate raw via flate2, no teste) adicionados.
- Gate: `[OK]   test (1913 passed, 87 ignored)` / `[OK]   gate verde`.
