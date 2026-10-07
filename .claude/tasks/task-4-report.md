Status: DONE_WITH_CONCERNS

Commits: see `git log e52ef17..HEAD` ("feat(winget): merged manifest parsing into PackageDetails")

Gate: NOT green due to another agent's file: `tests/http.rs:63 sends_validators_and_maps_304` fails (verify.ps1 stops at tests; fmt OK, clippy OK on rerun). Persisted over 2 reruns; not touched. `cargo test --quiet --test winget_manifest`: 6 passed.

TDD red: first run `error[E0583]: file not found for module manifest` (compile failure before implementing).

Mutation proofs:
[OK] mutação PEGA: '.or(root_type.clone())' -> '.or(None)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'self.short_description.or(self.description)' -> 'self.description.or(self.short_description)' fez o teste falhar (exit 101).

Decisions: added test `short_description_wins_over_description` (separate test, instead of extending installer_level_type_overrides_root) since the PowerToys starts_with did not pin the rule; no clippy lint allowances needed.

Concerns: gate failure above (other agent's http test); transient clippy failure seen once from others' in-progress files.

## Revisão (fix)
61055c5 test(winget): pin scope inheritance and homepage fallback
8f99175 feat(winget): merged manifest parsing into PackageDetails
[OK] mutação PEGA: 'i.scope.or(root_scope.clone())' -> 'i.scope.or(None)' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'self.package_url.or(self.publisher_url)' -> 'self.package_url' fez o teste falhar (exit 101).
