Status: DONE_WITH_CONCERNS

Commits:
ea0667f feat(buckets): add, list and remove GitHub buckets

Fix round (review): e5ab201 fix(buckets): clean catalog, index and http state on removal; stricter URLs

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 9s
[OK]   test (268 passed, 2 ignored) 14s  (gate da rodada de correção)
[OK]   gate verde

Network checks (read-only):
- buckets.json oficial: main/extras/versions/nirsoft/sysinternals/php/nonportable/java = ScoopInstaller/<Nome> (Sysinternals e Nirsoft em ScoopInstaller, NÃO niheaven); nerd-fonts = matthewjberger/scoop-nerd-fonts; games = ScoopInstaller/Games (NÃO Calinou). Usei o que o arquivo diz (divergência do brief nos itens sysinternals e games).
- codeload Main: tar.gz/HEAD -> 200; tar.gz/refs/heads/HEAD -> 404; tar.gz/refs/heads/master -> 200. Usei `.../tar.gz/HEAD` (funciona para qualquer branch padrão).

Mutation proofs:
- host não-GitHub: [OK] mutação PEGA: '!host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")' -> 'false' fez o teste falhar (exit 101).
- rm com app instalado: [OK] mutação PEGA: 'if !apps.is_empty() {' -> 'if false {' fez o teste falhar (exit 101).

Decisions:
- Config ganhou `buckets: Vec<BucketEntry{name,repo}>` (+Serialize e `Config::save`); `scoop_buckets` mantido; `buckets::resolved` une os dois (nome conhecido sem entrada usa o repo conhecido).
- `bucket_url` (fase 1) mantida intacta por causa de tests/scoop.rs; `Endpoints::production` usa o novo `repo_tarball_url` (HEAD), então main/extras agora usam HEAD.
- `remove_from_catalog` apaga a fonte `scoop:<nome>` via rusqlite direto (Catalog não tem remove_source e catalog/mod.rs está fora da lista de arquivos). `buckets::remove` só mexe na config; a CLI (fora do escopo) deve chamar remove + remove_from_catalog + Config::save.
- `installed_from_bucket` abre o state.db read-only e devolve vazio se não existe/ilegível.

Concerns:
- Comandos CLI `bucket add/list/rm` não foram ligados (src/cli não está nos arquivos da tarefa); só a API em src/buckets.rs.
- Primeira execução de mutate.ps1 estourou o timeout do shell (build concorrente de outros agentes); repetida com sucesso.

Fix round:
- Catalog::remove_source (reusa delete_source_rows de replace_source; apaga map rows e meta); buckets::remove_and_cleanup(data, cfg, name, state_db) remove catálogo, URL em http.json e regrava search.idx (apaga se falhar); remove_from_catalog (SQL direto) eliminada. Caller salva cfg.
- Entradas antigas `refs/heads/master` ficam órfãs no http.json (sem efeito funcional).
- parse_repo rejeita esquema não-https, partes . e .., caminho extra. installed_from_bucket agora Result e falha fechada.
- Mutação: [OK] mutação PEGA: '|| matches!(c, '-' | '_' | '.'))' -> '|| matches!(c, '-' | '_' | '.' | '/'))' fez o teste falhar (exit 101).
