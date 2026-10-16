# wingetrs — tarefas paralelas das etapas 2–3 do roteiro (Tasks 120–122)

> **For agentic workers:** execução pelo laço `run-task`. Briefs: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-15-wingetrs-paralelas.md`.

**Goal:** adiantar, em arquivos disjuntos das Tasks 114 e 116–119, três itens do roteiro (`docs/superpowers/plans/2026-10-15-wingetrs-roteiro-2b-2d.md`): pontos de medição da busca, hash em streaming do download e o oráculo de `list` versionado.

**Spec:** `docs/superpowers/reports/2026-10-15-desempenho-brainstorm.md` (Ponto 1 §1, Ponto 2 §1) e spec 2b §"Defeitos" / spec 2d (defeito do `download.rs:92`).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes de cada tarefa.
- **Nenhuma destas tarefas mede tempo.** A medição da etapa 2 roda depois, sozinha. Não rode `bench.ps1`.
- Outros agentes trabalham em paralelo no mesmo checkout: Task 114 em `src/bridge/mod.rs`, `src/tui/jobs.rs`, `tests/bridge.rs`, `tests/tui_jobs.rs`; Task 116 em `tests/install_archive.rs`; sondas em `target/probe-2b/`. `git add -- <seus arquivos>`, nunca `-A`; commit com `git commit -- <caminhos>`. Gate vermelho só em arquivo de outra tarefa ⇒ anote no relatório, não conserte.
- Sem dependência nova. `cargo update` proibido.
- Mensagens ao usuário em português; código e comentários em inglês.
- Nunca rodar `winget install/upgrade/uninstall` nem nada que mude a máquina.

## Review Focus

1. Span criado antes de `init_logging` não é registrado: medições de antes do log precisam de `Instant` e de um evento emitido depois.
2. Hash em streaming nas fronteiras de bloco (0 byte, bloco−1, bloco, bloco+1) dá o mesmo resultado que o hash em memória.
3. Oráculo: um id trocado precisa aparecer como discordância, não como acerto.

---

### Task 120: Pontos de medição da busca (sem medir)

**Modelo:** sonnet.

#### Where this fits

- Brainstorm, Ponto 1 §1: faltam spans para decidir onde vai o tempo de `wingetrs search`. **O número que decide é `search.join_wait`.**
- `src/cli/mod.rs`:
  - `run()` (~:882): dispara `EarlySearch` (thread), depois `data.ensure()?`, `init_logging(...)`, `Config::load(...)`, `startup.runtime` (span já existe), `rt.block_on(run_async(...))`, `startup.runtime_drop` (já existe).
  - `run_async` (~:617): `startup.http` (já existe); no ramo `Command::Search`: `index_file::probe(...)`, `refresh_if_stale`, `early.as_mut().and_then(EarlySearch::join)` (~:666), `search_best` (fallback), e a impressão (`println!`/`print!` de `output::search_table`).
  - `EarlySearch::join` (~:565).
  - `init_logging` usa `with_span_events(FmtSpan::CLOSE)` (~:342): spans fechados viram linhas com `time.busy`/`time.idle` no log.
- `src/catalog/index_file.rs`: `pub fn search` (~:412) → `fn search_with`.
- **Armadilha:** spans abertos **antes** de `init_logging` não vão para lugar nenhum (o subscriber ainda não existe).

#### What binds this task

1. **Antes do log** (`data.ensure()` e o próprio `init_logging`): meça com `std::time::Instant` e, logo depois de `init_logging`, emita um evento `tracing::debug!(target: "wingetrs::startup", stage = "startup.ensure", ms = <f64>)` e outro com `stage = "startup.log"`.
2. **Depois do log**, spans `tracing::debug_span!`:
   - `startup.config` em volta de `Config::load`;
   - `search.probe` em volta de `index_file::probe(...)` (só o probe, sem o `exists()`);
   - `search.refresh_check` em volta de `refresh_if_stale`;
   - **`search.join_wait`** em volta de `early.as_mut().and_then(EarlySearch::join)`;
   - `search.fallback` em volta de `search_best` (só quando roda);
   - `search.print` em volta da serialização + `print!`/`println!`.
3. **Dentro de `index_file::search_with`:** spans `idx.read`, `idx.parse`, `idx.scan`, `idx.sort` nas etapas que existirem (leitura do arquivo, validação do cabeçalho, varredura/pontuação, ordenação/corte). Se uma etapa não existir separada, não invente: diga no relatório. A thread antecipada roda antes do log: estes spans só aparecem quando `search_with` roda depois do subscriber (fallback ou TUI), e isso é aceito; registre no relatório.
4. Nenhuma mudança de comportamento nem de saída.

**Files:**
- Modify: `src/cli/mod.rs`, `src/catalog/index_file.rs`
- Test: `tests/cli_offline.rs` (ou o arquivo de testes de CLI offline que já existir; confira com `ls tests`)

Testes:
1. **`search_logs_join_wait`:** rode `wingetrs search <termo>` offline contra a fixture que os testes de CLI offline já usam, com `-v` ou a variável de log que eles usam (copie o padrão de um teste existente que leia o log ou o stderr); a saída de log contém `search.join_wait` e `startup.ensure`. Se nenhum teste existente lê o log, use `--verbose` com stderr (há `stderr_span_events(verbose)` em `src/cli/mod.rs:347`) e confira o stderr.
2. A saída padrão (`stdout`) do `search` é idêntica à de antes (um teste existente de saída da busca continua passando sem alteração).

#### Checks beyond the steps

1. Mutação: renomear `"search.join_wait"` para `"search.join_waitx"` ⇒ teste 1 **PEGA**.
2. `pwsh -File scripts/check_rules.ps1` verde (nada novo em stdout fora de `cli/`).
3. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(cli): spans for search startup, join wait and index stages`

