#Requires -Version 7.0
<#
.SYNOPSIS
    Check that generated task briefs are actually self-contained, before you
    dispatch them.

.DESCRIPTION
    Brief extractors slice a plan from one task heading to the next. Anything
    written under the MILESTONE heading -- closed decisions, shared rules,
    sequencing -- reaches nobody.

    That has happened: a milestone was written with four closed decisions and
    six shared rules in its preamble, and the generated briefs came out at 22 to
    71 lines, one of them with zero of the four decisions and none of the six
    with execution rules or a report contract. It was caught by reading, one
    minute before dispatch. This script exists so that does not have to be luck.

    The most reliable symptom is SIZE: a brief much shorter than its siblings
    lost context the others have. So the size check is relative to the batch
    median, not a fixed floor.

    EXIT CODE:
      0 -> every brief has the required sections and none is an outlier
      1 -> findings; the list is in the output
      2 -> the script could not run

.EXAMPLE
    pwsh -File check_briefs.ps1 -Dir .superpowers/sdd/my-plan 63 68
    pwsh -File check_briefs.ps1 -Dir .claude/tasks -Sections 'Verifica','Rules','Report contract'
#>
[CmdletBinding()]
param(
    # First and last task number in the batch. Without them, checks every brief.
    [Parameter(Position = 0)] [int]$From = 0,
    [Parameter(Position = 1)] [int]$To = 0,

    # Directory holding the briefs. Auto-detected when omitted.
    [Parameter()] [string]$Dir,

    # Glob matching brief files. The (\d+) group must capture the task number.
    [Parameter()] [string]$Pattern = 'task-*-brief.md',
    [Parameter()] [string]$NumberRegex = 'task-(\d+)-brief\.md',

    # Regexes for the sections every brief must carry. Defaults match both
    # English and Portuguese headings.
    [Parameter()] [string[]]$Sections = @(
        '(?i)#+\s+(Verifica|Verification|Checks beyond)',
        '(?i)#+\s+(Regras de execu|Execution rules)',
        '(?i)#+\s+(Contrato de relat|Report contract)',
        '(?i)(mutate\.(ps1|py)|no mutation proof|n[aã]o tem prova de muta)',
        '(?m)^\*\*(Files|Arquivos):\*\*'
    ),

    # A brief below median * this ratio is flagged as an outlier.
    [Parameter()] [double]$FloorRatio = 0.5
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = (Get-Location).Path
if (-not $Dir) {
    foreach ($C in @('.superpowers/sdd', '.claude/tasks', 'docs/tasks', '.tasks')) {
        if (Test-Path (Join-Path $Root $C)) { $Dir = $C; break }
    }
}
if (-not $Dir -or -not (Test-Path (Join-Path $Root $Dir))) {
    Write-Output "[!] Brief directory not found. Pass -Dir <path>."
    exit 2
}
$BriefRoot = (Resolve-Path (Join-Path $Root $Dir)).Path

$Briefs = @(Get-ChildItem -Path $BriefRoot -Filter $Pattern -Recurse -File | ForEach-Object {
        if ($_.Name -match $NumberRegex) {
            $n = [int]$Matches[1]
            if ($From -eq 0 -or ($n -ge $From -and $n -le $To)) {
                [pscustomobject]@{ Num = $n; File = $_ }
            }
        }
    } | Sort-Object Num)

if ($Briefs.Count -eq 0) {
    Write-Output "[!] No brief in the requested range under $BriefRoot"
    exit 2
}

$Findings = 0
function Add-Finding {
    param([int]$Task, [string]$Rule, [string]$Text)
    $script:Findings++
    Write-Output ("  task-{0,-3} [{1}] {2}" -f $Task, $Rule, $Text)
}

$LineCount = @{}
foreach ($B in $Briefs) { $LineCount[$B.Num] = (Get-Content -Path $B.File.FullName -Encoding utf8).Count }

# Batch median. A brief well below it lost context its siblings have -- that was
# exactly the symptom the one time this shipped badly.
$Sorted = @($LineCount.Values | Sort-Object)
$Median = $Sorted[[int]($Sorted.Count / 2)]
$Floor = [int]($Median * $FloorRatio)

Write-Output "=== Briefs ($($Briefs.Count)); median $Median lines; relative floor $Floor ==="

foreach ($B in $Briefs) {
    $Body = Get-Content -Path $B.File.FullName -Encoding utf8 -Raw

    foreach ($S in $Sections) {
        if ($Body -notmatch $S) {
            Add-Finding -Task $B.Num -Rule 'MISSING-SECTION' -Text "nothing matching /$S/"
        }
    }

    if ($LineCount[$B.Num] -lt $Floor) {
        Add-Finding -Task $B.Num -Rule 'OUTLIER' -Text (
            "$($LineCount[$B.Num]) lines against median $Median -- probably lost what " +
            "stayed under the milestone heading")
    }

    # Draft markers that survived into the brief.
    #
    # Only TODO and FIXME. An earlier version also matched TBD, "fill in" and
    # "to be defined", and its first finding was a line DENYING that any
    # placeholder existed: 'no step contains "TBD"'. A checker that fires on the
    # sentence asserting the opposite of the defect becomes noise, and noise
    # trains people to ignore the tool.
    foreach ($m in [regex]::Matches($Body, '(?m)^(?!\s*(#|\||`)).*\b(TODO|FIXME)\b.*$')) {
        Add-Finding -Task $B.Num -Rule 'DRAFT-MARKER' -Text $m.Value.Trim()
    }

    # A path the brief cites as EXISTING that does not exist in the repository.
    #
    # The executor trusts the brief; a dead reference sends it hunting for a file
    # that was never there, or worse, makes it invent one. Only repo-relative
    # paths in backticks with a directory and an extension; anything with a
    # placeholder (<, *, {, $, %) or on a line that announces a NEW file is skipped
    # -- briefs legitimately name files the task will create.
    # Paths the brief itself declares as new (any line carrying a "new" marker)
    # are legitimate everywhere else in the same brief too.
    $declaredNew = @{}
    foreach ($line in ($Body -split "`n")) {
        # "\b(" before "\(new\)" never matched: no word boundary between a space and "(".
        if ($line -match '(?i)(\b(novo|nova|novos|novas|new file|criar|cria |gerad)|\(new\))') {
            foreach ($m in [regex]::Matches($line, '`([A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.\-]+)+\.[A-Za-z0-9]{1,5})`')) {
                $declaredNew[$m.Groups[1].Value] = $true
            }
        }
    }
    foreach ($line in ($Body -split "`n")) {
        if ($line -match '(?i)(\b(novo|nova|novos|novas|new file|criar|cria |gerad)|\(new\))') { continue }
        foreach ($m in [regex]::Matches($line, '`([A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.\-]+)+\.[A-Za-z0-9]{1,5})`')) {
            $p = $m.Groups[1].Value
            if ($p -match '[<>*{}$%]' -or $p.StartsWith('.claude/tasks/') -or $declaredNew.ContainsKey($p)) { continue }
            # Only paths under a top-level directory the repository has: a vault path
            # (.obsidian/...) or an elided example (.../x) is not a repository claim.
            $topo = $p.Split('/')[0]
            if ($topo -match '^\.+$' -or -not (Test-Path -LiteralPath (Join-Path $Root $topo) -PathType Container)) { continue }
            if (-not (Test-Path -LiteralPath (Join-Path $Root $p))) {
                Add-Finding -Task $B.Num -Rule 'DEAD-REF' -Text "``$p`` is cited and does not exist"
            }
        }
    }

    # A rule the brief cites BY NAME from a project document was tried as a second
    # check (CITED-RULE, 2026-10-02) and REJECTED by measurement: on briefs 85-109 it
    # fired three times, all on legitimate paraphrases ("Sem posse, 404 -- nunca 403"
    # against CLAUDE.md's wording), and the case that motivated it -- Task 96 citing a
    # rule ARMADILHAS.md did not have -- is exactly a paraphrase it would accept. A
    # checker that fires on legitimate prose is noise. The rule lives in the
    # authoring skill instead: grep every cited rule before dispatching.
}

Write-Output ""
if ($Findings -eq 0) {
    Write-Output "[OK] The $($Briefs.Count) briefs carry the required sections and none is an outlier."
    Write-Output "[i] This does NOT mean the content is right -- it means nothing stayed stuck"
    Write-Output "    under the milestone heading. Read one brief end to end before dispatching."
    exit 0
}

Write-Output "[!] $Findings finding(s)."
Write-Output "[i] MISSING-SECTION and OUTLIER almost always mean the same thing: the content"
Write-Output "    is in the milestone preamble, and the extractor only sees one task heading"
Write-Output "    to the next. Repeat what binds the task INSIDE its own section."
exit 1
