Status: DONE

Commits: (see `git log --format='%h %s' 98257ad..HEAD`; one commit `feat(tui): list icons setting`)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (717 passed, 18 ignored) 52s
[OK]   gate verde

Mutation proofs:
- list_icons always sent (src/tui/app.rs, test list_icons_setting): `[OK] mutação PEGA: 'list_icons: s ...`
- list_fetch_cmds ignoring list_icons_visible (`if false ||`): `[OK] mutação PEGA: 'if !self.list_icons_visible ...`

Decisions:
- `config::LIST_ICONS = ["visible","opened"]` is the single list of valid values (first = default).
- No saved_list_icons/revert-on-failure (unlike icons): a failed save leaves the live value until restart. Not requested by the brief.
- No extra call to list_fetch_cmds needed: `update()` already runs it after every msg, including change_setting. While on the Settings tab it returns empty by design; it requests the visible rows on return to a list tab.
- Outside Files list, adapted to the signature/row-order change: tests/tui_jobs.rs (new arg), tests/tui_state.rs (existing SaveSettings literal gets `list_icons: None`; theme_setting_flips_dark Down count 7 -> 8 because Tema moved one row).
- Snapshot: new row consumed one blank row after Tema block (Aparência section grew by one).

Concerns: none
