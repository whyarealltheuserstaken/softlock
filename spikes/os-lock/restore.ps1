# Restores the eight lockdown values this probe is allowed to change.
# The journal is only C:\ProgramData\os-lock-probe\pending.json. A SYSTEM task
# passes that path. HKCU is written under the saved account SID when the caller
# is not that account. The task is removed only after the three HKLM values match.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script:JournalPath = 'C:\ProgramData\os-lock-probe\pending.json'
$taskName = 'os-lock-restore'
$runOnceName = 'os-lock-restore'
$cmdPath = 'C:\ProgramData\os-lock-probe\restore.cmd'
$hkcuSystem = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'
$hkcuExplorer = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'
$hklmSystem = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'

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

function Test-AllowedEntry($entry) {
    $hive = [string]$entry.hive
    $sub = [string]$entry.subkey
    $name = [string]$entry.name
    $kind = 'DWord'
    if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kind = [string]$entry.kind }
    if ($kind -ne 'DWord') { return $false }
    if ($hive -eq 'HKCU' -and $sub -eq $hkcuSystem) {
        return $name -eq 'DisableTaskMgr' -or $name -eq 'DisableLockWorkstation' -or $name -eq 'DisableChangePassword'
    }
    if ($hive -eq 'HKCU' -and $sub -eq $hkcuExplorer) {
        return $name -eq 'NoLogoff' -or $name -eq 'NoClose'
    }
    if ($hive -eq 'HKLM' -and $sub -eq $hklmSystem) {
        return $name -eq 'HideFastUserSwitching' -or $name -eq 'ConsentPromptBehaviorAdmin' -or $name -eq 'ConsentPromptBehaviorUser'
    }
    return $false
}

function Convert-DwordText([string]$data) {
    $number = 0
    if (-not [int]::TryParse($data, [ref]$number)) { return $null }
    return [int]$number
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
        if ($kind -eq 'DWord') {
            $data = "$([int]$raw)"
        } else {
            $data = ''
        }
    }
    return [pscustomobject]@{ exists = $exists; kind = $kind; data = $data }
}

function Value-Matches($entry, [string]$sid) {
    $key = Open-Key ([string]$entry.hive) ([string]$entry.subkey) $false $sid
    $read = Read-ValueText $key ([string]$entry.name)
    if ($null -ne $key) { $key.Close() }
    if (-not ($entry.present -eq $true)) { return -not $read.exists }
    if (-not $read.exists) { return $false }
    if ($read.kind -ne 'DWord') { return $false }
    $wanted = Convert-DwordText ([string]$entry.data)
    if ($null -eq $wanted) { return $false }
    $got = Convert-DwordText $read.data
    if ($null -eq $got) { return $false }
    return $got -eq $wanted
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

function Find-Allowed([string]$hive, [string]$name) {
    foreach ($entry in $script:Allowed) {
        if (([string]$entry.hive) -eq $hive -and ([string]$entry.name) -eq $name) { return $entry }
    }
    return $null
}

function Test-HklmConfirmed([string]$sid) {
    foreach ($name in @('HideFastUserSwitching', 'ConsentPromptBehaviorAdmin', 'ConsentPromptBehaviorUser')) {
        $found = Find-Allowed 'HKLM' $name
        if ($null -eq $found) { return $false }
        if (-not (Value-Matches $found $sid)) { return $false }
    }
    return $true
}

function Test-HkcuConfirmed([string]$sid) {
    foreach ($name in @('DisableTaskMgr', 'DisableLockWorkstation', 'DisableChangePassword', 'NoLogoff', 'NoClose')) {
        $found = Find-Allowed 'HKCU' $name
        if ($null -eq $found) { return $false }
        if (-not (Value-Matches $found $sid)) { return $false }
    }
    return $true
}

function Invoke-PendingRestore {
    if (-not (Test-Path -LiteralPath $script:JournalPath)) {
        Write-Host 'probe journal is missing; the restore task was left in place'
        return 1
    }
    $snapshot = Get-Content -LiteralPath $script:JournalPath -Raw | ConvertFrom-Json
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
    $script:Allowed = New-Object System.Collections.Generic.List[object]
    foreach ($entry in (Get-Items $rawValues)) {
        if (-not (Test-AllowedEntry $entry)) {
            Write-Host "ignored $([string]$entry.hive)\$([string]$entry.subkey) $([string]$entry.name)"
            continue
        }
        $script:Allowed.Add($entry)
    }
    $failed = $false
    foreach ($entry in $script:Allowed) {
        $needsMachine = ([string]$entry.hive) -eq 'HKLM'
        if ($needsMachine -and -not $elevated -and -not $system) { continue }
        if (Value-Matches $entry $sid) { continue }
        try {
            $key = Open-Key ([string]$entry.hive) ([string]$entry.subkey) $true $sid
            if ($null -eq $key) { throw 'the key could not be opened' }
            if (-not ($entry.present -eq $true)) {
                $key.DeleteValue([string]$entry.name, $false)
            } else {
                $data = Convert-DwordText ([string]$entry.data)
                if ($null -eq $data) { throw 'the saved value is not a DWORD' }
                $key.SetValue([string]$entry.name, $data, [Microsoft.Win32.RegistryValueKind]::DWord)
            }
            $key.Close()
            if (-not (Value-Matches $entry $sid)) { throw 'the saved value was not put back' }
        } catch {
            $failed = $true
            Write-Host "$([string]$entry.hive)\$([string]$entry.subkey) value $([string]$entry.name) was not put back. $($_.Exception.Message)"
        }
    }
    $hkcuOk = Test-HkcuConfirmed $sid
    $hklmOk = Test-HklmConfirmed $sid
    if ($hkcuOk) { Remove-RunOnce }
    if ($hklmOk) {
        Remove-Item -LiteralPath $script:JournalPath -Force
        Remove-RestoreTask
        Remove-ProbeService
        return 0
    }
    if (-not $elevated -and -not $system) {
        Write-Host 'HKLM was not restored. The SYSTEM logon task still has the journal.'
    }
    if ($failed) { return 1 }
    return 1
}

if ($MyInvocation.InvocationName -ne '.') {
    $given = ''
    for ($i = 0; $i -lt $args.Count; $i++) {
        if ($args[$i] -eq '-Journal' -and ($i + 1) -lt $args.Count) {
            $given = [string]$args[$i + 1]
        }
    }
    if ($given -ne $script:JournalPath) {
        Write-Host 'journal path is not the probe journal'
        exit 2
    }
    exit (Invoke-PendingRestore)
}
