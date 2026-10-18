# Task 182 — relatório

Status: DONE_WITH_CONCERNS

## Commits

```
319c751 fix(bridge): elevated uninstall rules accept real admin-owned locations
6a26d55 test(bridge): measure real HKLM uninstallers against the elevated rules
f2f9e43 fix(bridge): review of task 182
6200426 fix(bridge): elevated duplicate deletion counts only other HKLM records with a newer version
41cb79a feat(bridge): elevated helper runs revalidated HKLM uninstallers and deletes revalidated HKLM keys
```
(`git log --format='%h %s' b64193a..HEAD -- <arquivos da tarefa>`)

Arquivo Win32 novo: `src/bridge/arp_ops_win.rs` (não mexi em `staging_win.rs`); a linha `#[cfg(windows)] pub mod arp_ops_win;` entrou em `src/bridge/mod.rs` (o formatador do hook a pôs em ordem alfabética, antes de `elevate_win`).

## Gate

Rodei uma vez; falhou só em arquivo alheio (`src/install/planner.rs`, Task 156):

```
[OK]   rules 6s
[OK]   rules-selftest 14s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\install\planner.rs:10:
  (log completo: target/verify.log)
```

Provas dos meus arquivos (saída real):
- `rustfmt --edition 2024 --check` nos 7 arquivos da tarefa: `fmt=0`
- `cargo clippy --all-targets -- -D warnings`: só aparecem `src\winget_install\ops.rs:299:26` e `src\winget_install\plan.rs:37:1` (Task 181); nenhum dos meus arquivos.
- `cargo test --quiet --test it elevated` (elevated_uninstall + elevated_install + elevated_worker), depois da correção: `test result: ok. 66 passed; 0 failed; 3 ignored; 0 measured; 1397 filtered out`
- clippy `-D warnings` / rustfmt `--check` / `check_rules` repetidos depois da correção: nada nos meus arquivos, `fmt=0`, `rules=0`
- `cargo test --quiet --lib bridge::`: `test result: ok. 12 passed; 0 failed; 1 ignored`
- `pwsh -File scripts/check_rules.ps1`: `rules=0`

## Mutation proofs

1. Hklm32 com a view 64 (teste 5):
   `MSYS_NO_PATHCONV=1 pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -Anchor 'MachineHive::Hklm32 => Hive::MachineX86,' -Replacement 'MachineHive::Hklm32 => Hive::MachineX64,' -TestCommand 'cargo test --quiet --test it elevated_uninstall::delete_orphan_revalidates'`
   `[OK] mutação PEGA: 'MachineHive::Hklm32 => Hive::MachineX86,' -> 'MachineHive::Hklm32 => Hive::MachineX64,' fez o teste falhar (exit 101).` (falhou em `elevated_uninstall.rs:477`, asserção)
2. `in_admin_folder` com prefixo de texto (teste 2):
   `... -Path src/winget_install/elevated_rules.rs -Anchor 'p.len() > root.len() && root.iter().zip(p).all(|(a, b)| a == b)' -Replacement 'p.len() > root.len() && p.join("\\").starts_with(&root.join("\\"))' -TestCommand 'cargo test --quiet --test it elevated_uninstall::admin_folder_rule'`
   `[OK] mutação PEGA: 'p.len() > root.len() && root.iter().zip(p).all(|(a, b)| a == b)' -> 'p.len() > root.len() && p.join("\\").starts_with(&root.join("\\"))' fez o teste falhar (exit 101).` (linha 208: `C:\PFX\u.exe`)
3. `expand_known` aceita variável do ambiente (teste 1):
   `... -Path src/winget_install/elevated_rules.rs -Anchor '        _ => return None,' -Replacement '        _ => return std::env::var(name).ok(),' -TestCommand 'cargo test --quiet --test it elevated_uninstall::expand_known_only'`
   `[OK] mutação PEGA: '        _ => return None,' -> '        _ => return std::env::var(name).ok(),' fez o teste falhar (exit 101).` (linha 182: `%LOCALAPPDATA%`)

