Status: DONE

Commits:
14ac012 fix(install): charge shared-span rewinds by the largest parsed block; fail closed on gap underflow
eba5463 fix(install): accept shared cabinet spans in real MSIs; msi row limit is unsupported, not invalid

Gate:
```
[OK]   rules 5s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (1023 passed, 25 ignored) 40s
[OK]   gate verde
```
(A primeira tentativa do gate caiu em `fmt`/`clippy` por `tests/it/winget_applicability.rs`, da Task 135 em andamento; repeti depois do commit dela e passou.)

## Item 1: padrão medido da sobreposição

Ferramenta `dump` (bin no crate de sonda do scratchpad, `msiprobe/probe/src/bin/dump.rs`): ordena as CFFILE de cada pasta por (deslocamento, tamanho) e classifica cada entrada com `offset < end` em idêntica / contida / parcial.

- `pkgconf` (sha256 a236479b...): pasta 1, `PkgconfigExeFile` = `PkgconfExeFile` = (0, 274432). Idênticas.
- `yubico-piv-tool`: pasta 1, `fil6863BC33...` = `fil29F86072...` = (0, 361). Idênticas.
- Nos 42 inteiros (item 4): **idênticas 100%, 0 contidas, 0 parciais**, e toda duplicata está no deslocamento 0 da pasta (`max_dup_offset` = 0 nos 42). É a deduplicação do WiX (arquivos iguais guardados uma vez).

Correção: aceita só (offset, size) igual à entrada anterior; contida e parcial continuam "gabinete com arquivos sobrepostos". `folder_gaps` conta o trecho uma vez (união) e cobra cada duplicata com tamanho > 0 em `offset + MAX_BLOCK` antes de decodificar: o `cab` vendorizado (`seek_to_uncompressed_offset`) volta ao início da pasta e decodifica de novo até `offset` quando a duplicata está atrás do bloco atual. Sem essa cobrança, 1000 duplicatas de 16 bytes atrás do último bloco de uma pasta de 1 MiB decodificam ~1 GB sem cobrança (o teste mutado levou 20 s contra 6 s do módulo todo). `vendor/cab-0.6.0` não foi alterado.

## Item 4: sonda, antes x depois (46 recusados, um por vez, apagados; 46/46 hash ok)

| motivo antes | n | depois |
|---|---|---|
| gabinete com arquivos sobrepostos | 42 | 42 ok |
| tabela File do MSI: Number of rows is too large | 3 (`rust`, `rust-msvc`, `rust-gnu`) | 3 `Unsupported("MSI com mais de 65536 linhas numa tabela")`, `needs_installer=true` |
| nome inseguro no MSI: af_dict (`espeak-ng`) | 1 | igual, recusado |

Cobrança de rewind medida (soma por MSI): máx. 394 MB (`vagrant`, 12017 duplicatas), contra orçamento de 24,7 GB dele; `sonic-pi` 100 MB, `gedit` 72 MB, `mono` 56 MB; nenhum perto do orçamento. Resultados por item: `C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad\msiprobe\results138.json`; script `...\msiprobe\scripts\run138.py`.

## Item 5: comparação com a Task 119

`target/probe-2b/` (vetores da 119) não existe mais. Árvores gravadas em `...\msiprobe\tree138-pkgconf.json` e `...\msiprobe\tree138-yubico-piv-tool.json` (caminho, tamanho, sha256).
- `pkgconf`: 5 arquivos; `pkgconf.exe` 274432 `8f7caa49...` bate com o único arquivo citado no relatório da 119 (`PFiles64\pkgconf-3.0.8\pkgconf.exe 274.432 8f7caa49...`); `pkg-config.exe` (a duplicata) tem o mesmo sha. O resto da árvore: não comparado ao msiexec.
- `yubico-piv-tool`: 165 arquivos, não comparado ao msiexec.

## Testes (`tests/it/install_msi.rs`)

