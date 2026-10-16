Status: DONE_WITH_CONCERNS

Commits: see `git log --format='%h %s' 39db361..HEAD` (commit "feat(scripts): versioned list oracle against Get-WinGetPackage")

Gate: verify.ps1 stopped at `[FAIL] fmt exit 1` — "Diff in src/cli/mod.rs:916" (Task 120's file, not mine). Lines: `[OK]   rules 4s`, `[OK]   rules-selftest 13s`, `[FAIL] fmt exit 1`. check_rules.ps1 alone: exit 0.

Mutation proofs:
mutate.ps1 -Path scripts/oracle_list.ps1 -Anchor '$byId[$p.Id.ToLowerInvariant()]' -Replacement '(match by name)' -TestCommand 'pwsh -NoProfile -File scripts/oracle_list.ps1 -SelfTest'
[OK] mutação PEGA: '$byId[$p.Id.ToLowerInvariant()]' -> '($rows | Where-Object { $_.name.StartsWith($p.Name) } | Select-Object -First 1)' fez o teste falhar (exit 1).
SelfTest: `[OK]   oracle_list selftest` (swapped id lowers precision 2/3 and is not counted in recall).

Real run (Get-WinGetPackage, 116 winget ids, 277 rows):
recall bruto 114/116 = 98,3%; liquido 114/115 = 99,1% (1 aceita: Ruby 4.0)
concordancia de versao 98/114 = 86,0% (diffs: Unknown vs vazio, DesktopRuntime 8.0.31 vs 8.0.31.36421, Python.Launcher 3.14.7 vs 3.14.7150.0, "> 1.4.11" vs 1.4.11)
precisao 115/115 = 100,0%
faltas: RubyInstallerTeam.RubyWithDevKit.4.0 (ausente, aceita), MSYS2.MSYS2 (ausente)

Decisions: allow id is RubyWithDevKit.4.0 (real id, brief said 3.4 as example); MSYS2 not allowed (brief names only Ruby); extra fixture allow.json for selftest; name lookup = equal or wingetrs name starts with winget Name.

Concerns: gate red from Task 120 fmt (src/cli/mod.rs). Version concordance 86% is dominated by version-string formatting, not wrong correlation.

## Review fixes (800bb93)
Name-mismatch hits counted and listed (5th number), distinct-id precision, duplicate ids listed, version diffs split formato/real, null name safe (empty name never compatible).
Mutation: mutate.ps1 anchor 'if (-not (Test-NameCompat ...))' -> 'if ($false)': [OK] mutação PEGA: ... fez o teste falhar (exit 1).
Real run: recall 114/116 (98,3%) bruto, 114/115 (99,1%) líquido; versão 98/114 (formato 5, real 11); precisão 114/114 ids distintos; nome diferente 21; duplicado no wingetrs: Microsoft.PowerShell.
