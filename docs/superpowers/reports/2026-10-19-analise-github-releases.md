# Análise: instalar a partir de releases do GitHub

Data: 2026-10-19. Pedido do usuário: avaliar instalar um repositório do GitHub baixando um arquivo do release, sem manifesto do winget nem do Scoop. Item F do `BACKLOG.md`.

## Medições (2026-10-19, API pública, sem token)

- **Buckets locais:** muitos manifestos do Scoop já baixam de `github.com/<dono>/<repo>/releases/download/`.

  | Bucket | Manifestos | Baixam de release do GitHub |
  |---|---|---|
  | main | 1694 | 1220 |
  | extras | 2427 | 1466 |
  | nerd-fonts | 367 | 309 |
  | nonportable | 134 | 43 |

  A maioria dos repositórios populares já tem manifesto revisado. O caminho direto pelo GitHub serve para o que não está em bucket nenhum.
- **Hash:** nos 13 repositórios consultados, todos os arquivos de release trazem `digest: "sha256:…"` na API (`GET /repos/{o}/{r}/releases/latest`), 100% dos arquivos. Esse hash é o do arquivo guardado pelo GitHub. Ele protege o download, mas não diz se o autor é confiável.
- **Release imutável:** só 1 de 13 tinha `immutable: true` (`cli/cli`).
- **Limite da API sem token:** 60 pedidos por hora por IP (`X-RateLimit-Limit: 60`). Um `install` gasta 1 pedido.
  - Conferir atualizações de N apps pela API gasta N pedidos. Com mais de 60 apps, o limite estoura.
  - Alternativa medida: `https://github.com/<o>/<r>/releases/latest` responde `302` para `/releases/tag/<tag>`. Isso dá a versão sem gastar pedido da API (não traz o hash, que só é preciso na hora de instalar).
- **Escolha do arquivo.** Em 12 repositórios (ripgrep, bat, jq, fzf, neovim, gh, PowerShell, hugo, zoxide, lazygit, Windows Terminal, Notepad++), filtrando por nome Windows + x64 e descartando `.sha256`, `.sig` e afins:
  - **Escolha única direto:** 3 de 12 (bat, fzf, zoxide).
  - **Resolvidos por desempate:** mais 5 (ripgrep, lazygit, jq, neovim, gh). Os desempates são:
    - `msvc` vence `gnu`;
    - nomes com `32-bit` saem;
    - arquivos com o mesmo `digest` são o mesmo arquivo (o jq publica `jq-win64.exe` e `jq-windows-amd64.exe`, idênticos);
    - pacote compactado vence instalador.
  - **Precisam de escolha do usuário:** 4 de 12.
    - hugo: 4 variantes (`extended`, `withdeploy`).
    - PowerShell: `win-x64.zip` contra `fxdependent`.
    - Terminal: `.zip` contra `.msixbundle` contra `PreinstallKit`.
    - Notepad++: `portable` contra `minimalist`, `.7z` contra `.zip`. O repositório é só de Windows e os nomes não dizem "win", então o filtro por nome o perdeu.
  - Conclusão: heurística mais seletor (regra do CLI: setas e Enter) cobre os 12. Heurística sozinha erraria ou pararia em 4.

## Como encaixa no que já existe

- **Download e extração:** já existem.
  - Formatos `zip`, `7z`, `tar.gz`, `tar.xz`, `msi`, `nupkg` e `cab`, decididos pelo conteúdo (`sniff`, Task 204).
  - `.exe` solto é `ArchiveKind::Plain`.
  - `safepath`, tetos de entrada e progresso valem iguais.
- **Caminho mais barato: manifesto sintético do Scoop.** O pedido vira um manifesto em memória que entra no planejador da 2a sem mudar o executor:
  - `version` vem da tag, sem o `v`;
  - `url` é o `browser_download_url`;
  - `hash` vem do `digest`;
  - `extract_dir`: a pasta única do topo, se houver;
  - `bin`: os `.exe` do topo, mostrados no plano para confirmar.
- **Sem scripts:** `pre_install` e `installer.script` não existem nesse caminho. O portão da 2c não entra.
- **Instaladores (`.exe` de setup, `.msi`, `.msix`):** o motor da 2d confia no SHA de um manifesto revisado do winget-pkgs. Aqui o hash vem do próprio autor, por isso a recomendação é recusar na primeira versão. Se ficar, que seja com a mesma confirmação por item e UAC por instalador da decisão D, e com o aviso de origem não revisada.
- **Registro:** o estado guarda a origem `github:<dono>/<repo>`, a tag, o nome do arquivo e o digest. A atualização compara a tag nova (pelo redirecionamento `302`) com a registrada e reaplica a mesma regra de escolha. Se o nome do arquivo mudar de padrão entre versões, o seletor volta a aparecer.
- **Nome do app:** `gh/<dono>/<repo>`, para não colidir com apps do Scoop de mesmo nome.

