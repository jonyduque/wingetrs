Status: DONE_WITH_CONCERNS

Commits: um só, o que contém este relatório (`feat(tui): fetch icons of the visible list rows`); SHA em `git log --format='%h %s' e9fdd6d..HEAD` e na mensagem ao orquestrador.

Gate (CARGO_TARGET_DIR=target/t92):
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 54s
[OK]   test (711 passed, 18 ignored) 137s
[OK]   gate verde

Mutation proofs:
1. Não inserir em `icon_tried`:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '&& self.icon_tried.insert(key)' -Replacement '&& !self.icon_tried.contains(&key)' -TestCommand 'cargo test --quiet --test tui_state prefetch_asks_for_visible_winget_rows_once'`
   [OK] mutação PEGA: '&& self.icon_tried.insert(key)' -> '&& !self.icon_tried.contains(&key)' fez o teste falhar (exit 101).
   (falhou: prefetch_asks_for_visible_winget_rows_once, "the same screen again")
2. Não conferir a geração antes de cada pacote:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/run.rs -Anchor 'if !still_current(generation, mine) {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test tui_jobs prefetch_'`
   [OK] mutação PEGA: 'if !still_current(generation, mine) {' -> 'if false {' fez o teste falhar (exit 101).
   (falharam: prefetch_checks_the_generation_before_each_package, prefetch_drops_superseded_generation)
3. `IconMapped` devolver `Vec::new()`: mutação EQUIVALENTE, não aplicável (ver Concerns). Prova substituta, removendo o único caminho que codifica o ícone recém-mapeado:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '    cmds.extend(app.list_icon_cmds());' -Replacement '' -TestCommand 'cargo test --quiet --test tui_state icon_mapped_encodes_the_visible_row'`
   [OK] mutação PEGA: '    cmds.extend(app.list_icon_cmds());' -> '' fez o teste falhar (exit 101).
Extras (teto de 4):
   `-Path src/tui/run.rs -Anchor 'let Ok(slot) = Arc::clone(slots).acquire_owned().await else {' -Replacement 'let Ok(slot) = Arc::new(tokio::sync::Semaphore::new(64)).acquire_owned().await else {' -TestCommand 'cargo test --quiet --test tui_jobs prefetch_runs_at_most_four_at_once'`
   [OK] mutação PEGA: ... -> '...Semaphore::new(64))...' fez o teste falhar (exit 101).
   Mesmo anchor com `Semaphore::new(1)` (sem paralelismo): [OK] mutação PEGA (exit 101).
4. Medida (sem rede): `examples/tui_frame.rs frames` (o quadro da Task 85; não é teste `#[ignore]`, é o example), release, série "pesquisar icones on" (ícones ligados, `icon_map` vazio ⇒ cada Down emite `PrefetchIcons`), 3 rodadas intercaladas × 3 séries, mediana µs:
   - com PrefetchIcons: 487 476 490 | 445 455 455 | 478 470 475 (mediana das medianas 475)
   - sem (build com `list_icons_visible: false`, revertido): 557 485 471 | 490 503 467 | 457 477 475 (mediana 477)
   Iguais dentro do ruído (±50 µs entre séries). O quadro só mede `draw`; o trabalho novo é todo fora dele.

Decisions:
- `list_fetch_cmds` roda em `update()` junto de `list_icon_cmds` (que já roda após toda mensagem, desde a Task 85): cobre rolar, aba, busca/`SearchDone`, filtro, ordem, `InstalledLoaded`, e também fechar modal/ajuda; `icon_tried` impede repetição.
- Lado do trabalho testado sem `Ctx`: o `Ctx` é privado e `load_details` usa `CDN_BASE` fixo (sem wiremock possível). O laço virou `pub async fn prefetch_list(gen, mine, debounce, slots, keys, fetch)` + `pub fn still_current`; `spawn_prefetch` só injeta `prefetch_one` (load_details → load_icon → `IconMapped`). Testes contam chamadas do `fetch` em vez de requisições wiremock.
- A conferência da geração é feita só antes de cada pacote (a primeira, logo após o debounce, é o passo 2 do brief).
- Pânico de um pacote: o `JoinSet` é drenado até o fim e o primeiro pânico é relançado (`resume_unwind`); a task de cima o converte em `Msg::Panicked` como `join_details`.
- Semáforo `list_fetch_slots` no `Ctx` compartilhado entre gerações (teto global de 4).

Concerns:
- Brief diz que `list_icon_cmds` roda só "pela mudança de tela e pelo IconReady"; no código (Task 85) `update()` já o chama após toda mensagem, então `IconMapped` já gerava `EncodeCachedIcon` e a mutação 3 é equivalente. Não dupliquei a chamada no braço `IconMapped`; o teste 4 existe e pega a remoção do caminho real.
- Testes alterados (setup, não asserções): `list_never_fetches_icons` renomeado para `list_encodes_only_cached_icons` e passa a ignorar `PrefetchIcons` (a regra antiga "a lista nunca baixa" deixa de valer por design desta tarefa); `icon_encoded_for_an_old_protocol_is_discarded` desliga `list_icons_visible` (asserção de fatia exata).
- Requisições reais (versionData/manifesto/ícone) no terminal real: não medido.
