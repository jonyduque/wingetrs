Status: DONE

Commits:
c2d2d3b feat(install): pure install planner
(the other SHAs in `git log d4a1752..HEAD` — da11f61, c503914, 56a98c7, f0e05cb — belong to parallel tasks 29/26)

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 7s
[OK]   test (260 passed, 2 ignored) 68s
[OK]   gate verde

Test: `cargo test --quiet --test install_planner` -> `test result: ok. 7 passed; 0 failed`. TDD: the first run failed with E0432/E0425 (types missing), as expected.

Mutation proofs:
1. Step order (LinkCurrent moved before Persist; multi-line anchor from `steps.push(Step::Promote);` through `steps.push(Step::LinkCurrent);`, passed via bash $'..'):
   `pwsh -File scripts/mutate.ps1 -Path src/install/planner.rs -Anchor "<Promote..Persist loop..LinkCurrent>" -Replacement "<Promote, LinkCurrent, Persist loop>" -TestCommand 'cargo test --quiet --test install_planner step_order'`
   [OK] mutação PEGA: '    steps.push(Step::Promote);
2a. Sanitization of `:`:
   `-Anchor "'<' | '>' | ':' | '\"'" -Replacement "'<' | '>' | '\"'" -TestCommand '... shortcut_names'`
   [OK] mutação PEGA: ''<' | '>' | ':' | '"'' -> ''<' | '>' | '"'' fez o teste falhar (exit 101).
2b. Leaving `..`:
   `-Anchor 'if c == "." || c == ".." {' -Replacement 'if c == "." {' -TestCommand '... shortcut_names'`
   [OK] mutação PEGA: 'if c == "." || c == ".." {' -> 'if c == "." {' fez o teste falhar (exit 101).
3. Persist dir vs file inverted:
   `-Anchor "Some(last) if last.contains('.') => PersistKind::File," -Replacement "Some(last) if !last.contains('.') => PersistKind::File," -TestCommand '... persist_dir_vs_file'`
   [OK] mutação PEGA: 'Some(last) if last.contains('.') => PersistKind::File,' -> 'Some(last) if !last.contains('.') => PersistKind::File,' fez o teste falhar (exit 101).
4. AlreadyInstalled bypassed:
   `-Anchor 'Some(i) if i.version == m.version => {' -Replacement 'Some(i) if false && i.version == m.version => {' -TestCommand '... upgrade_and_same'`
   [OK] mutação PEGA: 'Some(i) if i.version == m.version => {' -> 'Some(i) if false && i.version == m.version => {' fez o teste falhar (exit 101).

Snapshot sample (all 15 `.snap.new` inspected before renaming to `.snap`), tests/snapshots/install_planner__persist-file-and-dir__main__i2p-zero.snap:
```
Instalar app 1.20 (bucket main, sem arquitetura)
  1. Instalar o lançador de comandos em C:\W\shims\.launcher\shim.exe
  2. Baixar [1] https://github.com/i2p-zero/i2p-zero/releases/download/v1.20/i2p-zero-win.v1.20.zip (i2p-zero-win.v1.20.zip, sha256 37c38e66a1f4935a21c2514eb4692827834ec8bd9c2dfc11d6871f7a07dbdd16)
  3. Extrair [1] i2p-zero-win.v1.20.zip para C:\W\apps\app\.staging-1.20 (só a subpasta i2p-zero-win.v1.20\router)
  4. Mover C:\W\apps\app\.staging-1.20 para C:\W\apps\app\1.20
  5. Persistir pasta C:\W\apps\app\1.20\conf -> C:\W\persist\app\conf (junção)
  6. Persistir arquivo C:\W\apps\app\1.20\i2p.base -> C:\W\persist\app\i2p.base (hardlink)
  7. Apontar C:\W\apps\app\current -> C:\W\apps\app\1.20 (junção)
  8. Criar comando i2p-zero em C:\W\shims (i2p-zero.cmd) -> C:\W\apps\app\current\bin\launch.bat
  9. Garantir C:\W\shims no PATH do usuário
 10. Registrar app 1.20 (main) em C:\W\state.db
```