1. `shared_span_extracts_both_files`: f_c com o mesmo (offset, size) de f_b em `three.cab` ⇒ `b.bin` e `c.txt` com o conteúdo de f_b.
2. `partial_and_contained_overlaps_refused`: f_c dentro de f_b (8) e passando do fim (40963) ⇒ "sobrepostos", nada gravado. O `overlapping_files_refused` existente cobre a outra ordem (mesmo offset, menor primeiro).
3. `many_entries_on_one_span_hit_the_budget`: gabinete gerado (`cab::CabinetBuilder`, sem compressão) com 1 MiB e 1000 entradas em (0, 1 MiB), orçamento 64 MiB ⇒ `TooLarge`; controle com 2 entradas extrai `out/App/f0000.bin`, `f0001.bin`, e no caso recusado `out/App` não existe.
   - extra `rereading_a_span_behind_the_reader_is_charged`: 1000 entradas de 16 bytes cruzando a fronteira dos dois últimos blocos ⇒ `TooLarge` (só pela cobrança de rewind); controle com 2 extrai os 16 bytes certos.
4. `file_past_folder_data_refused`: `one.cab` (1 bloco) com offset 40000 ⇒ "além dos dados". Não havia teste para essa recusa.
5. `msi_row_limit_needs_installer`: ponta a ponta, sem mapeamento isolado: o MSI do teste recria `Media` com chave int32 e 65537 linhas (o `msi` não grava 65537 strings no pool curto, por isso não `Property`) ⇒ `Unsupported("MSI com mais de 65536 linhas numa tabela")` e `needs_installer` verdadeiro. Mesmo teste: MSI cortado ao meio ⇒ `Archive` e `needs_installer` falso (outro erro do `msi` não muda de categoria).

