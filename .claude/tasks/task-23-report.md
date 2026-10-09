Status: DONE

Commits:
d30a130 feat(cli): complete Portuguese help with examples

Gate:
[OK]   fmt 3s
[OK]   clippy 8s
[OK]   test (167 passed, 2 ignored) 52s
[OK]   gate verde

Mutation proofs:
mutate.ps1 -Path src/cli/mod.rs -Anchor '/// Mostra só pacotes com atualização disponível' -Replacement '// sem descricao' -TestCommand 'cargo test --quiet --test cli_parse'
[OK] mutação PEGA: '// Mostra só pacotes com atualização disponível' -> '/ sem descricao' fez o teste falhar (exit 101).
(doc da opção `upgradable` de List usada como âncora de linha única; o `json` repete texto em 3 comandos, não é único.)

Decisions:
- --glyphs só na raiz (não global); teste glyphs_is_global virou glyphs_is_root_only (`--glyphs ascii list`) + teste de rejeição após subcomando.
- -h/--help global (propaga aos subcomandos); -V/--version só na raiz.
- "Uso:" via help_template + override_usage (clap não traduz "Usage:"/[OPTIONS]); hide_possible_values e hide_default_value, valores citados na descrição.
- O teste de descrição ignora args ocultos (--background).

Concerns: none

wingetrs --help:
Gerenciador de pacotes rápido sobre winget e scoop

Uso: wingetrs [OPÇÕES] [COMANDO]

Comandos:
  update  Atualiza o catálogo local
  search  Busca pacotes
  show    Mostra detalhes de um pacote
  list    Lista pacotes instalados

Opções:
      --glyphs <GLYPHS>  Conjunto de glifos da TUI (nerd, unicode ou ascii)
  -v, --verbose...       Verbosidade (-v, -vv): também loga no stderr
  -h, --help             Mostra a ajuda
  -V, --version          Mostra a versão

Exemplos:
  wingetrs                       abre a TUI
  wingetrs update                atualiza o catálogo
  wingetrs search powertoys      busca pacotes
  wingetrs show Microsoft.PowerToys
  wingetrs list --upgradable     lista pacotes com atualização

wingetrs search --help:
Busca pacotes

Uso: wingetrs search [OPÇÕES] <QUERY>

Argumentos:
  <QUERY>  Texto a buscar (nome, id ou palavra-chave)

Opções:
      --source <SOURCE>  Restringe a uma fonte (winget ou scoop)
      --json             Saída em JSON
  -v, --verbose...       Verbosidade (-v, -vv): também loga no stderr
  -h, --help             Mostra a ajuda
  -n, --limit <LIMIT>    Máximo de resultados (padrão: 20)

Exemplos:
  wingetrs search powertoys
  wingetrs search powertoys --source scoop -n 5
  wingetrs search git --json

## Adendo (orquestrador)
Acabamento em 39f6c30: nomes de valor em português (<TERMO>, <FONTE>, <N>, <GLIFOS>, <VERSÃO>) e -v/-h/-V por último; a ajuda colada acima é anterior a esse commit.
