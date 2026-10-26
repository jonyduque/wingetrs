<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo-dark.svg"><img src="docs/assets/logo-light.svg" alt="wingetrs" height="84"></picture></p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/licen%C3%A7a-MIT-blue" alt="Licença MIT"></a>
  <a href="https://github.com/jonyduque/wingetrs/releases"><img src="https://img.shields.io/github/v/release/jonyduque/wingetrs" alt="Último release"></a>
  <img src="https://img.shields.io/badge/plataforma-Windows-0078D6" alt="Plataforma Windows">
  <a href="https://github.com/jonyduque/wingetrs/actions/workflows/ci.yml"><img src="https://github.com/jonyduque/wingetrs/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
</p>

# wingetrs

Gerenciador de pacotes rápido para Windows, em Rust, com TUI e CLI. Usa o catálogo do winget e os buckets do Scoop.

Todo o texto da interface é em português.

## O que faz

- Busca e mostra pacotes do winget e do Scoop a partir de um catálogo local, que o `update` atualiza.
- Instala, atualiza e remove apps do Scoop sem precisar do Scoop. Extrai zip, 7z, MSI, CAB, `.tar.gz`, `.tar.xz` e `.nupkg` por conta própria e cria shims, PATH e atalhos.
- Instala pacotes do winget com motor próprio (MSI, WiX, Burn, Inno, NSIS, MSIX e exe silencioso) ou, quando não dá, pelo próprio winget.
- Lista o que está instalado no Windows e mostra o que tem atualização.
- Mostra progresso por fases, com relógio, em qualquer operação demorada.

## Instalação

Num PowerShell:

```powershell
irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1 | iex
```

O script faz quatro coisas:

1. baixa o `wingetrs.exe` do último release;
2. confere o SHA-256 contra o `wingetrs.exe.sha256` publicado junto;
3. copia o executável para a pasta de instalação;
4. põe essa pasta no PATH.

Num terminal normal, o script pergunta para quem instalar. "Para todos os usuários" pede autorização de administrador (UAC) e continua numa janela elevada. Num terminal de administrador, instala para todos os usuários sem perguntar. `-Scope User` ou `-Scope Machine` dispensa a pergunta.

| Escopo | Pasta | PATH |
|---|---|---|
| só para você | `%LOCALAPPDATA%\Programs\wingetrs` | do usuário |
| todos os usuários | `C:\Program Files\wingetrs` | da máquina |

Abra um terminal novo depois de instalar.

- **Atualizar:** `wingetrs self update` (a partir da 0.3.0), ou rode o mesmo comando de novo. A TUI avisa quando há versão nova e `Ctrl+U` atualiza.
- **Versão específica:** `& ([scriptblock]::Create((irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1))) -Version v0.2.1`
- **Simular antes:** o mesmo comando com `-DryRun` mostra o que seria feito (download, pasta, PATH, UAC) sem mudar nada e, no fim, pergunta se você quer instalar de verdade.
- **Remover:** o mesmo comando com `-Uninstall`. Os dados em `%LOCALAPPDATA%\wingetrs` (catálogo e apps instalados pelo wingetrs) ficam; para tirar os apps, use `wingetrs uninstall` antes.