Correção pedida pelo orquestrador (revisão de segurança, duplicata):
- volta a incluir HKCU: `... -Anchor '.filter(|o| is_machine(o.hive))' -Replacement '.filter(|o| is_machine(o.hive) || o.hive == Hive::User)' -TestCommand '... elevated_uninstall::delete_duplicate_needs_other_record'`
  `[OK] mutação PEGA: '.filter(|o| is_machine(o.hive))' -> '.filter(|o| is_machine(o.hive) || o.hive == Hive::User)' fez o teste falhar (exit 101).`
- qualquer HKLM de mesmo nome, sem exigir versão maior (a principal poderia ser apagada): `[OK] mutação PEGA: 'version_of(o).is_some_and(|v| v > mine)' -> 'version_of(o).is_some()' fez o teste falhar (exit 101).`
- sem recusar `d` em HKCU: pelo protocolo é inalcançável (`MachineHive` só tem HKLM), sobreviveu contra o módulo inteiro; acrescentei o teste puro `elevated_duplicate_never_for_a_user_record` e então `[OK] mutação PEGA: '    if !is_machine(d.hive) {` (falhou na linha 569).

Extras (recusas que acrescentei):
- sem o filtro `.exe`: `[OK] mutação PEGA: '.filter(|p| p.to_ascii_lowercase().ends_with(".exe"))' -> '.filter(|p| !p.is_empty())' fez o teste falhar (exit 101).`
- sem a recusa de interpretador: `[OK] mutação PEGA: 'if INTERPRETERS.contains(' -> 'if false && INTERPRETERS.contains(' fez o teste falhar (exit 101).`
- sem a exclusão das pastas graváveis do Windows: `[OK] mutação PEGA: '        return false;` (âncora de várias linhas; falhou `admin_folder_other_roots_and_user_writable_windows_folders` na linha 235)

### Roteiro ao vivo (item 4) — para o usuário, num terminal **elevado** (PowerShell)

```powershell
# 0. Backup das duas chaves Uninstall (restauração em caso de erro)
reg export "HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall" "$env:USERPROFILE\Desktop\uninst32.reg" /y
reg export "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall" "$env:USERPROFILE\Desktop\uninst64.reg" /y
# 1. Chave de teste HKLM 32 (desinstalador inexistente)
reg add "HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /v DisplayName /t REG_SZ /d "wingetrs test orphan" /f
reg add "HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan" /v UninstallString /t REG_SZ /d "\"C:\Program Files\wingetrs-test\u.exe\"" /f
```
Depois, num terminal **normal**: `cargo test --release --test it elevated_uninstall::live_delete_test_key -- --ignored` (um UAC; a chave tem de sumir).
Para a view 64: crie a mesma chave em `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan` e troque no teste `MachineHive::Hklm32`/`Hive::MachineX86` por `Hklm64`/`MachineX64` (o teste versionado cobre só a 32, como o brief pede). Conferência: `reg query "HKLM\...\Uninstall\wingetrs-test-orphan"` deve dizer que não existe, e a mesma chave na **outra** view não pode ter sido tocada.
Restauração (se algo der errado): `reg import "$env:USERPROFILE\Desktop\uninst32.reg"` e `uninst64.reg`; limpeza: `reg delete "HKLM\...\Uninstall\wingetrs-test-orphan" /f`.
Não rodei o teste ao vivo (pede UAC).

## Decisions

