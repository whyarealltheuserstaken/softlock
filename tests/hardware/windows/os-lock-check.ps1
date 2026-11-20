# Manual OS-lock check for Henry's laptop.
# Elevated. Uses the release build only. Does not pass cargo features.
# Does not reconnect a session.
#
#   powershell -ExecutionPolicy Bypass -File tests\hardware\windows\os-lock-check.ps1

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$started = [DateTime]::UtcNow
$stamp = $started.ToString('yyyyMMdd-HHmmss')
$outDir = Join-Path $root "suite-output\$stamp"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$resultJson = Join-Path $outDir 'result.json'
$resultTxt = Join-Path $outDir 'result.txt'
$script:Rows = New-Object System.Collections.Generic.List[object]
$script:SawUnlocked = $false

function Write-Result([string]$status, [string]$reason) {
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("status=$status")
    $lines.Add("reason=$reason")
    $lines.Add("unlocked=$($script:SawUnlocked)")
    foreach ($row in $script:Rows) {
        $lines.Add("$($row.name) elapsed_ms=$($row.elapsed_ms) confirmed=$($row.confirmed) unlocked=$($row.unlocked) detail=$($row.detail)")
    }
    $text = ($lines -join "`n") + "`n"
    [System.IO.File]::WriteAllText($resultTxt, $text, (New-Object System.Text.UTF8Encoding $false))
    $cases = @()
    foreach ($row in $script:Rows) {
        $cases += '{"name":"' + $row.name + '","elapsed_ms":' + $row.elapsed_ms + ',"confirmed":' + ($(if ($row.confirmed) { 'true' } else { 'false' })) + ',"unlocked":' + ($(if ($row.unlocked) { 'true' } else { 'false' })) + ',"detail":"' + ($row.detail -replace '"','\"') + '"}'
    }
    $body = '{"status":"' + $status + '","reason":"' + ($reason -replace '"','\"') + '","unlocked":' + ($(if ($script:SawUnlocked) { 'true' } else { 'false' })) + ',"cases":[' + ($cases -join ',') + ']}'
    [System.IO.File]::WriteAllText($resultJson, $body + "`n", (New-Object System.Text.UTF8Encoding $false))
}

function Refuse([string]$reason) {
    Write-Host "refusing: $reason"
    Write-Result 'refused' $reason
    exit 2
}

if ($env:GITHUB_ACTIONS -eq 'true') {
    Refuse 'this check is for the laptop, not a runner'
}

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Refuse 'not-elevated'
}

