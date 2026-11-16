# Proves the helper, the console-break handler, or the panic hook put the eight
# policy values back. taskkill uses /F /PID and not /T, so the helper survives.
param(
    [Parameter(Mandatory = $true)][ValidateSet('kill', 'ctrl', 'panic')][string]$Mode,
    [Parameter(Mandatory = $true)][string]$Exe
)
$ErrorActionPreference = 'Stop'
# A missing service makes sc.exe return 1060. That is the success case.
$PSNativeCommandUseErrorActionPreference = $false
$root = (Get-Location).Path
$exePath = (Resolve-Path $Exe).Path
$out = Join-Path $root "restore-$Mode"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force -Path $out | Out-Null

function Wait-Armed([string]$Dir) {
    $path = Join-Path $Dir 'armed.json'
    $deadline = (Get-Date).AddSeconds(90)
    while (-not (Test-Path $path)) {
        if ((Get-Date) -gt $deadline) {
            throw "armed.json did not appear in $Dir"
        }
        Start-Sleep -Milliseconds 200
    }
}

function Assert-Restored([string]$Dir, [string]$Label) {
    $deadline = (Get-Date).AddSeconds(60)
    $last = ''
    $code = 1
    while ((Get-Date) -lt $deadline) {
        $last = & $exePath --check-restored --output-dir $Dir 2>&1 | Out-String
        $code = $LASTEXITCODE
        if ($code -eq 0) { break }
        Start-Sleep -Milliseconds 400
    }
    if ($code -ne 0) { throw "$Label check-restored failed: $last" }
    Write-Output $last.TrimEnd()
    $revert = & $exePath --revert --output-dir $Dir 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or $revert -notmatch 'journal=absent') {
        throw "$Label revert did not report an absent journal: $revert"
    }
    Write-Output $revert.TrimEnd()
    & sc.exe query FailClosedProbe | Out-Null
    if ($LASTEXITCODE -eq 0) { throw "$Label service still installed" }
    Write-Output "$Label policies=original journal=absent service=gone"
}

function Stop-Probe([int]$ProbePid) {
    if ($ProbePid -le 0) { return }
    & taskkill.exe /F /PID $ProbePid | Out-Null
}

if ($Mode -eq 'kill') {
    $proc = Start-Process -FilePath $exePath -ArgumentList @('--arm-and-wait', '--output-dir', $out) -WorkingDirectory $root -PassThru -NoNewWindow
    try {
        Wait-Armed $out
        # /PID only. /T would kill the helper that restores the policies.
        & taskkill.exe /F /PID $proc.Id
        if ($LASTEXITCODE -ne 0) { throw "taskkill /F failed: $LASTEXITCODE" }
    }
    catch {
        Stop-Probe $proc.Id
        throw
    }
    Assert-Restored $out 'restore-step=kill'
    exit 0
}

if ($Mode -eq 'panic') {
    $proc = Start-Process -FilePath $exePath -ArgumentList @('--arm-and-wait', '--panic-after-arm', '--output-dir', $out) -WorkingDirectory $root -PassThru -NoNewWindow
    try {
        if (-not $proc.WaitForExit(90000)) { throw 'panic probe did not exit' }
    }
    catch {
        Stop-Probe $proc.Id
        throw
    }
    Assert-Restored $out 'restore-step=panic'
    exit 0
}

# CTRL_BREAK_EVENT can target one process group. CTRL_C_EVENT cannot.
# The child shares this console and is created with CREATE_NEW_PROCESS_GROUP.
Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class ProbeCtrl {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct STARTUPINFO {
        public int cb;
        public IntPtr lpReserved;
        public IntPtr lpDesktop;
        public IntPtr lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow;
        public short cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION {
        public IntPtr hProcess, hThread;
        public int dwProcessId, dwThreadId;
    }
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GenerateConsoleCtrlEvent(uint evt, uint group);
    [DllImport("kernel32.dll")]
    static extern bool SetConsoleCtrlHandler(IntPtr handler, bool add);
    [DllImport("kernel32.dll")]
    static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);
    static IntPtr Handle;
    public static int Start(string command, string dir) {
        var cmd = new StringBuilder(command, 32768);
        var si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        PROCESS_INFORMATION pi;
        if (!CreateProcessW(null, cmd, IntPtr.Zero, IntPtr.Zero, false, 0x00000200, IntPtr.Zero, dir, ref si, out pi)) {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        CloseHandle(pi.hThread);
        Handle = pi.hProcess;
        return pi.dwProcessId;
    }
    public static void Break(int pid) {
        SetConsoleCtrlHandler(IntPtr.Zero, true);
        if (!GenerateConsoleCtrlEvent(1, (uint)pid)) {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }
    public static void WaitExit() {
        uint result = WaitForSingleObject(Handle, 60000);
        CloseHandle(Handle);
        Handle = IntPtr.Zero;
        if (result != 0) {
            throw new InvalidOperationException("probe did not exit after CTRL_BREAK_EVENT, wait=" + result);
        }
    }
}
"@

$quoted = '"' + $exePath + '"'
$command = "$quoted --arm-and-wait --output-dir `"$out`""
$probePid = [ProbeCtrl]::Start($command, $root)
try {
    Wait-Armed $out
    [ProbeCtrl]::Break($probePid)
    [ProbeCtrl]::WaitExit()
}
catch {
    Stop-Probe $probePid
    throw
}
Assert-Restored $out 'restore-step=ctrl'
exit 0
