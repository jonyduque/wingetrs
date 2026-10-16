# Task 113 — Sonda: como saber que o winget pediu reinício

Status: DONE

Commits: (ver a mensagem final; este relatório é o único arquivo do commit `docs(probe): how winget reports a pending reboot`)

Gate: não rodado. Commit só de relatório (nenhum código); outro agente edita `src/bridge/spawn_win.rs` e `tests/bridge.rs` no mesmo checkout, e o gate mediria o trabalho dele, não o desta sonda.

Mutation proofs: no mutation proof — sonda sem código. A única "Check beyond the steps" (não rodar instalador, não escrever no registro) é de conduta: só rodei `reg query`, `Get-ItemProperty`/`Test-Path` (leitura), `winget --version`, `Get-AppxPackage` e leitura de bytes do `resources.pri` instalado.

Fontes: winget-cli no commit `39739564a4aaf1071b17d163ec332a08b9bcf05c` (`docs/reference/winget-cli/SOURCE_COMMIT.txt`), baixadas por `curl` de `raw.githubusercontent.com` e `gh api` para listar pastas. winget instalado nesta máquina: `v1.29.380`. Cultura e UI: `pt-BR`.

---

## 1. O texto do aviso

### 1.1 De onde sai (InstallFlow.cpp, commit 3973956)

`ReportInstallerResult::operator()` (l. 483–583):

- l. 489: se `installResult != 0` **e** não está em `InstallerSuccessCodes` do manifesto, procura em `ExpectedReturnCodes`.
- l. 496: `GetExpectedReturnCode(...)`; l. 127–128 mapeia `RebootRequiredToFinish` → HR `0x8A150109` com mensagem `Resource::String::InstallFlowReturnCodeRebootRequiredToFinish`.
- l. 501–506: `case APPINSTALLER_CLI_ERROR_INSTALL_REBOOT_REQUIRED_TO_FINISH:` → `SetFlags(RebootRequired)`, `Reporter.Warn() << returnCode.Message`, `terminationHR = S_OK`.
- Como `terminationHR` vira `S_OK`, nada mais é impresso: o `else` com `InstallFlowInstallSuccess` ("Instalado com êxito", l. 581) **não roda**. A saída do job termina com a linha do aviso e o código 0.
- Consequência: `0x8A15_0109` (que `winget_message` já mapeia, `src/bridge/mod.rs:196`) **nunca chega como código de saída** do `winget`; o caso real é código 0 + linha do aviso, sem "Instalado com êxito".

Quem gera `RebootRequiredToFinish` sem o manifesto declarar (`ManifestCommon.cpp`, `GetDefaultKnownReturnCodes`, l. 1088–1125):

| Tipo | Código do instalador | Enum |
|---|---|---|
| Msi / Wix / Burn | 3010 `ERROR_SUCCESS_REBOOT_REQUIRED` | `RebootRequiredToFinish` (→ aviso + 0) |
| Msi / Wix / Burn | 1641 `ERROR_SUCCESS_REBOOT_INITIATED` | `RebootInitiated` (→ erro `0x8A15010B`) |
| Inno | 8 | `RebootRequiredForInstall` (→ erro `0x8A15010A`) |
| Nullsoft | — | nenhum |

Manifestos podem declarar outros códigos em `ExpectedReturnCodes` com `returnResponse: rebootRequiredToFinish`; caem no mesmo `case`.

### 1.2 Chave e texto

Chave: **`InstallFlowReturnCodeRebootRequiredToFinish`**.

- `en-US` (`src/AppInstallerCLIPackage/Shared/Strings/en-us/winget.resw`, l. 1332–1333): `Restart your PC to finish installation.`
- `pt-BR` (`Localization/Resources/pt-BR/winget.resw`, l. 1304–1305): `Reiniciar seu PC para terminar a instalação.`

Nota de caminho: o brief supôs `Shared/Strings/<locale>/`; nesse commit `Shared/Strings/` só tem `en-us`. As traduções ficam em `Localization/Resources/<locale>/winget.resw`.