$biosKey = 'HKLM:\HARDWARE\DESCRIPTION\System\BIOS'
$bios = Get-ItemProperty -Path $biosKey -ErrorAction SilentlyContinue
if ($null -eq $bios) { Refuse 'identity-unreadable' }
$family = [string]$bios.SystemFamily
$version = [string]$bios.SystemVersion
$product = [string]$bios.SystemProductName
if ([string]::IsNullOrWhiteSpace($family) -or [string]::IsNullOrWhiteSpace($version) -or [string]::IsNullOrWhiteSpace($product)) {
    Refuse 'identity-unreadable'
}
$identity = "$family $version $product"
if ($identity -match 'ThinkStation P7' -or $identity -match '30F2' -or $identity -match '30F3') {
    Refuse 'school-workstation'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SoftLockOsCheck {
    [DllImport("kernel32.dll")]
    public static extern uint WTSGetActiveConsoleSessionId();
    [DllImport("kernel32.dll")]
    public static extern bool ProcessIdToSessionId(uint pid, out uint session);
    [DllImport("wtsapi32.dll", CharSet = CharSet.Unicode)]
    public static extern bool WTSQuerySessionInformationW(IntPtr server, int session, int klass, out IntPtr buffer, out uint bytes);
    [DllImport("wtsapi32.dll")]
    public static extern void WTSFreeMemory(IntPtr memory);
    [DllImport("kernel32.dll")]
    public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")]
    public static extern bool CloseHandle(IntPtr handle);
    [DllImport("ntdll.dll")]
    public static extern int NtSuspendProcess(IntPtr process);
    public static bool CurrentIsConsole() {
        uint session;
        if (!ProcessIdToSessionId((uint)System.Diagnostics.Process.GetCurrentProcess().Id, out session)) return false;
        return session == WTSGetActiveConsoleSessionId();
    }
    public static int ConnectState() {
        IntPtr buffer;
        uint bytes;
        if (!WTSQuerySessionInformationW(IntPtr.Zero, -1, 8, out buffer, out bytes) || buffer == IntPtr.Zero) return -1;
        try { return Marshal.ReadInt32(buffer); }
        finally { WTSFreeMemory(buffer); }
    }
    public static int SessionFlags() {
        IntPtr buffer;
        uint bytes;
        if (!WTSQuerySessionInformationW(IntPtr.Zero, -1, 25, out buffer, out bytes) || buffer == IntPtr.Zero || bytes < 20) return -1;
        try { return Marshal.ReadInt32(buffer, 16); }
        finally { WTSFreeMemory(buffer); }
    }
}
'@

if (-not [SoftLockOsCheck]::CurrentIsConsole()) {
    Refuse 'not-console'
}
$connect = [SoftLockOsCheck]::ConnectState()
# WTSConnectState: WTSActive = 0. Anything else, including a remote or disconnected session, stops the check.
if ($connect -ne 0) {
    Refuse 'remote-session'
}

Set-Location -LiteralPath $root
Write-Host "Building the release binaries. No extra cargo features."
& cargo build --release --locked -p softlock-service -p softlock-lockhost -p softlock-app
if ($LASTEXITCODE -ne 0) { Refuse 'release-build-failed' }

$serviceExe = Join-Path $root 'target\release\softlock-service.exe'
$appExe = Join-Path $root 'target\release\softlock-app.exe'
$service = Start-Process -FilePath $serviceExe -ArgumentList '--foreground' -PassThru -WindowStyle Hidden
$app = Start-Process -FilePath $appExe -PassThru
Start-Sleep -Seconds 2

function Read-Exact($stream, [byte[]]$buffer) {
    $got = 0
    while ($got -lt $buffer.Length) {
        $n = $stream.Read($buffer, $got, $buffer.Length - $got)
        if ($n -le 0) { return $false }
        $got += $n
    }
    return $true
}

function Send-SoftLock([string]$json) {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'softlock-agent', [System.IO.Pipes.PipeDirection]::InOut)
    $pipe.Connect(2000)
    $body = [Text.Encoding]::UTF8.GetBytes($json)
    $prefix = [BitConverter]::GetBytes([uint32]$body.Length)
    $pipe.Write($prefix, 0, 4)
    $pipe.Write($body, 0, $body.Length)
    $pipe.Flush()
    $lenBuf = New-Object byte[] 4
    if (-not (Read-Exact $pipe $lenBuf)) { $pipe.Dispose(); return '' }
    $len = [BitConverter]::ToUInt32($lenBuf, 0)
    $buf = New-Object byte[] $len
    if (-not (Read-Exact $pipe $buf)) { $pipe.Dispose(); return '' }
    $pipe.Dispose()
    return [Text.Encoding]::UTF8.GetString($buf)
}

function Get-SoftLockState {
    try {
        [void](Send-SoftLock '{"v":1,"id":1,"kind":"Hello","client_version":"0"}')
        $status = Send-SoftLock '{"v":1,"id":2,"kind":"GetStatus"}'
        $match = [regex]::Match($status, '"state":"([^"]+)"')
        if ($match.Success) { return $match.Groups[1].Value }
    } catch {
        return ''
    }
    return ''
}

function Test-OsConfirmed {
    $flags = [SoftLockOsCheck]::SessionFlags()
    $state = [SoftLockOsCheck]::ConnectState()
    # SessionFlags 0 is the workstation lock. Connect state 4 is disconnected.
    return ($flags -eq 0 -or $state -eq 4)
}

function Wait-SoftLock([string]$wanted) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $state = Get-SoftLockState
        if ($state -eq $wanted) { return $true }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

