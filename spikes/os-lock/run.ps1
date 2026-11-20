# OS-lock proof. An elevated prompt on the console. Does not unlock Windows.
# --ci is the hosted-runner job. --self-check restores a dummy value and does not lock.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ci = $false
$selfCheck = $false
foreach ($arg in $args) {
    switch ($arg) {
        '--ci' { $ci = $true }
        '--self-check' { $selfCheck = $true }
        default { Write-Host "Unknown argument $arg"; exit 2 }
    }
}
if ($ci -and $env:GITHUB_ACTIONS -ne 'true') {
    Write-Host '--ci applies only when GITHUB_ACTIONS=true; ignoring --ci'
    $ci = $false
}

function Test-SchoolWorkstation {
    try {
        $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('HARDWARE\DESCRIPTION\System\BIOS')
        if ($null -eq $key) { return $false }
        $name = [string]$key.GetValue('SystemProductName')
        $key.Close()
        if ($name.ToLowerInvariant().Contains('thinkstation p7')) { return $true }
    } catch { }
    return $false
}

if (Test-SchoolWorkstation) {
    Write-Host 'refusing: school workstation'
    exit 2
}

. (Join-Path $PSScriptRoot 'restore.ps1')

$pendingDir = Join-Path $env:LOCALAPPDATA 'os-lock-probe'
$pendingPath = Join-Path $pendingDir 'pending.json'
$restoreScript = Join-Path $PSScriptRoot 'restore.ps1'
$taskName = 'os-lock-restore'
$runOnceName = 'os-lock-restore'

