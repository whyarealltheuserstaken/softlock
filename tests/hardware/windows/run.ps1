# SoftLock Windows hardware suite. School-safe unless --admin is passed.
# Does not show a UAC prompt and does not install a SYSTEM service.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ci = $false
$admin = $false
foreach ($arg in $args) {
    switch ($arg) {
        '--ci' { $ci = $true }
        '-Ci' { $ci = $true }
        '--admin' { $admin = $true }
        '-Admin' { $admin = $true }
        default {
            Write-Error "Unknown argument $arg"
            exit 2
        }
    }
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
Set-Location -LiteralPath $root
$started = [DateTime]::UtcNow
$budgetMs = 300000
$script:Armed = $false
$script:SawLock = $false
$script:Changes = New-Object System.Collections.Generic.List[object]
$script:Restored = $false
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

function Save-Pending {
    if (-not (Test-Path -LiteralPath $pendingDir)) {
        New-Item -ItemType Directory -Path $pendingDir | Out-Null
    }
    $parts = @()
    foreach ($entry in $script:Changes) {
        $present = if ($entry.present) { 'true' } else { 'false' }
        $sub = ([string]$entry.subkey).Replace('\', '\\')
        $parts += '{"hive":"' + $entry.hive + '","subkey":"' + $sub + '","name":"' + $entry.name + '","present":' + $present + ',"data":' + [int]$entry.data + '}'
    }
    $json = '{"values":[' + ($parts -join ',') + ']}'
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
    $present = $false
    $data = 0
    $key = Open-Hive $hive $subkey $false
    if ($null -ne $key) {
        foreach ($existing in @($key.GetValueNames())) {
            if ($existing -eq $name) {
                $present = $true
                $data = [int]$key.GetValue($name)
            }
        }
        $key.Close()
    }
    $script:Changes.Add([pscustomobject]@{
        hive = $hive
        subkey = $subkey
        name = $name
        present = $present
        data = $data
    })
    Save-Pending
}

function Set-SuiteDword([string]$hive, [string]$subkey, [string]$name, [int]$value) {
    $key = Open-Hive $hive $subkey $true
    $key.SetValue($name, $value, [Microsoft.Win32.RegistryValueKind]::DWord)
    $read = [int]$key.GetValue($name)
    $key.Close()
    return $read
}

function Restore-Suite {
    if ($script:Restored) {
        return
    }
    $script:Restored = $true
    if ($null -ne $native) {
        try {
            if ($script:Armed -and -not $script:SawLock) {
                $native::LockNow()
            }
        } catch {
        }
        try { $native::ReleaseAll() } catch { }
    }
    foreach ($entry in $script:Changes) {
        try {
            $key = Open-Hive $entry.hive $entry.subkey $true
            if ($null -eq $key) { continue }
            if ($entry.present) {
                $key.SetValue([string]$entry.name, [int]$entry.data, [Microsoft.Win32.RegistryValueKind]::DWord)
            } else {
                $key.DeleteValue([string]$entry.name, $false)
            }
            $key.Close()
        } catch {
        }
    }
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
    }
}

function Read-Answer([string]$prompt, [int]$seconds) {
    Write-Host $prompt
    if ($ci -or [Console]::IsInputRedirected) {
        return 'skip'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    $buf = New-Object System.Text.StringBuilder
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Elapsed-Ms) -gt $budgetMs) { return 'skip' }
        if ([Console]::KeyAvailable) {
            $key = [Console]::ReadKey($true)
            if ($key.Key -eq 'Enter') {
                return $buf.ToString().Trim()
            }
            if ($key.Key -eq 'Backspace') {
                if ($buf.Length -gt 0) { $buf.Length = $buf.Length - 1 }
                continue
            }
            if ($key.KeyChar -ne 0) { [void]$buf.Append($key.KeyChar) }
        } else {
            Start-Sleep -Milliseconds 50
        }
    }
    return 'skip'
}

