# Task 204: CAB standalone (yori `.cab#/x.7z`) pelo conteúdo

**Modelo:** sonnet (revisor sonnet).

#### Where this fits

- Achado da prova ao vivo (seção B, 2026-10-19): `wingetrs install main/yori --allow-scripts --yes` falhou com
  `✗ falha no passo 5 (extrair [1]): formato não suportado pelo extrator: conteúdo não reconhecido como 7z`.
  O manifesto do yori baixa `http://www.malsmith.net/download/?obj=yori/2.70/yori-ypm-amd64.cab#/ypm.7z`
  (e mais 4 `.cab#/<nome>.7z`): o arquivo é um **CAB** (`MSCF`) renomeado para `.7z`. O Scoop extrai com o
  7-Zip, que decide pelo conteúdo. 9 manifestos dos buckets locais citam `.cab` (`Select-String
  ~/scoop/buckets/*/bucket/*.json -Pattern '\.cab(#|")' -List`).
- `src/install/sniff.rs` (`Sniffed::{SevenZip, Zip, Pe, Other}`) já decide pelo conteúdo quando o nome é `.7z`
  (`src/install/extract.rs` ~:444). `src/install/archive.rs` `ArchiveKind::of` escolhe pela extensão.
- O crate `cab` já é dependência (usado em `src/install/extract_msi.rs`, com teto de entradas/tamanhos e
  `catch_decoder`): reaproveite as mesmas proteções.

#### What binds this task

1. `sniff` reconhece CAB (`MSCF` + 4 bytes zero, `4D 53 43 46 00 00 00 00`) como `Sniffed::Cab`.
2. Arquivo nomeado `.7z` cujo conteúdo é CAB é extraído como CAB (mesmo destino e regras da extração 7z:
   `safepath` em cada nome, sem sobrescrever fora do staging, teto de entradas e de bytes antes de alocar,
   `catch_decoder`/`catch_unwind` em volta do crate de terceiros).
3. Nome `.cab` (sem fragmento) também extrai como CAB (`ArchiveKind::Cab` ou equivalente; o plano mostra
   "CAB").
4. CAB multivolume (`cfhdrPREV_CABINET`/`cfhdrNEXT_CABINET`) e entradas continuadas (`iFolder >= 0xFFFD`) são
   recusados com mensagem clara (como no MSI).
5. Nenhuma mudança no caminho do MSI.

#### Tests (literais)

- `sniff_detects_cab`, `seven_z_named_cab_content_extracts` (fixture CAB pequena gerada por
  `scripts/make_fixtures.py` ou montada em teste com o crate `cab` se ele escreve; se não escreve, gere com
  `makecab.exe` NUNCA: gere os bytes em Python stdlib `-I`, CAB sem compressão é simples), `cab_extension_extracts`,
  `cab_multivolume_refused`, `cab_path_escape_refused` (nome `..\x`), `cab_entry_cap_refused`.
- Uma fixture que falha só em cada recusa (regra do projeto).

#### Checks beyond the steps

- Mutação: remover o braço `Cab` do `sniff` ⇒ `seven_z_named_cab_content_extracts` cai.
- Mutação: remover a recusa de multivolume ⇒ `cab_multivolume_refused` cai.

**Files:** `src/install/sniff.rs`, `src/install/extract.rs`, `src/install/archive.rs`, novo `src/install/extract_cab.rs`
(se preciso, com `mod` em `src/install/mod.rs`), `src/cli/output.rs` só se o nome do formato aparecer no plano,
testes em `tests/install_extract/` ou `tests/it/` existentes (sem arquivo novo em `tests/it/` sem avisar),
`scripts/make_fixtures.py`, `tests/fixtures/`.

**Restrição desta rodada:** o usuário está fazendo a prova ao vivo com `C:\Program Files\wingetrs\wingetrs.exe`
copiado de `target\release`. **Não rode `cargo build --release` nem nada que regrave `target\release\wingetrs.exe`.**
Gate normal (`scripts/verify.ps1`) pode rodar. Nunca rode o wingetrs instalado nem baixe o yori real; se quiser
conferir com o arquivo real, pare e diga no relatório (o orquestrador faz).

**Commit:** `feat(install): extract cabinet archives detected by content`