Idiomas que o winget traz (11): `en-us` (fonte) + `de-DE es-ES fr-FR it-IT ja-JP ko-KR pt-BR ru-RU zh-CN zh-TW` (`gh api repos/microsoft/winget-cli/contents/Localization/Resources?ref=3973956`).

Texto da chave em cada um (copiado do `.resw`, byte a byte):

```
en-us: Restart your PC to finish installation.
de-DE: Starten Sie den PC neu, um die Installation abzuschließen.
es-ES: Reinicie el equipo para finalizar la instalación.
fr-FR: Redémarrez votre PC pour terminer l’installation.
it-IT: Riavvia il PC per completare l'installazione..
ja-JP: PC を再起動してインストールを完了します。
ko-KR: PC를 다시 시작하여 설치를 완료합니다.
pt-BR: Reiniciar seu PC para terminar a instalação.
ru-RU: Перезапустите компьютер, чтобы завершить установку.
zh-CN: 重启电脑以完成安装。
zh-TW: 重新啟動您的電腦以完成安裝。
```

Armadilhas no próprio texto: `it-IT` termina com **dois pontos** (`..`); `fr-FR` usa apóstrofo tipográfico `’` (U+2019), `it-IT` usa `'` ASCII. Qualquer comparação tem de usar o texto copiado, não redigitado.

### 1.3 Conferência com o winget instalado (v1.29.380)

Medido lendo os bytes dos `.pri` instalados (só leitura):

- `…\Microsoft.DesktopAppInstaller_1.29.380.0_x64__8wekyb3d8bbwe\resources.pri` contém em UTF-8 `Restart your PC to finish installation.` → `True`.
- `…\Microsoft.DesktopAppInstaller_1.29.380.0_neutral_split.language-pt_8wekyb3d8bbwe\resources.pri` contém em UTF-8 `Reiniciar seu PC para terminar a instalação.` → `True` (1 ocorrência; o PRI deduplica as duas chaves de mesmo texto).

Ou seja, o texto do commit de referência é o mesmo que o winget desta máquina imprime. As traduções vêm num pacote de recurso separado (`split.language-pt`), escolhido pela língua de UI do Windows.

### 1.4 Outras chaves de reinício (para não confundir)

| Chave | en-us | pt-BR |
|---|---|---|
| `InstallFlowReturnCodeRebootRequiredForInstall` | Your PC will restart to finish installation. | Seu PC será reiniciado para terminar a instalação. |
| `InstallFlowReturnCodeRebootInitiated` | Installation failed. Restart your PC then try again. | Falha na instalação. Reinicie o computador e tente novamente. |
| `APPINSTALLER_CLI_ERROR_INSTALL_REBOOT_REQUIRED_TO_FINISH` | Restart your PC to finish installation. | Reiniciar seu PC para terminar a instalação. |
| `APPINSTALLER_CLI_ERROR_INSTALL_REBOOT_REQUIRED_FOR_INSTALL` | Installation failed. Restart your PC then try again. | Falha na instalação. Reinicie o computador e tente novamente. |
| `APPINSTALLER_CLI_ERROR_INSTALL_REBOOT_INITIATED` | Your PC will restart to finish installation. | Seu PC será reiniciado para terminar a instalação. |

Os textos de `…ForInstall` e `…Initiated` estão **trocados** entre as chaves `InstallFlowReturnCode*` e as `APPINSTALLER_CLI_ERROR_*` (upstream). Não afeta o caso `ToFinish` (texto igual nas duas), e os outros dois casos já chegam como código ≠ 0 (`0x8A15010A`/`0x8A15010B`), que `winget_message` trata pelo código.

Também existe `InstallFlowRegistrationDeferred` ("Successfully installed. Restart the application to complete the upgrade." / "Instalado com êxito. Reinicie o aplicativo para concluir a atualização.") — reinício do **aplicativo** (MSIX), não do PC; não deve casar.

## 2. Sinais de reinício pendente no registro (esta máquina, agora)

Comandos (PowerShell; no Git Bash o `/v` vira caminho e `reg` responde "sintaxe inválida"):

