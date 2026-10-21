# wingetrs

Gerenciador de pacotes para Windows, em Rust, com TUI e CLI. Usa o catálogo do winget e os buckets do Scoop.

- Busca e mostra pacotes do winget e do Scoop a partir de um catálogo local, que o `update` atualiza.
- Instala, atualiza e remove apps do Scoop sem precisar do Scoop: extrai zip, 7z, MSI, CAB, `.tar.gz`, `.tar.xz` e `.nupkg` por conta própria e cria shims, PATH e atalhos.
- Instala pacotes do winget com motor próprio (MSI, WiX, Burn, Inno, NSIS, MSIX e exe silencioso) ou, quando não dá, pelo próprio winget.
- Lista o que está instalado no Windows e mostra o que tem atualização.

Todo o texto da interface é em português.

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

O escopo segue o terminal:

| Terminal | Pasta | PATH |
|---|---|---|
| normal | `%LOCALAPPDATA%\Programs\wingetrs` (só para você) | do usuário |
| "Executar como administrador" | `C:\Program Files\wingetrs` (todos os usuários) | da máquina |

Abra um terminal novo depois de instalar.

- **Atualizar:** rode o mesmo comando de novo.
- **Versão específica:** `& ([scriptblock]::Create((irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1))) -Version v0.2.1`
- **Remover:** o mesmo comando com `-Uninstall`. Os dados em `%LOCALAPPDATA%\wingetrs` (catálogo e apps instalados pelo wingetrs) ficam; para tirar os apps, use `wingetrs uninstall` antes.

Também dá para baixar o `wingetrs.exe` direto da página de [releases](https://github.com/jonyduque/wingetrs/releases).

### Qual escopo escolher

Instaladores de pacotes do winget para a máquina inteira pedem o UAC um por um e funcionam nos dois escopos.

O auxiliar elevado (`bridge-worker`) é outra história. Ele cuida da ponte `winget` elevada, das remoções de programas registrados em HKLM e da limpeza dessas chaves. Por segurança, ele só roda se o `wingetrs.exe` estiver numa pasta que só administradores gravam, como `C:\Program Files\wingetrs`. Instalado só para você, o wingetrs avisa e recusa essas operações. Para tê-las, instale num terminal de administrador.

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

`wingetrs <comando> --help` mostra as opções e exemplos de cada comando.

`--dry-run` mostra o plano sem mudar nada. Scripts de manifestos do Scoop só rodam com `--allow-scripts`.

## Compilar

Requer Rust 1.93 ou mais novo.

```powershell
cargo build --release
pwsh -File scripts/verify.ps1   # regras, fmt, clippy e testes
```

## Licença

MIT. O diretório `docs/reference/winget-cli/` traz trechos do [winget-cli](https://github.com/microsoft/winget-cli) (MIT), no commit indicado em `SOURCE_COMMIT.txt`.
