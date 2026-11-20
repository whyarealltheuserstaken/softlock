# Restores the lockdown values this probe changed.
# HKCU RunOnce calls this as the signed-in user. A SYSTEM task calls it after a crash
# so HKLM is restored too. HKCU is written under the saved account SID when the
# caller is not that account.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$pending = Join-Path $env:LOCALAPPDATA 'os-lock-probe\pending.json'
$taskName = 'os-lock-restore'
$runOnceName = 'os-lock-restore'
$cmdPath = 'C:\ProgramData\os-lock-probe\restore.cmd'

function Get-Items($value) {
    $list = New-Object System.Collections.Generic.List[object]
    if ($null -eq $value) { return ,$list }
    $typeName = $value.GetType().FullName
    $scalar = ($typeName -eq 'System.Management.Automation.PSCustomObject') -or ($value -is [string])
    if (-not $scalar -and ($value -is [System.Collections.IEnumerable])) {
        foreach ($item in $value) {
            if ($null -ne $item) { $list.Add($item) }
        }
    } else {
        $list.Add($value)
    }
    return ,$list
}

function Convert-RegistryData([string]$kind, [string]$data) {
    switch ($kind) {
        'DWord' { return [int]$data }
        'QWord' { return [int64]$data }
        default { return $data }
    }
}

function Open-UsersKey([string]$sid, [string]$subkey, [bool]$writable) {
    $path = $sid + '\' + $subkey
    $root = [Microsoft.Win32.Registry]::Users
    $key = $root.OpenSubKey($path, $writable)
    if ($null -eq $key -and $writable) { $key = $root.CreateSubKey($path) }
    return $key
}

function Open-Key([string]$hive, [string]$subkey, [bool]$writable, [string]$sid) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $current = $identity.User.Value
    if ($hive -eq 'HKLM') {
        $root = [Microsoft.Win32.Registry]::LocalMachine
        $key = $root.OpenSubKey($subkey, $writable)
        if ($null -eq $key -and $writable) { $key = $root.CreateSubKey($subkey) }
        return $key
    }
    if ($sid -and $current -ne $sid) { return (Open-UsersKey $sid $subkey $writable) }
    $root = [Microsoft.Win32.Registry]::CurrentUser
    $key = $root.OpenSubKey($subkey, $writable)
    if ($null -eq $key -and $writable) { $key = $root.CreateSubKey($subkey) }
    return $key
}