- Lança com `Command` (`env_clear` + `installer_env` da Task 180, `TEMP` numa pasta de execução nova em `Program Files\wingetrs\staging`, cwd = pasta do desinstalador), não com `ShellLauncher`: o `ShellExecuteExW` herda o ambiente do auxiliar (do usuário), e o orquestrador pediu ambiente do zero como na 180. Janela aparece normal (GUI via CreateProcess).
- `WorkerOps` ganhou operações de baixo nível com padrão "não suportado" (`known_folders`, `read_arp`, `arp_entries`, `exists` = true, `msiexec`, `run_uninstaller(program,args)`, `delete_arp_key(hive,key)`); toda a regra (revalidar, recalcular, recusar) fica em `worker.rs`, testável. `ElevatedRunner` ganhou os dois métodos com padrão `Remote("não suportado")` para não quebrar fakes da Task 181.
- Recusas extras: interpretadores (`cmd`, `powershell`, `pwsh`, `rundll32`, `regsvr32`, `wscript`, `cscript`, `mshta`) mesmo em pasta de admin (os argumentos podem apontar para arquivo do usuário); pastas graváveis pelo usuário dentro de `Windows` (`Temp`, `Tasks`, `tracing`, `System32\Tasks`, `System32\spool`, `SysWOW64\Tasks`); `msiexec` só `/x{GUID}` (ou `/x {GUID}`) seguido de `/quiet /passive /q /qn /qb /qb- /qb! /norestart REBOOT=ReallySuppress`, argumentos reconstruídos.
- A checagem de link é feita no próprio `delete_key`, no handle aberto com `REG_OPTION_OPEN_LINK` que é apagado (`RegDeleteTree(handle, NULL)` + `RegDeleteKeyEx(parent, key, KEY_WOW64_64KEY)`), depois da revalidação da razão (a leitura usa `read_details`, que segue link).
- `exists` real usa `symlink_metadata` (link pendente conta como existente: nada vira sobra por engano).

- **Duplicata (mudança em relação ao brief, pedida pelo orquestrador):** o brief mandava comparar com HKLM 64, HKLM 32 e HKCU, e o teste 6 apagava com par em HKCU. Agora o auxiliar usa `elevated_duplicate`: só HKLM conta (o `arp_entries` do Windows nem lê HKCU), a chave alvo tem de ser HKLM e outro registro HKLM (outra chave ou view) com o mesmo nome tem de ter `DisplayVersion` estritamente maior (`core::version`). "Principal" = a de maior versão; versões iguais ou ausentes ⇒ recusa (não dá para saber qual fica). `is_duplicate` continua puro como no brief. O teste 6 foi reescrito: par HKCU idêntico ⇒ recusa; par HKLM mais novo ⇒ apaga.

## Concerns

- Kindle (S3: HKLM32 2.8.3 + HKCU 2.9.1): com a regra nova, o auxiliar elevado **não** apaga a HKLM como duplicata (o par está em HKCU). O cliente precisa de outro caminho (ex.: oferecer o desinstalador da HKLM, ou "sobra" se o desinstalador sumiu). Decisão de produto para o orquestrador.

- TDD: escrevi a implementação antes de rodar os testes (não vi o vermelho inicial); as mutações acima mostram que os testes falham pelas regras.
- `in_admin_folder` é por lista de pastas, não por ACL: uma subpasta de `Program Files` com ACL frouxa (app que dá Modify a Users) ainda passa. Checagem por DACL do arquivo e da cadeia de pastas seria a regra completa; fica para decisão.
- Desinstalador NSIS se copia para `%TEMP%` (`Au_.exe`) e o original sai na hora: o código devolvido é o do original e a pasta de execução é apagada enquanto a cópia ainda roda (o `Drop` só avisa). Mesmo comportamento do winget sem `_?=`; não tratado aqui.
- `live_delete_test_key` não foi rodado (pede UAC); não há teste unitário do `delete_key`/link (regra: testes não tocam o registro real).
- "Recusa nova em parser — rodar contra o corpus real": não há corpus de ARP real na sonda; as regras de `msiexec`/interpretador/pastas não foram conferidas contra as entradas ARP desta máquina (não medido).


## Revisão de segurança (f2f9e43)

Arquivos fora da lista original, com motivo: `src/winget_install/arp_read_win.rs` (Task 172; agora expõe `uninstall_root` e `details_of`, para o delete ler pelo mesmo handle e testar a view), `tests/it/elevated_install.rs` (Task 180; só o caso `/VERYSILENT /TYPE=full` virou `/PASSWORD=x`, porque o item Inno passou a aceitar `/TYPE=`).

