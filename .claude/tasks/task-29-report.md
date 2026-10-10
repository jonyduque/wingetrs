Status: DONE_WITH_CONCERNS
Commits:
c503914 feat(install): install state database and install.json
Gate:
[OK]   fmt 2s
[OK]   clippy 16s
[OK]   test (253 passed, 2 ignored) 19s
[OK]   gate verde
Mutation proofs:
- [OK] mutação PEGA: 'tx.execute("DELETE FROM artifacts WHERE app = ?1", params![app.app])?;' -> '' fez o teste falhar (exit 101).
- [OK] mutação PEGA: 'let install = read_install_json(&path);' -> 'let Some(install) = read_install_json(&path) else { continue };let install = Some(install);' fez o teste falhar (exit 101).
Decisions:
- path_users recebe (app, entry): "other than app" exige saber o app; a assinatura do brief tinha só entry.
- Broken = install.json ilegível em current (current ausente/apontando para pasta apagada): listado com version None e bucket None. O flag `broken` do ListRow fica para a Task 37 (deriva de source==Wingetrs && version==None).
- Versão: state.db tem prioridade, depois install.json.
Concerns: nenhum outro arquivo precisou de ajuste (nenhum match exaustivo em InstallSource). A primeira verify falhou por arquivos de outros agentes em edição (extract.rs); passou na repetição.

Revisão (fix): commit seguinte "fix(install): path users use PATH equivalence; pin scan and atomic install.json".
- path_users compara via path_edit::norm (agora pub(crate)); mutação 'norm(&t) == want' -> 't == entry' PEGA; mutação 'if !path.is_dir() {' -> 'if false {' PEGA (stray.txt no teste).
- write_install_json atômico (tmp + rename); open em arquivo lixo => Err; journal_mode wal testado.
- "Quebrado" = InstalledApp com source Wingetrs e version None (Task 37 usa).
- Gate: [OK] test (262 passed, 2 ignored) / [OK] gate verde
