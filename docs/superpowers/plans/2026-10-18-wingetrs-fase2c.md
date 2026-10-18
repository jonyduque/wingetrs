# wingetrs — Fase 2c: scripts PowerShell, innosetup e installer.file/args (plano)

> **For agentic workers:** execução pelo laço `run-task`. Brief: `pwsh -File scripts/brief.ps1 <N> -Plan docs/superpowers/plans/2026-10-18-wingetrs-fase2c.md`.

**Spec:** `docs/superpowers/specs/2026-10-15-wingetrs-fase2c-design.md` (decidida em 2026-10-18; **D1 = B, prelúdio próprio**: nada da lib do Scoop é baixado nem carregado).

## Global Constraints

- Ler `CLAUDE.md` e `docs/ARMADILHAS.md` antes da tarefa.
- Agentes nunca executam scripts de manifesto reais nem o innounp sobre instaladores reais (só testes `#[ignore = "live: ..."]`).
- Testes sem rede (fixtures e `wiremock`), sem PATH/registro reais.
- Sem crate nova. Texto ao usuário em português.

## Review Focus

1. Chave de script só na variante de arquitetura (`architecture.64bit.pre_install`) e ausente na raiz.
2. Manifesto com várias chaves ao mesmo tempo (innosetup + post_install): a camada mais arriscada vence.
3. Hook como string única × array de linhas; linhas vazias; comentários.
4. Manifesto que já planeja na 2a/2b nunca é contado como 2c.
5. Catálogo com manifesto JSON inválido: contado à parte, nunca derruba o oráculo.

---

### Task 142: Oráculo da 2c — classificação por sub-caso e funções do Scoop usadas

**Modelo:** opus (revisor sonnet).

#### Where this fits

- `src/install/manifest.rs`: `SCRIPT_KEYS` (~:206, 7 chaves), recusa de `innosetup` (~:225–228 "usa innosetup: suportado na fase 2c"), `resolve(raw_json, host)` (~:292) com a regra de variante de arquitetura (`field(root, variant, ...)`).
- `examples/oracle_2a.rs`: oráculo da 2a sobre o `catalog.db` (`is_simple`, leitura dos manifestos do Scoop do catálogo, contagem por motivo). Copie o modo de abrir o catálogo e de iterar os manifestos dele.
- Spec 2c, tabela de sub-casos (seção a): `S-psm` (psmodule), `S-inno` (innosetup), `S-file` (`installer.file`/`args` com `.exe`/`.msi`/`.cmd`, sem script), `S-ps1` (`installer.file` em `.ps1`), `S-hook` (hooks de script: `pre_install`, `post_install`, `pre_uninstall`, `post_uninstall`, `installer.script`, `uninstaller.script`). Ordem de risco crescente: S-psm < S-inno < S-file < S-ps1 < S-hook.

#### What binds this task

1. `pub fn classify(raw_json: &str) -> Classification` em `src/install/classify_2c.rs` (novo, puro; `mod` em `src/install/mod.rs`): devolve o conjunto de sub-casos presentes (raiz **ou** qualquer variante de `architecture`) e o mais arriscado (`worst`). Manifesto sem nenhum ⇒ `None` (não é 2c). JSON inválido ⇒ `Err`.
2. `pub fn script_commands(raw_json: &str) -> Vec<String>`: para todos os scripts do manifesto (hooks e `installer.script`/`uninstaller.script`, string ou array, raiz e variantes), os **nomes de comando** chamados: primeira palavra de cada comando (início de linha, depois de `|`, `;`, `{`, `(`, `&&`, `||`, `=`), ignorando comentários `#`, strings literais e here-strings (`@' '@`, `@" "@`). Léxico simples e documentado; não precisa ser perfeito (o oráculo mede, não decide segurança).
3. `examples/oracle_2c.rs` sobre o `catalog.db` real (só leitura, como o `oracle_2a`): imprime
   - por bucket (main, extras, demais somados): total de manifestos, quantos são 2c, contagem por `worst` e por presença de cada sub-caso;
   - quantos 2c **só** têm S-psm/S-inno/S-file (entregáveis sem PowerShell);
   - ranking dos comandos de script: nome, nº de manifestos que usam, e se é função definida no Scoop (lista fixa no exemplo, ver item 4) ou cmdlet/outra coisa;
   - cobertura acumulada: "implementando as N funções do Scoop mais usadas, X% dos manifestos S-hook/S-ps1 ficam só com funções suportadas" para N = 5, 10, 20, 30, todas.
4. A lista de funções do Scoop vem de uma tabela no próprio exemplo, extraída do fonte do Scoop (`lib/*.ps1`, `function <nome>`), com o commit citado em comentário. Pode ler o fonte em `~/scoop/apps/scoop/current/lib` se existir (só leitura) ou do GitHub (`ScoopInstaller/Scoop`, branch master); diga qual usou.
5. Saída do oráculo colada no relatório e salva em `docs/superpowers/reports/2026-10-18-oraculo-2c.md` (tabelas em Markdown).

**Files:**
- Create: `src/install/classify_2c.rs`, `examples/oracle_2c.rs`, `docs/superpowers/reports/2026-10-18-oraculo-2c.md`
- Modify: `src/install/mod.rs`
- Test: `tests/it/classify_2c.rs` (**já criado pelo orquestrador** com uma linha `//!`; não edite `tests/it/main.rs`)

Testes (`tests/it/classify_2c.rs`, JSON escrito no teste):
1. `none_for_simple_manifest`: manifesto só com `url`/`bin` ⇒ `None`.
2. `one_fixture_per_layer`: um manifesto por sub-caso ⇒ o sub-caso certo.
3. `variant_only_key_counts`: `architecture.64bit.post_install` sem raiz ⇒ S-hook.
4. `worst_wins`: innosetup + post_install ⇒ `worst = S-hook`, conjunto com os dois.
5. `installer_file_ps1_vs_exe`: `installer.file = "x.ps1"` ⇒ S-ps1; `"setup.exe"` com `args` ⇒ S-file; `installer.script` ⇒ S-hook.
6. `commands_skip_comments_strings_herestrings`: script com `# Add-Path x`, `'shim'`, here-string com `ensure` e chamadas reais `Add-Path $dir`, `Remove-Item x | Out-Null` ⇒ só `Add-Path`, `Remove-Item`, `Out-Null`.
7. `invalid_json_is_err`.

#### Checks beyond the steps

1. Mutação: ignorar as variantes de `architecture` ⇒ teste 3 **PEGA**.
2. Mutação: não pular here-strings ⇒ teste 6 **PEGA**.
3. Rode `cargo run --release --example oracle_2c -- <catalog.db real>` (caminho em `%LOCALAPPDATA%\wingetrs\catalog.db`) e cole a saída.
4. Gate: `pwsh -File scripts/verify.ps1`.

**Commit:** `feat(install): phase 2c oracle (sub-cases and Scoop functions used by manifest scripts)`
