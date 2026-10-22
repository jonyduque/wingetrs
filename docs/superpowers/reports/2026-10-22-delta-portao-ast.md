# Delta do portão de scripts com AST (Task 227), 2026-10-22

Catálogo local (4135 pacotes Scoop), cópia somente leitura; nada executado. Amostra de 200 antes (0 recusas novas, nenhum efeito observado). Commit ac8520b, pwsh 7.6.7.

# Delta do portão com o parser do PowerShell (Task 227)

PowerShell: `C:\Program Files\PowerShell\7\pwsh.exe`

Scripts analisados: **1728**

| Veredito | Só léxico | Léxico + AST |
|---|---|---|
| Confirm | 1392 | 1372 |
| ConfirmStrong | 257 | 273 |
| Refuse | 79 | 83 |

Aceitos pelo léxico que passam a Refuse: **4** de 1649 (0.24 %; limite da tarefa: 5 %)
Falhas do parser (ParserFailed): **0**; erro de sintaxe: **2**

Tempo: primeira resposta (com a partida) 873.1956ms; parser no total 11.2509806s; por script 6.510984ms; execução inteira 13.0050488s

## Scripts com veredito pior (20)

| App | Hook | Veredito | Flags novas |
|---|---|---|---|
| scoop:main/azure-cli | pre_uninstall | Confirm → ConfirmStrong | CalledFile linha 2: roda um arquivo que não é mostrado por outro programa |
| scoop:main/cygwin | installer.script | Confirm → ConfirmStrong | CalledFile linha 7: roda um arquivo que não é mostrado por outro programa |
| scoop:main/tlrc | installer.script | Confirm → Refuse | Elevation linha 5: Start-Process com -Verb calculado (pode pedir elevação) |
| scoop:extras/anaconda3 | uninstaller.script | Confirm → ConfirmStrong | CalledFile linha 1: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/autohotkey | installer.script | Confirm → ConfirmStrong | CalledFile linha 3: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/ditto | pre_install | Confirm → ConfirmStrong | CalledFile linha 3: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/everything-lite | pre_install | Confirm → ConfirmStrong | CalledFile linha 2: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/everything | pre_install | Confirm → ConfirmStrong | CalledFile linha 5: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/fastcopy | pre_uninstall | Confirm → ConfirmStrong | CalledFile linha 3: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/focus-editor | installer.script | Confirm → ConfirmStrong | CalledFile linha 2: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/gpg4win | pre_uninstall | Confirm → ConfirmStrong | CalledFile linha 1: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/jkrypto | pre_install | Confirm → Refuse | SyntaxError linha 1: o PowerShell não entende o script (erro de sintaxe na linha 1) |
| scoop:extras/miniconda3 | uninstaller.script | Confirm → ConfirmStrong | CalledFile linha 5: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/miniforge3 | uninstaller.script | Confirm → ConfirmStrong | CalledFile linha 4: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/mp3tag | pre_install | Confirm → ConfirmStrong | CalledFile linha 6: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/multipar | pre_uninstall | Confirm → ConfirmStrong | CalledFile linha 1: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/paint.net-plugin-boltbait | pre_install | Confirm → Refuse | DynamicCode linha 2: carrega código .NET na hora |
| scoop:extras/vcredist2010 | post_install | Confirm → ConfirmStrong | CalledFile linha 6: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/winaero-tweaker | installer.script | Confirm → ConfirmStrong | CalledFile linha 1: roda um arquivo que não é mostrado por outro programa |
| scoop:extras/xenu | installer.script | Confirm → Refuse | SyntaxError linha 2: o PowerShell não entende o script (erro de sintaxe na linha 2) |