function Test-Elevated {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Current-Sid {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    return $identity.User.Value
}

function Register-RunOnceEntry {
    $run = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce')
    $command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$restoreScript`""
    $run.SetValue($runOnceName, $command)
    $run.Close()
}

function Register-RestoreTaskEntry {
    $dir = 'C:\ProgramData\os-lock-probe'
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $cmd = Join-Path $dir 'restore.cmd'
    $line = "@echo off`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$restoreScript`"`r`n"
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($cmd, $line, $utf8)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = @(& schtasks.exe /Create /TN $taskName /SC ONLOGON /RU SYSTEM /RL HIGHEST /F /TR $cmd 2>&1 | ForEach-Object { "$_" })
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    if ($code -ne 0) {
        $detail = $lines -join ' '
        throw "schtasks create failed: $detail"
    }
}

function Save-PendingFile($entries) {
    if (-not (Test-Path -LiteralPath $pendingDir)) {
        New-Item -ItemType Directory -Path $pendingDir | Out-Null
    }
    $items = @()
    foreach ($entry in (Get-Items $entries)) {
        $present = if ($entry.present -eq $true) { 'true' } else { 'false' }
        $kind = if ($entry.kind) { [string]$entry.kind } else { 'DWord' }
        $created = if ($entry.created_key -eq $true) { 'true' } else { 'false' }
        $data = [string]$entry.data
        $items += '{"hive":"' + [string]$entry.hive + '","subkey":"' + ([string]$entry.subkey).Replace('\', '\\') + '","name":"' + [string]$entry.name + '","present":' + $present + ',"kind":"' + $kind + '","data":"' + $data + '","created_key":' + $created + '}'
    }
    $body = '{"sid":"' + (Current-Sid) + '","values":[' + ($items -join ',') + ']}'
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText($pendingPath, $body, $utf8)
}

function Read-Original([string]$hive, [string]$subkey, [string]$name) {
    $root = if ($hive -eq 'HKLM') { [Microsoft.Win32.Registry]::LocalMachine } else { [Microsoft.Win32.Registry]::CurrentUser }
    $key = $root.OpenSubKey($subkey, $false)
    if ($null -eq $key) {
        return [pscustomobject]@{ hive = $hive; subkey = $subkey; name = $name; present = $false; kind = 'DWord'; data = ''; created_key = $true }
    }
    $found = $false
    $kind = 'DWord'
    $data = ''
    foreach ($existing in @($key.GetValueNames())) {
        if ($existing -ne $name) { continue }
        $found = $true
        $kind = $key.GetValueKind($existing).ToString()
        $raw = $key.GetValue($existing)
        $data = "$raw"
    }
    $key.Close()
    if (-not $found) {
        return [pscustomobject]@{ hive = $hive; subkey = $subkey; name = $name; present = $false; kind = 'DWord'; data = ''; created_key = $false }
    }
    return [pscustomobject]@{ hive = $hive; subkey = $subkey; name = $name; present = $true; kind = $kind; data = $data; created_key = $false }
}

function Set-Dword([string]$hive, [string]$subkey, [string]$name, [int]$data) {
    $root = if ($hive -eq 'HKLM') { [Microsoft.Win32.Registry]::LocalMachine } else { [Microsoft.Win32.Registry]::CurrentUser }
    $key = $root.CreateSubKey($subkey)
    if ($null -eq $key) { throw "could not open $hive\$subkey" }
    $key.SetValue($name, $data, [Microsoft.Win32.RegistryValueKind]::DWord)
    $key.Close()
}

function Read-Dword([string]$hive, [string]$subkey, [string]$name) {
    $root = if ($hive -eq 'HKLM') { [Microsoft.Win32.Registry]::LocalMachine } else { [Microsoft.Win32.Registry]::CurrentUser }
    $key = $root.OpenSubKey($subkey, $false)
    if ($null -eq $key) { return $null }
    $value = $key.GetValue($name, $null)
    $key.Close()
    if ($null -eq $value) { return $null }
    return [int]$value
}

function Invoke-SelfCheck {
    if (-not (Test-Elevated)) { throw 'self-check needs an elevated prompt' }
    if (Test-Path -LiteralPath $pendingPath) { throw 'a restore is already pending; run restore.ps1 before --self-check' }
    $probe = 'Software\os-lock-probe-citest'
    $roots = @(
        @{ hive = 'HKCU'; root = [Microsoft.Win32.Registry]::CurrentUser },
        @{ hive = 'HKLM'; root = [Microsoft.Win32.Registry]::LocalMachine }
    )
    try {
        foreach ($item in $roots) { $item.root.DeleteSubKeyTree($probe, $false) }
        Set-Dword 'HKCU' $probe 'Keep' 4
        Set-Dword 'HKLM' $probe 'Keep' 6
        $entries = @()
        $entries += (Read-Original 'HKCU' $probe 'Keep')
        $entries += (Read-Original 'HKLM' $probe 'Keep')
        $entries += (Read-Original 'HKCU' $probe 'Extra')
        Save-PendingFile $entries
        Register-RunOnceEntry
        Register-RestoreTaskEntry
        Set-Dword 'HKCU' $probe 'Keep' 9
        Set-Dword 'HKLM' $probe 'Keep' 8
        Set-Dword 'HKCU' $probe 'Extra' 1
        $code = Invoke-PendingRestore
        if ($code -ne 0) { throw "restore exited $code" }
        $hkcu = Read-Dword 'HKCU' $probe 'Keep'
        $hklm = Read-Dword 'HKLM' $probe 'Keep'
        $extra = Read-Dword 'HKCU' $probe 'Extra'
        if ($hkcu -ne 4) { throw 'HKCU Keep was not put back' }
        if ($hklm -ne 6) { throw 'HKLM Keep was not put back' }
        if ($null -ne $extra) { throw 'Extra was created by the probe and was not deleted' }
        if (Test-Path -LiteralPath $pendingPath) { throw 'pending.json was left behind' }
        $run = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce', $false)
        if ($null -ne $run) {
            $left = $run.GetValue($runOnceName, $null)
            $run.Close()
            if ($null -ne $left) { throw 'RunOnce was left behind' }
        }
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { & schtasks.exe /Query /TN $taskName 2>&1 | Out-Null; $taskCode = $LASTEXITCODE } finally { $ErrorActionPreference = $previous }
        if ($taskCode -eq 0) { throw 'the restore task was left behind' }
        Write-Host 'self-check ok'
    } finally {
        foreach ($item in $roots) {
            try { $item.root.DeleteSubKeyTree($probe, $false) } catch { }
        }
        if (Test-Path -LiteralPath $pendingPath) { Invoke-PendingRestore | Out-Null }
    }
}

if ($selfCheck) {
    try {
        Invoke-SelfCheck
        exit 0
    } catch {
        Write-Host $_.Exception.Message
        exit 1
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location -LiteralPath $root
$started = [DateTime]::UtcNow
$script:Rows = New-Object System.Collections.Generic.List[object]
$script:Failed = $false
$script:MustEndLocked = $false
$script:SuiteError = ''
$script:Restored = $false
$script:Heartbeat = $null
$outDir = Join-Path $root ('suite-output\' + $started.ToString('yyyyMMddTHHmmssZ'))
$exe = Join-Path $PSScriptRoot 'target\release\os-lock-proof.exe'
$policies = @(
    @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableTaskMgr'; apply = 1 },
    @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableLockWorkstation'; apply = 1 },
    @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableChangePassword'; apply = 1 },
    @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; name = 'NoLogoff'; apply = 1 },
    @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; name = 'NoClose'; apply = 1 },
    @{ hive = 'HKLM'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'HideFastUserSwitching'; apply = 1 },
    @{ hive = 'HKLM'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'ConsentPromptBehaviorAdmin'; apply = 1 },
    @{ hive = 'HKLM'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'ConsentPromptBehaviorUser'; apply = 0 }
)

function Add-Row([string]$id, [string]$name, [string]$status, $skip, [bool]$needsAdmin, [int]$duration, [string]$detail, [string]$measurements) {
    if (-not $measurements) { $measurements = '{}' }
    $script:Rows.Add([pscustomobject]@{
        id = $id
        name = $name
        status = $status
        skip = $skip
        physical = $false
        needs_admin = $needsAdmin
        duration_ms = $duration
        detail = $detail
        measurements = $measurements
    })
    if ($status -eq 'fail') { $script:Failed = $true }
}

function Invoke-Probe {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = @(& $exe @args 2>&1 | ForEach-Object { "$_" })
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    $jsonLine = ''
    foreach ($line in $lines) {
        $text = "$line"
        if ($text.StartsWith('{')) { $jsonLine = $text }
    }
    $parsed = $null
    if ($jsonLine) {
        try { $parsed = $jsonLine | ConvertFrom-Json } catch { $parsed = $null }
    }
    $joined = $lines -join "`n"
    return [pscustomobject]@{ code = $code; json = $parsed; text = $joined }
}

function Json-Measure($parsed) {
    if ($null -eq $parsed) { return '{}' }
    $api = if ($parsed.api_ok -eq $true) { 'true' } else { 'false' }
    $notified = if ($parsed.notified -eq $true) { 'true' } else { 'false' }
    $elapsed = '0'
    if ($parsed.PSObject.Properties.Name -contains 'elapsed_us' -and $null -ne $parsed.elapsed_us) { $elapsed = "$($parsed.elapsed_us)" }
    $win32 = '0'
    if ($parsed.PSObject.Properties.Name -contains 'win32' -and $null -ne $parsed.win32) { $win32 = "$($parsed.win32)" }
    $kind = ''
    if ($parsed.PSObject.Properties.Name -contains 'kind' -and $parsed.kind) { $kind = [string]$parsed.kind }
    $locked = 'null'
    if ($parsed.PSObject.Properties.Name -contains 'session_locked') {
        if ($parsed.session_locked -eq $true) { $locked = 'true' } elseif ($parsed.session_locked -eq $false) { $locked = 'false' }
    }
    return '{"api_ok":' + $api + ',"notified":' + $notified + ',"kind":"' + $kind + '","elapsed_us":' + $elapsed + ',"win32":' + $win32 + ',"session_locked":' + $locked + '}'
}

function Add-ProbeRow([string]$id, [string]$name, $result, [int]$duration) {
    $parsed = $result.json
    $measurements = Json-Measure $parsed
    if ($result.code -ne 0 -or $null -eq $parsed) {
        $detail = $result.text
        if (-not $detail) { $detail = "exit $($result.code)" }
        Add-Row $id $name 'fail' $null $true $duration $detail $measurements
        return
    }
    $elapsed = '0'
    if ($parsed.PSObject.Properties.Name -contains 'elapsed_us') { $elapsed = "$($parsed.elapsed_us)" }
    $kind = ''
    if ($parsed.kind) { $kind = [string]$parsed.kind }
    $detail = "elapsed_us=$elapsed kind=$kind api=$($parsed.api_ok)"
    Write-Host "timing $id api=$($parsed.api_ok) notified=$($parsed.notified) kind=$kind elapsed_us=$elapsed win32=$($parsed.win32)"
    $status = if ($parsed.pass -eq $true) { 'pass' } else { 'fail' }
    if ($parsed.error) { $detail = "$detail $($parsed.error)" }
    Add-Row $id $name $status $null $true $duration $detail $measurements
}

function Read-BeatCount([string]$path) {
    if (-not (Test-Path -LiteralPath $path)) { return 0 }
    $stream = [System.IO.File]::Open($path, 'Open', 'Read', 'ReadWrite')
    $reader = New-Object System.IO.StreamReader($stream)
    try {
        $count = 0
        while ($null -ne $reader.ReadLine()) { $count++ }
        return $count
    } finally {
        $reader.Close()
    }
}

function Stop-Heartbeat {
    if ($null -eq $script:Heartbeat) { return }
    try {
        if (-not $script:Heartbeat.HasExited) { Stop-Process -Id $script:Heartbeat.Id -Force -ErrorAction SilentlyContinue }
    } catch { }
}

function Remove-Reconnect {
    $path = Join-Path $outDir 'reconnect.on'
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
}

function Wait-Connected {
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do {
        $status = Invoke-Probe 'status'
        if ($status.json -and $status.json.connected -eq $true) { return $status }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    return (Invoke-Probe 'status')
}

function Write-ProofReport([bool]$restored, [string]$state, [string]$primary, [string]$fallback) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $finished = [DateTime]::UtcNow
    $pass = 0
    $fail = 0
    $skip = 0
    $rows = @()
    foreach ($row in (Get-Items $script:Rows)) {
        if ($row.status -eq 'pass') { $pass++ } elseif ($row.status -eq 'fail') { $fail++ } else { $skip++ }
        $skipJson = 'null'
        if ($row.status -eq 'skip') { $skipJson = '"' + [string]$row.skip + '"' }
        $needs = if ($row.needs_admin) { 'true' } else { 'false' }
        $duration = "$($row.duration_ms)"
        $detail = ([string]$row.detail).Replace('\', '\\').Replace('"', '\"').Replace("`r", ' ').Replace("`n", ' ')
        $rows += '{"id":"' + $row.id + '","name":"' + $row.name + '","status":"' + $row.status + '","skip_reason":' + $skipJson + ',"physical":false,"needs_admin":' + $needs + ',"duration_ms":' + $duration + ',"detail":"' + $detail + '","measurements":' + $row.measurements + '}'
    }
    $mode = if ($ci) { 'ci' } else { 'manual' }
    $durationMs = [int]($finished - $started).TotalMilliseconds
    $restoredJson = if ($restored) { 'true' } else { 'false' }
    $locked = if ($state -eq 'os-locked') { 'true' } else { 'false' }
    $config = '{"primary":"' + $primary + '","fallback":"' + $fallback + '","mode":"' + $mode + '"}'
    $json = '{"schema":"softlock-hardware-suite/1","suite":"windows","started_utc":"' + $started.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') + '","finished_utc":"' + $finished.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') + '","duration_ms":' + "$durationMs" + ',"mode":"' + $mode + '","config":' + $config + ',"tests":[' + ($rows -join ',') + '],"summary":{"pass":' + "$pass" + ',"fail":' + "$fail" + ',"skip":' + "$skip" + '},"fail_closed":' + $locked + ',"serial_collected":false,"restored":' + $restoredJson + ',"interrupted":false,"state":"' + $state + '"}'
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText((Join-Path $outDir 'result.json'), $json, $utf8)
    $seconds = [int]($durationMs / 1000)
    $text = @(
        "Windows OS-lock proof  schema=softlock-hardware-suite/1  suite=windows",
        "pass=$pass fail=$fail skip=$skip duration=${seconds}s state=$state serial-collected=false restored=$restored interrupted=false",
        "primary=$primary fallback=$fallback"
    )
    foreach ($row in (Get-Items $script:Rows)) {
        $text += "$($row.id) $($row.status) $($row.detail)"
    }
    [System.IO.File]::WriteAllText((Join-Path $outDir 'result.txt'), ($text -join "`r`n") + "`r`n", $utf8)
    Write-Host "result pass=$pass fail=$fail skip=$skip state=$state restored=$restored"
    Write-Host "pick primary=$primary fallback=$fallback"
    Write-Host "result.json $(Join-Path $outDir 'result.json')"
}

function Write-SkipReport([string]$reason, [string]$detail) {
    $ids = @(
        @{ id = 'preflight'; name = 'Preflight' },
        @{ id = 'lock-blocked'; name = 'LockWorkStation while the policy is set' },
        @{ id = 'lock-workstation'; name = 'LockWorkStation after the policy is cleared' },
        @{ id = 'service-disconnect'; name = 'Disconnect the console session from session 0' },
        @{ id = 'own-session'; name = 'Disconnect this session' },
        @{ id = 'background-app'; name = 'Background app kept running' },
        @{ id = 'restore'; name = 'Restore lockdown values' }
    )
    foreach ($item in $ids) {
        if ($item.id -eq 'restore') { Add-Row $item.id $item.name 'pass' $null $false 0 'nothing to restore' '{}' }
        else { Add-Row $item.id $item.name 'skip' $reason $true 0 $detail '{}' }
    }
    Write-ProofReport $true 'never-unlocked' 'none' 'none'
    $script:ReportWritten = $true
    if ($ci) { exit 1 }
    exit 0
}

$script:ExitCode = 1
$script:ReportWritten = $false
try {
    if (-not (Test-Elevated)) { Write-SkipReport 'needs-admin' 'not admin' }
    if (Test-Path -LiteralPath $pendingPath) {
        $left = Invoke-PendingRestore
        if ($left -ne 0) { throw 'a previous restore is still pending' }
    }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo build --release --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
    $build = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($build -ne 0) { throw 'cargo build failed' }
    if (-not (Test-Path -LiteralPath $exe)) { throw 'the probe executable was not built' }
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null

    $pre = Invoke-Probe 'preflight'
    if ($pre.code -ne 0 -or $null -eq $pre.json) { throw "preflight failed: $($pre.text)" }
    if (-not $pre.json.elevated) { Write-SkipReport 'needs-admin' 'not admin' }
    if (-not $pre.json.console_present) { Write-SkipReport 'not-applicable' 'no console session' }
    if ($pre.json.remote -eq $true) { Write-SkipReport 'not-applicable' 'remote session' }
    Add-Row 'preflight' 'Preflight' 'pass' $null $true 0 "session=$($pre.json.session) console=$($pre.json.console)" '{}'

    $saved = New-Object System.Collections.Generic.List[object]
    foreach ($policy in $policies) {
        $saved.Add((Read-Original $policy.hive $policy.subkey $policy.name))
    }
    Save-PendingFile $saved
    Register-RunOnceEntry
    Register-RestoreTaskEntry
    $script:MustEndLocked = $true
    foreach ($policy in $policies) {
        Set-Dword $policy.hive $policy.subkey $policy.name ([int]$policy.apply)
        $seen = Read-Dword $policy.hive $policy.subkey $policy.name
        if ($seen -ne [int]$policy.apply) { throw "$($policy.name) did not read back as $($policy.apply)" }
    }

    $beat = Join-Path $outDir 'heartbeat.txt'
    $script:Heartbeat = Start-Process -FilePath $exe -ArgumentList @('heartbeat', '--file', $beat, '--allow-local') -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 500
    $beforeLines = Read-BeatCount $beat

    $watch = [Diagnostics.Stopwatch]::StartNew()
    $blocked = Invoke-Probe 'measure-blocked' '--allow-local'
    Add-ProbeRow 'lock-blocked' 'LockWorkStation while the policy is set' $blocked ([int]$watch.ElapsedMilliseconds)

    $lockPolicy = $policies | Where-Object { $_.name -eq 'DisableLockWorkstation' } | Select-Object -First 1
    Set-Dword $lockPolicy.hive $lockPolicy.subkey $lockPolicy.name 0
    $watch.Restart()
    $locked = Invoke-Probe 'measure-lock' '--allow-local'
    Add-ProbeRow 'lock-workstation' 'LockWorkStation after the policy is cleared' $locked ([int]$watch.ElapsedMilliseconds)
    Set-Dword $lockPolicy.hive $lockPolicy.subkey $lockPolicy.name 1

    $service = Invoke-Probe 'install-service' '--output-dir' $outDir '--allow-local'
    $serviceUp = $service.code -eq 0
    if ($serviceUp) {
        [System.IO.File]::WriteAllText((Join-Path $outDir 'reconnect.on'), "on`n")
        $watch.Restart()
        $fromService = Invoke-Probe 'service-disconnect' '--output-dir' $outDir '--allow-local'
        Add-ProbeRow 'service-disconnect' 'Disconnect the console session from session 0' $fromService ([int]$watch.ElapsedMilliseconds)
        $back = Wait-Connected
        $connected = $back.json -and $back.json.connected -eq $true
        if ($connected -or -not $ci) {
            $watch.Restart()
            $ownArgs = @('measure-self', '--allow-local')
            if ($connected) { $ownArgs += @('--reconnect', '--output-dir', $outDir) }
            $own = Invoke-Probe @ownArgs
            Add-ProbeRow 'own-session' 'Disconnect this session' $own ([int]$watch.ElapsedMilliseconds)
            if ($connected) { Wait-Connected | Out-Null }
        } else {
            Add-Row 'own-session' 'Disconnect this session' 'fail' $null $true 0 'session stayed disconnected' '{}'
        }
    } else {
        $why = $service.text
        if (-not $why) { $why = 'the session 0 service did not start' }
        Add-Row 'service-disconnect' 'Disconnect the console session from session 0' 'fail' $null $true 0 $why '{}'
        if ($ci) {
            Add-Row 'own-session' 'Disconnect this session' 'fail' $null $true 0 'the session 0 service was not running, so disconnect was not called' '{}'
        } else {
            $watch.Restart()
            $own = Invoke-Probe 'measure-self' '--allow-local'
            Add-ProbeRow 'own-session' 'Disconnect this session' $own ([int]$watch.ElapsedMilliseconds)
        }
    }

    Start-Sleep -Milliseconds 400
    $alive = $false
    if ($null -ne $script:Heartbeat) {
        $script:Heartbeat.Refresh()
        $alive = -not $script:Heartbeat.HasExited
    }
    $afterLines = Read-BeatCount $beat
    $grew = $afterLines -gt $beforeLines
    $appStatus = if ($alive -and $grew) { 'pass' } else { 'fail' }
    $aliveJson = if ($alive) { 'true' } else { 'false' }
    $lineText = "$afterLines"
    Add-Row 'background-app' 'Background app kept running' $appStatus $null $false 0 "alive=$alive lines=$lineText" ('{"alive":' + $aliveJson + ',"lines":' + $lineText + '}')
} catch {
    if (-not $script:SuiteError) { $script:SuiteError = $_.Exception.Message }
    $script:Failed = $true
    Write-Host $script:SuiteError
} finally {
    Stop-Heartbeat
    Remove-Reconnect
    if ($script:ReportWritten) {
        try { Invoke-PendingRestore | Out-Null } catch { }
        try { Invoke-Probe 'uninstall-service' | Out-Null } catch { }
    } else {
        $restoreCode = 1
        try { $restoreCode = Invoke-PendingRestore } catch { $restoreCode = 1; $script:SuiteError = $_.Exception.Message }
        $script:Restored = $restoreCode -eq 0
        if (-not $script:Restored) {
            Add-Row 'restore' 'Restore lockdown values' 'fail' $null $true 0 'a lockdown value was not put back' '{}'
        } else {
            Add-Row 'restore' 'Restore lockdown values' 'pass' $null $true 0 '' '{}'
        }
        try { Invoke-Probe 'uninstall-service' | Out-Null } catch { }
        $lockOk = $false
        $serviceOk = $false
        $ownOk = $false
        foreach ($row in (Get-Items $script:Rows)) {
            if ($row.status -ne 'pass') { continue }
            if ($row.id -eq 'lock-workstation') { $lockOk = $true }
            if ($row.id -eq 'service-disconnect') { $serviceOk = $true }
            if ($row.id -eq 'own-session') { $ownOk = $true }
        }
        $lockText = if ($lockOk) { 'true' } else { 'false' }
        $serviceText = if ($serviceOk) { 'true' } else { 'false' }
        $ownText = if ($ownOk) { 'true' } else { 'false' }
        $primary = 'none'
        $fallback = 'none'
        if (Test-Path -LiteralPath $exe) {
            $decision = Invoke-Probe 'decide' '--lock' $lockText '--service' $serviceText '--own' $ownText
            if ($decision.json) {
                $primary = [string]$decision.json.primary
                $fallback = [string]$decision.json.fallback
            }
        }
        $status = $null
        try { $status = Invoke-Probe 'status' } catch { $status = $null }
        $atSignIn = $false
        if ($status -and $status.json -and ($status.json.locked -eq $true -or $status.json.connected -eq $false)) { $atSignIn = $true }
        $state = 'never-unlocked'
        if ($atSignIn) { $state = 'os-locked' }
        try { Write-ProofReport $script:Restored $state $primary $fallback } catch { Write-Host $_.Exception.Message }
        if ($script:MustEndLocked -and -not $atSignIn) {
            $watchEnd = [Diagnostics.Stopwatch]::StartNew()
            if ($ci) {
                $end = Invoke-Probe 'measure-lock' '--allow-local'
            } else {
                $end = Invoke-Probe 'stay-disconnected' '--allow-local'
            }
            $endMs = [int]$watchEnd.ElapsedMilliseconds
            if ($end.json -and ($end.json.pass -eq $true -or $end.json.api_ok -eq $true)) {
                Add-Row 'end-locked' 'End at the Windows sign-in screen' 'pass' $null $true $endMs '' (Json-Measure $end.json)
                $atSignIn = $true
            } else {
                $detail = if ($end) { $end.text } else { 'the ending lock was not called' }
                Add-Row 'end-locked' 'End at the Windows sign-in screen' 'fail' $null $true $endMs $detail '{}'
            }
            $state = 'never-unlocked'
            if ($atSignIn) { $state = 'os-locked' }
            try { Write-ProofReport $script:Restored $state $primary $fallback } catch { Write-Host $_.Exception.Message }
        } elseif ($script:MustEndLocked) {
            Add-Row 'end-locked' 'End at the Windows sign-in screen' 'pass' $null $true 0 'already at the sign-in screen' '{}'
            try { Write-ProofReport $script:Restored 'os-locked' $primary $fallback } catch { Write-Host $_.Exception.Message }
        }
        if ($script:Failed -or -not $script:Restored) { $script:ExitCode = 1 } else { $script:ExitCode = 0 }
    }
}

exit $script:ExitCode
