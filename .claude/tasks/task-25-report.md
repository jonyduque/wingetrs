Status: DONE_WITH_CONCERNS

Commits:
ee4d43f feat(install): verified downloads with blob cache

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 107s
[OK]   test (231 passed, 2 ignored) 18s
[OK]   gate verde

Mutation proofs:
[OK] mutação PEGA: 'HashAlgo::Md5 => hex_of(&Md5::digest(bytes)),' -> 'HashAlgo::Md5 => spec.hex.clone(),' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'verify(spec, &bytes).then(|| data.blobs_dir().join(name))' -> 'Some(data.blobs_dir().join(name))' fez o teste falhar (exit 101).
[OK] mutação PEGA: (bloco de 2 linhas: blobs.get + verify no cache -> Some(path) sem checar)
[OK] mutação PEGA: 'let _ = std::fs::remove_file(p);' -> 'let _ = p;' fez o teste falhar (exit 101).
Nota: a 1a tentativa das 3 primeiras SOBREVIVEU (brief so testava md5 true; cache tem checagem dupla get+verify). Adicionados testes verify_md5_rejects_wrong_digest e index_pointing_at_wrong_blob_is_not_trusted; depois as provas pegaram.

Decisions:
- HashAlgo/HashSpec(+parse)/Source definidos provisoriamente em download.rs (// TEMP, Task 24 ainda nao existia); testes importam de install::download, nao de manifest.
- Blob em blobs\ nomeado pelo sha256 do conteudo (sem extensao/file_name); extracao deve usar Source.file_name para decidir o tipo.
- get_to_file mapeia erros de E/S para HttpError::Network (sem nova variante); falha em um source nao cancela os outros, erro devolvido apos join_all; index.json gravado uma vez ao fim.
- Limite 4 via tokio Semaphore; cache hit nao consome permit.

Concerns:
- Unificar tipos TEMP com manifest.rs (Task 24) e trocar o import em tests/install_download.rs.
- leftovers.ps1 lista bash/conhost de outros agentes (nao meus).

---
Revisão (CHANGES_REQUESTED) tratada no commit `fix(install): per-chunk download timeout, tested concurrency limit, shared hash types`:
- get_to_file usa dl_client (só connect timeout) + tokio::time::timeout de 30 s (configurável por Http::with_chunk_timeout) em send() e em cada chunk. Testes com servidor TCP cru: lento-mas-contínuo > total de 1 s => sucesso; parado => HttpError::Timeout.
- Teste de concorrência (8 fontes, contador no servidor, max em 2..=4).
- index.json: tmp com pid+seq; falha ao gravar vira tracing::warn.
- Tipos TEMP removidos; usa install::manifest.
Gate: [OK] rules / fmt / clippy 56s / test (262 passed, 2 ignored) / gate verde
Mutation proofs:
[OK] mutação PEGA: 'const MAX_PARALLEL: usize = 4;' -> 'const MAX_PARALLEL: usize = 4000;' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'self.dl_client.get(url).send()' -> 'self.client.get(url).send()' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'tokio::time::timeout(idle, stream.next())' -> 'tokio::time::timeout(Duration::from_secs(3000), stream.next())' fez o teste falhar (exit 101).
