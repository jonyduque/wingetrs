Status: DONE (optimization REVERTED: sem ganho medido)

Commits:
2348ee9 perf(catalog): index lower(id) for case-insensitive id lookups
8634bf2 Revert "perf(catalog): index lower(id) for case-insensitive id lookups"
(base f28b1fd; the revert removes the index and both tests, since they require the index. Net diff vs base: none.)

Gate (before the optimization commit):
[OK]   test (630 passed, 16 ignored) 207s
[OK]   gate verde

Mutation proofs (all on 2348ee9):
[OK] mutação PEGA: 'CREATE INDEX IF NOT EXISTS ix_pkg_id_lower ON packages(lower(id));' -> '' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'const FIND_BY_ID_WHERE: &str = "lower(id) = lower(?1)";' -> 'const FIND_BY_ID_WHERE: &str = "id = ?1 COLLATE NOCASE";' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'const FIND_BY_ID_WHERE: &str = "lower(id) = lower(?1)";' -> 'const FIND_BY_ID_WHERE: &str = "lower(id) = ?1";' fez o teste falhar (exit 101).

EXPLAIN (base, la):  SCAN packages USING COVERING INDEX sqlite_autoindex_packages_1
EXPLAIN (nova, la2): SEARCH packages USING COVERING INDEX ix_pkg_id_lower (<expr>=?)

Measurements (base binary = f28b1fd, which already has the show.find_by_id span; roots la / la2):
Series 1 (second run; first run had max(nova) 139.1 > max(base) 128.6 wall, delta -22.7 ms, so rerun per rule):
wall base med=86.2 max=316.9 | nova med=71.2 max=162.5 | delta pareado med=-15.8 ms
piso noop 31.3 ms | sobre o piso: base 54.9, nova 39.9 (L = 5.5 ms)
span base med=14.6 max=19.5 | nova med=0.2 max=0.3 | delta -14.3 ms
Series 2 (install ripgrep --dry-run; first attempt invalid: I ran the new exe on la by mistake, so la was rebuilt and redone):
wall base med=133.4 max=192.2 | nova med=134.4 max=170.6 | delta pareado med=+2.5 ms (L = 10.2) -> passes
Series 3 (update --force, local msix, span update.catalog_write{source=winget}):
wall base med=2970.7 | nova med=3004.4 | delta +110.3 ms
span base med=748.0 max=964.0 | nova med=816.0 max=1030.0 | delta pareado med=+104.0 ms (+13.9% > +10% limit) -> FAILS
brutos span base: 807 839 934 964 748 731 712 804 664 678 886 715 827 689 660
brutos span nova: 930 973 865 959 926 1030 816 771 781 763 828 761 778 807 799

Decisions:
- Series 3 breaches the write-cost limit (+13.9% vs +10%), so reverted despite the show gain (-14 ms span, -15.8 ms wall). I did not rerun series 3 (the brief only allows a rerun when the max criterion alone fails).
- Revert also dropped the tests (they depend on the index). Reintroducible from 2348ee9 if the write cost is mitigated.

Concerns:
- The index would make `show`/dependency lookup ~15 ms faster at the cost of ~100 ms more on each winget catalog write. A cheaper variant (create the index after bulk insert, or only for the scoop sources) was not tried.
- Cleanup done: la*, tgt-*, perf73 exes removed, worktree gone, test registry key absent, leftovers clean. Several python.exe from 11/10 20:24 exist and are not mine (python server PID 19820 was stopped by the script).