Decisions:
- Added `pub root: PathBuf` to `InstallPlan` (deviation from the Interfaces block): Display must print full paths, but `Promote`/`LinkCurrent`/`EnsureLauncher`/`RecordState`/`RemoveOldVersion` carry no paths; `Layout` has no `PartialEq` and layout.rs is not my file, so the root is stored and Display rebuilds a `Layout` from it.
- `$architecture` with `arch = None` uses `Arch::host()` (compile-time constant, still pure). Variables replaced longest-first (`$persist_dir`, `$architecture` before `$dir`, `$app`).
- One `Extract` per source; Display says "Copiar" instead of "Extrair" for non-archive files (anything other than .zip/.tar.gz/.tgz). Shim-name conflicts compare case-insensitively (Windows file names). Persist kind is decided by the last component of the source (`in_version`).
- Shortcut names: empty components are dropped (all-empty ⇒ `_`); `.lnk` is appended to the last component, not set as extension (names may contain dots). Absolute `env_add_path` entries (after substitution) are kept as-is; relative ones are joined to `current`.

Concerns:
- `Download` Display shows the URL, file name and hash but not the cache destination (the planner does not know the cache dir).
- Commit trailer uses the session's attribution lines (Claude Opus 5.5 + Claude-Session) instead of the brief's `Co-Authored-By: Claude <noreply@anthropic.com>`.

## Fix round (review CHANGES_REQUESTED)

Commit: 4b292f3 fix(install): planner refuses unsafe manifest paths; pin conflicts and keep_old

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 5s
[OK]   test (268 passed, 2 ignored) 54s
[OK]   gate verde

Changes: new `src/install/safepath.rs` (`safe_components -> Option<Vec<String>>`, `is_windows_unfriendly`), moved verbatim from extract.rs; extract.rs keeps a 1-line wrapper mapping `None` to `ExtractError::UnsafePath` (install_extract: 13 passed, unchanged). Planner: `PlanError::UnsafePath` for persist source/target, bin path, shortcut target/icon (also refuses empty/`.`-only paths); env_add_path resolved under `current`: empty/`.` dropped, exact `$dir` keeps `current`, anything that escapes `current` (absolute, `..`, sibling prefix like `$dir-evil`) is refused. Shortcut components: trailing `.`/space become `_` (`foo.` -> `foo_`), reserved device names get a `_` prefix (`CON` -> `_CON`). New tests: unsafe paths (13 cases), empty/dot env entries, ShimConflict("X") case-insensitive, keep_old, reserved/trailing names, persist with a different target.

Mutation proofs:
[OK] mutação PEGA: 'Some(c) if !c.is_empty() => Ok(c),' -> 'Some(c) => Ok(c), None => Ok(Vec::new()),' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'let comps = safepath::safe_components(rel).ok_or_else(unsafe_path)?;' -> 'let comps = rel.split(|c: char| c == std::path::MAIN_SEPARATOR || c == 47 as char).filter(|c| !c.is_empty()).map(String::from).collect::<Vec<_>>();' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'Some(_) => return Err(unsafe_path()),' -> 'Some(r) => r,' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if matches!(raw.trim(), "" | ".") {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !names.insert(name.to_lowercase()) {' -> 'if false && !names.insert(name.to_lowercase()) {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !names.insert(name.to_lowercase()) {' -> 'if !names.insert(name.clone()) {' fez o teste falhar (exit 101).
[OK] mutação PEGA: '&& !input.keep_old' -> '&& true' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'out.insert(0, '_');' -> '' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'out.push_str(&tail);' -> 'out.push_str(&c[kept..]);' fez o teste falhar (exit 101).

Concerns (fix round): in scoop, `env_add_path: "."` adds the app dir itself; per the orchestrator's decision it is now dropped, while an explicit `$dir` still adds `current`. Snapshots unchanged.
