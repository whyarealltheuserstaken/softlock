# Manual OS-lock check for Henry's laptop.
# Elevated. Uses the release build only. Does not pass cargo features.
# Does not reconnect a session. Does not stop an installed service.
#
# From a non-elevated prompt, in the repo root, fetch and build first:
#   cargo fetch
#   cargo build --release --locked --offline -p softlock-service -p softlock-lockhost -p softlock-app
# Then, elevated:
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
$script:Outcome = 'fail'
$script:Reason = 'interrupted'
$script:ExitCode = 1
$script:Started = New-Object System.Collections.Generic.List[int]
$script:Suspended = New-Object System.Collections.Generic.List[uint32]
$script:HostsAtStart = New-Object System.Collections.Generic.List[int]
$script:Service = $null
$script:SkipPipe = $false
$script:Cleaned = $false

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

function Read-BiosValue($item, [string]$name) {
    if ($null -eq $item) { return '' }
    $prop = $item.PSObject.Properties[$name]
    if ($null -eq $prop -or $null -eq $prop.Value) { return '' }
    return [string]$prop.Value
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
$family = Read-BiosValue $bios 'SystemFamily'
$version = Read-BiosValue $bios 'SystemVersion'
$product = Read-BiosValue $bios 'SystemProductName'
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
    [DllImport("ntdll.dll")]
    public static extern int NtResumeProcess(IntPtr process);
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
Write-Host "Building the release binaries offline. No extra cargo features."
Write-Host "If this fails, run cargo fetch from a non-elevated prompt, then build there."
& cargo build --release --locked --offline -p softlock-service -p softlock-lockhost -p softlock-app
if ($LASTEXITCODE -ne 0) { Refuse 'release-build-failed' }

function Read-Exact($stream, [byte[]]$buffer) {
    $got = 0
    while ($got -lt $buffer.Length) {
        $n = $stream.Read($buffer, $got, $buffer.Length - $got)
        if ($n -le 0) { return $false }
        $got += $n
    }
    return $true
}

function Send-SoftLock([string]$hello, [string]$status) {
    if ($script:SkipPipe) { return '' }
    if ($null -ne $script:Service -and $script:Service.HasExited) { return '' }
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'softlock-agent', [System.IO.Pipes.PipeDirection]::InOut)
    $pipe.Connect(200)
    $last = ''
    foreach ($json in @($hello, $status)) {
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
        $last = [Text.Encoding]::UTF8.GetString($buf)
    }
    $pipe.Dispose()
    return $last
}