1. DACL real: `elevated_rules::sddl_admin_only(sddl, AclLevel)` (puro). O dono tem de ser SY, BA ou TrustedInstaller; ACE inherit-only (IO) é ignorada; ACE de negação não concede nada; código ou tipo de ACE desconhecido ⇒ recusa. Direitos perigosos por nível: no arquivo, escrita/acréscimo/GW + DELETE/WD/WO/GA; na pasta-mãe, isso + DELETE_CHILD; acima dela, só DELETE/WD/WO/GA/DELETE_CHILD. Motivo, medido nesta máquina: `C:\` dá `LC` a AU e `ProgramData` dá `DCLCRPCR` a BU (SDDLs reais nos testes `acl::real_descriptors`). Exigir isso em todos os níveis recusaria qualquer desinstalador. `arp_ops_win::hold_admin_chain` abre o .exe (compartilhando só leitura, sem seguir ponto de reanálise) e cada pasta até a raiz (sem compartilhar exclusão, para não serem renomeadas), lê dono+DACL por `GetSecurityInfo` no handle e segura tudo até o processo terminar. O teste ao vivo `arp_ops_win::tests::live_admin_chain` (só lê ACLs) foi rodado: `test result: ok. 1 passed` (aceita `System32\notepad.exe`, recusa um .exe no temp do usuário).
2. Componente terminado em ponto ou espaço ⇒ recusa (`trailing_dot_or_space_component_refused`).
3. `arg_paths`: todo `X:\`/`X:/` (sem letra ou dígito antes) ou `\\` nos argumentos tem de estar em pasta de administrador (`ARG_OUTSIDE_ADMIN`). `INTERPRETERS` ficou com 20 nomes.
4. `WorkerOps::delete_arp_key(hive, key, confirm)`: abre uma vez com `REG_OPTION_OPEN_LINK` + DELETE + leitura, recusa link, lê pelo handle (`details_of`), chama `confirm` (a revalidação do worker) e apaga pelo mesmo handle (`RegDeleteTreeW(handle, NULL)`). A chave vazia sai por `RegDeleteKeyExW(parent, nome)`: a API não tem forma por handle. Ela falha enquanto houver subchaves, então o sucesso prova que a árvore sumiu. Só admin grava essas chaves.
5. `arp_read_win::uninstall_root` + teste unitário `uninstall_root_picks_the_view_of_each_hive`. `delete_subkey_all("")` passa NULL (winreg 0.56 `reg_key.rs:633`), e a doc do `RegDeleteTreeW` diz: "If this parameter is NULL, the subkeys and values of this key are deleted".
6. Inno: `/TASKS=`, `/MERGETASKS=`, `/COMPONENTS=`, `/TYPE=`, `/LANG=` com valor `[A-Za-z0-9_,!-]+`; `/NOICONS`, `/NORESTARTAPPLICATIONS`, `/RESTARTAPPLICATIONS` (`inno_name_list_switches`).

Testes: `cargo test --quiet --test it elevated` ⇒ `test result: ok. 76 passed; 0 failed; 3 ignored`; `cargo test --quiet --lib` ⇒ `test result: ok. 63 passed; 0 failed; 2 ignored`; clippy `-D warnings` sem nada nos meus arquivos; `rustfmt --check` ⇒ `fmt=0`; `check_rules` ⇒ `rules=0`. Gate completo não repetido (regra: uma vez; a falha anterior era de arquivo alheio).

Mutações (todas `MSYS_NO_PATHCONV=1 pwsh -File scripts/mutate.ps1 ...`):
- `[OK] mutação PEGA: 'if !allow || flags.as_bytes().chunks(2).any(|f| f == b"IO") {' -> 'if !allow {' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'if !trusted_sid(owner) || dacl.contains("NO_ACCESS_CONTROL") {' -> 'if dacl.contains("NO_ACCESS_CONTROL") {' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'AclLevel::Parent => replace | WRITE_DATA | APPEND_DATA | DELETE_CHILD | GENERIC_WRITE,' -> 'AclLevel::Parent => replace | APPEND_DATA | DELETE_CHILD | GENERIC_WRITE,' fez o teste falhar (exit 101).` (CRMLog real)
- `[OK] mutação PEGA: '(!n.ends_with(['.', ' '])).then(|| n.to_uppercase())' -> 'Some(n.to_uppercase())' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'if !args_in_admin_folder(&cmd.args, &k) {' -> 'if false && !args_in_admin_folder(&cmd.args, &k) {' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '    "explorer.exe",' -> '    "explorer.exe.off",' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'Hive::MachineX86 => (HKEY_LOCAL_MACHINE, UNINSTALL_X86, KEY_WOW64_64KEY),' -> 'Hive::MachineX86 => (HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_64KEY),' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'Hive::MachineX86 => (HKEY_LOCAL_MACHINE, UNINSTALL_X86, KEY_WOW64_64KEY),' -> 'Hive::MachineX86 => (HKEY_LOCAL_MACHINE, UNINSTALL_X86, 0),' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'matches!(b, b'_' | b',' | b'!' | b'-'))' -> 'matches!(b, b'_' | b',' | b'!' | b'-' | b'.'))' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: 'matches!(b, b'_' | b',' | b'!' | b'-'))' -> 'matches!(b, b'_' | b',' | b'!' | b'-' | b':'))' fez o teste falhar (exit 101).`

Pendências da revisão:
- A checagem de link e a ordem ler → confirmar → apagar no `delete_key` real não têm teste unitário (exigiriam o registro real); a prova é o `live_delete_test_key` (não rodado, pede UAC).
- Não medi quantos desinstaladores HKLM reais desta máquina passam no DACL + `in_admin_folder` + regra dos argumentos (nomes 8.3 como `C:\PROGRA~1\` são recusados). Uma sonda sobre o ARP real é recomendada antes de ligar a UI.
- Pastas acima da mãe com escrita aberta (ex.: `ProgramData`) são aceitas de propósito (ver item 1).

## Medição: desinstaladores HKLM reais (6a26d55)

Comando (só leitura, nada executado): `cargo run --quiet --release --example elevated_arp_probe` (exit 0). O exemplo passa cada entrada visível de HKLM 64/32 (com DisplayName, sem SystemComponent) pelo `serve` real (`RunUninstaller` com a regra recalculada, `nsis_hint=false`) e depois pela regra de dono/DACL da cadeia. Ficou como exemplo, não como teste ignorado, porque o binário `it` não compila agora: `tests/it/wi_orphans.rs` da Task 183, em andamento, importa `orphans::ArpRegistry` etc., que ainda não existem.

Saída real:

```
totais: {"inno": 19, "msi": 31, "outro": 48}
| resultado / motivo | total | msi | inno | outro | exemplos |
|---|---|---|---|---|---|
| DACL/dono (File) | 1 | 0 | 0 | 1 | Microsoft OneDrive |
| DACL/dono (Parent) | 1 | 0 | 0 | 1 | Microsoft Edge |
| desinstalador fora de pasta de administrador | 8 | 0 | 4 | 4 | Windows Driver Package - Actions Semiconductor Co., LTD (WinUSB) USB  (08/08/2013 2.0.0.3); NVM for Windows 1.2.2; EPSON L3250 Series Printer Uninstall; Pacote de Driver do Windows - libusb-win32 castor_libusb0 (02/23/2013 1.2.6.0); Ruby 3.4.9-1-x64 |
| desinstalador não encontrado | 2 | 0 | 0 | 2 | Cities: Skylines; Counter-Strike: Global Offensive |
| desinstalador por interpretador recusado | 4 | 0 | 0 | 4 | NVIDIA Driver de gráficos 616.92; NVIDIA Software do sistema PhysX 9.26.0703; NVIDIA FrameView SDK 1.4.10624.35034762; NVIDIA Driver de áudio HD 1.4.6.3 |
| parâmetros do msiexec recusados | 1 | 0 | 0 | 1 | Update for  (KB2504637) |
| passa (Interactive) | 18 | 0 | 0 | 18 | Android Studio; Docker Desktop; Microsoft 365 - pt-br; Microsoft 365 Apps para Grandes Empresas - pt-br; Microsoft OneNote - it-it |
| passa (Msi) | 31 | 31 | 0 | 0 | Google Chrome; CMake; Microsoft Visual C++ 2010  x64 Redistributable - 10.0.40219; PDF24 Creator; SafeSign IC MiniDriver |
| passa (Quiet) | 32 | 0 | 15 | 17 | 7-Zip 26.04 (x64); CPUID HWMonitor 1.68; CrystalDiskInfo 9.9.3; CrystalDiskMark 9.0.3; Git |
```

Leitura:
- **MSI: 31/31 passam** (regra Msi, `msiexec` de System32). Nenhum motivo recusa MSI.
- **Inno: 15/19 passam; 4 (21%) recusados por "fora de pasta de administrador"**, acima do limiar de ~20%. Os 4, com a ACL real (`Get-Acl`):
  - `NVM for Windows` em `C:\Users\jonyd\AppData\Local\nvm\unins000.exe`: pasta do usuário. **Recusa correta** (é o caso S3).
  - `VIDaaS Connect` (`C:\VIDaaS Connect\`) e `WebConnection` (`C:\Arquivos TJSP\`): o .exe dá `0x1301bf` (modificar) a AU. **Recusa correta**: qualquer usuário troca o desinstalador.
  - `Ruby 3.4.9-1-x64` (`C:\Ruby34-x64\`): pasta `D:PAI(...FA;;;BA)(...0x1200a9;;;BU)`, dono BA. Só administrador grava. **Falso positivo**: passaria na regra de DACL, mas não está na lista de pastas conhecidas.
  - Entre os Inno legítimos, a recusa indevida é 1 de 16 (6%).
- Outros recusados (não são MSI nem Inno): 4 NVIDIA por `RunDll32 "…\NVI2.DLL",UninstallPackage` (interpretador); 2 `dpinst64` em `C:\PROGRA~1\DIFX\…` (nome 8.3 não casa com `Program Files`); EPSON em `System32\spool\DRIVERS\x64\3` (a exclusão de `System32\spool` é larga demais: só `spool\drivers\color` é gravável pelo usuário); OneDrive (`XA` condicional para BU, recusado por cautela) e Edge (a pasta `Installer` dá algo a um SID de app). Steam/CS:GO e Cities: o "programa" é `steam.exe steam://…` e o caminho não existe nesta instalação. `KB2504637`: `msiexec /package … /uninstall …`.

