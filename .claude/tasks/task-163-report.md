Status: DONE_WITH_CONCERNS

Commits:
bde72ad docs(report): phase 2c validation (oracle before/after, script host cost)

Gate:
[OK]   rules 9s
[OK]   rules-selftest 23s
[OK]   fmt 3s
[OK]   clippy 18s
[OK]   test (1726 passed, 86 ignored) 78s
[OK]   gate verde

Mutation proofs: no mutation proof (tarefa de medição, declarado no brief). Checks beyond the steps:
1. Saídas reais do oráculo e do bench coladas em `docs/superpowers/reports/2026-10-18-fase2c-validacao.md` (seções 1 e 2).
   Resumo: 1318/1403 (93,9 %) dos 2c planejam; 77 recusados pelo portão; 8 outros erros (.zst/.rar/arquitetura).
   Bench (mediana do delta, 15 execuções, -Baseline noop): pwsh 7 vazio 583,1 ms; pwsh 7 + prelúdio 717,3 ms; 5.1 vazio 487,1 ms; 5.1 + prelúdio 548,4 ms.
2. `scripts/leftovers.ps1` depois do bench: só os processos que já estavam vivos antes da tarefa (bash.exe PIDs 30488 e 12064, conhost.exe 13024, todos "desde 18/10 02:03", de outra sessão); nenhum processo deste agente. Não encerrados (não são meus).
3. Gate acima.

`if ($global)` (pedido extra): recusados só por `-Global` dentro de `if ($global)`: 0 (o único com `GlobalSwitch`, `main/python`, também tem HKLM no mesmo tipo de bloco). Recusados só por qualquer construção dentro de `if ($global)`: 38 de 77 (7zip, ghostscript, git-with-openssh, git, pwsh, python, vim, vulkan, alacritty, astrogrep, ...); nenhum reatribui `$global`. Opções A/B/C/D com prós e contras na seção 3 do relatório; recomendação C (apagar o corpo do bloco no texto que roda) ou A. Portão não alterado.

Prova ao vivo: não executada; comandos exatos (pester, xming, yori) na seção 7 do relatório.

Decisions:
- "Instaláveis antes" = 0 por definição (a 2a recusava todo sub-caso 2c), não remedido com o código antigo; dito no relatório.
- Instalável usa `resolve(Arch::host())` como o `oracle_2a`; razões do portão contadas a partir de `scan_manifest(m, PRELUDE_FUNCTIONS)` (todos os hooks), uma vez por manifesto.
- `if ($global)` medido apagando o corpo dos blocos e replanejando (mais exato que olhar a linha do flag); casamento de chaves pula aspas e `#`, não here-strings.
- Bench: o pwsh 7 entra por `locate` (caminho tem espaço e `bench.ps1 -Args` divide por espaço); modo extra `prelude` além do runner vazio pedido.
- Trailer do commit: usei o `Co-Authored-By` da sessão (Claude Opus 5.5 + Claude-Session), não o `Claude <noreply@...>` do brief.

Concerns:
- A tabela (f) Riscos e o texto D2-1 da spec ainda falam da lib do Scoop (opção A); com D1 = B estão obsoletos. A proposta D2-1 da seção 6 já está ajustada; a spec é do orquestrador.
- 38 dos 77 recusados (git, python, 7zip, pwsh, vim) são recusados por código que nunca roda; decisão do usuário pendente.
- Veredito Confirm do `yori` (S-hook da prova ao vivo) não medido; o relatório manda conferir no `--dry-run`.
- `.join` com dado externo: nenhum (o bench só junta nomes constantes à pasta dada na linha de comando; o oráculo não monta caminhos).
