Status: DONE

Commits:
4fd9be9 feat(icons): pick, fetch and cache winget package icons

Gate:
[OK]   clippy 33s
[OK]   test (657 passed, 16 ignored) 376s
[OK]   gate verde

Mutation proofs:
1 (preferencia): mutate.ps1 src/sources/winget/manifest.rs, dark => 0, `cargo test --quiet --lib pick_icon_prefers`
[OK] mutação PEGA: 'Some(t) if t.eq_ignore_ascii_case("dark") => 1,' -> '... => 0,' fez o teste falhar (exit 101).
2 (integridade): src/icons.rs, `put_verified(&icon.sha256,..)` -> `put_verified(&sha256_hex(&bytes),..)` (grava mesmo com hash errado), `cargo test --quiet --test icons`
[OK] mutação PEGA: 'match blobs.put_verified(&icon.sha256, &bytes) {' -> 'match blobs.put_verified(&crate::cache::sha256_hex(&bytes), &bytes) {' fez o teste falhar (exit 101).
3 (limite): `if bytes.len() > MAX_ICON_BYTES {` -> `if false {`
[OK] mutação PEGA: ... fez o teste falhar (exit 101).
4 (cache): `blobs.get(&icon.sha256)` -> `.filter(|_| false)` (sempre rede)
[OK] mutação PEGA: ... fez o teste falhar (exit 101).

Decisions:
- `PackageDetails.icon` leva `#[serde(skip)]` para a saida JSON/CLI nao mudar.
- Testes 1-3 em `mod tests` de manifest.rs; teste 6 em tests/tui_state.rs; `Msg::DetailsLoaded` ganhou `icon`, ajustados tests/tui_loop.rs e tui_state.rs (fora da lista Files, consequencia do campo novo).
- Temporario unico de icons.json: `icons.json.<pid>-<seq>.tmp` (padrao de write_index/BlobStore); `fetch` usa `blobs.get` (que renova mtime, Task 81).
- Cargo target: target/t83.

Concerns: none (leftovers.ps1 lista um bash.exe anterior a esta tarefa, nao meu).