Ajustes propostos (**não aplicados**; aguardam decisão):
1. (Inno, o que passa do limiar) Aceitar programa fora das raízes conhecidas quando a cadeia real de dono/DACL passa: a DACL vira a regra principal e a lista de pastas fica só como recusa rápida dos lugares sabidamente graváveis (perfil do usuário, `Windows\Temp` etc.). Recupera o Ruby e mantém a recusa de VIDaaS e TJSP, que caem pela DACL.
2. Expandir nomes 8.3 com `GetLongPathNameW` antes da regra de pasta (recupera os 2 `dpinst64`).
3. Estreitar `System32\spool` para `System32\spool\drivers\color` (recupera EPSON).
4. (Opcional) `rundll32` só com DLL absoluta cuja cadeia passa na DACL (recupera NVIDIA). Hoje o interpretador é recusado sempre.

## Ajustes aprovados (319c751)

(1) fora das pastas conhecidas a checagem real de dono/DACL decide (`refused_location` só recusa o que é sempre gravável: perfis, `Windows\Temp`, `Tasks`, `tracing`, `spool\drivers\color`, `Registration\CRMLog`); (2) nomes 8.3 expandidos com `GetLongPathNameW` (inexistente ⇒ não encontrado); (3) `spool` estreitado; (4) `rundll32` do sistema só como `"<dll>",<Export>`, DLL em Program Files (nunca de `SystemRoot`/`SysWOW64`, revisão de LOLBins) com DACL checada e sem interpretador no resto. A segunda medição mostrou falsos positivos do avaliador, corrigidos: ACE de CREATOR OWNER fora de IO e SIDs de serviço (`S-1-5-80-*`, `S-1-5-99-*`) não concedem nada a usuário; condição de ACE `XA` com parênteses lida inteira.