```
> reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"
ERRO: O sistema não pôde localizar a chave do Registro ou valor especificado.      (exit 1)

> reg query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired"
ERRO: O sistema não pôde localizar a chave do Registro ou valor especificado.      (exit 1)

> reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager" /v PendingFileRenameOperations
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager
    PendingFileRenameOperations    REG_MULTI_SZ    *1\??\C:\Windows\System32\spool\drivers\x64\3\New\MXDWDRV.DLL\0*1\??\C:\Windows\...  (exit 0)
```

Resumo (`Get-ItemProperty` / `Test-Path`):

| Sinal | Existe agora? |
|---|---|
| CBS `RebootPending` | não (`Test-Path` → `False`); existe a irmã `SessionsPending`, que não é sinal de reinício |
| WU `Auto Update\RebootRequired` | não (`False`) |
| `PendingFileRenameOperations` | **sim, 96 strings** (pares origem/destino): drivers do spooler (`spool\drivers\x64\3\New\*.DLL`, `V4Dirs`), fontes do Office (`OFFSYM*.TTF`), `EdgeUpdate\1.3.279.5`/`1.3.283.15`, OneDrive (`OneDriveSetup.exe`, `26.183.0920.0005\*.dll`), Office ClickToRun (`Updates\16.0.20527.20054`) |

Último boot: 2026-10-09 23:34. Portanto o PFRO fica não vazio por dias, preenchido por atualizadores em segundo plano (Edge, OneDrive, Office, spooler) que nada têm a ver com o wingetrs.

### 2.1 O que cada tecnologia de instalador usa

