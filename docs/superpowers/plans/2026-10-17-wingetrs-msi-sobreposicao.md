# wingetrs — MSI reais recusados pela checagem de sobreposição (Task 138)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 138 -Plan docs/superpowers/plans/2026-10-17-wingetrs-msi-sobreposicao.md`.

**Goal:** a extração própria de MSI volta a aceitar os MSI reais que a sonda da Task 119 extraía de forma idêntica ao `msiexec /a`, sem perder a proteção contra bomba; e MSI que o crate `msi` não lê por limite dele vira `NãoSuportado` (candidato à reserva), não "arquivo inválido".

**Origem (sonda de 2026-10-17, 119 MSI simples do Scoop, só extração própria):** 63 ok, 46 recusados, 0 `needs_installer`:
- **42** "gabinete com arquivos sobrepostos" (`folder_gaps`, `src/install/extract_msi.rs` ~:233). Entre eles `aws`, `chuck`, `cppcheck`, `mono`, `vagrant`, `calibre-normal`, `cyberduck`, `github-copilot`, `webex`, `yubico-piv-tool` (4 MB), `pkgconf` (512 KB). A sonda S3 (Task 119) tinha 73/73 idênticos ao `msiexec /a` antes dessa checagem existir: é falso positivo da checagem acrescentada na revisão da 2b.
- **3** `rust`, `rust-msvc`, `rust-gnu`: "tabela File do MSI: Number of rows is too large (66064 > 65536)" — limite do crate `msi`, não defeito do arquivo.
- **1** `espeak-ng`: nome `af_dict  ` (espaços no fim) — recusa correta pela `safepath`; continua como está (nome inseguro nunca cai na reserva, spec 2b).
- Lista e resultados: `C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\msiprobe\results.json` (`id`, `url`, `size`, `error`) e a sonda `...\msiprobe\probe\` (crate com `wingetrs` por path, `[patch.crates-io]` do `cab` e `Cargo.lock` do repo; roda com `CARGO_TARGET_DIR` = target do repo).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- Sem crate nova; o `cab` vendorizado (`vendor/cab-0.6.0`) pode ser lido, e só alterado se o brief da checagem exigir (diga no relatório).
- Downloads de investigação só no scratchpad, um por vez, apagados no fim; o usuário autorizou baixar os MSI do catálogo. Nunca `msiexec`.
- A proteção contra bomba continua: o total gravado é limitado pelo orçamento de bytes, e nenhuma entrada força decodificar mais do que o orçamento.

## Review Focus

1. Duas entradas `CFFILE` com o mesmo deslocamento e tamanho (deduplicação do WiX "smart cabbing") ou uma contida em outra.
2. Muitas entradas apontando para o mesmo trecho (amplificação): o orçamento de bytes gravados para antes de encher o disco.
3. Entradas que ultrapassam os dados da pasta continuam recusadas.
4. O `msi` com mais de 65536 linhas em outra tabela que não `File`.

---

### Task 138: Sobreposição legítima em gabinete de MSI; limite de linhas do `msi` como NãoSuportado

**Modelo:** opus (revisor opus).

#### Where this fits

- `src/install/extract_msi.rs`: `folder_gaps` (~:231–260) ordena `(uncompressed_offset, uncompressed_size)` e recusa `offset < end`; `MAX_BLOCK` (~:28); o total da pasta é cobrado do orçamento antes de decodificar (resumo da 2b); erros mapeados para `ExtractError::Archive`/`Unsupported`.
- `src/install/extract.rs`: `needs_installer` (~:274) só aceita `ExtractError::Unsupported` + extensão MSI + assinatura OLE.
- Testes: `tests/it/install_msi.rs` (MSI e gabinete gerados no teste), `tests/it/install_msi_layout.rs`, `tests/it/install_msi_live.rs` (`#[ignore = "live: ..."]`).

#### What binds this task

1. **Investigue primeiro** com `pkgconf` e `yubico-piv-tool` (os menores da lista): despeje as entradas `CFFILE` de cada pasta (nome, deslocamento, tamanho) e diga no relatório qual é o padrão real da sobreposição (idênticas? contidas? parciais?). A correção segue o padrão medido, não o palpite do Review Focus.
2. **Correção da checagem:** aceite o padrão real medido no item 1 (ex.: entradas idênticas ou contidas) e continue recusando o que não for legítimo segundo a medição. `folder_gaps` passa a calcular os bytes fora dos arquivos sobre a **união** dos trechos (nunca subtrair duas vezes o mesmo trecho). A amplificação (N entradas no mesmo trecho) continua limitada pelo orçamento de bytes **gravados**; prove com teste.
3. **Limite de linhas do `msi`:** o erro do crate `msi` "Number of rows is too large" (em qualquer tabela) vira `ExtractError::Unsupported("MSI com mais de 65536 linhas numa tabela")`, para cair na reserva com confirmação. Nenhum outro erro do `msi` muda de categoria.
4. **Medição:** rode de novo a sonda do scratchpad nos 46 recusados (um por vez, apagando cada um) e cole a tabela antes × depois por motivo. Meta: os 42 da sobreposição passam, os 3 do `rust` viram `needs_installer=true`, o `espeak-ng` continua recusado.
5. Para 2 dos que passarem (os menores), compare a árvore extraída com o layout da Task 119 se ainda houver vetor, senão diga "não comparado ao msiexec".

**Files:**
- Modify: `src/install/extract_msi.rs` (e `vendor/cab-0.6.0` só se inevitável)
- Test: `tests/it/install_msi.rs`

Testes (`tests/it/install_msi.rs`, gabinetes gerados no teste):
1. Gabinete com duas entradas no mesmo trecho (o padrão do item 1) ⇒ extrai os dois arquivos com o conteúdo certo.
2. Sobreposição que o item 1 classificou como ilegítima ⇒ recusada com a mensagem atual.
3. Muitas entradas (ex.: 1000) no mesmo trecho de 1 MiB com orçamento de 64 MiB ⇒ recusado pelo orçamento, e nada gravado na pasta de destino real (olhe uma pasta que existiria).
4. Entrada além dos dados da pasta ⇒ continua recusada.
5. `needs_installer` verdadeiro para o erro do item 3 (unidade sobre o mapeamento do erro; não precisa de MSI com 65537 linhas se o mapeamento for testável isolado — diga como provou).

#### Checks beyond the steps

1. Mutação: voltar a recusar o padrão legítimo ⇒ teste 1 **PEGA**.
2. Mutação: somar os tamanhos sem a união em `folder_gaps` ⇒ algum teste **PEGA** (diga qual).
3. Mutação: mapear o erro de linhas para `Archive` ⇒ teste 5 **PEGA**.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `fix(install): accept shared cabinet spans in real MSIs; msi row limit is unsupported, not invalid`
