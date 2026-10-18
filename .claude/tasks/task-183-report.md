Status: DONE

Commits:
12e83be feat(winget-install): orphan ARP records — verified .reg export, then delete (HKCU in process, HKLM via helper)

Gate:
[OK]   rules 9s
[OK]   rules-selftest 26s
[OK]   fmt 3s
[OK]   clippy 20s
[OK]   test (1558 passed, 85 ignored) 88s
[OK]   gate verde

Mutation proofs:
1. export_args with /reg:32 and /reg:64 swapped:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/orphans.rs -Anchor 'Hive::MachineX64 => ("HKLM", Some("/reg:64")),\n        Hive::MachineX86 => ("HKLM", Some("/reg:32")),' -Replacement '<the same with 32/64 swapped>' -TestCommand 'cargo test --quiet --test it wi_orphans::export_args_views'`
   [OK] mutação PEGA: 'Hive::MachineX64 => ("HKLM", Some("/reg:64")),
2. check_export skipped:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/orphans.rs -Anchor '    check_export(&file, &o.key)?;\n    match machine_hive' -Replacement '    match machine_hive' -TestCommand 'cargo test --quiet --test it wi_orphans::empty_export_never_deletes'`
   [OK] mutação PEGA: '    check_export(&file, &o.key)?;
3. still_orphan checked after the export (-AnchorFile/-ReplacementFile in scratchpad t183-anchor.txt / t183-repl.txt, the still_orphan block moved below `reg.export(...)?`):
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/orphans.rs -AnchorFile .../t183-anchor.txt -ReplacementFile .../t183-repl.txt -TestCommand 'cargo test --quiet --test it wi_orphans::no_longer_orphan_is_skipped'`
   [OK] mutação PEGA: '    if !reg.still_orphan(o.hive, &o.key) {
4. Live (read-only): `cargo test --release --quiet --test it live_export_hklm_without_admin -- --ignored`
   running 1 test
   .
   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1541 filtered out; finished in 0.09s

Decisions:
- `remove_orphan` never reuses an existing backup path: when `backups\<ts>\<name>.reg` exists it tries `<name>-2.reg` … `-100.reg` (only `backup_file` gives the plain name). Without this, a stale valid `.reg` would pass `check_export` after an export that wrote nothing, and two keys that sanitize to the same name in one session would overwrite each other's backup with `/y`. Test `existing_backup_is_never_reused`.
- `ts` goes through the same sanitizer as the key (it's a path component).
- `check_export`: reads at most 8 MiB (reading is capped before decoding), refuses odd-length UTF-16, the header must be exactly the first line, and the key line must start with `[HKEY_` and end with `\Uninstall\<key>]` (case-insensitive), so a subkey `[...\<key>\Sub]` doesn't count.
- `SystemArp::still_orphan` uses the same expansion and existence rule as `installed::arp_win::stale_uninstaller` (user env, `try_exists` errors count as existing). A missing key or read error returns false, so nothing gets deleted.
- `remove_orphan` also refuses an invalid key (`valid_arp_key`) before calling anything (test `invalid_key_is_refused_before_anything`).

Concerns:
- An extra mutation on the backup-reuse guard (`Ok(_) => continue` -> `return Ok(c)`) didn't compile (INCONCLUSIVO). That guard is not in the brief's list. I didn't try another form.
- `src/winget_install/elevated_rules.rs` shows as modified in the working tree by another agent. I didn't touch it and it isn't in my commit.