function Power-Delta([datetime]$since) {
    $sleep = 0
    $wake = 0
    try {
        $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-Power'; StartTime = $since.ToLocalTime() } -ErrorAction Stop)
        foreach ($event in $events) {
            if ($event.Id -eq 42) { $sleep++ }
            if ($event.Id -eq 107 -or $event.Id -eq 507) { $wake++ }
        }
    } catch {
    }
    return [pscustomobject]@{ sleep = $sleep; wake = $wake }
}

function Restore-Changes {
    foreach ($entry in $script:Changes) {
        try {
            $key = Open-Hive $entry.hive $entry.subkey $true
            if ($null -eq $key) { continue }
            if ($entry.present) {
                $key.SetValue([string]$entry.name, [int]$entry.data, [Microsoft.Win32.RegistryValueKind]::DWord)
            } else {
                $key.DeleteValue([string]$entry.name, $false)
            }
            $key.Close()
        } catch {
        }
    }
}

function Add-Result([string]$id, [string]$status, [string]$reason, $measurements) {
    if ($null -eq $measurements) { $measurements = @{} }
    $results.Add([pscustomobject]@{
        id = $id
        status = $status
        reason = $reason
        measurements = $measurements
    })
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
                $text = ([string]$value).Replace('\', '\\').Replace('"', "'")
                $encoded = '"' + $text + '"'
            }
            $bits += '"' + $key + '":' + $encoded
        }
        $reason = ([string]$row.reason).Replace('\', '\\').Replace('"', "'")
        $rows += '{"id":"' + $row.id + '","status":"' + $row.status + '","reason":"' + $reason + '","measurements":{' + ($bits -join ',') + '}}'
    }
    return '[' + ($rows -join ',') + ']'
}

trap {
    Restore-Suite
    break
}