## `.exe` instalador ou programa

A API só dá nome, tamanho e digest; a decisão vem do conteúdo, depois do download e do hash conferido. Medido em 2026-10-19:

| Arquivo | Overlay (bytes depois da última seção) | Marca | `requestedExecutionLevel` |
|---|---|---|---|
| `jq-windows-amd64.exe` (programa) | 0 | nenhuma | `asInvoker` |
| `lazygit.exe` (programa, de dentro do zip) | 0 | nenhuma | sem manifesto |
| `npp.8.9.8.1.Installer.x64.exe` (instalador) | 6,7 MB de 7 MB | `NullsoftInst` | `requireAdministrator` |

Sinais, do mais forte ao mais fraco:

1. **Overlay grande.** `sniff::pe_overlay_start` já calcula onde ele começa. Antes de medir, desconte a assinatura Authenticode, que fica no overlay; o tamanho dela está no diretório de dados de segurança, entrada 4.
2. **Marca do gerador:**
   - NSIS: `NullsoftInst`;
   - Inno Setup: `Inno Setup`/`rDlPtS`;
   - WiX Burn: seção `.wixburn`;
   - 7z SFX: assinatura 7z no overlay;
   - também `InstallShield` e `SquirrelSetup`.
3. **`requireAdministrator`** no manifesto do PE. É só um indício.
4. **Nome do arquivo** (`setup`, `installer`): serve só para desempatar.
5. **Mesma URL num manifesto do winget-pkgs:** o `InstallerType` revisado decide.

Falhas possíveis:
- instalador sem marca conhecida: o overlay grande decide;
- programa com dados no overlay (Electron num `.exe` só): parece instalador e vai para o seletor com o motivo.

Nos dois casos o resultado é programa, que vira atalho de comando, ou instalador, que é recusado com o nome do gerador.

## Riscos

- **Confiança.** Ninguém revisou o arquivo. O `digest` só garante que o arquivo baixado é o que está no GitHub. Conta invadida, repositório transferido (a API segue redirecionamentos) ou tag reescrita entregam binário novo com hash "certo".
  - O plano precisa dizer "origem não revisada: <dono>/<repo>". A confirmação nunca fica implícita em `--yes`, igual ao `--allow-scripts`.
  - Atestados do GitHub (sigstore) seriam a prova de origem, mas o custo não foi medido e eles exigiriam crates novos.
- **Limite de 60 por hora:** `list --upgradable` pela API estoura com muitos apps. Por isso o redirecionamento `302`.
  - Token (`GITHUB_TOKEN` no ambiente) sobe o limite para 5000 por hora. Ler o token do `gh` seria ler credencial de outra ferramenta e precisa de decisão do usuário.
- **Repositório sem release, só com código-fonte, ou só com `.deb`/`.tar.gz` de Linux:** recusa clara.
- **Pré-release e rascunho:** `releases/latest` ignora os dois. Pedir uma tag exata (`@<tag>`) usa `/releases/tags/<tag>`.
- **Antivírus e SmartScreen:** o arquivo baixado pelo wingetrs não leva marca da web, como no caminho do Scoop. Nada muda.

## Proposta de fatias (cada uma vira tarefa com brief)

1. **Cliente de releases.** `latest` e `tags/<tag>`, mais a tag pelo `302`. Erros tipados: offline, `404` e limite (`403`/`429`, com a hora de `X-RateLimit-Reset`). Testes com `wiremock` e fixtures JSON dos 12 repositórios acima, só com nomes e digests.
2. **Escolha do arquivo.** Função pura com os desempates medidos, mais um teste de tabela com os 12 casos (3 únicos, 5 desempatados, 4 ambíguos). Ambíguo vira seletor, nunca chute.
3. **Manifesto sintético e plano.** `install gh:<dono>/<repo>[@tag]`. Antes de usar o caminho direto, procura nos buckets um manifesto cujo `url` aponte para o mesmo repositório (os 1220 + 1466 acima) e oferece o manifesto revisado primeiro.
4. **Atualização e remoção:** pelo estado registrado.
5. **TUI:** busca por `gh:` (depois, se o usuário quiser).

Não medido: tempo de implementação; custo de verificar atestados; quantos apps sem bucket o usuário instalaria assim.

## Recomendação

Viável e barato no que importa: download, hash, extração e shims já existem, e o `digest` da API resolve o hash. A parte nova é a escolha do arquivo (4 de 12 precisam do usuário) e o aviso de confiança.

Sugestão: primeira versão só com arquivos compactados e `.exe` solto, o bucket do Scoop oferecido primeiro, e instaladores recusados. Decisão do usuário quando chegar a vez no backlog.
