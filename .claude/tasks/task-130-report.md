Status: DONE_WITH_CONCERNS

Commits:
3a906dd feat(examples): phase 2b coverage by download sub-case

Gate:
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 3s
[OK]   test (967 passed, 23 ignored) 60s
[OK]   gate verde

Run: `cargo run --release --example oracle_2a -- tests/fixtures/scoop/manifests <catalog.db>` (the brief's `tests/fixtures/scoop` fails with "stream did not contain valid UTF-8" because that dir holds Main-mini.tar.gz and `manifests/`; the manifests subdir is the right arg).

Real table (catalog 4115 scoop manifests; denominator = all manifests of the bucket):
```
  bucket         total               2a               2b
  extras          2425     1092 (45.0%)     1312 (54.1%)
  main            1690     1294 (76.6%)     1391 (82.3%)
extras: 7z com extract_dir 37, 7z sem 72, msi com 68, msi sem 14, nupkg 29, plain 208, tar.gz 23, zip 861
        unsupported: pre_install 686, post_install 174, installer 137, innosetup 82, pre_uninstall 11, post_uninstall 8,
        psmodule 5, uninstaller 4, .rar 4, plan unsafe path 1, no arch variant 1
main:   7z com 20, 7z sem 26, msi com 35, msi sem 2, nupkg 8, plain 298, tar.gz 111, tar.xz 6, zip 885
        unsupported: pre_install 171, post_install 58, installer 40, innosetup 17, pre_uninstall 2, post_uninstall 1,
        psmodule 5, .rar 1, .zst 2, plan duplicate shim name 1, no arch variant 1
```
7z+msi: Main 20+26+35+2 = 83 (spec estimate 84), Extras 37+72+68+14 = 191 (matches). 2a column equals the spec's "2a simples" (Main 1.288 est. vs 1.294; Extras 1.092).

unsup-7z check. Before (2a report, docs/superpowers/reports/2026-10-10-fase2a-validacao.md:176):
`fixture unsup-7z__main__cdrtools.json: resolve: não suportado: download .7z: suportado na fase 2b`
After (Task 128 renamed it; current run):
`fixture sevenzip__main__cdrtools.json: ok` and `fixture msi__main__7zip19.00-helper.json: ok`; summary `fixtures: 19 total, 17 ok, 2 unsup-* recusados como esperado, 0 divergentes`.

Mutation proofs: no mutation proof. The change is an example binary (no tests); the verifiable outputs are the real run above. Not run: mutate.ps1 has no test command that exercises an example.

Decisions:
- Sub-case of a manifest = most demanding kind among its sources (msi > 7z > nupkg > tar.xz > tar.gz > zip > plain); extract_dir split only for 7z/msi (matches the spec table).
- "2a" column = ok manifests whose kind is zip, tar.gz or plain; "2b" = every manifest plan_install accepts.
- Coverage runs over all scoop manifests (not only "simple"), so scripts count in the denominator like the spec table.

Concerns: none. Only examples/oracle_2a.rs edited; rustfmt applied to that file only.