function Read-ValueText($key, [string]$name) {
    $exists = $false
    $kind = ''
    $data = ''
    if ($null -eq $key) { return [pscustomobject]@{ exists = $false; kind = ''; data = '' } }
    foreach ($existing in @($key.GetValueNames())) {
        if ($existing -ne $name) { continue }
        $exists = $true
        $kind = $key.GetValueKind($existing).ToString()
        $raw = $key.GetValue($existing, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($kind -eq 'DWord' -or $kind -eq 'QWord') { $data = "$raw" } else { $data = [string]$raw }
    }
    return [pscustomobject]@{ exists = $exists; kind = $kind; data = $data }
}

function Value-Matches($entry, [string]$sid) {
    $key = Open-Key ([string]$entry.hive) ([string]$entry.subkey) $false $sid
    $read = Read-ValueText $key ([string]$entry.name)
    if ($null -ne $key) { $key.Close() }
    if (-not ($entry.present -eq $true)) { return -not $read.exists }
    if (-not $read.exists) { return $false }
    $kindName = 'DWord'
    if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
    return ($read.kind -eq $kindName) -and ($read.data -eq [string]$entry.data)
}

function Test-CreatedKey($entry) {
    return ($entry.PSObject.Properties.Name -contains 'created_key') -and ($entry.created_key -eq $true)
}

function Remove-CreatedKey($entry, [string]$sid) {
    if (-not (Test-CreatedKey $entry)) { return }
    $key = Open-Key ([string]$entry.hive) ([string]$entry.subkey) $true $sid
    if ($null -eq $key) { return }
    $empty = ($key.ValueCount -eq 0 -and $key.SubKeyCount -eq 0)
    $key.Close()
    if (-not $empty) { return }
    $path = [string]$entry.subkey
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $useSid = $entry.hive -eq 'HKCU' -and $sid -and $identity.User.Value -ne $sid
    if ($useSid) {
        $parentPath = $sid
        $leaf = $path
        $slash = $path.LastIndexOf('\')
        if ($slash -ge 0) {
            $parentPath = $sid + '\' + $path.Substring(0, $slash)
            $leaf = $path.Substring($slash + 1)
        }
        $parent = [Microsoft.Win32.Registry]::Users.OpenSubKey($parentPath, $true)
        if ($null -eq $parent) { return }
        try { $parent.DeleteSubKey($leaf, $false) } finally { $parent.Close() }
        return
    }
    $root = if ($entry.hive -eq 'HKLM') { [Microsoft.Win32.Registry]::LocalMachine } else { [Microsoft.Win32.Registry]::CurrentUser }
    $slash = $path.LastIndexOf('\')
    if ($slash -lt 0) {
        $root.DeleteSubKey($path, $false)
        return
    }
    $parent = $root.OpenSubKey($path.Substring(0, $slash), $true)
    if ($null -eq $parent) { return }
    try { $parent.DeleteSubKey($path.Substring($slash + 1), $false) } finally { $parent.Close() }
}

function Remove-RunOnce {
    $runOnce = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce', $true)
    if ($null -eq $runOnce) { return }
    $runOnce.DeleteValue($runOnceName, $false)
    $runOnce.Close()
}

function Remove-RestoreTask {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & schtasks.exe /Delete /TN $taskName /F 2>&1 | Out-Null } finally { $ErrorActionPreference = $previous }
    if (Test-Path -LiteralPath $cmdPath) { Remove-Item -LiteralPath $cmdPath -Force -ErrorAction SilentlyContinue }
}

function Remove-ProbeService {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & sc.exe stop OsLockProbe 2>&1 | Out-Null
        & sc.exe delete OsLockProbe 2>&1 | Out-Null
    } finally { $ErrorActionPreference = $previous }
}

function Invoke-PendingRestore {
    if (-not (Test-Path -LiteralPath $pending)) {
        Remove-RunOnce
        Remove-RestoreTask
        Remove-ProbeService
        return 0
    }
    $snapshot = Get-Content -LiteralPath $pending -Raw | ConvertFrom-Json
    $sid = ''
    if ($snapshot.PSObject.Properties.Name -contains 'sid' -and $snapshot.sid) { $sid = [string]$snapshot.sid }
    $rawValues = $null
    if ($snapshot.PSObject.Properties.Name -contains 'values' -and $null -ne $snapshot.values) {
        $rawValues = $snapshot.values
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    $elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $system = $identity.User.Value -eq 'S-1-5-18'
    $held = New-Object System.Collections.Generic.List[object]
    $failed = $false
    foreach ($entry in (Get-Items $rawValues)) {
        $needsMachine = $entry.hive -eq 'HKLM'
        if ($needsMachine -and -not $elevated -and -not $system) {
            $held.Add($entry)
            continue
        }
        if (Value-Matches $entry $sid) {
            try { Remove-CreatedKey $entry $sid } catch {
                $failed = $true
                Write-Host "$($entry.hive)\$($entry.subkey) was created by the probe and was not deleted. $($_.Exception.Message)"
                $held.Add($entry)
            }
            continue
        }
        try {
            $key = Open-Key ([string]$entry.hive) ([string]$entry.subkey) $true $sid
            if ($null -eq $key) { throw 'the key could not be opened' }
            $created = -not ($entry.present -eq $true)
            if ($created) {
                $key.DeleteValue([string]$entry.name, $false)
            } else {
                $kindName = 'DWord'
                if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
                $data = if ($entry.PSObject.Properties.Name -contains 'data') { [string]$entry.data } else { '0' }
                $kind = [Microsoft.Win32.RegistryValueKind]$kindName
                $key.SetValue([string]$entry.name, (Convert-RegistryData $kindName $data), $kind)
            }
            $key.Close()
            if (-not (Value-Matches $entry $sid)) { throw 'the saved value was not put back' }
            Remove-CreatedKey $entry $sid
        } catch {
            $failed = $true
            $verb = if ($entry.present -eq $true) { 'was changed and was not put back' } else { 'was created by the probe and was not deleted' }
            Write-Host "$($entry.hive)\$($entry.subkey) value $($entry.name) $verb. $($_.Exception.Message)"
            $held.Add($entry)
        }
    }
    if ($held.Count -gt 0) {
        $items = @()
        foreach ($entry in $held) {
            $present = if ($entry.present -eq $true) { 'true' } else { 'false' }
            $kind = if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { [string]$entry.kind } else { 'DWord' }
            $data = if ($entry.PSObject.Properties.Name -contains 'data') { [string]$entry.data } else { '' }
            $created = 'false'
            if (Test-CreatedKey $entry) { $created = 'true' }
            $items += '{"hive":"' + [string]$entry.hive + '","subkey":"' + ([string]$entry.subkey).Replace('\', '\\').Replace('"', '\"') + '","name":"' + [string]$entry.name + '","present":' + $present + ',"kind":"' + $kind + '","data":"' + $data.Replace('\', '\\').Replace('"', '\"') + '","created_key":' + $created + '}'
        }
        $body = '{"sid":"' + $sid + '","values":[' + ($items -join ',') + ']}'
        $utf8 = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($pending, $body, $utf8)
        if (-not $elevated -and -not $system) {
            Write-Host "HKLM was not restored. From an elevated prompt run: powershell -ExecutionPolicy Bypass -File `"$PSCommandPath`""
        }
        if ($failed) { return 1 }
        return 0
    }
    Remove-Item -LiteralPath $pending -Force
    Remove-RunOnce
    Remove-RestoreTask
    Remove-ProbeService
    return 0
}

if ($MyInvocation.InvocationName -ne '.') {
    exit (Invoke-PendingRestore)
}
