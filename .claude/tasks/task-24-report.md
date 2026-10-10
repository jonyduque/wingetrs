Status: DONE

Commits:
795d77c feat(install): typed scoop manifest with support check

Gate:
[OK]   rules 3s
[OK]   fmt 1s
[OK]   clippy 14s
[OK]   test (231 passed, 2 ignored) 81s
[OK]   gate verde

Mutation proofs (cargo test --quiet --test install_manifest):
- arch overrides root (swap field precedence): [OK] mutação PEGA: '.and_then(|v| v.get(key)) .or_else(|| root.get(key))' -> swapped ... (ripgrep_x64 etc. failed)
- '#/' rename (use URL basename): [OK] mutação PEGA: 'Some(n) => n.to_string(),' -> 'Some(_) => base.rsplit(...)...' fez o teste falhar (exit 101).
- no-hash refused (continue instead): [OK] mutação PEGA: 'return unsupported("sem hash para verificar o download");' -> 'continue;' fez o teste falhar (exit 101).

Decisions:
- serde_json::Value parsing (fields are polymorphic) instead of derive structs.
- Extension check uses the saved file name (after #/ rename); also rejects .txz/.tbz2/.zst.
- Script keys and innosetup are checked on root and on the chosen arch variant; persist is root-only.
- X86 host prefers only 32bit; X64 prefers 64bit then 32bit; Arm64 arm64, 64bit, 32bit.
- Hash with unknown prefix/non-hex is Invalid, not Unsupported.

Concerns: none (gate first failed due to another agent's tests/buckets.rs; passed on retry)

## Revisão (round 2)
Commit: 1adc107 test(install): pin manifest variant overrides, hash counts and arch preference
Gate: [OK]   test (247 passed, 2 ignored) / [OK]   gate verde
Decisão: mais hashes que URLs => Invalid (checado primeiro); menos => Unsupported "sem hash".
Mutações PEGAS: apagar check_keys(v)? da variante; p.to_ascii_lowercase()->p; hashes.len() > urls.len() -> false.
Testes novos: hash_count_mismatch, support_keys_inside_architecture_variant, extension_is_decided_by_the_renamed_file, variant_replaces_root_fields, host_arch_preference_and_hash_parse.