Write-Host ""
Write-Host "1. In the release tray, choose Lock for real."
Write-Host "   The Windows sign-in screen should appear. Sign in with Windows Hello."
Read-Host "Press Enter after you are signed back in"
$trayLocked = $false
$deadline = [DateTime]::UtcNow.AddSeconds(2)
while ([DateTime]::UtcNow -lt $deadline) {
    if (Test-OsConfirmed) { $trayLocked = $true }
    Start-Sleep -Milliseconds 100
}
$script:Rows.Add([pscustomobject]@{
    name = 'tray-hard-lock'
    elapsed_ms = 0
    confirmed = $trayLocked
    unlocked = $false
    detail = $(if ($trayLocked) { 'session was locked or disconnected' } else { 'sign-in was not observed after the prompt' })
})

function Invoke-Fault([string]$name, [scriptblock]$action) {
    Write-Host ""
    Write-Host "Lock SoftLock from the tray (the normal lock, not Lock for real)."
    Read-Host "Press Enter when the overlay is up"
    if (-not (Wait-SoftLock 'Locked')) {
        $script:Rows.Add([pscustomobject]@{ name = $name; elapsed_ms = 0; confirmed = $false; unlocked = $false; detail = 'did not read Locked' })
        return
    }
    & $action
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $confirmed = $false
    $unlocked = $false
    while ($watch.Elapsed.TotalSeconds -lt 5) {
        $state = Get-SoftLockState
        if ($state -eq 'Unlocked') {
            $unlocked = $true
            $script:SawUnlocked = $true
        }
        if (Test-OsConfirmed) {
            $confirmed = $true
            break
        }
        Start-Sleep -Milliseconds 50
    }
    $script:Rows.Add([pscustomobject]@{
        name = $name
        elapsed_ms = [int]$watch.Elapsed.TotalMilliseconds
        confirmed = $confirmed
        unlocked = $unlocked
        detail = $(if ($confirmed) { 'os-locked or disconnected' } else { 'not confirmed within 5s' })
    })
    Write-Host "$name elapsed_ms=$([int]$watch.Elapsed.TotalMilliseconds) confirmed=$confirmed unlocked=$unlocked"
    if (-not $confirmed) { return }
    Write-Host "Sign in with Windows Hello if Windows is still locked."
    Read-Host "Press Enter when the desktop is back"
}

Invoke-Fault 'kill-host' {
    $hosts = @(Get-Process -Name 'softlock-lockhost' -ErrorAction SilentlyContinue)
    foreach ($proc in $hosts) { Stop-Process -Id $proc.Id -Force }
}

Invoke-Fault 'suspend-host' {
    # NtSuspendProcess suspends every thread of the Lock Host. This check does not resume it
    # and does not reconnect the session. Stop-Process ends it after the measurement.
    $hosts = @(Get-Process -Name 'softlock-lockhost' -ErrorAction SilentlyContinue)
    foreach ($proc in $hosts) {
        # PROCESS_SUSPEND_RESUME. The handle is closed after the call. The process stays suspended.
        $handle = [SoftLockOsCheck]::OpenProcess(0x0800, $false, [uint32]$proc.Id)
        if ($handle -eq [IntPtr]::Zero) { continue }
        [void][SoftLockOsCheck]::NtSuspendProcess($handle)
        [void][SoftLockOsCheck]::CloseHandle($handle)
    }
}

Invoke-Fault 'kill-service' {
    if ($null -ne $service -and -not $service.HasExited) {
        Stop-Process -Id $service.Id -Force
    }
    Get-Process -Name 'softlock-service' -ErrorAction SilentlyContinue | Stop-Process -Force
}

if ($null -ne $app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
Get-Process -Name 'softlock-lockhost','softlock-service','softlock-app' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$failed = @($script:Rows | Where-Object { -not $_.confirmed -or $_.unlocked })
if ($failed.Count -gt 0 -or $script:SawUnlocked) {
    Write-Result 'fail' 'a case missed the OS lock or read Unlocked'
    exit 1
}
Write-Result 'pass' 'os-locked or disconnected, and SoftLock did not read Unlocked'
Write-Host "Wrote $resultJson"
exit 0