Testes: `cargo test --quiet --test it elevated` ⇒ `test result: ok. 89 passed; 0 failed; 3 ignored`; `--lib bridge::` ⇒ `ok. 12 passed`; fmt ok; `rules=0`; clippy sem nada nos meus arquivos.

Mutações (todas PEGA):
```
[OK] mutação PEGA: 'under(&parts, &k.user_profiles)' -> 'false' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'let long = ops.long_path(&path).map_err(|_| Some(not_found(&path)))?;' -> 'let long = path.clone();' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'System32\spool\drivers\color' -> 'System32\spool' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !in_program_files(&dll, k) {' -> 'if false {' fez o teste falhar (exit 101).   (aceita DLL do System32)
[OK] mutação PEGA: 'if names_interpreter(&rest, &INTERPRETERS) {' -> 'if false {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'file == "rundll32.exe" && is_system_rundll32(&program, &k)' -> 'file == "rundll32.exe"' fez o teste falhar (exit 101).
[OK] mutação PEGA: '|| ["CO", "S-1-3-0"].contains(&sid)' -> '' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'let Some(close) = closing_paren(body) else {' -> 'let Some(close) = body.find(')') else {' fez o teste falhar (exit 101).
```

Medição nova (`cargo run --quiet --release --example elevated_arp_probe`, exit 0):

