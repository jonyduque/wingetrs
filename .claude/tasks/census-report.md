# Bridge census (probe, examples/bridge_census.rs)

Full run over 15791 winget packages (latest version, host x64, Scope user preferred, default InstallerMode). Runtime 431.9 s (concurrency 16, manifests only). Raw: scratchpad `census.tsv`, `census-summary.txt`.

## Outcomes
| outcome | n | % |
|---|---|---|
| native | 9521 | 60.3 |
| portable | 3219 | 20.4 |
| bridge | 2998 | 19.0 |
| refused | 51 | 0.3 |
| error | 2 | 0.0 |

## Bridge + refused reasons (prefix before ':')
| n | reason |
|---|---|
| 1656 | bridge: tem dependências |
| 782 | bridge: parâmetros do instalador fora da lista permitida para elevação |
| 554 | bridge: tipo zip |
| 6 | bridge: tipo portable com escopo máquina |
| 21 | refused: URL do instalador recusada |
| 18 | refused: nenhum instalador para esta arquitetura (x64) |
| 3 | refused: arquitetura x64 declarada não suportada |
| 10 | refused: zip portable com N arquivos aninhados (máximo 64) (N = 76..270) |

## "tem dependências" (1656)
- ONLY package deps: 1608 (97%). With package deps: 1613; with windows features: 45; libraries: 0; external: 3.
- Installer kind of these rows: zip 464, inno 270, wix 201, nullsoft 189, portable 150, msi 144, exe 130, msix 76.

## "parâmetros fora da lista" (782) by kind
nullsoft 479, exe 193, burn 37, inno 34, wix 31, msi 8.

## Zip bridge rows by NestedInstallerType
Caution: this histogram counts all bridge rows with kind=zip, i.e. "tipo zip" (554) AND zip rows that bridged for "tem dependências" (464), total 1018 (the row sum). By nested: portable 468, inno 168, exe 113, msi 95, nullsoft 92, wix 59, burn 17, appx 5, msix 5. (To isolate the 554 "tipo zip" rows, filter census.tsv column 3.)

## Errors: 2
2x `HTTP 404` (Coninfer.ECLiPSeCLP.7.0, Coninfer.ECLiPSeCLP.7.1). Only one distinct message.

## Notes
- kind/nested/deps for bridge and refused rows come from `select::choose` run separately in the example (no `src/` change); for native rows from `plan.installer`.
- Packages with no `winget_hash`: 0 (would appear as error rows).
- No `pub` added to `src/`.
