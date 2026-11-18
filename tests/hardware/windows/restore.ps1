# Restores values and power-button actions the suite changed.
# HKCU RunOnce calls this after a crash. HKLM needs an elevated prompt.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$pending = Join-Path $env:LOCALAPPDATA 'softlock-hwsuite\pending.json'
if (-not (Test-Path -LiteralPath $pending)) {
    exit 0
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

$snapshot = Get-Content -LiteralPath $pending -Raw | ConvertFrom-Json
$values = @()
if ($snapshot.PSObject.Properties.Name -contains 'values' -and $null -ne $snapshot.values) {
    $values = @($snapshot.values)
}
$power = @()
if ($snapshot.PSObject.Properties.Name -contains 'power' -and $null -ne $snapshot.power) {
    $power = @($snapshot.power)
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$held = @()
$failed = $false
foreach ($entry in $values) {
    if ($entry.hive -eq 'HKLM' -and -not $elevated) {
        $held += $entry
        continue
    }
    $root = if ($entry.hive -eq 'HKLM') {
        [Microsoft.Win32.Registry]::LocalMachine
    } else {
        [Microsoft.Win32.Registry]::CurrentUser
    }
    try {
        $key = $root.OpenSubKey([string]$entry.subkey, $true)
        if ($null -eq $key) {
            if (-not $entry.present) { continue }
            $key = $root.CreateSubKey([string]$entry.subkey)
        }
        if ($entry.present) {
            $kindName = 'DWord'
            if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
            $data = if ($entry.PSObject.Properties.Name -contains 'data') { [string]$entry.data } else { '0' }
            $kind = [Microsoft.Win32.RegistryValueKind]$kindName
            $key.SetValue([string]$entry.name, (Convert-RegistryData $kindName $data), $kind)
        } else {
            $key.DeleteValue([string]$entry.name, $false)
        }
        $key.Close()
    } catch {
        $failed = $true
        Write-Output "$($entry.hive)\$($entry.subkey) $($entry.name): $($_.Exception.Message)"
        $held += $entry
    }
}

$powerLeft = @()
foreach ($entry in $power) {
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    $setting = [string]$entry.setting
    $acText = Read-Native { powercfg /SETACVALUEINDEX SCHEME_CURRENT $sub $setting ([int]$entry.ac) }
    $acCode = $LASTEXITCODE
    $dcText = Read-Native { powercfg /SETDCVALUEINDEX SCHEME_CURRENT $sub $setting ([int]$entry.dc) }
    $dcCode = $LASTEXITCODE
    $active = Read-Native { powercfg /SETACTIVE SCHEME_CURRENT }
    if ($acCode -ne 0 -or $dcCode -ne 0 -or $LASTEXITCODE -ne 0) {
        $failed = $true
        Write-Output "powercfg $setting failed: $acText $dcText $active"
        $powerLeft += $entry
    }
}

if ($held.Count -gt 0 -or $powerLeft.Count -gt 0) {
    $items = @()
    foreach ($entry in $held) {
        $present = if ($entry.present) { 'true' } else { 'false' }
        $kind = if ($entry.PSObject.Properties.Name -contains 'kind') { [string]$entry.kind } else { 'DWord' }
        $data = if ($entry.PSObject.Properties.Name -contains 'data') { [string]$entry.data } else { '0' }
        $items += '{"hive":"' + $entry.hive + '","subkey":"' + ([string]$entry.subkey).Replace('\', '\\').Replace('"', '\"') + '","name":"' + $entry.name + '","present":' + $present + ',"kind":"' + $kind + '","data":"' + ($data.Replace('\', '\\').Replace('"', '\"')) + '"}'
    }
    $powerJson = @()
    foreach ($entry in $powerLeft) {
        $powerJson += '{"setting":"' + $entry.setting + '","ac":' + [int]$entry.ac + ',"dc":' + [int]$entry.dc + '}'
    }
    $body = '{"values":[' + ($items -join ',') + '],"power":[' + ($powerJson -join ',') + ']}'
    [System.IO.File]::WriteAllText($pending, $body, (New-Object System.Text.UTF8Encoding $false))
    $command = "powershell -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    if (-not $elevated) {
        Write-Output "HKLM was not restored. From an elevated prompt run: $command"
    }
    if ($failed) { exit 1 }
    exit 0
}

Remove-Item -LiteralPath $pending -Force
$runOnce = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce', $true)
if ($null -ne $runOnce) {
    $runOnce.DeleteValue('softlock-hwsuite', $false)
    $runOnce.Close()
}
exit 0
