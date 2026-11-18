# Windows hardware suite. A normal run is bench. --school is the school profile.
# Does not show a UAC prompt and does not install a SYSTEM service.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ci = $false
$admin = $false
$school = $false
foreach ($arg in $args) {
    switch ($arg) {
        '--ci' { $ci = $true }
        '-Ci' { $ci = $true }
        '--admin' { $admin = $true }
        '-Admin' { $admin = $true }
        '--school' { $school = $true }
        '-School' { $school = $true }
        default {
            Write-Error "Unknown argument $arg"
            exit 2
        }
    }
}
if ($ci -and $env:GITHUB_ACTIONS -ne 'true') {
    Write-Host '--ci applies only when GITHUB_ACTIONS=true; ignoring --ci'
    $ci = $false
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
Set-Location -LiteralPath $root
$started = [DateTime]::UtcNow
$budgetMs = 300000
$script:Armed = $false
$script:SawLock = $false
$script:LeftLocked = $false
$script:Failed = $false
$script:LockFired = $false
$script:Locking = $false
$script:LockError = ''
$script:Interrupted = $false
$script:Changes = New-Object System.Collections.Generic.List[object]
$script:Staged = New-Object System.Collections.Generic.List[object]
$script:Power = New-Object System.Collections.Generic.List[object]
$script:Restored = $false
$script:HklmCommand = ''
$script:ExitCode = 1
$script:ReportWritten = $false
$plan = $null
$facts = ''
$exe = $null
$factsPath = $null
$planPath = $null
$script:RestoreErrors = @()
$script:SuiteError = ''
$script:ActuallyLocked = $false
$pendingDir = Join-Path $env:LOCALAPPDATA 'softlock-hwsuite'
$pendingPath = Join-Path $pendingDir 'pending.json'
$restoreScript = Join-Path $PSScriptRoot 'restore.ps1'
$native = $null
$results = New-Object System.Collections.Generic.List[object]

$script:Utf8 = New-Object System.Text.UTF8Encoding $false
function Write-Utf8([string]$Path, [string]$Text) {
    [System.IO.File]::WriteAllText($Path, $Text, $script:Utf8)
}

function Elapsed-Ms {
    return [int]([DateTime]::UtcNow - $started).TotalMilliseconds
}

function Sync-Interrupt {
    if ($null -ne $native -and $native::WasInterrupted()) { $script:Interrupted = $true }
}

function Save-Pending {
    if (-not (Test-Path -LiteralPath $pendingDir)) {
        New-Item -ItemType Directory -Path $pendingDir | Out-Null
    }
    $parts = @()
    foreach ($entry in $script:Changes) {
        $present = if ($entry.present) { 'true' } else { 'false' }
        $kind = if ($entry.PSObject.Properties.Name -contains 'kind') { [string]$entry.kind } else { 'DWord' }
        $parts += '{"hive":"' + $entry.hive + '","subkey":"' + (Escape-JsonString ([string]$entry.subkey)) + '","name":"' + (Escape-JsonString ([string]$entry.name)) + '","present":' + $present + ',"kind":"' + (Escape-JsonString $kind) + '","data":"' + (Escape-JsonString ([string]$entry.data)) + '"}'
    }
    $power = @()
    foreach ($entry in $script:Power) {
        $power += '{"setting":"' + (Escape-JsonString ([string]$entry.setting)) + '","ac":' + [int]$entry.ac + ',"dc":' + [int]$entry.dc + '}'
    }
    $json = '{"values":[' + ($parts -join ',') + '],"power":[' + ($power -join ',') + ']}'
    Write-Utf8 $pendingPath $json
    $quoted = '"' + $restoreScript.Replace('"', '') + '"'
    $command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File $quoted"
    $runOnce = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce')
    $runOnce.SetValue('softlock-hwsuite', $command)
    $runOnce.Close()
}

function Open-Hive([string]$hive, [string]$subkey, [bool]$writable) {
    $rootKey = if ($hive -eq 'HKLM') { [Microsoft.Win32.Registry]::LocalMachine } else { [Microsoft.Win32.Registry]::CurrentUser }
    if ($writable) {
        return $rootKey.CreateSubKey($subkey)
    }
    return $rootKey.OpenSubKey($subkey, $false)
}

function Remember-Dword([string]$hive, [string]$subkey, [string]$name) {
    foreach ($entry in $script:Changes) {
        if ($entry.hive -eq $hive -and $entry.subkey -eq $subkey -and $entry.name -eq $name) {
            return
        }
    }
    foreach ($entry in $script:Staged) {
        if ($entry.hive -eq $hive -and $entry.subkey -eq $subkey -and $entry.name -eq $name) {
            return
        }
    }
    $present = $false
    $kind = 'DWord'
    $data = ''
    $key = Open-Hive $hive $subkey $false
    if ($null -ne $key) {
        foreach ($existing in @($key.GetValueNames())) {
            if ($existing -eq $name) {
                $present = $true
                $kind = $key.GetValueKind($name).ToString()
                $raw = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                if ($kind -eq 'Binary') {
                    $data = [Convert]::ToBase64String([byte[]]$raw)
                } elseif ($kind -eq 'MultiString') {
                    $data = ([string[]]$raw) -join "`n"
                } else {
                    $data = [string]$raw
                }
            }
        }
        $key.Close()
    }
    $script:Staged.Add([pscustomobject]@{
        hive = $hive
        subkey = $subkey
        name = $name
        present = $present
        kind = $kind
        data = $data
    })
}

function Commit-Staged {
    if ($script:Staged.Count -eq 0) { return }
    foreach ($entry in $script:Staged.ToArray()) {
        $script:Changes.Add($entry)
    }
    $script:Staged.Clear()
    Save-Pending
}

function Discard-Staged {
    $script:Staged.Clear()
}

function Write-SuiteDword([string]$hive, [string]$subkey, [string]$name, [int]$value) {
    Remember-Dword $hive $subkey $name
    try {
        $read = Set-SuiteDword $hive $subkey $name $value
        Commit-Staged
        return $read
    } catch {
        Discard-Staged
        throw
    }
}

function Test-AccessDenied($record) {
    $text = ''
    if ($null -ne $record -and $null -ne $record.Exception) { $text = [string]$record.Exception.Message }
    if ($text -match 'administrator' -or $text -match 'Access is denied' -or $text -match 'not allowed' -or $text -match 'privilege') {
        return $true
    }
    $ex = $null
    if ($null -ne $record) { $ex = $record.Exception }
    while ($null -ne $ex) {
        if ($ex -is [System.UnauthorizedAccessException] -or $ex -is [System.Security.SecurityException]) { return $true }
        $ex = $ex.InnerException
    }
    return $false
}

function Set-SuiteDword([string]$hive, [string]$subkey, [string]$name, [int]$value) {
    $key = Open-Hive $hive $subkey $true
    $key.SetValue($name, $value, [Microsoft.Win32.RegistryValueKind]::DWord)
    $read = [int]$key.GetValue($name)
    $key.Close()
    return $read
}

function Test-SuiteElevated {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Convert-RegistryData([string]$kind, [string]$data) {
    switch ($kind) {
        'DWord' { return [int]$data }
        'QWord' { return [int64]$data }
        'Binary' {
            if ([string]::IsNullOrEmpty($data)) { return [byte[]]@() }
            return [Convert]::FromBase64String($data)
        }
        'MultiString' {
            if ([string]::IsNullOrEmpty($data)) { return [string[]]@() }
            return [string[]]$data.Split("`n")
        }
        default { return $data }
    }
}

function Set-PowerIndex([string]$setting, [int]$ac, [int]$dc) {
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    $acText = Read-Native { powercfg /SETACVALUEINDEX SCHEME_CURRENT $sub $setting $ac }
    if ($LASTEXITCODE -ne 0) { return "powercfg AC $setting failed: $acText" }
    $dcText = Read-Native { powercfg /SETDCVALUEINDEX SCHEME_CURRENT $sub $setting $dc }
    if ($LASTEXITCODE -ne 0) { return "powercfg DC $setting failed: $dcText" }
    $active = Read-Native { powercfg /SETACTIVE SCHEME_CURRENT }
    if ($LASTEXITCODE -ne 0) { return "powercfg SETACTIVE failed: $active" }
    return ''
}

function Read-PowerIndex([string]$setting) {
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    $text = Read-Native { powercfg /q SCHEME_CURRENT $sub $setting }
    $ac = [regex]::Match($text, 'Current AC Power Setting Index:\s+0x([0-9A-Fa-f]+)')
    $dc = [regex]::Match($text, 'Current DC Power Setting Index:\s+0x([0-9A-Fa-f]+)')
    if (-not $ac.Success -or -not $dc.Success) { return $null }
    return [pscustomobject]@{
        setting = $setting
        ac = [Convert]::ToInt32($ac.Groups[1].Value, 16)
        dc = [Convert]::ToInt32($dc.Groups[1].Value, 16)
    }
}

function Ensure-DoNothing([string]$setting) {
    # powercfg /SETACVALUEINDEX writes the active scheme. Re-read the index:
    # an unelevated process often gets a privilege error, or exit 0 with the index unchanged.
    $before = Read-PowerIndex $setting
    if ($null -eq $before) {
        if (-not (Test-SuiteElevated)) { return 'needs-admin' }
        return 'powercfg /q did not return the current index'
    }
    if ($before.ac -eq 0 -and $before.dc -eq 0) { return '' }
    $err = Set-PowerIndex $setting 0 0
    $after = Read-PowerIndex $setting
    if ($null -ne $after -and $after.ac -eq 0 -and $after.dc -eq 0) {
        $known = $false
        foreach ($entry in $script:Power.ToArray()) {
            if ([string]$entry.setting -eq $setting) { $known = $true }
        }
        if (-not $known) {
            $script:Power.Add($before)
            Save-Pending
        }
        return ''
    }
    if (-not (Test-SuiteElevated) -or $err -match 'administrator' -or $err -match 'privilege') {
        return 'needs-admin'
    }
    if ($err) { return $err }
    return 'powercfg index was not 0 after set'
}

function Complete-Interrupted {
    if (-not $script:Interrupted) { return }
    if ($null -eq $plan) {
        $have = $false
        foreach ($got in $results) {
            if ($got.id -eq 'parser-self-test') { $have = $true }
        }
        if (-not $have) { Add-Result 'parser-self-test' 'skip' 'interrupted' @{} }
        return
    }
    foreach ($row in @($plan.rows)) {
        if ($row.id -eq 'restore') { continue }
        if ($row.action -eq 'record' -or $row.action -eq 'skip') { continue }
        $have = $false
        foreach ($got in $results) {
            if ($got.id -eq $row.id) { $have = $true }
        }
        if (-not $have) { Add-Result $row.id 'skip' 'interrupted' @{} }
    }
}

function Write-FallbackReport([string]$outDir, [bool]$restored) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $pass = 0
    $fail = 0
    $skip = 0
    $tests = @()
    foreach ($row in $results) {
        if ($row.status -eq 'pass') { $pass++ }
        elseif ($row.status -eq 'fail') { $fail++ }
        else { $skip++ }
        $reason = 'null'
        if ($row.status -eq 'skip') { $reason = '"' + (Escape-JsonString ([string]$row.reason)) + '"' }
        $detail = ''
        if ($row.status -ne 'skip') { $detail = Escape-JsonString ([string]$row.reason) }
        $tests += '{"id":"' + $row.id + '","name":"' + $row.id + '","status":"' + $row.status + '","skip_reason":' + $reason + ',"physical":false,"needs_admin":false,"duration_ms":0,"detail":"' + $detail + '","measurements":{}}'
    }
    if ($tests.Count -eq 0) {
        $skip = 1
        $tests += '{"id":"parser-self-test","name":"Parser self-test","status":"skip","skip_reason":"interrupted","physical":false,"needs_admin":false,"duration_ms":0,"detail":"interrupted","measurements":{}}'
    }
    $state = if ($script:ActuallyLocked) { 'os-locked' } else { 'never-unlocked' }
    $locked = if ($script:ActuallyLocked) { 'true' } else { 'false' }
    $interrupted = if ($script:Interrupted) { 'true' } else { 'false' }
    $restoredBit = if ($restored) { 'true' } else { 'false' }
    $mode = if ($ci) { 'ci' } elseif ($admin) { 'admin' } elseif ($school) { 'school' } else { 'bench' }
    $startedUtc = $started.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss') + 'Z'
    $finishedUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss') + 'Z'
    $json = '{"schema":"softlock-hardware-suite/1","suite":"windows","started_utc":"' + $startedUtc + '","finished_utc":"' + $finishedUtc + '","duration_ms":' + (Elapsed-Ms) + ',"mode":"' + $mode + '","config":{},"tests":[' + ($tests -join ',') + '],"summary":{"pass":' + $pass + ',"fail":' + $fail + ',"skip":' + $skip + '},"fail_closed":' + $locked + ',"serial_collected":false,"restored":' + $restoredBit + ',"interrupted":' + $interrupted + ',"state":"' + $state + '"}'
    Write-Utf8 (Join-Path $outDir 'result.json') $json
    $text = "Windows hardware suite  schema=softlock-hardware-suite/1  suite=windows`npass=$pass fail=$fail skip=$skip duration=$(([int]((Elapsed-Ms) / 1000)))s state=$state serial-collected=false restored=$restoredBit interrupted=$interrupted`n"
    if ($script:HklmCommand) {
        $text += "`nHKLM was not restored. From an elevated prompt run: $($script:HklmCommand)`n"
    }
    Write-Utf8 (Join-Path $outDir 'result.txt') $text
    Write-Host $text
}

function Write-SuiteReport([bool]$restored) {
    if ($script:ReportWritten) { return }
    $script:ReportWritten = $true
    $stamp = $started.ToString('yyyyMMdd-HHmmss')
    $outDir = Join-Path $root "suite-output\$stamp"
    $canReport = $exe -and $facts -and (Test-Path -LiteralPath $exe)
    if (-not $canReport) {
        Write-FallbackReport $outDir $restored
        if ($script:Interrupted) { $script:ExitCode = 130 } else { $script:ExitCode = 1 }
        return
    }
    $bundlePath = Join-Path $env:TEMP ("softlock-hw-bundle-{0}.json" -f $PID)
    $lockedBit = if ($script:ActuallyLocked) { 'true' } else { 'false' }
    $interruptedBit = if ($script:Interrupted) { 'true' } else { 'false' }
    $restoredBit = if ($restored) { 'true' } else { 'false' }
    $startedUtc = $started.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss') + 'Z'
    $finishedUtc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss') + 'Z'
    $bundle = '{"facts":' + $facts + ',"elapsed_ms":' + (Elapsed-Ms) + ',"rows":' + (ConvertTo-ResultJson) + ',"started_utc":"' + $startedUtc + '","finished_utc":"' + $finishedUtc + '","interrupted":' + $interruptedBit + ',"restored":' + $restoredBit + ',"locked":' + $lockedBit + ',"fail_closed":' + $lockedBit + ',"hklm_command":"' + (Escape-JsonString $script:HklmCommand) + '"}'
    Write-Utf8 $bundlePath $bundle
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    & $exe report --bundle $bundlePath --out-dir $outDir
    $script:ExitCode = $LASTEXITCODE
    if ($script:HklmCommand) { Write-Host "HKLM was not restored. From an elevated prompt run: $($script:HklmCommand)" }
    foreach ($path in @($factsPath, $planPath, $bundlePath)) {
        if ($path -and (Test-Path -LiteralPath $path)) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
}

function Read-Answer([string]$prompt, [int]$seconds) {
    Write-Host $prompt
    $redirected = $false
    try { $redirected = [Console]::IsInputRedirected } catch { $redirected = $true }
    if ($redirected) { return 'eof' }
    $previous = $false
    try {
        $previous = [Console]::TreatControlCAsInput
        [Console]::TreatControlCAsInput = $true
    } catch {
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    $buf = New-Object System.Text.StringBuilder
    $kind = 'timeout'
    while ([DateTime]::UtcNow -lt $deadline) {
        Sync-Interrupt
        if ($script:Interrupted) { $kind = 'interrupt'; break }
        if ((Elapsed-Ms) -gt $budgetMs) { $kind = 'budget'; break }
        try {
            if (-not [Console]::KeyAvailable) {
                Start-Sleep -Milliseconds 50
                continue
            }
            $key = [Console]::ReadKey($true)
        } catch {
            $kind = 'eof'
            break
        }
        $controlC = ($key.KeyChar -eq 3) -or ($key.Key -eq 'C' -and ($key.Modifiers -band [ConsoleModifiers]::Control))
        if ($controlC) {
            $script:Interrupted = $true
            $kind = 'interrupt'
            break
        }
        if ($key.Key -eq 'Enter') {
            $text = $buf.ToString().Trim().ToLowerInvariant()
            if ($text -eq '' -or $text -eq 'y' -or $text -eq 'yes' -or $text -eq 'ok' -or $text -eq 'go') {
                $kind = 'go'
            } elseif ($text -eq 'skip' -or $text -eq 's' -or $text -eq 'n' -or $text -eq 'no') {
                $kind = 'operator'
            } else {
                Write-Host 'Press Enter, or type skip.'
                $buf.Length = 0
                continue
            }
            break
        }
        if ($key.Key -eq 'Backspace') {
            if ($buf.Length -gt 0) { $buf.Length = $buf.Length - 1 }
            continue
        }
        if ($key.KeyChar -ne 0) { [void]$buf.Append($key.KeyChar) }
    }
    try { [Console]::TreatControlCAsInput = $previous } catch { }
    return $kind
}

function Test-DisplayStep([string]$prompt, $before, $after) {
    $step = if ($prompt.StartsWith('Unplug')) { 'disconnect' } else { 'connect' }
    $path = Join-Path $env:TEMP ("softlock-hw-hotplug-{0}.json" -f $PID)
    $json = '{"step":"' + $step + '","before":' + (ConvertTo-HotplugJson $before) + ',"after":' + (ConvertTo-HotplugJson $after) + '}'
    Write-Utf8 $path $json
    $raw = & $exe hotplug --json $path
    $code = $LASTEXITCODE
    Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    if ($code -ne 0 -or -not $raw) { return $false }
    $parsed = $raw | ConvertFrom-Json
    return [bool]$parsed.seen
}

function ConvertTo-HotplugJson($sample) {
    return '{"active":' + [int]$sample.active + ',"monitors":' + [int]$sample.monitors + ',"adds":' + [int]$sample.adds + ',"removes":' + [int]$sample.removes + ',"changes":' + [int]$sample.changes + '}'
}

function Power-Since($afterRecord) {
    if ($null -eq $afterRecord) {
        return [pscustomobject]@{ known = $false; sleep = 0; press = 0 }
    }
    $sleep = 0
    $press = 0
    try {
        $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-Power' } -MaxEvents 40 -ErrorAction Stop)
        foreach ($event in $events) {
            if ([int64]$event.RecordId -le [int64]$afterRecord) { continue }
            if ($event.Id -eq 41 -or $event.Id -eq 42 -or $event.Id -eq 109 -or $event.Id -eq 506) { $sleep++ }
        }
    } catch {
        return [pscustomobject]@{ known = $false; sleep = 0; press = 0 }
    }
    return [pscustomobject]@{ known = $true; sleep = $sleep; press = $press }
}

function Latest-PowerRecord {
    try {
        $event = Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-Power' } -MaxEvents 1 -ErrorAction Stop
        return [int64]$event.RecordId
    } catch {
        return $null
    }
}

function Escape-JsonString([string]$text) {
    if ($null -eq $text) { return '' }
    $builder = New-Object System.Text.StringBuilder ($text.Length + 8)
    foreach ($ch in $text.ToCharArray()) {
        $code = [int]$ch
        if ($code -eq 8) { [void]$builder.Append('\b') }
        elseif ($code -eq 9) { [void]$builder.Append('\t') }
        elseif ($code -eq 10) { [void]$builder.Append('\n') }
        elseif ($code -eq 12) { [void]$builder.Append('\f') }
        elseif ($code -eq 13) { [void]$builder.Append('\r') }
        elseif ($code -eq 34) { [void]$builder.Append('\"') }
        elseif ($code -eq 92) { [void]$builder.Append('\\') }
        elseif ($code -lt 32) { [void]$builder.Append(('\u{0:x4}' -f $code)) }
        else { [void]$builder.Append($ch) }
    }
    return $builder.ToString()
}

function Read-Native([scriptblock]$command) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = @(& $command 2>&1 | ForEach-Object { "$_" })
        return ($lines -join "`n")
    } finally {
        $ErrorActionPreference = $previous
    }
}

function Probe-Error($probe) {
    if ($null -eq $probe) { return '' }
    if ($probe.PSObject.Properties.Name -contains 'error') { return [string]$probe.error }
    return ''
}

function Snapshot-Data($key, [string]$name) {
    $kind = $key.GetValueKind($name).ToString()
    $raw = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $data = ''
    if ($kind -eq 'Binary') {
        $data = [Convert]::ToBase64String([byte[]]$raw)
    } elseif ($kind -eq 'MultiString') {
        $data = ([string[]]$raw) -join "`n"
    } else {
        $data = [string]$raw
    }
    return [pscustomobject]@{ kind = $kind; data = $data }
}

function Value-Matches($entry) {
    $key = Open-Hive $entry.hive $entry.subkey $false
    if ($null -eq $key) { return -not $entry.present }
    $found = $null
    foreach ($existing in @($key.GetValueNames())) {
        if ($existing -eq [string]$entry.name) { $found = Snapshot-Data $key $existing }
    }
    $key.Close()
    if (-not ($entry.present -eq $true)) { return $null -eq $found }
    if ($null -eq $found) { return $false }
    $kindName = 'DWord'
    if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
    return ($found.kind -eq $kindName) -and ($found.data -eq [string]$entry.data)
}

function Restore-Command($entry) {
    $path = "$($entry.hive)\$($entry.subkey)"
    $name = [string]$entry.name
    if (-not ($entry.present -eq $true)) {
        return "reg delete `"$path`" /v $name /f"
    }
    $kindName = 'DWord'
    if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
    $type = switch ($kindName) {
        'QWord' { 'REG_QWORD' }
        'String' { 'REG_SZ' }
        'ExpandString' { 'REG_EXPAND_SZ' }
        'MultiString' { 'REG_MULTI_SZ' }
        'Binary' { 'REG_BINARY' }
        default { 'REG_DWORD' }
    }
    return "reg add `"$path`" /v $name /t $type /d $($entry.data) /f"
}

function Restore-Changes {
    $errors = New-Object System.Collections.Generic.List[string]
    $elevated = Test-SuiteElevated
    $remaining = New-Object System.Collections.Generic.List[object]
    foreach ($entry in $script:Changes.ToArray()) {
        if ($entry.hive -eq 'HKLM' -and -not $elevated) {
            $remaining.Add($entry)
            continue
        }
        if (Value-Matches $entry) { continue }
        try {
            $key = Open-Hive $entry.hive $entry.subkey $true
            if ($null -eq $key) { throw "cannot open $($entry.hive)\$($entry.subkey)" }
            $created = -not ($entry.present -eq $true)
            if ($created) {
                $key.DeleteValue([string]$entry.name, $false)
            } else {
                $kindName = 'DWord'
                if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
                $kind = [Microsoft.Win32.RegistryValueKind]$kindName
                $value = Convert-RegistryData $kindName ([string]$entry.data)
                $key.SetValue([string]$entry.name, $value, $kind)
            }
            $key.Close()
            if (-not (Value-Matches $entry)) {
                if ($created) { throw 'the suite created this value and it is still present' }
                throw 'the saved value was not put back'
            }
        } catch {
            $remaining.Add($entry)
            $verb = if ($entry.present -eq $true) { 'was changed and was not put back' } else { 'was created by the suite and was not deleted' }
            $line = "$($entry.hive)\$($entry.subkey) value $($entry.name) $verb. $($_.Exception.Message). Run: $(Restore-Command $entry)"
            $errors.Add($line)
            Write-Host $line
        }
    }
    $powerLeft = New-Object System.Collections.Generic.List[object]
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    foreach ($entry in $script:Power.ToArray()) {
        try {
            $current = Read-PowerIndex ([string]$entry.setting)
            if ($null -ne $current -and $current.ac -eq [int]$entry.ac -and $current.dc -eq [int]$entry.dc) {
                continue
            }
            $err = Set-PowerIndex $entry.setting ([int]$entry.ac) ([int]$entry.dc)
            if ($err) {
                $powerLeft.Add($entry)
                $line = "power $($entry.setting) was changed and was not put back. $err. Run: powercfg /SETACVALUEINDEX SCHEME_CURRENT $sub $($entry.setting) $($entry.ac) & powercfg /SETDCVALUEINDEX SCHEME_CURRENT $sub $($entry.setting) $($entry.dc) & powercfg /SETACTIVE SCHEME_CURRENT"
                $errors.Add($line)
                Write-Host $line
            }
        } catch {
            $powerLeft.Add($entry)
            $errors.Add("power $($entry.setting): $($_.Exception.Message)")
        }
    }
    $script:Changes = $remaining
    $script:Power = $powerLeft
    if ($remaining.Count -gt 0 -or $powerLeft.Count -gt 0) {
        Save-Pending
    }
    $hklmLeft = $false
    foreach ($entry in $remaining.ToArray()) {
        if ($entry.hive -eq 'HKLM') { $hklmLeft = $true }
    }
    if ($hklmLeft -and -not $elevated) {
        $script:HklmCommand = "powershell -ExecutionPolicy Bypass -File `"$restoreScript`""
        foreach ($entry in $remaining.ToArray()) {
            if ($entry.hive -ne 'HKLM') { continue }
            $line = "$($entry.hive)\$($entry.subkey) value $($entry.name) was changed and was not put back. Run: $(Restore-Command $entry)"
            $errors.Add($line)
            Write-Host $line
        }
        $errors.Add("HKLM was not restored. From an elevated prompt run: $($script:HklmCommand)")
    }
    return @($errors)
}

function Restore-Suite {
    if ($script:Restored) { return @($script:RestoreErrors) }
    $script:Restored = $true
    $errors = New-Object System.Collections.Generic.List[string]
    if ($null -ne $native) {
        try { $native::ReleaseAll() } catch { $errors.Add("release: $($_.Exception.Message)") }
    }
    foreach ($item in @(Restore-Changes)) {
        if ($item) { $errors.Add([string]$item) }
    }
    if ($script:Changes.Count -eq 0 -and $script:Power.Count -eq 0) {
        if (Test-Path -LiteralPath $pendingPath) {
            Remove-Item -LiteralPath $pendingPath -Force -ErrorAction SilentlyContinue
        }
        try {
            $runOnce = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce', $true)
            if ($null -ne $runOnce) {
                $runOnce.DeleteValue('softlock-hwsuite', $false)
                $runOnce.Close()
            }
        } catch {
            $errors.Add("RunOnce: $($_.Exception.Message)")
        }
    }
    $script:RestoreErrors = @($errors)
    return @($errors)
}

function Invoke-FailClosedLock {
    if ($script:LockFired) { return }
    if ($null -eq $native) { throw 'fail-closed lock has no listener' }
    $native::LockNow()
    $script:LockFired = $true
    $script:SawLock = $true
    $script:LeftLocked = $true
}

function Add-Result([string]$id, [string]$status, [string]$reason, $measurements) {
    if ($null -eq $measurements) { $measurements = @{} }
    $results.Add([pscustomobject]@{
        id = $id
        status = $status
        reason = $reason
        measurements = $measurements
    })
    if ($status -eq 'fail') { $script:Failed = $true }
}

function ConvertTo-ResultJson {
    $rows = @()
    foreach ($row in $results) {
        $bits = @()
        foreach ($key in @($row.measurements.Keys)) {
            $value = $row.measurements[$key]
            if ($value -is [bool]) {
                $encoded = if ($value) { 'true' } else { 'false' }
            } elseif ($value -is [int] -or $value -is [long]) {
                $encoded = "$value"
            } else {
                $encoded = '"' + (Escape-JsonString ([string]$value)) + '"'
            }
            $bits += '"' + $key + '":' + $encoded
        }
        $reason = Escape-JsonString ([string]$row.reason)
        $rows += '{"id":"' + $row.id + '","status":"' + $row.status + '","reason":"' + $reason + '","measurements":{' + ($bits -join ',') + '}}'
    }
    return '[' + ($rows -join ',') + ']'
}

if (-not (Test-SuiteElevated)) {
    Write-Host 'Admin rows will skip. To run them, open an elevated prompt and pass --admin.'
}

try {
    $detect = $null
    Add-Type -Path (Join-Path $PSScriptRoot 'Native.cs') -ErrorAction Stop
    $native = [SoftLock.Hardware.SoftLockHw]
    $native::InstallCtrlHandler()
    $native::StartListener()
    $detect = $native::DetectJson() | ConvertFrom-Json

    $version = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $powerText = Read-Native { powercfg.exe /a }
    $laptop = $null
    try {
        $chassis = @(Get-CimInstance -ClassName Win32_SystemEnclosure -ErrorAction Stop | Select-Object -ExpandProperty ChassisTypes)
        $laptop = $false
        foreach ($code in $chassis) {
            if (@(8, 9, 10, 11, 14, 30, 31, 32) -contains [int]$code) { $laptop = $true }
        }
    } catch {
        $laptop = $null
    }
    $biometric = $null
    try {
        $found = @(Get-PnpDevice -Class Biometric -Status OK -ErrorAction Stop)
        $biometric = $found.Count -gt 0
    } catch {
        $biometric = $null
    }
    $pin = $false
    foreach ($pinPath in @(
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\LogonUI\NgcPin',
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\LogonUI\NgcPin'
    )) {
        if (Test-Path -LiteralPath $pinPath) { $pin = $true }
    }

    $bioJson = if ($null -eq $biometric) { 'null' } else { if ($biometric) { 'true' } else { 'false' } }
    $pinJson = if ($pin) { 'true' } else { 'false' }
    $laptopJson = if ($null -eq $laptop) { 'null' } else { if ($laptop) { 'true' } else { 'false' } }
    $powerEscaped = Escape-JsonString $powerText
    $internalJson = 'null'
    if ($detect.PSObject.Properties.Name -contains 'internal' -and $null -ne $detect.internal) {
        $internalJson = if ($detect.internal) { 'true' } else { 'false' }
    }
    $factsPath = Join-Path $env:TEMP ("softlock-hw-facts-{0}.json" -f $PID)
    $facts = '{' +
        '"ci":' + $(if ($ci) { 'true' } else { 'false' }) +
        ',"admin":' + $(if ($admin) { 'true' } else { 'false' }) +
        ',"school":' + $(if ($school) { 'true' } else { 'false' }) +
        ',"elevated":' + $(if ($detect.elevated) { 'true' } else { 'false' }) +
        ',"github_actions":' + $(if ($env:GITHUB_ACTIONS -eq 'true') { 'true' } else { 'false' }) +
        ',"current_build":"' + (Escape-JsonString ([string]$version.CurrentBuild)) + '"' +
        ',"installation_type":"' + (Escape-JsonString ([string]$version.InstallationType)) + '"' +
        ',"powercfg_a":"' + $powerEscaped + '"' +
        ',"monitor_count":' + [int]$detect.monitors +
        ',"biometric":' + $bioJson +
        ',"pin":' + $pinJson +
        ',"client_protocol":' + [int]$detect.protocol +
        ',"chassis_laptop":' + $laptopJson +
        ',"ac_line":' + [int]$detect.ac +
        ',"internal_display":' + $internalJson +
        '}'
    Write-Utf8 $factsPath $facts

    $manifest = 'tests\hardware\windows\harness\Cargo.toml'
    & cargo test --locked --release --manifest-path $manifest -- --test-threads=8
    Sync-Interrupt
    if ($script:Interrupted) { throw 'interrupted' }
    if ($LASTEXITCODE -ne 0) {
        Add-Result 'parser-self-test' 'fail' 'cargo test failed' @{}
    } else {
        Add-Result 'parser-self-test' 'pass' '' @{}
    }
    & cargo build --locked --release --manifest-path $manifest
    Sync-Interrupt
    if ($script:Interrupted) { throw 'interrupted' }
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
    $exe = Join-Path $root 'tests\hardware\windows\harness\target\release\softlock-win-hwsuite.exe'
    $planPath = Join-Path $env:TEMP ("softlock-hw-plan-{0}.json" -f $PID)
    & $exe plan --facts $factsPath | Out-File -FilePath $planPath -Encoding utf8
    Sync-Interrupt
    if ($script:Interrupted) { throw 'interrupted' }
    if ($LASTEXITCODE -ne 0) { throw 'plan failed' }
    $planRaw = [System.IO.File]::ReadAllText($planPath).TrimStart([char]0xFEFF)
    $plan = $planRaw | ConvertFrom-Json
    $script:overlay = $null

    function Get-Overlay {
        if ($null -eq $script:overlay) {
            $script:overlay = $native::OverlayJson() | ConvertFrom-Json
        }
        return $script:overlay
    }

    $needsButtons = $false
    foreach ($row in @($plan.rows)) {
        if ($row.action -eq 'prompt') { $needsButtons = $true }
    }
    if ($needsButtons) {
        $lidHeld = Ensure-DoNothing '5ca83367-6e45-459f-a27b-476b1d01c936'
        $buttonHeld = Ensure-DoNothing '7648efa3-dd9c-4e3e-b566-50f929386280'
        $lidPrompt = $false
        foreach ($planned in @($plan.rows)) {
            if ($planned.id -eq 'lid-cycle' -and $planned.action -eq 'prompt') { $lidPrompt = $true }
        }
        if ($lidPrompt -and $lidHeld -eq 'needs-admin') {
            Add-Result 'lid-cycle' 'skip' 'needs-admin' @{ stderr = 'powercfg LIDACTION needs administrator' }
        } elseif ($lidPrompt -and $lidHeld) {
            Add-Result 'lid-cycle' 'fail' 'lid policy was not changed' @{ stderr = $lidHeld }
        }
        if ($buttonHeld -and $buttonHeld -ne 'needs-admin') {
            Add-Result 'power-key' 'fail' 'power button policy was not changed' @{ stderr = $buttonHeld }
        }
    }

    foreach ($row in @($plan.rows)) {
        Sync-Interrupt
        if ($script:Interrupted) { break }
        if ($row.action -eq 'skip' -or $row.action -eq 'record') { continue }
        if ($row.id -eq 'parser-self-test' -or $row.id -eq 'restore') { continue }
        $already = $false
        foreach ($got in $results.ToArray()) {
            if ($got.id -eq $row.id) { $already = $true }
        }
        if ($already) { continue }
        if ($script:LockFired -and $row.action -eq 'prompt') {
            Add-Result $row.id 'skip' 'stopped after fail-closed lock' @{}
            continue
        }
        if ((Elapsed-Ms) -gt ($budgetMs - 5000)) {
            Add-Result $row.id 'skip' 'budget' @{}
            continue
        }
        $beforeCount = $results.Count
        try {
        switch ($row.id) {
            'overlay-topmost' {
                $probe = Get-Overlay
                $ok = $probe.ok -and $probe.topmost -and $probe.click_through -and $probe.covered -and ($probe.windows -eq $probe.monitors)
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{
                    topmost = [bool]$probe.topmost
                    monitors = [int]$probe.monitors
                }
            }
            'overlay-capture' {
                $probe = Get-Overlay
                $ok = $probe.ok -and $probe.affinity -and -not $probe.marker_visible
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{
                    affinity = [bool]$probe.affinity
                    marker_visible = [bool]$probe.marker_visible
                }
            }
            'toast-hkcu-restore' {
                $policy = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Policies\Microsoft\Windows\CurrentVersion\PushNotifications')
                $blocked = $false
                if ($null -ne $policy -and $policy.ValueCount -gt 0) { $blocked = $true }
                if ($null -ne $policy) { $policy.Close() }
                if ($blocked) {
                    Add-Result $row.id 'skip' 'not-applicable' @{}
                } else {
                    $sub = 'Software\Microsoft\Windows\CurrentVersion\PushNotifications'
                    $read = Write-SuiteDword 'HKCU' $sub 'ToastEnabled' 0
                    $restoredNow = @(Restore-Changes)
                    if ($restoredNow.Count -gt 0) {
                        Add-Result $row.id 'fail' ($restoredNow -join '; ') @{ value = $read; stderr = ($restoredNow -join '; ') }
                    } else {
                        Add-Result $row.id $(if ($read -eq 0) { 'pass' } else { 'fail' }) '' @{ value = $read }
                    }
                }
            }
            'input-hook-injected' {
                $probe = $native::HookJson() | ConvertFrom-Json
                $ok = $probe.ok -and $probe.installed -and ($probe.injected_seen -ge 1)
                $reason = ''
                $extra = @{ injected_seen = [int]$probe.injected_seen }
                if (-not $ok) {
                    $reason = 'hook was not installed'
                    $extra.stderr = Probe-Error $probe
                }
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) $reason $extra
            }
            'keep-awake-request' {
                $probe = $native::KeepAwakeJson() | ConvertFrom-Json
                Start-Sleep -Milliseconds 400
                $requests = Read-Native { powercfg.exe /requests }
                $requestFile = Join-Path $env:TEMP ("softlock-hw-requests-{0}.txt" -f $PID)
                Write-Utf8 $requestFile $requests
                $seenRaw = & $exe requests --text $requestFile --needle 'hardware suite'
                $native::ReleaseKeepAwake()
                $seen = $seenRaw | ConvertFrom-Json
                Remove-Item -LiteralPath $requestFile -Force -ErrorAction SilentlyContinue
                $apiOk = [bool]$probe.system
                $seenOk = [bool]$seen.system
                $reason = ''
                $status = 'pass'
                $extra = @{
                    api_system = $apiOk
                    api_display = [bool]$probe.display
                    system = $seenOk
                    display = [bool]$seen.display
                    system_error = [int]$probe.system_error
                }
                $adminMeasure = (-not (Test-SuiteElevated)) -or ($requests -match 'administrator') -or ($requests -match 'privileges')
                if (-not $seenOk -and $adminMeasure) {
                    $status = 'skip'
                    $reason = 'needs-admin'
                } elseif (-not $seenOk) {
                    if ($env:GITHUB_ACTIONS -eq 'true' -and $apiOk) {
                        $status = 'skip'
                        $reason = 'ci'
                    } else {
                        $status = 'fail'
                        $snippet = ($requests -replace '\s+', ' ').Trim()
                        if ($snippet.Length -gt 160) { $snippet = $snippet.Substring(0, 160) }
                        $reason = "win32=$($probe.system_error) api=$apiOk powercfg=$seenOk $(Probe-Error $probe) $snippet"
                        $extra.stderr = ((Probe-Error $probe) + ' ' + $snippet).Trim()
                        Write-Host "keep-awake refused: $reason"
                    }
                } elseif (-not $apiOk) {
                    $status = 'fail'
                    $reason = "win32=$($probe.system_error) $(Probe-Error $probe)"
                    $extra.stderr = Probe-Error $probe
                }
                Add-Result $row.id $status $reason $extra
            }
            'cad-hkcu-readback' {
                $pairs = @(
                    @{ sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableTaskMgr' },
                    @{ sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; name = 'NoLogoff' },
                    @{ sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableChangePassword' },
                    @{ sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableLockWorkstation' },
                    @{ sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; name = 'NoClose' }
                )
                $ok = $true
                foreach ($pair in $pairs) {
                    if ((Write-SuiteDword 'HKCU' $pair.sub $pair.name 1) -ne 1) { $ok = $false }
                }
                $restoredNow = @(Restore-Changes)
                if ($restoredNow.Count -gt 0) {
                    Add-Result $row.id 'fail' ($restoredNow -join '; ') @{ values = 5; stderr = ($restoredNow -join '; ') }
                } else {
                    Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{ values = 5 }
                }
            }
            'cad-hklm-readback' {
                if ($row.action -eq 'skip') { continue }
                try {
                    $sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'
                    $read = Write-SuiteDword 'HKLM' $sub 'HideFastUserSwitching' 1
                    $restoredNow = @(Restore-Changes)
                    if ($restoredNow.Count -gt 0) {
                        Add-Result $row.id 'fail' ($restoredNow -join '; ') @{ value = $read; stderr = ($restoredNow -join '; ') }
                    } else {
                        Add-Result $row.id $(if ($read -eq 1) { 'pass' } else { 'fail' }) '' @{ value = $read }
                    }
                } catch {
                    Add-Result $row.id 'fail' 'HKLM write was refused' @{ stderr = $_.Exception.Message }
                }
            }
            'fail-closed-blocked' {
                if ($row.action -eq 'skip') { continue }
                $script:Armed = $true
                $sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'
                Write-SuiteDword 'HKCU' $sub 'DisableLockWorkstation' 1 | Out-Null
                $probe = $native::MeasureLockJson() | ConvertFrom-Json
                $restoredNow = @(Restore-Changes)
                if ($probe.locked) {
                    $script:LockFired = $true
                    $script:SawLock = $true
                    $script:LeftLocked = $true
                }
                if ($restoredNow.Count -gt 0) {
                    Add-Result $row.id 'fail' ($restoredNow -join '; ') @{ stderr = ($restoredNow -join '; ') }
                } elseif ($probe.locked -or -not $probe.api) {
                    Add-Result $row.id 'fail' 'the session was not held unlocked' @{
                        elapsed_ms = [int]$probe.elapsed_ms
                        stderr = Probe-Error $probe
                    }
                } else {
                    $script:Armed = $false
                    Add-Result $row.id 'pass' '' @{ locked = $false }
                }
            }
            'display-enum' {
                $again = $native::DetectJson() | ConvertFrom-Json
                $ok = [int]$again.monitors -ge 1
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{ displays = [int]$again.monitors }
            }
            'display-hotplug' {
                if ($row.action -eq 'skip') { continue }
                $before = $native::HotplugJson() | ConvertFrom-Json
                $held = $null
                $missed = $false
                $prompts = @($row.prompts)
                foreach ($prompt in $prompts) {
                    $answer = Read-Answer $prompt 25
                    if ($answer -eq 'interrupt') { break }
                    if ($answer -ne 'go') { $held = $answer; break }
                    $after = $native::HotplugJson() | ConvertFrom-Json
                    if (-not (Test-DisplayStep $prompt $before $after)) { $missed = $true }
                    $before = $after
                }
                if ($script:Interrupted) {
                } elseif ($missed) {
                    Add-Result $row.id 'fail' 'not detected' @{
                        active = [int]$before.active
                        monitors = [int]$before.monitors
                    }
                } elseif ($held -eq 'budget') {
                    Add-Result $row.id 'skip' 'budget' @{}
                } elseif ($held -eq 'operator') {
                    Add-Result $row.id 'skip' 'operator' @{}
                } elseif ($null -ne $held) {
                    Add-Result $row.id 'skip' 'no-input' @{}
                } else {
                    Add-Result $row.id 'pass' '' @{
                        active = [int]$before.active
                        monitors = [int]$before.monitors
                    }
                }
            }
            'lid-cycle' {
                if ($row.action -eq 'skip') { continue }
                $held = Ensure-DoNothing '5ca83367-6e45-459f-a27b-476b1d01c936'
                if ($held -eq 'needs-admin') {
                    Add-Result $row.id 'skip' 'needs-admin' @{ stderr = 'powercfg LIDACTION needs administrator' }
                } elseif ($held) {
                    Add-Result $row.id 'fail' 'lid policy was not changed' @{ stderr = $held }
                } else {
                    $before = $native::SnapshotJson() | ConvertFrom-Json
                    $record = Latest-PowerRecord
                    $answer = Read-Answer @($row.prompts)[0] 25
                    $after = $native::SnapshotJson() | ConvertFrom-Json
                    $power = Power-Since $record
                    $moved = ($after.lid_close -gt $before.lid_close) -and ($after.lid_open -gt $before.lid_open)
                    $transition = ($after.suspend -gt $before.suspend) -or ($power.known -and $power.sleep -gt 0)
                    if ($script:Interrupted) {
                    } elseif ($transition) {
                        Add-Result $row.id 'fail' 'power transition started' @{ slept = $true }
                    } elseif ($answer -eq 'budget') {
                        Add-Result $row.id 'skip' 'budget' @{}
                    } elseif ($answer -eq 'operator') {
                        Add-Result $row.id 'skip' 'operator' @{}
                    } elseif ($answer -ne 'go') {
                        Add-Result $row.id 'skip' 'no-input' @{}
                    } elseif ($moved) {
                        Add-Result $row.id 'pass' '' @{ seen = $true }
                    } else {
                        Add-Result $row.id 'fail' 'not detected' @{ seen = $false }
                    }
                }
            }
            'power-key' {
                Add-Result $row.id 'skip' 'no language-independent power-button signal while the action is Do nothing' @{}
            }
            'session-lock' {
                if ($row.action -eq 'skip') { continue }
                $answer = Read-Answer @($row.prompts)[0] 20
                if ($script:Interrupted) {
                } elseif ($answer -eq 'budget') {
                    Add-Result $row.id 'skip' 'budget' @{}
                } elseif ($answer -eq 'operator') {
                    Add-Result $row.id 'skip' 'operator' @{}
                } elseif ($answer -ne 'go') {
                    Add-Result $row.id 'skip' 'no-input' @{}
                } else {
                    $script:Armed = $true
                    $probe = $native::MeasureLockJson() | ConvertFrom-Json
                    if ($probe.locked) {
                        $script:LockFired = $true
                        $script:SawLock = $true
                    }
                    $unlocked = $false
                    if ($probe.locked) {
                        $unlocked = $native::WaitForUnlock(40000)
                    }
                    $ok = $probe.api -and $probe.locked -and $unlocked -and ($probe.elapsed_ms -lt 1000)
                    if ($ok) {
                        $script:Armed = $false
                        $script:LockFired = $false
                        $script:LeftLocked = $false
                        Add-Result $row.id 'pass' '' @{ locked = $true; elapsed_ms = [int]$probe.elapsed_ms }
                    } else {
                        if ($probe.locked) { $script:LeftLocked = $true }
                        Add-Result $row.id 'fail' 'not detected' @{
                            locked = [bool]$probe.locked
                            elapsed_ms = [int]$probe.elapsed_ms
                            stderr = Probe-Error $probe
                        }
                    }
                }
            }
            default { }
        }
        } catch {
            Discard-Staged
            if ($results.Count -eq $beforeCount) {
                $message = ''
                if ($null -ne $_.Exception) { $message = $_.Exception.Message }
                if ((Test-AccessDenied $_) -and -not (Test-SuiteElevated)) {
                    Add-Result $row.id 'skip' 'needs-admin' @{ stderr = $message }
                } else {
                    Add-Result $row.id 'fail' 'setting failed' @{ stderr = $message }
                }
            }
        }
        if ($script:Interrupted) { break }
    }
} catch {
    if (-not $script:Interrupted -and $_.Exception) {
        $script:SuiteError = $_.Exception.Message
        $script:Failed = $true
    }
} finally {
    $cleanup = New-Object System.Collections.Generic.List[string]
    try { Sync-Interrupt } catch { [void]$cleanup.Add("interrupt flag: $($_.Exception.Message)") }
    try {
        if ($script:Interrupted -and $null -eq $plan -and $exe -and $factsPath -and (Test-Path -LiteralPath $exe) -and (Test-Path -LiteralPath $factsPath)) {
            $recovered = & $exe plan --facts $factsPath
            if ($LASTEXITCODE -eq 0 -and $recovered) {
                $plan = $recovered | ConvertFrom-Json
            }
        }
    } catch { [void]$cleanup.Add("plan: $($_.Exception.Message)") }
    try { Complete-Interrupted } catch { [void]$cleanup.Add("interrupt rows: $($_.Exception.Message)") }
    $restoreErrors = @()
    try { $restoreErrors = @(Restore-Suite) } catch { $restoreErrors = @("restore: $($_.Exception.Message)") }
    foreach ($item in $cleanup.ToArray()) { $restoreErrors += $item }
    if ($script:SuiteError) { $restoreErrors += $script:SuiteError }
    try {
        $failed = $script:Failed
        foreach ($item in @($restoreErrors)) { if ($item) { $failed = $true } }
        $script:ActuallyLocked = $false
        if ($failed) {
            if ($null -eq $native) { throw 'fail-closed lock has no listener' }
            $state = $native::SessionStateJson() | ConvertFrom-Json
            if (-not ($state.ok -and $state.locked)) {
                Write-Host 'Locking Windows because a row failed.'
                $native::LockNow()
                $deadline = [DateTime]::UtcNow.AddSeconds(2)
                do {
                    Start-Sleep -Milliseconds 100
                    $state = $native::SessionStateJson() | ConvertFrom-Json
                    if ($state.ok -and $state.locked) { break }
                } while ([DateTime]::UtcNow -lt $deadline)
            }
            if ($state.ok -and $state.locked) {
                $script:ActuallyLocked = $true
            } else {
                $script:LockError = "session was not locked (flags=$($state.flags))"
            }
        } elseif ($null -ne $native) {
            $state = $native::SessionStateJson() | ConvertFrom-Json
            if ($state.ok -and $state.locked) { $script:ActuallyLocked = $true }
        }
    } catch {
        $script:LockError = $_.Exception.Message
        $script:ActuallyLocked = $false
    }
    if ($script:LockError) { $restoreErrors += "fail-closed lock: $($script:LockError)" }
    $restored = $true
    try {
        $real = @($restoreErrors | Where-Object { $_ })
        $restored = ($real.Count -eq 0)
        if (-not $restored) {
            Write-Host ($real -join "`n")
            Add-Result 'restore' 'fail' ($real -join '; ') @{ stderr = ($real -join '; ') }
        } else {
            Add-Result 'restore' 'pass' '' @{}
        }
    } catch { $restored = $false }
    try { Write-SuiteReport $restored } catch { }
    if ($script:Interrupted) { exit 130 }
    exit $script:ExitCode
}
