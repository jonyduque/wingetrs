1. Status: DONE
2. Commits:
d10ed9c feat: flag ARP records whose uninstaller is missing
3. Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 2s
[OK]   clippy 16s
[OK]   test (800 passed, 20 ignored) 286s
[OK]   gate verde
4. Mutation proofs:
- Remove powershell.exe/winget.exe from the exclusion list (replaced by pwsh.exe x3): `[OK] mutação PEGA: '    "powershell.exe", ...` (missing_uninstaller_cases)
- No %VAR% expansion: `[OK] mutação PEGA: 'out.push_str(&expand(&tail[..j])?);' -> 'out.push_str(&tail[..j]);' fez o teste falhar (exit 101).`
- `git grep -n "delete_subkey\|remove_dir\|remove_file" src/installed`: no output (exit 1).
- Live test (live_stale_arp_records, --release --ignored): Cities: Skylines, Counter-Strike: Global Offensive (steam.exe), USB-Driver-Installer 1.0.3.0 (uninst.exe); stale count = 3, as expected.
5. Decisions:
- missing_uninstaller lives in arp.rs (pure), but is called from arp_win.rs after filter_arp, so filter_arp stays pure/disk-free; RawArpEntry.uninstall_string is cloned before filter_arp.
- ListRow/OtherRecord JSON carry `stale` (null when absent); `second` closure got a `stale` flag.
- Added `stale: None` to InstalledApp literals in state.rs, appx_win.rs, scoop.rs and tests/{correlate,installed,overrides}.rs (compile-required, outside the Files list).
6. Concerns: none (no dedicated test for the CLI `(sobra provável)` suffix; not requested).

Review fixes: cases for pwsh/rundll32/cmd, token-aware .exe, try_exists, list_text/JSON test. Mutations PEGA: pwshx.exe, rundll33.exe, cmdx.exe. Gate: [OK] test (801 passed, 20 ignored) / [OK] gate verde.