---

### Task 121: Hash do download em streaming

**Modelo:** sonnet.

#### Where this fits

- `src/install/download.rs`:
  - `download_one` (~:135): baixa para `tmp` com `http.get_to_file`, depois **`std::fs::read(&tmp)` lê o arquivo inteiro** (~:151), chama `verify(&src.hash, &bytes)` e, se o algoritmo não for sha256, calcula `Sha256::digest(&bytes)` para o nome do blob.
  - `cached` (~:110): `BlobStore::get(&name)` também lê o blob inteiro (`src/cache/mod.rs:188`, que confere sha256 = nome) e, se não for sha256, `verify(spec, &bytes)`.
  - `verify(spec, bytes)` (~:25) e `hex_of` (~:16) ficam (outros usos).
- As fases 2b e 2d baixam arquivos de centenas de MB; ler tudo na memória é o defeito.

#### What binds this task

1. Nova função em `download.rs`:

```rust
/// Streams `path` once and returns (hex of `algo`, hex of sha256); one pass, 64 KiB buffer.
fn hash_file(path: &Path, algo: HashAlgo) -> std::io::Result<(String, String)>;
```

   Para `HashAlgo::Sha256` os dois valores são o mesmo digest (não calcule duas vezes). Hex em minúsculas, igual a `hex_of`.
2. `download_one` usa `hash_file` em vez de `std::fs::read` + `verify` + `Sha256::digest`. Comparação com o manifesto: case-insensitive, como `verify`. Rode o `hash_file` dentro de `tokio::task::spawn_blocking` com o handle **sempre aguardado** e pânico propagado com `resume_unwind` (mesmo padrão do `rename` logo abaixo, ~:166).
3. `cached` deixa de usar `BlobStore::get` para blobs de download: monta `data.blobs_dir().join(&name)`, se existir chama `hash_file` e confere sha256 = `name` **e**, se o algoritmo não for sha256, o hash do manifesto. Não mexa em `BlobStore` (o catálogo usa).
4. Nenhuma mudança de comportamento: os mesmos erros (`HashMismatch` etc.), o mesmo nome de blob, o mesmo índice.

**Files:**
- Modify: `src/install/download.rs`
- Test: testes de módulo em `src/install/download.rs` (`#[cfg(test)] mod tests`, já que `hash_file` é privada) e os testes existentes de download (`grep -ln "fetch_all\|fetch_each" tests`)

Testes (de módulo):

```rust
#[test]
fn hash_file_matches_in_memory_at_block_edges() {
    let dir = tempfile::tempdir().unwrap();
    for len in [0usize, 1, 65_535, 65_536, 65_537, 1_048_579] {
        let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
        let p = dir.path().join(format!("f{len}"));
        std::fs::write(&p, &data).unwrap();
        for algo in [HashAlgo::Sha256, HashAlgo::Sha1, HashAlgo::Sha512] {
            let (h, s) = hash_file(&p, algo).unwrap();
            let want = match algo {
                HashAlgo::Sha256 => hex_of(&Sha256::digest(&data)),
                HashAlgo::Sha1 => hex_of(&Sha1::digest(&data)),
                HashAlgo::Sha512 => hex_of(&Sha512::digest(&data)),
                // add the MD5 arm the same way if HashAlgo has it
            };
            assert_eq!(h, want, "{algo:?} len {len}");
            assert_eq!(s, hex_of(&Sha256::digest(&data)), "sha256 len {len}");
        }
    }
}
```

(Se `HashAlgo` tiver `Md5`, cubra também; o `match` precisa ser exaustivo.)

Também: um teste de `cached` com blob adulterado (conteúdo trocado com o mesmo nome) ⇒ `None`. Os testes existentes de download (wiremock) continuam passando sem mudança.

