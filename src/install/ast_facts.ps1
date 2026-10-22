$PSModuleAutoLoadingPreference = 'None'
$ProgressPreference = 'SilentlyContinue'
$ErrorActionPreference = 'Stop'
# Task 227: facts of PowerShell's own parser about a script, for the wingetrs gate. Reads one
# request per line on stdin (`<id><TAB><base64 of the UTF-8 script>`), parses it (never runs
# it) and writes one record per line, then `END<TAB><id>`. Only the language and .NET types:
# no command of any module, nothing written, no network. The wire format is in script_ast.rs.
$utf8 = [System.Text.UTF8Encoding]::new($false)
$reader = [System.IO.StreamReader]::new([Console]::OpenStandardInput(), $utf8)
$writer = [System.IO.StreamWriter]::new([Console]::OpenStandardOutput(), $utf8)
$globalIf = '^\s*\$global(\s+-and\s+\$\w+)?\s*$'

function Esc([string]$s) {
    if ($null -eq $s) { return '' }
    return $s.Replace('%', '%25').Replace("`t", '%09').Replace("`r", '%0D').Replace("`n", '%0A')
}

# The type name without the `ExpressionAst`/`Ast` suffix.
function Short([string]$k) {
    if ($k.EndsWith('ExpressionAst')) { return $k.Substring(0, $k.Length - 13) }
    if ($k.EndsWith('Ast')) { return $k.Substring(0, $k.Length - 3) }
    return $k
}

# `s:`/`e:` for strings; otherwise `?:` and the type (or the raw text when $raw).
function ArgOf($a, [bool]$raw) {
    $k = $a.GetType().Name
    if ($k -eq 'StringConstantExpressionAst') { return 's:' + $a.Value }
    if ($k -eq 'ExpandableStringExpressionAst') { return 'e:' + $a.Value }
    if ($raw) { return '?:' + $a.Extent.Text }
    return '?:' + (Short $k)
}

# One letter per ancestor, from the root: B (script block expression), F (function), G (first
# clause body of `if ($global)` or `if ($global -and $x)`); `.` when none.
function Chain($n) {
    $s = ''
    $prev = $n
    $p = $n.Parent
    while ($null -ne $p) {
        $k = $p.GetType().Name
        if ($k -eq 'ScriptBlockExpressionAst') { $s = 'B' + $s }
        elseif ($k -eq 'FunctionDefinitionAst') { $s = 'F' + $s }
        elseif ($k -eq 'IfStatementAst' -and $p.Clauses.Count -gt 0 -and
            [object]::ReferenceEquals($p.Clauses[0].Item2, $prev) -and
            $p.Clauses[0].Item1.Extent.Text -match $globalIf) { $s = 'G' + $s }
        $prev = $p
        $p = $p.Parent
    }
    if ($s -eq '') { return '.' }
    return $s
}

function Line($sb, [object[]]$fields) {
    $parts = [System.Collections.Generic.List[string]]::new()
    foreach ($f in $fields) { $parts.Add((Esc ([string]$f))) }
    [void]$sb.Append(($parts -join "`t")).Append("`n")
}

function Facts([string]$text, $sb) {
    $tokens = $null
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$errors)
    foreach ($e in $errors) { Line $sb @('ERR', $e.Extent.StartLineNumber, $e.ErrorId) }
    foreach ($n in $ast.FindAll({ $true }, $true)) {
        $k = $n.GetType().Name
        $at = $n.Extent.StartLineNumber
        if ($k -eq 'UsingStatementAst') {
            Line $sb @('USE', $at, $n.UsingStatementKind.ToString().ToLowerInvariant())
        } elseif ($k -eq 'CommandAst') {
            $op = switch ($n.InvocationOperator.ToString()) { 'Ampersand' { 'amp' } 'Dot' { 'dot' } default { 'none' } }
            $els = $n.CommandElements
            $f = [System.Collections.Generic.List[object]]::new()
            $f.AddRange([object[]]@('CMD', $at, $op, (Short $els[0].GetType().Name), (Chain $n), $n.GetCommandName()))
            for ($i = 1; $i -lt $els.Count; $i++) {
                $el = $els[$i]
                if ($el.GetType().Name -eq 'CommandParameterAst') {
                    $f.Add('p:' + $el.ParameterName)
                    if ($null -ne $el.Argument) { $f.Add((ArgOf $el.Argument $false)) }
                } else {
                    $f.Add((ArgOf $el $false))
                }
            }
            Line $sb $f.ToArray()
        } elseif ($n -is [System.Management.Automation.Language.MemberExpressionAst]) {
            $target = if ($n.Expression.GetType().Name -eq 'TypeExpressionAst') {
                '[' + $n.Expression.TypeName.FullName + ']'
            } else { $n.Expression.Extent.Text }
            $member = if ($n.Member.GetType().Name -eq 'StringConstantExpressionAst') { $n.Member.Value } else { $n.Member.Extent.Text }
            $f = [System.Collections.Generic.List[object]]::new()
            $f.AddRange([object[]]@('MEM', $at, [int][bool]$n.Static, (Chain $n), $target, $member))
            if ($null -ne $n.Arguments) {
                foreach ($a in $n.Arguments) { $f.Add((ArgOf $a $true)) }
            }
            Line $sb $f.ToArray()
        } elseif ($k -eq 'AssignmentStatementAst' -and $n.Left.GetType().Name -eq 'VariableExpressionAst') {
            Line $sb @('SET', $at, (Chain $n), $n.Left.VariablePath.UserPath)
        } elseif ($k -eq 'FunctionDefinitionAst') {
            Line $sb @('FN', $at, (Chain $n), $n.Name)
        }
    }
}

while ($true) {
    $request = $reader.ReadLine()
    if ($null -eq $request) { break }
    $tab = $request.IndexOf("`t")
    if ($tab -lt 1) { break }
    $id = $request.Substring(0, $tab)
    $sb = [System.Text.StringBuilder]::new()
    # Any failure ends the process without `END`: the caller refuses the script.
    try {
        Facts $utf8.GetString([Convert]::FromBase64String($request.Substring($tab + 1))) $sb
    } catch {
        break
    }
    [void]$sb.Append('END').Append("`t").Append((Esc $id)).Append("`n")
    $writer.Write($sb.ToString())
    $writer.Flush()
}