```
totais: {"inno": 19, "msi": 31, "outro": 48}
| resultado / motivo | total | msi | inno | outro | exemplos |
|---|---|---|---|---|---|
| DACL/dono (File) | 2 | 0 | 2 | 0 | VIDaaS Connect; WebConnection versão 2.0 |
| desinstalador fora de pasta de administrador | 2 | 0 | 1 | 1 | NVM for Windows 1.2.2; USB-Driver-Installer 1.0.3.0 |
| desinstalador não encontrado | 2 | 0 | 0 | 2 | Cities: Skylines; Counter-Strike: Global Offensive |
| parâmetros do msiexec recusados | 1 | 0 | 0 | 1 | Update for  (KB2504637) |
| passa (Interactive) | 27 | 0 | 0 | 27 | Windows Driver Package - Actions Semiconductor Co., LTD (WinUSB) USB  (08/08/2013 2.0.0.3); Android Studio; Docker Desktop; EPSON L3250 Series Printer Uninstall; Pacote de Driver do Windows - libusb-win32 castor_libusb0 (02/23/2013 1.2.6.0) |
| passa (Msi) | 31 | 31 | 0 | 0 | Google Chrome; CMake; Microsoft Visual C++ 2010  x64 Redistributable - 10.0.40219; PDF24 Creator; SafeSign IC MiniDriver |
| passa (Quiet) | 33 | 0 | 16 | 17 | 7-Zip 26.04 (x64); CPUID HWMonitor 1.68; CrystalDiskInfo 9.9.3; CrystalDiskMark 9.0.3; Git |
```

Meta atingida: Ruby, os 2 drivers (8.3), EPSON e NVIDIA passam (Edge e OneDrive também); continuam recusados NVM (perfil do usuário), VIDaaS e TJSP (DACL do .exe dá modificar a AU) e o USB-Driver-Installer do S3. Restam: 2 jogos Steam (`steam.exe steam://…`, caminho que não existe) e o KB2504637 (`msiexec /package … /uninstall …`).