Antes da implementação: 1, 3, 3-extra falharam com "sobrepostos" e 5 com `Archive("tabela Media do MSI: Number of rows is too large (65537 > 65536)")`. Depois: `test result: ok. 24 passed; 0 failed` em `install_msi::`.

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor 'if last == Some((offset, size)) {' -Replacement 'if false && last == Some((offset, size)) {' -TestCommand 'cargo test --quiet --test it install_msi::shared_span_extracts_both_files'`
   `[OK] mutação PEGA: 'if last == Some((offset, size)) {' -> 'if false && last == Some((offset, size)) {' fez o teste falhar (exit 101).`
2. Somar os tamanhos sem a união (`in_files += size;` antes do `continue` da duplicata), `-TestCommand 'cargo test --quiet --test it install_msi::'`:
   ```
   [OK] mutação PEGA: '                rewinds = rewinds.saturating_add(offset + MAX_BLOCK);
               }
               continue;' -> '                rewinds = rewinds.saturating_add(offset + MAX_BLOCK);
               }
               in_files += size;
               continue;' fez o teste falhar (exit 101).
   ```
   Pegaram `shared_span_extracts_both_files` e `many_entries_on_one_span_hit_the_budget`: `end - in_files` estoura (pânico em debug em `extract_msi.rs:272`, convertido em `Decoder` pelo `catch_decoder`).
3. `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor 'return ExtractError::Unsupported("MSI com mais de 65536 linhas numa tabela".to_string());' -Replacement 'return ExtractError::Archive("MSI com mais de 65536 linhas numa tabela".to_string());' -TestCommand 'cargo test --quiet --test it install_msi::msi_row_limit_needs_installer'`
   `[OK] mutação PEGA: 'return ExtractError::Unsupported("MSI com mais de 65536 linhas numa tabela".to_string());' -> 'return ExtractError::Archive("MSI com mais de 65536 linhas numa tabela".to_string());' fez o teste falhar (exit 101).`
4. Extra (cobrança de rewind): `-Anchor 'rewinds = rewinds.saturating_add(offset + MAX_BLOCK);' -Replacement 'rewinds = rewinds.saturating_add(0);' -TestCommand 'cargo test --quiet --test it install_msi::rereading_a_span_behind_the_reader_is_charged'`
   `[OK] mutação PEGA: 'rewinds = rewinds.saturating_add(offset + MAX_BLOCK);' -> 'rewinds = rewinds.saturating_add(0);' fez o teste falhar (exit 101).`
5. Gate: acima.

Decisions:
- Só idênticas aceitas: a medição dos 42 não mostrou contida nem parcial; aceitar "contida" seria palpite.
- Cobrança de rewind `offset + MAX_BLOCK` por duplicata com tamanho > 0 (limite superior do que o `cab` decodifica de novo sem entregar), cobrada antes de decodificar; duplicata de tamanho 0 não volta o leitor e não é cobrada.
- Erro de linhas reconhecido por `ErrorKind::InvalidData` + prefixo "Number of rows is too large" (texto do `msi` 0.10.0 travado), num `msi_error` usado no `Package::open` e no `rows()` (qualquer tabela).

Concerns:
- Prova 2 depende do estouro aritmético em debug: em release a subtração daria a volta. A checagem real é o teste; se quiser robustez em release, trocar por `checked_sub` com erro de arquivo.
- Comparação com msiexec só de 1 arquivo do `pkgconf`; `yubico-piv-tool` não comparado ao msiexec (vetores da 119 apagados).

## Correções (revisão, commit 14ac012)

1. Cobrança por duplicata passa de `offset + MAX_BLOCK` (0x8000) para `offset + PARSED_BLOCK` (0xFFFF): o parser do `cab` lê `cbUncomp` como `u16` (`vendor/cab-0.6.0/src/folder.rs:282`). Blocos maiores que 0x8000 continuam aceitos nessa cobrança. Teste novo `rewind_charge_covers_the_largest_parsed_block`: 101 entradas de 1 byte no deslocamento 0, orçamento de 5.000.000 (cabem 100 × 0x8000, não 100 × 0xFFFF) ⇒ `TooLarge`, e `out/App` não existe.
   `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor 'rewinds = rewinds.saturating_add(offset + PARSED_BLOCK);' -Replacement 'rewinds = rewinds.saturating_add(offset + MAX_BLOCK);' -TestCommand 'cargo test --quiet --test it install_msi::rewind_charge_covers_the_largest_parsed_block'`
   `[OK] mutação PEGA: 'rewinds = rewinds.saturating_add(offset + PARSED_BLOCK);' -> 'rewinds = rewinds.saturating_add(offset + MAX_BLOCK);' fez o teste falhar (exit 101).`
2. `end - in_files` virou `checked_sub` que devolve `Archive("gabinete com tamanhos de arquivos incoerentes")`. Esse braço é inalcançável com as checagens atuais (os trechos ordenados e sem sobreposição dão `in_files <= end`), por isso não tem fixture; ele só existe para falhar fechado. Com isso, a mutação 2 da entrega original (somar sem a união) passa a cair nesse `Archive` em vez de num pânico de debug.
3. Teste novo `shared_empty_entries_extract_uncharged`: 101 entradas idênticas de tamanho 0, no mesmo orçamento de 5.000.000 ⇒ extrai os 101 arquivos vazios. Ele também prova que entrada vazia não é cobrada:
   `pwsh -File scripts/mutate.ps1 -Path src/install/extract_msi.rs -Anchor 'if size > 0 {' -Replacement 'if true {' -TestCommand 'cargo test --quiet --test it install_msi::shared_empty_entries_extract_uncharged'`
   `[OK] mutação PEGA: 'if size > 0 {' -> 'if true {' fez o teste falhar (exit 101).`

Sonda de novo (`results138b.json`, hash ok, downloads apagados): `pkgconf` ok, 5 arquivos, 1.307.648 bytes; `vagrant` ok, 35.125 arquivos, 1.051.680.715 bytes. Os dois iguais à rodada anterior. A cobrança do `vagrant` agora é 12017 × 0xFFFF ≈ 788 MB, contra um orçamento de 24,7 GB.

Gate:
```
[OK]   rules 4s
[OK]   rules-selftest 13s
[OK]   fmt 2s
[OK]   clippy 10s
[OK]   test (1025 passed, 25 ignored) 43s
[OK]   gate verde
```

Concern novo: a checagem "além dos dados da pasta" ainda usa `num_data_blocks × 0x8000`. Um gabinete real com blocos maiores que 0x8000 seria recusado por ela. Nenhum dos 119 do corpus caiu nisso, e a revisão não pediu mudança.
