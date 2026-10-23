# Prova curta antes da v0.4.0 — roteiro (2026-10-22)

Cerca de 15 minutos. Prova o que só roda de verdade numa máquina: o auxiliar elevado da Task 202 (catálogo assinado), a recusa de nomes soltos com o processo elevado (Task 233), o portão de scripts com AST (Task 227) e a reserva por instalador (Task 196). Cada passo diz antes o que observar.

Instala de verdade dois pacotes pequenos (nano, do Scoop, e Notepad++, do winget, para todos os usuários) e remove os dois no fim.

## Preparação (terminal de administrador)

O auxiliar só sobe se o `wingetrs.exe` estiver numa pasta que só administradores gravam. Copie o binário novo por cima do instalado, guardando o atual:

```powershell
Copy-Item 'C:\Program Files\wingetrs\wingetrs.exe' 'C:\Program Files\wingetrs\wingetrs-0.3.0.exe'
Copy-Item 'C:\Users\jonyd\Projetos\wingetrs\target\release\wingetrs.exe' 'C:\Program Files\wingetrs\wingetrs.exe' -Force
```

Em cada terminal usado abaixo, ligue o registro de texto (o orquestrador lê depois):

```powershell
$env:WINGETRS_TRANSCRIPT = "$env:TEMP\wr-prova-$(if (([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole('Administrators')) {'admin'} else {'user'}).txt"
```

## A. Script do Scoop num terminal normal (Task 227)

```powershell
wingetrs install nano --allow-scripts
```

Observe:
- antes de instalar, a tela mostra o script `pre_install` e a análise; o veredito é "confirmar" (não "recusado");
- a instalação termina com "Instalado: nano";
- `nano --version` responde num terminal novo.

## B. O mesmo num terminal de administrador (Task 227)

```powershell
wingetrs uninstall nano   # no terminal normal, antes
wingetrs install nano --allow-scripts   # agora no terminal de admin
```

Observe: recusa antes de baixar qualquer coisa, dizendo que script de manifesto não roda como administrador e para abrir um terminal sem administrador. Nada é instalado.

## C. Pacote do winget para todos os usuários pelo auxiliar (Task 202) — terminal normal

```powershell
wingetrs install Notepad++.Notepad++ --scope machine --dry-run
wingetrs install Notepad++.Notepad++ --scope machine
```

Observe:
- no `--dry-run`, o plano diz "Roda como administrador (pede permissão de administrador)";
- "Aguardando autorização para executar como administrador" e **um** UAC (o do wingetrs, não o do instalador);
- a fase "Baixando e conferindo o instalador de Notepad++ como administrador", com relógio;
- termina instalado; `wingetrs list` mostra o Notepad++.

Se falhar com erro de catálogo ou de assinatura, copie a mensagem: é o ponto que só esta prova mostra.

## D. Remoção pelo auxiliar — terminal normal

```powershell
wingetrs uninstall Notepad++.Notepad++
```

Observe: um UAC, remoção concluída, Notepad++ some da lista.

## E. Reserva por instalador (Task 196/202) — terminal normal

O binário em `target\release` não está numa pasta só de administrador, então o auxiliar não pode subir:

```powershell
& 'C:\Users\jonyd\Projetos\wingetrs\target\release\wingetrs.exe' install Notepad++.Notepad++ --scope machine
```

Observe: aviso de que o auxiliar não está disponível e, em vez dele, "Aguardando autorização…" com o UAC **do próprio instalador** (o nome no UAC é o do Notepad++, não o do wingetrs). **Recuse** esse UAC: a saída deve dizer que nada foi alterado.

## F. Terminal de administrador: winget sem PATH (Task 233)

```powershell
wingetrs
```

Na TUI, aba Atualizações: os pacotes do winget aparecem (o winget foi achado pela pasta de administrador). Saia com `q`.

## G. `install.ps1` novo — terminal normal (sem reinstalar)

```powershell
irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1 | iex
```

Observe: seletor com setas para o escopo; escolha "Para todos os usuários"; "Aguardando autorização…"; **recuse** o UAC; a saída diz que nada foi alterado. (Não aceite: reinstalaria a v0.3.0 por cima do binário de teste.)

## Fim

```powershell
wingetrs uninstall nano   # se ainda instalado
Copy-Item 'C:\Program Files\wingetrs\wingetrs-0.3.0.exe' 'C:\Program Files\wingetrs\wingetrs.exe' -Force   # admin; ou mantenha o novo
```

Diga ao orquestrador o resultado de cada letra (ok / o que apareceu). Os registros ficam em `%TEMP%\wr-prova-user.txt` e `%TEMP%\wr-prova-admin.txt`.

## Resultado (usuário, 2026-10-23)

| Passo | Resultado |
|---|---|
| A | ✗ Extração falhou: `compression method not supported: 9` (zip Deflate64; defeito da 2a, não destas tasks). Desfazer ok. ⇒ Task 235. Repetir A depois dela. |
| B | ✓ Recusado antes do script ("o wingetrs não analisa scripts com o processo elevado"). |
| C | ✓ Um UAC, fase "Baixando e conferindo… como administrador", instalado. 1ª tentativa: "o pacote está em uso" com o Notepad++ aberto (mensagem certa, vinda do código de retorno do manifesto). |
| D | Não feito (o usuário usa o Notepad++). Remoção pelo auxiliar segue sem prova nesta rodada. |
| E | ✓ Aviso da pasta de administrador, UAC do próprio instalador, instalado (UAC aceito; "nada foi alterado" ao recusar não testado aqui). |
| F | ✓ TUI no terminal de admin mostra os pacotes do winget. |
| G | ✓ Seletor de escopo, UAC com a mensagem, recusa ⇒ "nada foi alterado". |

O transcript do terminal de administrador não foi criado; B aparece no arquivo do terminal normal.
