Status: DONE

Commits:
c1edf63 feat(installed): collect ARP, MSIX and scoop installs

Gate:
[OK]   fmt 1s
[OK]   clippy 10s
[OK]   test (98 passed, 1 ignored) 36s
[OK]   gate verde

Red: `cargo test --quiet --test installed` (mod.rs sem os itens) -> error[E0432]: unresolved imports `wingetrs::installed::filter_arp`, ... ; error[E0425]: cannot find function `scan_all`.
Live: apps=250 errors=[] elapsed_ms=270 (outras rodadas: 399, 306; a primeira, com MSIX serial, deu 531-801 ms e falhava < 500).

Mutation proofs:
[OK] mutação PEGA: 'raw.parent_key_name.is_some()' -> 'false' fez o teste falhar (exit 101).
[OK] mutação PEGA: '[p[1] as char, p[0] as char]' -> '[p[0] as char, p[1] as char]' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'product_code: Some(raw.key_name)' -> 'product_code: if raw.windows_installer == Some(1) { Some(raw.key_name) } else { None }' fez o teste falhar (exit 101).

Decisions:
- MSIX serial levava ~530 ms (DisplayName/PublisherDisplayName por pacote); leitura das propriedades paralelizada em 8 threads (appx_win.rs) para cumprir elapsed_ms < 500.
- Lookup do upgrade_code usa a chave em maiúsculas (unpack_guid devolve maiúsculas).
- Clippy pediu as_chunks/slice-before-as_bytes; ajustado no código, testes sem alteração. Nenhum allow de lint foi necessário.
- scoop: name = nome do app; app sem manifest/install válidos é ignorado.

Concerns: o live depende da máquina (variação 270-400 ms); margem para 500 ms é moderada.
