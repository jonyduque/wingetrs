Status: DONE

Commits:
42e0863 fix(install): MSI layout depth limit independent of file order; cover collision arms
8de1745 feat(install): pure MSI source layout with the msiexec /a collision rule

Gate: (checkout principal, depois de 95ef55a da Task 123; arquivos não commitados da Task 127 na árvore)
[OK]   rules 6s
[OK]   rules-selftest 39s
[OK]   fmt 6s
[OK]   clippy 73s
[OK]   test (908 passed, 23 ignored) 77s
[OK]   gate verde

Mutation proofs (todas em src/install/msi_layout.rs, `-TestCommand 'cargo test --quiet --test install_msi_layout <teste>'`):
1. Inverter destino e origem em `DefaultDir` (`-Anchor '|(_, src)| src' -Replacement '|(dst, _)| dst'`):
   - teste `source_name_after_colon_and_long_form`:
     [OK] mutação PEGA: '|(_, src)| src' -> '|(dst, _)| dst' fez o teste falhar (exit 101).
   - teste `two_levels_destination_differs_from_source`:
     [OK] mutação PEGA: '|(_, src)| src' -> '|(dst, _)| dst' fez o teste falhar (exit 101).
2. Empate de versão escolhe a maior sequência (`-Anchor '_ => a.sequence < b.sequence' -Replacement '_ => a.sequence > b.sequence'`, teste `collision_rules`):
   [OK] mutação PEGA: '_ => a.sequence < b.sequence' -> '_ => a.sequence > b.sequence' fez o teste falhar (exit 101).
3. (revisão) Limite de profundidade só pelo trecho percorrido (`-Anchor 'if base_depth + chain.len() > MAX_DEPTH' -Replacement 'if chain.len() > MAX_DEPTH'`, teste `depth_limit_independent_of_file_order`):
   [OK] mutação PEGA: 'if base_depth + chain.len() > MAX_DEPTH' -> 'if chain.len() > MAX_DEPTH' fez o teste falhar (exit 101).
4. (revisão) Não versionado vence versionado (`-Anchor '(None, Some(_)) => false' -Replacement '(None, Some(_)) => true'`, teste `collision_rules`, caso a'):
   [OK] mutação PEGA: '(None, Some(_)) => false' -> '(None, Some(_)) => true' fez o teste falhar (exit 101).

Decisions:
- Vetores do teste 7: um crate descartável no scratchpad (`msi = "=0.10.0"`, `Cargo.lock` copiado da sonda `target/probe-2b/s3`, `--offline`) extraiu as linhas Directory/Component/File de `071-ncftpclient`, `074-isocreator` e `008-pkgconf`. Usei 3 arquivos de cada, conferidos contra `target/probe-2b/msi-admin/<id>/SourceDir`. O crate foi apagado depois. `Cargo.toml` não foi tocado.
- O memo guarda (profundidade, caminho), e o limite (64 diretórios abaixo da raiz) é checado como profundidade herdada + trecho percorrido.
- Versões: um `Version` que não começa por dígito (chave de arquivo companheiro) conta como não versionado. Uma parte não numérica vale 0, e uma parte faltante vale 0 (`1.0` = `1.0.0`).
- Cada nome de diretório ou de arquivo precisa virar exatamente um componente seguro: `/` ou `\` no nome dá `Unsafe`, e `FileName` igual a `.` também dá `Unsafe`. Um componente ausente da tabela Component, ou cujo `Directory_` não existe, dá `MissingDirectory`.
- `src/install/mod.rs` não foi editado: o stub já declarava `pub mod msi_layout;`, conforme o orquestrador.

Concerns:
- O primeiro gate (8de1745) rodou num worktree temporário no scratchpad, já removido, porque o checkout principal não compilava (Task 123 em andamento) e o disco estava cheio. O gate acima, no checkout principal, cobre os dois commits.
