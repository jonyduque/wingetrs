Status: DONE

Commits:
5b93345 feat(install): safe archive extraction

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 46s
[OK]   test (231 passed, 2 ignored) 17s
[OK]   gate verde

Mutation proofs:
[OK] mutação PEGA: '".." => return Err(unsafe_path()),' -> '".." => {}' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if self.armed {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if ty.is_symlink() || ty.is_hard_link() {' -> 'if false {' fez o teste falhar (exit 101).
(A prova do armed so pegou depois de eu adicionar ao teste do tar a asserção de que a pasta pai fica vazia; antes sobrevivia porque o teste so olhava dest.)

Decisions:
- ZipWriter aceitou os nomes adulterados diretamente; nao precisei patch de bytes.
- Temp irmao `.wingetrs-extract-<pid>-<n>`; se dest existe, faz merge (varios sources); senao rename.
- Zip: todos os nomes validados antes de escrever; symlink via unix_mode tambem recusado. Entradas fora de extract_dir sao ignoradas (mas validadas).
- extract_dir/extract_to tambem passam por safe_components; qualquer ':' em componente e recusado.

Concerns: none

## Revisao (commit f0e05cb, fix(install): manifest-supplied extract paths and Windows reserved names are refused)
Gate: [OK] gate verde (253 passed, 2 ignored). Novo: extract_all/ExtractSource (todos os sources numa temp, um unico merge/rename final; extract() e wrapper); nomes reservados Windows e componentes com . ou espaco final => UnsafePath; testes de extract_to/extract_dir com .., zip symlink, tar hardlink.
Mutacoes (todas "[OK] mutação PEGA", comando: pwsh -File scripts/mutate.ps1 -Path src/install/extract.rs -Anchor A -Replacement R -TestCommand 'cargo test --quiet --test install_extract'):
- A='.extract_to\n .map(safe_components)' R=map sem validacao => PEGA
- A='.extract_dir\n .map(safe_components)' R=idem => PEGA
- A='c if is_windows_unfriendly(c) => return Err(unsafe_path()),' R='c if false => ...' => PEGA
- A='if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {' R='if false {' => PEGA
- A='if ty.is_symlink() || ty.is_hard_link() {' R='if ty.is_symlink() {' => PEGA
- A='merge_into(&tmp, dest)?;\n guard' R='guard' => PEGA (sanidade)
Follow-up anotado: zip bomb (limite de tamanho/razao) nao tratado.