try {
    $detect = $null
    Add-Type -Path (Join-Path $PSScriptRoot 'Native.cs') -ErrorAction Stop
    $native = [SoftLock.Hardware.SoftLockHw]
    $native::StartListener()
    $detect = $native::DetectJson() | ConvertFrom-Json

    $version = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $powerText = & powercfg.exe /a 2>&1 | Out-String
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
    $powerFile = Join-Path $env:TEMP ("softlock-hw-power-{0}.txt" -f $PID)
    Write-Utf8 $powerFile $powerText
    $powerEscaped = ([System.IO.File]::ReadAllText($powerFile)).Replace('\', '\\').Replace('"', '\"').Replace("`r", '\r').Replace("`n", '\n')
    $factsPath = Join-Path $env:TEMP ("softlock-hw-facts-{0}.json" -f $PID)
    $facts = '{' +
        '"ci":' + $(if ($ci) { 'true' } else { 'false' }) +
        ',"admin":' + $(if ($admin) { 'true' } else { 'false' }) +
        ',"elevated":' + $(if ($detect.elevated) { 'true' } else { 'false' }) +
        ',"github_actions":' + $(if ($env:GITHUB_ACTIONS -eq 'true') { 'true' } else { 'false' }) +
        ',"current_build":"' + [string]$version.CurrentBuild + '"' +
        ',"installation_type":"' + [string]$version.InstallationType + '"' +
        ',"powercfg_a":"' + $powerEscaped + '"' +
        ',"monitor_count":' + [int]$detect.monitors +
        ',"biometric":' + $bioJson +
        ',"pin":' + $pinJson +
        ',"client_protocol":' + [int]$detect.protocol +
        ',"chassis_laptop":' + $laptopJson +
        ',"ac_line":' + [int]$detect.ac +
        '}'
    Write-Utf8 $factsPath $facts

    & cargo test --locked --release -p softlock-win-hwsuite -- --test-threads=8
    if ($LASTEXITCODE -ne 0) {
        Add-Result 'parser-self-test' 'fail' 'cargo test failed' @{}
    } else {
        Add-Result 'parser-self-test' 'pass' '' @{}
    }
    $exe = Join-Path $root 'target\release\softlock-win-hwsuite.exe'
    $planPath = Join-Path $env:TEMP ("softlock-hw-plan-{0}.json" -f $PID)
    & $exe plan --facts $factsPath | Out-File -FilePath $planPath -Encoding utf8
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

    foreach ($row in @($plan.rows)) {
        if ($row.action -eq 'skip' -or $row.action -eq 'record') { continue }
        if ($row.id -eq 'parser-self-test') { continue }
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
                    Add-Result $row.id 'skip' 'school IT controls this' @{}
                } else {
                    $sub = 'Software\Microsoft\Windows\CurrentVersion\PushNotifications'
                    Remember-Dword 'HKCU' $sub 'ToastEnabled'
                    $read = Set-SuiteDword 'HKCU' $sub 'ToastEnabled' 0
                    Restore-Changes
                    Add-Result $row.id $(if ($read -eq 0) { 'pass' } else { 'fail' }) '' @{ value = $read }
                }
            }
            'input-hook-injected' {
                $probe = $native::HookJson() | ConvertFrom-Json
                $ok = $probe.ok -and $probe.installed -and ($probe.injected_seen -ge 1)
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{
                    injected_seen = [int]$probe.injected_seen
                }
            }
            'keep-awake-request' {
                $probe = $native::KeepAwakeJson() | ConvertFrom-Json
                Start-Sleep -Milliseconds 400
                $requests = & powercfg.exe /requests 2>&1 | Out-String
                $requestFile = Join-Path $env:TEMP ("softlock-hw-requests-{0}.txt" -f $PID)
                Write-Utf8 $requestFile $requests
                $seenRaw = & $exe requests --text $requestFile --needle 'SoftLock hardware suite'
                $native::ReleaseKeepAwake()
                $seen = $seenRaw | ConvertFrom-Json
                Remove-Item -LiteralPath $requestFile -Force -ErrorAction SilentlyContinue
                $ok = $probe.api -and $seen.system -and $seen.display
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{
                    system = [bool]$seen.system
                    display = [bool]$seen.display
                }
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
                    Remember-Dword 'HKCU' $pair.sub $pair.name
                    if ((Set-SuiteDword 'HKCU' $pair.sub $pair.name 1) -ne 1) { $ok = $false }
                }
                Restore-Changes
                Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{ values = 5 }
            }
            'cad-hklm-readback' {
                if ($row.action -eq 'skip') { continue }
                try {
                    $sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'
                    Remember-Dword 'HKLM' $sub 'HideFastUserSwitching'
                    $read = Set-SuiteDword 'HKLM' $sub 'HideFastUserSwitching' 1
                    Restore-Changes
                    Add-Result $row.id $(if ($read -eq 1) { 'pass' } else { 'fail' }) '' @{ value = $read }
                } catch {
                    Add-Result $row.id 'skip' 'HKLM write was refused' @{}
                }
            }
            'fail-closed-blocked' {
                if ($row.action -eq 'skip') { continue }
                $sub = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'
                Remember-Dword 'HKCU' $sub 'DisableLockWorkstation'
                Set-SuiteDword 'HKCU' $sub 'DisableLockWorkstation' 1 | Out-Null
                $probe = $native::MeasureLockJson() | ConvertFrom-Json
                Restore-Changes
                if ($probe.locked) {
                    $script:Armed = $true
                    $script:SawLock = $true
                    Add-Result $row.id 'fail' 'the session locked while the policy was set' @{ elapsed_ms = [int]$probe.elapsed_ms }
                } else {
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
                $before = [int]($native::DetectJson() | ConvertFrom-Json).monitors
                $skipped = $false
                $ok = $true
                $prompts = @($row.prompts)
                foreach ($prompt in $prompts) {
                    $answer = Read-Answer $prompt 25
                    if ($answer -eq 'skip') { $skipped = $true; break }
                    $now = [int]($native::DetectJson() | ConvertFrom-Json).monitors
                    if ($prompt.StartsWith('Plug') -and $now -le $before) { $ok = $false }
                    if ($prompt.StartsWith('Unplug') -and $now -ge $before) { $ok = $false }
                    $before = $now
                }
                if ($skipped) { Add-Result $row.id 'skip' 'skipped' @{} }
                else { Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{ displays = $before } }
            }
            'lid-cycle' {
                if ($row.action -eq 'skip') { continue }
                $before = $native::SnapshotJson() | ConvertFrom-Json
                $mark = [DateTime]::UtcNow
                $answer = Read-Answer @($row.prompts)[0] 25
                if ($answer -eq 'skip') {
                    Add-Result $row.id 'skip' 'skipped' @{}
                } else {
                    $after = $native::SnapshotJson() | ConvertFrom-Json
                    $power = Power-Delta $mark
                    $moved = ($after.lid_close -gt $before.lid_close) -or ($after.suspend -gt $before.suspend) -or ($after.display_off -gt $before.display_off) -or ($power.sleep -gt 0)
                    Add-Result $row.id $(if ($moved) { 'pass' } else { 'fail' }) '' @{ seen = [bool]$moved }
                }
            }
            'power-key' {
                if ($row.action -eq 'skip') { continue }
                $before = $native::SnapshotJson() | ConvertFrom-Json
                $mark = [DateTime]::UtcNow
                $answer = Read-Answer @($row.prompts)[0] 25
                if ($answer -eq 'skip') {
                    Add-Result $row.id 'skip' 'skipped' @{}
                } else {
                    $after = $native::SnapshotJson() | ConvertFrom-Json
                    $power = Power-Delta $mark
                    $moved = ($after.suspend -gt $before.suspend) -or ($after.resume -gt $before.resume) -or ($power.sleep -gt 0) -or ($power.wake -gt 0)
                    Add-Result $row.id $(if ($moved) { 'pass' } else { 'fail' }) '' @{ seen = [bool]$moved }
                }
            }
            'session-lock' {
                if ($row.action -eq 'skip') { continue }
                $answer = Read-Answer @($row.prompts)[0] 20
                if ($answer -eq 'skip') {
                    Add-Result $row.id 'skip' 'skipped' @{}
                } else {
                    $script:Armed = $true
                    $probe = $native::MeasureLockJson() | ConvertFrom-Json
                    if ($probe.locked) { $script:SawLock = $true }
                    $unlocked = $false
                    if ($probe.locked) {
                        $unlocked = $native::WaitForUnlock(40000)
                    }
                    $ok = $probe.locked -and $unlocked -and ($probe.elapsed_ms -lt 1000)
                    Add-Result $row.id $(if ($ok) { 'pass' } else { 'fail' }) '' @{
                        locked = [bool]$probe.locked
                        elapsed_ms = [int]$probe.elapsed_ms
                    }
                }
            }
            default { }
        }
        } catch {
            if ($results.Count -eq $beforeCount) {
                Add-Result $row.id 'fail' 'row failed' @{}
            }
        }
    }

    $bundlePath = Join-Path $env:TEMP ("softlock-hw-bundle-{0}.json" -f $PID)
    $stamp = $started.ToString('yyyyMMdd-HHmmss')
    $outDir = Join-Path $root "suite-output\$stamp"
    $bundle = '{"facts":' + $facts + ',"elapsed_ms":' + (Elapsed-Ms) + ',"rows":' + (ConvertTo-ResultJson) + '}'
    Write-Utf8 $bundlePath $bundle
    & $exe report --bundle $bundlePath --out-dir $outDir
    $code = $LASTEXITCODE
    Remove-Item -LiteralPath $factsPath, $planPath, $bundlePath, $powerFile -Force -ErrorAction SilentlyContinue
    exit $code
} finally {
    Restore-Suite
}
