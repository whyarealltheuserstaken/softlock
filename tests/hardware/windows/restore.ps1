# Restores HKCU (and HKLM, if this process may write it) values the suite changed.
# RunOnce calls this after a crash or a reboot. It does not unlock Windows.

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$pending = Join-Path $env:LOCALAPPDATA 'softlock-hwsuite\pending.json'
if (-not (Test-Path -LiteralPath $pending)) {
    exit 0
}

$snapshot = Get-Content -LiteralPath $pending -Raw | ConvertFrom-Json
$values = @()
if ($null -ne $snapshot.values) {
    $values = @($snapshot.values)
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$held = @()
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
    $key = $root.OpenSubKey([string]$entry.subkey, $true)
    if ($null -eq $key) {
        if (-not $entry.present) {
            continue
        }
        $key = $root.CreateSubKey([string]$entry.subkey)
    }
    if ($entry.present) {
        $key.SetValue([string]$entry.name, [int]$entry.data, [Microsoft.Win32.RegistryValueKind]::DWord)
    } else {
        $key.DeleteValue([string]$entry.name, $false)
    }
    $key.Close()
}

if ($held.Count -gt 0) {
    Write-Output 'An HKLM value is still set. Run this script again from an elevated prompt.'
    exit 0
}

Remove-Item -LiteralPath $pending -Force
$runOnce = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce', $true)
if ($null -ne $runOnce) {
    $runOnce.DeleteValue('softlock-hwsuite', $false)
    $runOnce.Close()
}
exit 0