Também dá para baixar o `wingetrs.exe` direto da página de [releases](https://github.com/jonyduque/wingetrs/releases).

### Qual escopo escolher

Instaladores de pacotes do winget para a máquina inteira pedem o UAC um por um e funcionam nos dois escopos.

O auxiliar elevado (`bridge-worker`) é outra história. Ele cuida da ponte `winget` elevada, das remoções de programas registrados em HKLM e da limpeza dessas chaves. Por segurança, ele só roda se o `wingetrs.exe` estiver numa pasta que só administradores gravam, como `C:\Program Files\wingetrs`. Instalado só para você, o wingetrs avisa e recusa essas operações. Para tê-las, instale para todos os usuários.

## Uso

```text
wingetrs                       abre a TUI
wingetrs update                atualiza o catálogo
wingetrs search powertoys      busca pacotes
wingetrs show Microsoft.PowerToys
wingetrs list --upgradable     lista pacotes com atualização
wingetrs install ripgrep       instala um app do Scoop
wingetrs install 7zip.7zip     instala um pacote do winget
wingetrs upgrade --all         atualiza tudo que o wingetrs instalou
wingetrs bucket add extras     adiciona um bucket do Scoop
```

Outros comandos: `uninstall`, `self update` (atualiza o próprio wingetrs), `repair` (reconstrói o estado a partir do disco) e `cleanup` (limpa registros de apps cujo desinstalador sumiu).

`wingetrs <comando> --help` mostra as opções e exemplos de cada comando.

Opções que valem para vários comandos:

- `--dry-run` mostra o plano sem mudar nada.
- `--yes` não pergunta nada.
- `--scope user|machine` escolhe o escopo de um pacote do winget.
- `--color auto|always|never` e `--glyphs nerd|unicode|ascii` ajustam a saída.

## TUI

Rode `wingetrs` sem argumentos. As abas são Busca, Instalados e Atualizações.

| Tecla | Ação |
|---|---|
| `↑↓` `j` `k` | mover seleção |
| `PgUp` `PgDn` | mover de 10 em 10 |
| `←→` / `Tab` | trocar aba |
| `Enter` | foco nos detalhes |
| `Espaço` | marcar (busca vazia) |
| `Ins` `Ctrl+Espaço` | marcar (sempre) |
| `i` | instalar (aba Busca, Scoop) |
| `u` | remover (aba Instalados) |
| `U` | atualizar (aba Atualizações) |
| `f` | fila de operações |
| `Ctrl+S` | filtrar fonte |
| `Ctrl+E` | mostrar (aba Instalados) |
| `Ctrl+R` | atualizar catálogo |
| `Esc` | voltar / limpar / sair |
| `q` `Ctrl+C` | sair |

## Segurança

- O instalador de um pacote do winget só roda com o SHA-256 conferido.
- Scripts e instaladores de terceiros de manifestos do Scoop só rodam com `--allow-scripts`, e antes aparecem na tela com a análise. `--yes` não libera isso.
- O `msiexec /a` (reserva para MSI que o extrator não suporta) só roda com `--allow-msiexec` ou confirmação naquele item.
- Todo caminho vindo de manifesto ou de arquivo baixado é validado antes de virar caminho no disco (recusa `..`, caminho absoluto, drive, UNC e nomes reservados do Windows).
- O auxiliar elevado não procura executáveis no PATH, monta o próprio ambiente do zero e não grava em pastas do usuário.
- O wingetrs só lê os arquivos do winget e do Scoop. O `installed.db` do winget é aberto somente para leitura.
- Dado externo malformado vira erro, não pânico: um manifesto ruim não derruba a TUI.

## Dados

Tudo fica em `%LOCALAPPDATA%\wingetrs`: catálogo, buckets, cache, apps instalados, shims, dados persistentes e logs. Fora dessa pasta, o wingetrs mexe só no que um app instalado pede: PATH do usuário (`HKCU\Environment`), atalhos em `Start Menu\Programs\wingetrs Apps` e a Lixeira. Pacotes do winget gravam onde o próprio instalador grava.

## Compilar

Requer Rust 1.93 ou mais novo.

```powershell
cargo build --release
pwsh -File scripts/verify.ps1   # regras, fmt, clippy e testes
```

## Licença

MIT. As licenças das dependências embutidas no executável estão em `THIRD_PARTY_LICENSES.txt` (gerado por `scripts/third_party.ps1` e anexado a cada release) e os avisos principais em `NOTICE`. O diretório `docs/reference/winget-cli/` traz trechos do [winget-cli](https://github.com/microsoft/winget-cli) (MIT), no commit indicado em `SOURCE_COMMIT.txt`.
