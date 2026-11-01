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
$presentFn = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Select-CoolerPresentReconnectTarget' }, $true)
if (-not $presentFn) { throw 'present target selection function missing' }
. ([scriptblock]::Create($presentFn.Extent.Text))

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

function Refuses($name, $body) {
  $refused = $false
  try { & $body | Out-Null } catch { $refused = $true }
  if (-not $refused) { throw "$name failed: unsafe selection accepted" }
  "PASS $name"
}
$normalId = 'USB\VID_33C3&PID_7791\COOLER'
$otherNormalId = 'USB\VID_33C3&PID_7788\SECOND'
$normal = Node 'USB\ROOT_HUB30\HUB' 'Port_#0013.Hub_#0001' $normalId
Check 'explicit present reset selects exact cooler parent' (Select-CoolerPresentReconnectTarget @($normal)) $normalId
Check 'composite MI interface and unrelated nodes are excluded' (Select-CoolerPresentReconnectTarget @($normal, (Node 'hub' 'Port_#0002.Hub_#0001' 'USB\VID_33C3&PID_7791&MI_00\CHILD'), (Node 'hub' 'Port_#0003.Hub_#0001' 'USB\VID_9999&PID_7791\OTHER'))) $normalId
Refuses 'zero normal parents refuse explicit reset' { Select-CoolerPresentReconnectTarget @() }
Refuses 'only composite interface cannot be reset' { Select-CoolerPresentReconnectTarget @((Node 'hub' 'Port_#0002.Hub_#0001' 'USB\VID_33C3&PID_7791&MI_00\CHILD')) }
Refuses 'multiple normal parents refuse explicit reset' { Select-CoolerPresentReconnectTarget @($normal, (Node 'hubB' 'Port_#0004.Hub_#0002' $otherNormalId)) }
Refuses 'present missing parent refused' { Select-CoolerPresentReconnectTarget @((Node $null 'Port_#0013.Hub_#0001' $normalId)) }
Refuses 'present missing location refused' { Select-CoolerPresentReconnectTarget @((Node 'hub' $null $normalId)) }
Refuses 'present malformed location refused' { Select-CoolerPresentReconnectTarget @((Node 'hub' 'other Port_#0013' $normalId)) }
Refuses 'present zero port refused' { Select-CoolerPresentReconnectTarget @((Node 'hub' 'Port_#0000.Hub_#0001' $normalId)) }
Refuses 'present invalid port refused' { Select-CoolerPresentReconnectTarget @((Node 'hub' 'Port_#0256.Hub_#0001' $normalId)) }
Refuses 'present self-parent refused' { Select-CoolerPresentReconnectTarget @((Node $normalId 'Port_#0013.Hub_#0001' $normalId)) }

# Guard the effect boundary as well as the selector: no all-failed fallback and
# the sole-target check must precede the only hardware mutation call.
$source = [IO.File]::ReadAllText($file)
if ($source.Contains('$targets = $failed')) { throw 'unsafe all-failed fallback' }
$guard = $source.IndexOf('if ($targets.Count -ne 1)')
$cycle = $source.IndexOf('$cycleResult = [Hub]::Cycle(')
if ($guard -lt 0 -or $cycle -le $guard) { throw 'single-target guard missing before cycle' }
'PASS only one verified target can reach USB reset'
$defaultNoop = $source.IndexOf('if ($screen -and -not $ResetPresent)')
$explicit = $source.IndexOf('$targets = @(Select-CoolerPresentReconnectTarget $presentCandidates)')
$hubGuard = $source.IndexOf('if ($paths.Count -ne 1)')
$identityGuard = $source.IndexOf("throw 'present cooler identity or parent/location changed; no USB port reset'")
$dryGuard = $source.IndexOf('if ($DryRun) { Log "WOULD cycle')
if ($defaultNoop -lt 0 -or $explicit -le $defaultNoop -or $explicit -ge $cycle) { throw 'ResetPresent must be opt-in after default normal-device no-op' }
if ($hubGuard -lt 0 -or $hubGuard -ge $cycle -or $identityGuard -lt 0 -or $identityGuard -ge $cycle) { throw 'hub-interface or fresh present identity guard missing before cycle' }
if ($dryGuard -lt 0 -or $dryGuard -ge $cycle -or $source.Substring($dryGuard,$cycle-$dryGuard).IndexOf('continue') -lt 0) { throw 'DryRun must skip the mutation for eligible present targets' }
$cycleCalls = @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Member.Value -eq 'Cycle' }, $true))
if ($cycleCalls.Count -ne 1) { throw 'USB effect must have one guarded call site' }
$resetParam = @($ast.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'ResetPresent' })
if ($resetParam.Count -ne 1 -or $null -ne $resetParam[0].DefaultValue -or $resetParam[0].StaticType -ne [switch]) { throw 'ResetPresent must be a switch defaulting off' }
'PASS opt-in, single hub interface, fresh parent/location and DryRun guard the sole USB effect'
