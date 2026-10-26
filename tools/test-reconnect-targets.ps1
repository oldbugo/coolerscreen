# Pure selection tests. Extract only the function: never load the script's PnP/USB operations.
$ErrorActionPreference = 'Stop'
$file = Join-Path (Split-Path -Parent $PSScriptRoot) 'scripts/reconnect.ps1'
$parseErrors = $null
$tokens = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($file, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
$fn = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Select-CoolerReconnectTargets' }, $true)
if (-not $fn) { throw 'target selection function missing' }
. ([scriptblock]::Create($fn.Extent.Text))

function Node($parent, $location, $name) { [pscustomobject]@{ Parent = $parent; Location = $location; Node = [pscustomobject]@{ InstanceId = $name } } }
function Check($name, $actual, $expected) {
  $names = @($actual | ForEach-Object { $_.Node.InstanceId }) -join ','
  if ($names -ne $expected) { throw "$name failed: expected '$expected', got '$names'" }
  "PASS $name"
}
$history = @((Node 'hubA' 'port13' 'oldA'), (Node 'hubB' 'port4' 'oldB'))
Check 'exact remembered pair excludes unrelated failure' (Select-CoolerReconnectTargets @((Node 'hubA' 'port13' 'cooler'), (Node 'hubA' 'port2' 'other')) $history) 'cooler'
Check 'crossed history pairs do not match' (Select-CoolerReconnectTargets @((Node 'hubA' 'port4' 'other')) $history) ''
Check 'no history does not reset arbitrary failed ports' (Select-CoolerReconnectTargets @((Node 'hubA' 'port13' 'unknown')) @()) ''
Check 'missing parent is not a match' (Select-CoolerReconnectTargets @((Node $null 'port13' 'unknown')) $history) ''
Check 'missing location is not a match' (Select-CoolerReconnectTargets @((Node 'hubA' $null 'unknown')) $history) ''
Check 'multiple matches stay explicit for caller to reject' (Select-CoolerReconnectTargets @((Node 'hubA' 'port13' 'one'), (Node 'hubB' 'port4' 'two')) $history) 'one,two'
Check 'no failed ports' (Select-CoolerReconnectTargets @() $history) ''

# Guard the effect boundary as well as the selector: no all-failed fallback and
# the sole-target check must precede the only hardware mutation call.
$source = [IO.File]::ReadAllText($file)
if ($source.Contains('$targets = $failed')) { throw 'unsafe all-failed fallback' }
$guard = $source.IndexOf('if ($targets.Count -ne 1)')
$cycle = $source.IndexOf('$cycleResult = [Hub]::Cycle(')
if ($guard -lt 0 -or $cycle -le $guard) { throw 'single-target guard missing before cycle' }
'PASS only one verified target can reach USB reset'
