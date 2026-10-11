Status: DONE

Commits:
6a18ba4 perf(upgrade): look up the installed app's catalog row instead of loading the catalog

Gate:
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (444 passed, 16 ignored) 19s
[OK]   gate verde
(an earlier run failed only on Task 47's in-progress files and on one timing-flaky test, apply_downloads_the_batch_in_parallel, under parallel load; it passed 3/3 alone and in the final gate)

Mutation proofs:
1. Bucket respected (get -> find_by_id(app)?.into_iter().next()):
[OK] mutação PEGA: '.get(&crate::sources::scoop::source_name(bucket), app)?' -> '.find_by_id(app)?.into_iter().next()' fez o teste falhar (exit 101).
2. Exact case (app -> &app.to_lowercase()):
[OK] mutação PEGA: '.get(&crate::sources::scoop::source_name(bucket), app)?' -> '.get(&crate::sources::scoop::source_name(bucket), &app.to_lowercase())?' fez o teste falhar (exit 101).
3. Identical candidate: mutação não aplicável: construção única (target::candidate reusado, agora pub(crate)).
4. Callers swap: no mutation proof (equivalent by construction). grep resolve_target in install_cmd.rs/jobs.rs shows only install (install_cmd.rs:204) and plan_install (jobs.rs:85).

Decisions:
- plan_upgrade_all: missing state.db row -> Err("{app}: não está no registro do wingetrs") via ok_or_else.
- Shared catalog fixture extracted as world_with_extras() for both tests.

Concerns:
- Benchmark/span measurement not done (orchestrator's job): ganho não medido.