- **MSI** (winget passa `/quiet /norestart`, `ManifestCommon.cpp` l. ~1042):
  - Arquivo em uso → o Windows Installer agenda a troca no reinício (`MoveFileEx(MOVEFILE_DELAY_UNTIL_REBOOT)` → **PendingFileRenameOperations**) e pede reinício "whether or not there are any ForceReboot or ScheduleReboot actions" ([ScheduleReboot action](https://learn.microsoft.com/en-us/windows/win32/msi/schedulereboot-action)).
  - `ScheduleReboot`: pede reinício ao fim; com `/norestart` o `msiexec` sai **3010**. Não escreve sinal próprio no registro — só o código de saída.
  - `ForceReboot`: reinicia no meio da instalação e registra a continuação em `RunOnce` ([ForceReboot action](https://learn.microsoft.com/en-us/windows/win32/msi/forcereboot-action)); com `/norestart` costuma sair 1641/3010.
  - CBS `RebootPending` e WU `RebootRequired` são do servicing do Windows e do Windows Update; o MSI não os escreve.
- **NSIS** (winget passa só `/S`): `Delete /REBOOTOK` e `Rename /REBOOTOK` agendam no reinício (MoveFileEx → **PFRO**) e ligam o *reboot flag*; `SetRebootFlag`/`IfRebootFlag`/`Reboot` são decisão do script ([NSIS Docs, Chapter 4](https://nsis.sourceforge.io/Docs/Chapter4.html)). O código de saída **não** reflete o reboot flag e o winget não tem código padrão de reinício para Nullsoft ⇒ o winget imprime "Instalado com êxito", sem aviso.
- **Inno** (winget passa `/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`): flag `restartreplace` de `[Files]` registra a troca "by calling MoveFileEx or by creating an entry in wininit.ini" (→ **PFRO**) ([Files section](https://jrsoftware.org/ishelp/topic_filessection.htm)); `AlwaysRestart=yes` idem sem PFRO. Com `/NORESTART` a instalação bem-sucedida sai **0** ([Setup exit codes](https://jrsoftware.org/ishelp/topic_setupexitcodes.htm)); o código **8** é só "Preparing to Install … needs to be restarted" (falha antes de instalar) ⇒ winget `RebootRequiredForInstall` = erro `0x8A15010A`, não aviso.

## 3. Recomendação: **(A)** — casar o texto do `.resw`, sem (B)

### (A) casar o texto do `.resw`

Como: guardar as 11 strings da §1.2 copiadas verbatim do `.resw` do commit de referência (não traduzidas à mão), com comentário citando arquivo/chave/commit; com código 0, se alguma linha do `Tail` (sem os escapes VT, `trim`) for igual a uma delas ⇒ "Atualizado; reinicie o computador para concluir." Casar contra as 11, não só contra o idioma atual: dispensa descobrir a língua de UI e cobre usuário com UI em outro idioma.

- **Falso positivo:** praticamente nenhum. A linha só é impressa nesse `case` (l. 501–506) e o winget não repassa a saída do instalador (ShellExecute sem pipe). O texto igual em `APPINSTALLER_CLI_ERROR_INSTALL_REBOOT_REQUIRED_TO_FINISH` só aparece como mensagem de HRESULT, que vem com código ≠ 0.
- **Falso negativo:**
  - (i) manifesto que põe 3010 em `InstallerSuccessCodes` → cai no `else`, só "Instalado com êxito";
  - (ii) NSIS/Inno que precisam de reinício e saem 0;
  - (iii) tradução alterada numa versão futura do winget → silêncio (o estado volta ao de hoje, "Atualizado."), nunca um erro;
  - (iv) idioma fora dos 11 → o winget cai em en-us, que está na lista.

### (B) comparar os sinais do registro antes/depois do job

- **Falso positivo, alto e medido aqui:** o PFRO já tem 96 strings e é escrito em segundo plano por EdgeUpdate, OneDrive, Office ClickToRun e spooler. Um delta durante um `winget upgrade` de minutos pode vir de qualquer um deles. CBS `RebootPending`/WU `RebootRequired` aparecem quando o Windows Update termina de instalar algo no mesmo intervalo. "Já existia antes" exige delta, e delta não atribui a causa. Num lote com vários jobs, o delta de um job não se separa do de outro. Também lê `HKLM` e exige parsear o `REG_MULTI_SZ`.
- **Falso negativo:** MSI com `ScheduleReboot` sem arquivo em uso (só 3010, nada no registro); Inno `AlwaysRestart`; NSIS que chama `SetRebootFlag` sem `/REBOOTOK`.
- Único ganho real: NSIS/Inno com troca de arquivo em uso, que (A) não vê — e é justamente onde o ruído de fundo do PFRO o torna não confiável.

### (C) os dois, com (B) só reforçando

- Se "reforçar" = exigir (A) **e** (B), perde os casos MSI 3010 sem PFRO: pior que (A).
- Se = (A) **ou** (B), herda todo o falso positivo de (B).
- Num papel só "informativo" acrescenta leitura de `HKLM` e um `_win.rs` sem mudar a decisão.

Conclusão: **(A)**. É determinístico, não lê `HKLM` e cobre o caminho que o winget de fato sinaliza (MSI/Wix/Burn 3010 e manifestos com `rebootRequiredToFinish`). Os falsos negativos (i) e (ii) são também falsos negativos do próprio winget — ele mesmo não avisa — e o wingetrs fica no mesmo nível dele.

Decisions:
- Listei as 11 traduções em vez de só en-US/pt-BR: (A) fica mais simples e robusto casando todas.
- Usei PowerShell para o `reg query` com `/v` porque o Git Bash converte `/v` em caminho (saída "sintaxe inválida" registrada).
- Conferi o texto no `resources.pri` instalado para provar que o commit de referência bate com a v1.29.380 desta máquina.

Concerns:
- Não medido: se o `Reporter.Warn()` emite escapes VT (cor amarela) quando a saída é pipe. A implementação deve remover `ESC[…m` antes de comparar, ou casar por `contains`.
- Não medido: a codificação da saída do winget em pipe nesta máquina para o texto com acento (`instalação`). Se o bridge já decodifica UTF-8 corretamente para as outras mensagens, vale o mesmo aqui.
- Não medido ao vivo (o brief proíbe rodar instalador): a sequência real "aviso como última linha, código 0". Ela foi deduzida do código-fonte (l. 501–506 e 561–582).
- `winget_message` mapeia `0x8A15_0109`, mas esse código nunca sai do processo `winget` (vira `S_OK`); o braço não está errado, mas não cobre o caso real.