function Get-SoftLockState {
    try {
        $status = Send-SoftLock '{"v":1,"id":1,"kind":"Hello","client_version":"0"}' '{"v":1,"id":2,"kind":"GetStatus"}'
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

function Resume-Suspended {
    foreach ($procId in $script:Suspended) {
        $handle = [SoftLockOsCheck]::OpenProcess(0x0800, $false, $procId)
        if ($handle -eq [IntPtr]::Zero) { continue }
        [void][SoftLockOsCheck]::NtResumeProcess($handle)
        [void][SoftLockOsCheck]::CloseHandle($handle)
    }
}

function Stop-Started {
    foreach ($procId in $script:Started) {
        Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
    }
}

function Stop-AppearedHosts {
    $hosts = @(Get-Process -Name 'softlock-lockhost' -ErrorAction SilentlyContinue)
    foreach ($proc in $hosts) {
        if ($script:HostsAtStart -notcontains $proc.Id) {
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        }
    }
}

function Complete-Check {
    if ($script:Cleaned) { return }
    $script:Cleaned = $true
    Resume-Suspended
    Stop-AppearedHosts
    Stop-Started
    Write-Result $script:Outcome $script:Reason
    Write-Host "Wrote $resultJson"
}

function Invoke-LaptopCheck {
    $installed = Get-Service -Name 'SoftLockAgent' -ErrorAction SilentlyContinue
    $running = @(Get-Process -Name 'softlock-service' -ErrorAction SilentlyContinue)
    if (($null -ne $installed -and $installed.Status -ne 'Stopped') -or $running.Count -gt 0) {
        $script:Outcome = 'refused'
        $script:Reason = 'installed-service: run Stop-Service SoftLockAgent from an elevated prompt, then run this script again. This script does not stop an installed service. After the check, run Start-Service SoftLockAgent.'
        $script:ExitCode = 2
        return
    }
    foreach ($proc in @(Get-Process -Name 'softlock-lockhost' -ErrorAction SilentlyContinue)) {
        $script:HostsAtStart.Add($proc.Id)
    }

    $serviceExe = Join-Path $root 'target\release\softlock-service.exe'
    $appExe = Join-Path $root 'target\release\softlock-app.exe'
    $script:Service = Start-Process -FilePath $serviceExe -ArgumentList '--foreground' -PassThru -WindowStyle Hidden
    $script:Started.Add($script:Service.Id)
    $app = Start-Process -FilePath $appExe -PassThru
    $script:Started.Add($app.Id)
    Start-Sleep -Seconds 2
    $script:Service.Refresh()
    if ($script:Service.HasExited) {
        $script:Outcome = 'refused'
        $script:Reason = 'release-service-exited'
        $script:ExitCode = 2
        return
    }

    if (Test-OsConfirmed) {
        $script:Outcome = 'refused'
        $script:Reason = 'already-locked'
        $script:ExitCode = 2
        return
    }
    Write-Host "Press Enter, then choose Lock for real within 15 s."
    $poll = [Diagnostics.Stopwatch]::StartNew()
    $enteredAt = $null
    $firstLock = $null
    while ($true) {
        if ($null -eq $enteredAt -and [Console]::KeyAvailable) {
            $key = [Console]::ReadKey($true)
            if ($key.Key -eq 'Enter') { $enteredAt = $poll.Elapsed }
        }
        if ($null -eq $firstLock -and (Test-OsConfirmed)) {
            $firstLock = $poll.Elapsed
        }
        if ($null -ne $enteredAt) {
            if ($null -ne $firstLock) { break }
            if (($poll.Elapsed - $enteredAt).TotalSeconds -ge 15) { break }
        }
        Start-Sleep -Milliseconds 50
    }
    $elapsed = 0
    if ($null -ne $firstLock -and $null -ne $enteredAt) {
        $delta = [int]($firstLock - $enteredAt).TotalMilliseconds
        if ($delta -gt 0) { $elapsed = $delta }
    } elseif ($null -eq $firstLock -and $null -ne $enteredAt) {
        $elapsed = [int]($poll.Elapsed - $enteredAt).TotalMilliseconds
    }
    $script:Rows.Add([pscustomobject]@{
        name = 'tray-hard-lock'
        elapsed_ms = $elapsed
        confirmed = ($null -ne $firstLock)
        unlocked = $false
        detail = $(if ($null -ne $firstLock) { 'first locked reading' } else { 'no lock within 15 s' })
    })
    if ($null -ne $firstLock) {
        Write-Host "Sign in with Windows Hello."
        Read-Host "Press Enter when the desktop is back"
    }

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
        while ($watch.ElapsedMilliseconds -lt 1000) {
            if (Test-OsConfirmed) {
                $confirmed = $true
                break
            }
            if (-not $script:SkipPipe) {
                $state = Get-SoftLockState
                if ($state -eq 'Unlocked') {
                    $unlocked = $true
                    $script:SawUnlocked = $true
                }
            }
            Start-Sleep -Milliseconds 20
        }
        $elapsed = [int]$watch.Elapsed.TotalMilliseconds
        if ($elapsed -ge 1000) { $confirmed = $false }
        $script:Rows.Add([pscustomobject]@{
            name = $name
            elapsed_ms = $elapsed
            confirmed = $confirmed
            unlocked = $unlocked
            detail = $(if ($confirmed) { 'os-locked or disconnected' } else { 'not confirmed within 1000 ms' })
        })
        Write-Host "$name elapsed_ms=$elapsed confirmed=$confirmed unlocked=$unlocked"
        if (-not $confirmed) { return }
        Write-Host "Sign in with Windows Hello."
        Read-Host "Press Enter when the desktop is back"
    }

    Invoke-Fault 'kill-host' {
        $hosts = @(Get-Process -Name 'softlock-lockhost' -ErrorAction SilentlyContinue)
        foreach ($proc in $hosts) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    }

    Invoke-Fault 'suspend-host' {
        $hosts = @(Get-Process -Name 'softlock-lockhost' -ErrorAction SilentlyContinue)
        foreach ($proc in $hosts) {
            $handle = [SoftLockOsCheck]::OpenProcess(0x0800, $false, [uint32]$proc.Id)
            if ($handle -eq [IntPtr]::Zero) { continue }
            [void][SoftLockOsCheck]::NtSuspendProcess($handle)
            [void][SoftLockOsCheck]::CloseHandle($handle)
            $script:Suspended.Add([uint32]$proc.Id)
        }
    }

    Invoke-Fault 'kill-service' {
        $script:SkipPipe = $true
        if ($null -ne $script:Service -and -not $script:Service.HasExited) {
            Stop-Process -Id $script:Service.Id -Force -ErrorAction SilentlyContinue
        }
    }

    $failed = @($script:Rows | Where-Object { -not $_.confirmed -or $_.unlocked -or ($_.name -ne 'tray-hard-lock' -and $_.elapsed_ms -ge 1000) })
    if ($failed.Count -gt 0 -or $script:SawUnlocked) {
        $script:Outcome = 'fail'
        $script:Reason = 'a case missed the OS lock, took 1000 ms or more, or read Unlocked'
        $script:ExitCode = 1
        return
    }
    $script:Outcome = 'pass'
    $script:Reason = 'os-locked or disconnected, and SoftLock did not read Unlocked'
    $script:ExitCode = 0
}

trap {
    if ($script:Outcome -eq 'fail' -and $script:Reason -eq 'interrupted') {
        $script:Reason = 'interrupted'
    }
    if ($script:ExitCode -eq 0) { $script:ExitCode = 1 }
    Complete-Check
    break
}

try {
    [Console]::CancelKeyPress.Add({
        param($sender, $eventArgs)
        $eventArgs.Cancel = $true
        if ($script:Outcome -eq 'fail' -and $script:Reason -eq 'interrupted') {
            $script:Reason = 'interrupted'
        }
        if ($script:ExitCode -eq 0) { $script:ExitCode = 1 }
        Complete-Check
        [Environment]::Exit($script:ExitCode)
    })
} catch {
}

try {
    Invoke-LaptopCheck
} catch {
    if ($script:Outcome -eq 'fail' -and $script:Reason -eq 'interrupted') {
        $script:Reason = $_.Exception.Message
    }
    if ($script:ExitCode -eq 0) { $script:ExitCode = 1 }
} finally {
    Complete-Check
}

exit $script:ExitCode