#### Checks beyond the steps

1. Mutação: no laço de leitura, trocar o fatiamento `&buf[..n]` por `&buf[..]` (ou o equivalente que alimente bytes além do lido) ⇒ `hash_file_matches_in_memory_at_block_edges` **PEGA**.
2. Mutação: em `cached`, pular a conferência do sha256 do arquivo ⇒ teste do blob adulterado **PEGA**.
3. `grep -n "fs::read(" src/install/download.rs` só acha o `read_index`/`prune_index` (índice pequeno), nunca o download nem o blob.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(install): hash downloads in one streaming pass instead of reading them into memory`

---

### Task 122: Oráculo de `list` versionado

**Modelo:** sonnet.

#### Where this fits

- Brainstorm, Ponto 2 §1: o `oracle.py` das Tasks 15–21 nunca foi para o git. Último resultado medido: 114/116 (Task 21; faltas MSYS2 e RubyWithDevKit; Ruby é divergência aceita, `tests/correlate.rs:352`). Relatórios anteriores: `.claude/tasks/task-17-report.md`, `.claude/tasks/task-21-report.md` (leia só as seções do oráculo).
- Nesta máquina: módulo `Microsoft.WinGet.Client` 1.29.380 disponível (`Get-WinGetPackage`, mesmo motor COM do `winget list`).
- `target/release/wingetrs.exe list --json` (277 linhas hoje) devolve objetos com `name`, `app_key`, `origin`, `installed_version`, `correlation`, `latest`, `state`, … Formas de `correlation`: a string `"Unmatched"`, ou `{"Matched": {"source", "package_id", "rule"}}`, ou `{"Ambiguous": {...}}`.

#### What binds this task

Criar `scripts/oracle_list.ps1` (PowerShell 7, só leitura):

1. **Entradas:**
   - winget: `Get-WinGetPackage` (sem argumentos), mantendo só os que têm `Source -eq 'winget'` (ids do catálogo). Sem o módulo ⇒ sai com erro claro; **não** parseie a tabela do `winget list`.
   - wingetrs: `-Exe` (padrão `target/release/wingetrs.exe`) `list --json`; ou `-WingetrsJson <arquivo>` e `-WingetJson <arquivo>` para rodar offline sobre capturas.
2. **Casamento por linha:** para cada pacote do winget (`Id`, `InstalledVersion`, `Name`), procure no JSON do wingetrs uma linha `Matched` com `package_id == Id` (case-insensitive) e `source == "winget"`.
3. **Quatro números**, impressos e também em `-OutJson`:
   - **recall** = ids do winget que o wingetrs casou / total do winget;
   - **concordância por linha** = desses casados, quantos têm a mesma versão instalada (normalize só espaços); o resto é listado;
   - **precisão** = linhas `Matched` (source winget) do wingetrs cujo `package_id` o winget também lista / total de `Matched` winget do wingetrs;
   - **faltas por classe:** cada id do winget sem casamento, com a linha do wingetrs que tem o mesmo `Name` (se houver) e o estado dela (`Unmatched`, `Ambiguous`, ausente).
4. **Divergências aceitas:** arquivo `scripts/oracle_list.allow.json` com `[{"id": "RubyInstallerTeam.RubyWithDevKit.3.4", "motivo": "..."}]` (confira o id exato na saída real). Imprime taxa **bruta** e **líquida** (sem as aceitas) separadas.
5. Saída curta (≤ 30 linhas), em português.

**Files:**
- Create: `scripts/oracle_list.ps1`, `scripts/oracle_list.allow.json`, `tests/fixtures/oracle_list/winget.json`, `tests/fixtures/oracle_list/wingetrs.json`
- Test: autoteste do próprio script (`-SelfTest`) sobre as fixtures

Fixtures pequenas (5–8 pacotes cada), escritas à mão, com: um casado com versão igual, um casado com versão diferente, um ausente, um `Ambiguous`, e **um id trocado** (wingetrs casou o pacote do winget X com o id Y). `-SelfTest` roda o script sobre elas e confere os números esperados (escreva-os no script).

#### Checks beyond the steps

1. **Prova do instrumento:** no `-SelfTest`, o id trocado derruba a precisão e **não** conta no recall. Mutação: comparar `package_id` sem considerar o id (ex.: casar só por nome) ⇒ `-SelfTest` falha. Cole a saída.
2. Rode uma vez de verdade (`pwsh -File scripts/oracle_list.ps1`) e cole os quatro números no relatório. **Não** altere código de produto para melhorar o número.
3. `pwsh -File scripts/check_rules.ps1` verde.

**Commit:** `feat(scripts): versioned list oracle against Get-WinGetPackage`
