Status: DONE

Commits: see `git log b0c1bf6..HEAD` (scoop commit: "feat(scoop): bucket tarball import and manifest details"; other agents' commits interleave)

Gate:
[OK]   fmt 1s
[OK]   clippy 14s
[OK]   test (87 passed, 0 ignored) 6s
[OK]   gate verde
(an earlier run failed clippy only on src/sources/winget/{index,mod}.rs, another agent's files; passed on retry)

Red: stub run `cargo test --quiet --test scoop` -> "test result: FAILED. 0 passed; 7 failed". Green: "7 passed; 0 failed".

Mutation proofs:
[OK] mutação PEGA: 'Err(_) => skipped.push(file),' -> 'Err(e) => return Err(e),' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if let Some(a) = alias {' -> 'if let Some(a) = None::<&str> {' fez o teste falhar (exit 101).

Decisions:
- Backslash literals written as char::from(92) (tooling mangled escapes); behavior identical.
- Fixed arch order 64bit, 32bit, arm64 (serde_json map is sorted); used for both commands and installers.
- Alias = element 1 of a bin array item; manifest without "version" is a Json error (skipped).
- fetch_bucket wiremock test added (304 when If-None-Match present).

Concerns: none

## Revisão (round 2)
Commit: test(scoop): pin bucket path filter and per-arch bin (ver git log). Adicionados 4 testes (path filter, bin por arquitetura, neutral com architecture {}, formas malformadas sem pânico); fallback neutral agora aplica quando nenhum installer foi produzido; escapes viraram '\'.
Mutation proofs:
[OK] mutação PEGA: '[_, "bucket", file]' -> '[.., "bucket", file]' fez o teste falhar (exit 101).  (precisou de casos "bucket/top.json" e "A/B/bucket/deep.json")
[OK] mutação PEGA: 'a.get(key)' -> 'a.get("zzz")' fez o teste falhar (exit 101).
Gate: verify.ps1 falha só em fmt de tests/installed.rs (outro agente); scoop: rustfmt limpo, clippy --all-targets sem avisos em scoop, `--test scoop` 11 passed. Gate completo não verde por arquivo de outro agente.
