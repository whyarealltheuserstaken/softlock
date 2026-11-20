# OS-lock proof. An elevated prompt on the console. Does not unlock Windows.
# --ci is the hosted-runner job. --self-check exercises the real lockdown values
# and the SYSTEM restore task, and only on a GitHub-hosted runner.

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

function Read-BiosString([string]$name) {
    $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('HARDWARE\DESCRIPTION\System\BIOS')
    if ($null -eq $key) { return $null }
    try {
        $value = $key.GetValue($name, $null)
    } finally {
        $key.Close()
    }
    if ($null -eq $value) { return $null }
    $text = [string]$value
    if ($text.Length -eq 0) { return $null }
    return $text
}

function Test-MtmPrefix([string]$value) {
    $upper = $value.ToUpperInvariant()
    foreach ($token in ($upper -split '[^A-Za-z0-9]')) {
        if ($token.StartsWith('30F2') -or $token.StartsWith('30F3')) { return $true }
    }
    return $false
}

function Test-SchoolWorkstation {
    $family = Read-BiosString 'SystemFamily'
    $version = Read-BiosString 'SystemVersion'
    $product = Read-BiosString 'SystemProductName'
    if ($null -eq $family -or $null -eq $version -or $null -eq $product) {
        $missing = @()
        if ($null -eq $family) { $missing += 'SystemFamily' }
        if ($null -eq $version) { $missing += 'SystemVersion' }
        if ($null -eq $product) { $missing += 'SystemProductName' }
        return ('unreadable ' + ($missing -join ' '))
    }
    foreach ($value in @($family, $version, $product)) {
        if ($value.ToLowerInvariant().Contains('thinkstation p7')) { return 'school' }
        if (Test-MtmPrefix $value) { return 'school' }
    }
    return 'no'
}

$school = Test-SchoolWorkstation
if ($school.StartsWith('unreadable')) {
    $fields = $school.Substring(11).Trim()
    Write-Host "refusing: workstation identity could not be read: $fields"
    exit 2
}
if ($school -eq 'school') {
    Write-Host 'refusing: school workstation'
    exit 2
}

. (Join-Path $PSScriptRoot 'restore.ps1')
$script:DeferProbeCleanup = $true

$probeDir = 'C:\ProgramData\os-lock-probe'
$pendingPath = Join-Path $probeDir 'pending.json'
$script:Hosted = ($env:GITHUB_ACTIONS -eq 'true' -and $env:RUNNER_ENVIRONMENT -eq 'github-hosted')
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

$script:PrivilegesReady = $false

function Enable-ProbePrivileges {
    if ($script:PrivilegesReady) { return }
    if (-not ('OsLockPriv' -as [type])) {
        $source = @'
using System;
using System.Runtime.InteropServices;
public class OsLockPriv {
    [DllImport("advapi32.dll", SetLastError=true)]
    public static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    public static extern bool LookupPrivilegeValue(string system, string name, out long luid);
    [DllImport("advapi32.dll", SetLastError=true)]
    public static extern bool AdjustTokenPrivileges(IntPtr token, bool disable, ref TokenPrivilege privilege, int length, IntPtr previous, IntPtr returned);
    [StructLayout(LayoutKind.Sequential)]
    public struct TokenPrivilege { public int Count; public long Luid; public int Attr; }
    public static void Enable(string name) {
        IntPtr token;
        if (!OpenProcessToken(System.Diagnostics.Process.GetCurrentProcess().Handle, 0x28, out token)) {
            throw new System.ComponentModel.Win32Exception();
        }
        long luid;
        if (!LookupPrivilegeValue(null, name, out luid)) {
            throw new System.ComponentModel.Win32Exception();
        }
        TokenPrivilege privilege = new TokenPrivilege();
        privilege.Count = 1;
        privilege.Luid = luid;
        privilege.Attr = 2;
        if (!AdjustTokenPrivileges(token, false, ref privilege, 0, IntPtr.Zero, IntPtr.Zero)) {
            throw new System.ComponentModel.Win32Exception();
        }
    }
}
'@
        Add-Type -TypeDefinition $source
    }
    [OsLockPriv]::Enable('SeRestorePrivilege')
    [OsLockPriv]::Enable('SeTakeOwnershipPrivilege')
    $script:PrivilegesReady = $true
}

function New-ProbeDirectorySecurity {
    $acl = New-Object System.Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    $system = New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-18'
    $admin = New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-32-544'
    $rights = [System.Security.AccessControl.FileSystemRights]::FullControl
    $inherit = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $prop = [System.Security.AccessControl.PropagationFlags]::None
    $allow = [System.Security.AccessControl.AccessControlType]::Allow
    $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($system, $rights, $inherit, $prop, $allow)))
    $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($admin, $rights, $inherit, $prop, $allow)))
    $acl.SetOwner($admin)
    return $acl
}

function New-ProbeFileSecurity {
    $acl = New-Object System.Security.AccessControl.FileSecurity
    $acl.SetAccessRuleProtection($true, $false)
    $system = New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-18'
    $admin = New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-32-544'
    $rights = [System.Security.AccessControl.FileSystemRights]::FullControl
    $inherit = [System.Security.AccessControl.InheritanceFlags]::None
    $prop = [System.Security.AccessControl.PropagationFlags]::None
    $allow = [System.Security.AccessControl.AccessControlType]::Allow
    $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($system, $rights, $inherit, $prop, $allow)))
    $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($admin, $rights, $inherit, $prop, $allow)))
    $acl.SetOwner($admin)
    return $acl
}

function Write-ProbeFile([string]$path, [byte[]]$bytes) {
    $name = [System.IO.Path]::GetFileName($path)
    if (-not (Test-KnownProbeName $name)) { throw 'refusing to write an unexpected probe file' }
    if (-not (Test-ProbeDirTrusted)) { throw 'refusing: probe directory is not owned by SYSTEM or Administrators' }
    if (Test-ReparsePath $path) { throw 'probe file is a reparse point' }
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Force
    }
    if (Test-Path -LiteralPath $path) { throw 'probe file could not be replaced' }
    Enable-ProbePrivileges
    $security = New-ProbeFileSecurity
    $rights = [System.Security.AccessControl.FileSystemRights]::FullControl
    $stream = [System.IO.FileStream]::new($path, [System.IO.FileMode]::CreateNew, $rights, [System.IO.FileShare]::None, 4096, [System.IO.FileOptions]::None, $security)
    try {
        if ($bytes.Length -gt 0) { $stream.Write($bytes, 0, $bytes.Length) }
    } finally {
        $stream.Dispose()
    }
    if (-not (Test-OwnerTrusted $path)) { throw 'refusing: probe file is not owned by SYSTEM or Administrators' }
}

function Initialize-ProbeDir {
    if (Test-ReparsePath 'C:\ProgramData') { throw 'ProgramData is a reparse point' }
    if (Test-Path -LiteralPath $probeDir) {
        if (-not (Test-ProbeDirTrusted)) { throw 'refusing: probe directory is not owned by SYSTEM or Administrators' }
    } else {
        Enable-ProbePrivileges
        $security = New-ProbeDirectorySecurity
        [System.IO.Directory]::CreateDirectory($probeDir, $security) | Out-Null
        if (-not (Test-ProbeDirTrusted)) { throw 'refusing: probe directory was not created for SYSTEM and Administrators' }
    }
    $bytes = [System.IO.File]::ReadAllBytes($restoreScript)
    Write-ProbeFile (Join-Path $probeDir 'restore.ps1') $bytes
}

function Register-RunOnceEntry {
    $installed = Join-Path $probeDir 'restore.ps1'
    $run = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce')
    $command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$installed`" -Journal `"$pendingPath`""
    $run.SetValue($runOnceName, $command)
    $run.Close()
}

function Register-RestoreTaskEntry {
    $installed = Join-Path $probeDir 'restore.ps1'
    $cmd = Join-Path $probeDir 'restore.cmd'
    if (-not (Test-OwnerTrusted $installed)) { throw 'refusing to register an untrusted restore script' }
    $line = "@echo off`r`npowershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$installed`" -Journal `"$pendingPath`"`r`n"
    $utf8 = New-Object System.Text.UTF8Encoding $false
    Write-ProbeFile $cmd $utf8.GetBytes($line)
    if (-not (Test-OwnerTrusted $cmd)) { throw 'refusing to register an untrusted restore command' }
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
    $items = @()
    foreach ($entry in (Get-Items $entries)) {
        if (-not (Test-AllowedEntry $entry)) { throw 'refusing to journal a value outside the lockdown allow-list' }
        if ($entry.kind -and [string]$entry.kind -ne 'DWord') { throw 'refusing to journal a non-DWORD value' }
        $present = if ($entry.present -eq $true) { 'true' } else { 'false' }
        $data = '0'
        if ($entry.present -eq $true) {
            $number = 0
            if (-not [int]::TryParse([string]$entry.data, [ref]$number)) { throw 'refusing to journal a non-DWORD value' }
            $data = "$number"
        }
        $items += '{"hive":"' + [string]$entry.hive + '","subkey":"' + ([string]$entry.subkey).Replace('\', '\\') + '","name":"' + [string]$entry.name + '","present":' + $present + ',"kind":"DWord","data":"' + $data + '","created_key":false}'
    }
    $body = '{"sid":"' + (Current-Sid) + '","values":[' + ($items -join ',') + ']}'
    $utf8 = New-Object System.Text.UTF8Encoding $false
    Write-ProbeFile $pendingPath $utf8.GetBytes($body)
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
        if ($kind -ne 'DWord') { throw "refusing to change non-DWORD value $name" }
        $raw = $key.GetValue($existing, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $data = "$([int]$raw)"
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

function Test-TaskPresent {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & schtasks.exe /Query /TN $taskName 2>&1 | Out-Null
        return $LASTEXITCODE -eq 0
    } finally { $ErrorActionPreference = $previous }
}

function Start-SystemRestore {
    if (-not (Test-ProbeDirTrusted)) { throw 'refusing to run the restore task from an untrusted probe directory' }
    if (-not (Test-OwnerTrusted (Join-Path $probeDir 'restore.ps1'))) { throw 'refusing to run an untrusted restore script' }
    if (-not (Test-OwnerTrusted (Join-Path $probeDir 'restore.cmd'))) { throw 'refusing to run an untrusted restore command' }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & schtasks.exe /Run /TN $taskName 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'schtasks /Run failed' }
    } finally { $ErrorActionPreference = $previous }
}

function Set-ProbeAway($entry) {
    $current = $null
    if ($entry.present -eq $true) { $current = [int]$entry.data }
    $away = 7
    if ($current -eq 7) { $away = 6 }
    Set-Dword ([string]$entry.hive) ([string]$entry.subkey) ([string]$entry.name) $away
}

function Wait-TaskStays([int]$seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    do {
        if (-not (Test-TaskPresent)) { throw 'the SYSTEM task removed itself before HKLM was in the journal' }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
}

function Wait-TaskGone([int]$seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    do {
        if (-not (Test-TaskPresent)) { return }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'the SYSTEM task was left behind after HKLM was restored'
}

function Wait-PolicyValue($entry, [int]$seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    $wanted = $null
    if ($entry.present -eq $true) { $wanted = [int]$entry.data }
    do {
        $got = Read-Dword ([string]$entry.hive) ([string]$entry.subkey) ([string]$entry.name)
        if ($null -eq $wanted -and $null -eq $got) { return }
        if ($null -ne $wanted -and $null -ne $got -and $got -eq $wanted) { return }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "timed out waiting for $([string]$entry.name)"
}

function Clear-ProbeDirForTest {
    if (-not (Test-Path -LiteralPath $probeDir)) { return }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & takeown.exe /F $probeDir /A /R /D Y | Out-Null
    & icacls.exe $probeDir /grant '*S-1-5-32-544:(OI)(CI)F' /T | Out-Null
    & cmd.exe /c 'rmdir /s /q C:\ProgramData\os-lock-probe' | Out-Null
    $ErrorActionPreference = $previous
    if (Test-Path -LiteralPath $probeDir) { throw 'the plant directory could not be removed' }
}

function Grant-PlantBatchLogon([string]$user) {
    if (-not ('OsLockRights' -as [type])) {
        $source = @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public class OsLockRights {
    [StructLayout(LayoutKind.Sequential)]
    public struct LsaUnicodeString {
        public ushort Length;
        public ushort MaximumLength;
        public IntPtr Buffer;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct LsaObjectAttributes {
        public int Length;
        public IntPtr RootDirectory;
        public IntPtr ObjectName;
        public uint Attributes;
        public IntPtr SecurityDescriptor;
        public IntPtr SecurityQualityOfService;
    }
    [DllImport("advapi32.dll", SetLastError=true)]
    public static extern uint LsaOpenPolicy(IntPtr system, ref LsaObjectAttributes attributes, uint access, out IntPtr handle);
    [DllImport("advapi32.dll", SetLastError=true)]
    public static extern uint LsaAddAccountRights(IntPtr policy, IntPtr sid, LsaUnicodeString[] rights, uint count);
    [DllImport("advapi32.dll")]
    public static extern uint LsaClose(IntPtr handle);
    [DllImport("advapi32.dll")]
    public static extern uint LsaNtStatusToWinError(uint status);
    public static void Grant(byte[] sidBytes, string right) {
        LsaObjectAttributes attributes = new LsaObjectAttributes();
        attributes.Length = Marshal.SizeOf(typeof(LsaObjectAttributes));
        IntPtr policy;
        uint status = LsaOpenPolicy(IntPtr.Zero, ref attributes, 0x810, out policy);
        if (status != 0) { throw new Win32Exception((int)LsaNtStatusToWinError(status)); }
        IntPtr sid = Marshal.AllocHGlobal(sidBytes.Length);
        Marshal.Copy(sidBytes, 0, sid, sidBytes.Length);
        LsaUnicodeString name = new LsaUnicodeString();
        name.Buffer = Marshal.StringToHGlobalUni(right);
        name.Length = (ushort)(right.Length * 2);
        name.MaximumLength = (ushort)((right.Length + 1) * 2);
        try {
            status = LsaAddAccountRights(policy, sid, new LsaUnicodeString[] { name }, 1);
        } finally {
            Marshal.FreeHGlobal(name.Buffer);
            Marshal.FreeHGlobal(sid);
            LsaClose(policy);
        }
        if (status != 0) { throw new Win32Exception((int)LsaNtStatusToWinError(status)); }
    }
}
'@
        Add-Type -TypeDefinition $source
    }
    $account = New-Object System.Security.Principal.NTAccount($user)
    $sid = $account.Translate([System.Security.Principal.SecurityIdentifier])
    $bytes = New-Object byte[] $sid.BinaryLength
    $sid.GetBinaryForm($bytes, 0)
    [OsLockRights]::Grant($bytes, 'SeBatchLogonRight')
}

function Wait-PlantDir([int]$seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    do {
        if (Test-Path -LiteralPath $probeDir) { return $true }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    return (Test-Path -LiteralPath $probeDir)
}

function Invoke-PlantChecks {
    $plantUser = 'oslockplant'
    $plantPassword = 'OsLock-Plant9!'
    $plantTask = 'os-lock-plant'
    $plantScript = 'C:\Users\Public\os-lock-plant.ps1'
    $previous = $ErrorActionPreference
    try {
        Enable-ProbePrivileges
        Clear-ProbeDirForTest
        $ErrorActionPreference = 'Continue'
        $added = @(& net.exe user $plantUser $plantPassword /add 2>&1 | ForEach-Object { "$_" })
        if ($LASTEXITCODE -ne 0 -and (($added -join ' ') -notmatch 'already exists')) {
            $detail = $added -join ' '
            throw "could not create the plant user: $detail"
        }
        $ErrorActionPreference = 'Stop'
        Grant-PlantBatchLogon $plantUser
        $plantBody = "Set-StrictMode -Version Latest`r`n`$ErrorActionPreference = 'Stop'`r`nNew-Item -ItemType Directory -Path 'C:\ProgramData\os-lock-probe' | Out-Null`r`nSet-Content -LiteralPath 'C:\ProgramData\os-lock-probe\restore.ps1' -Value 'planted'`r`n"
        $utf8 = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($plantScript, $plantBody, $utf8)
        $ErrorActionPreference = 'Continue'
        & schtasks.exe /Delete /TN $plantTask /F 2>&1 | Out-Null
        & schtasks.exe /Create /TN $plantTask /SC ONCE /ST 00:00 /RU $plantUser /RP $plantPassword /RL LIMITED /F /TR "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\Users\Public\os-lock-plant.ps1" | Out-Null
        $scheduled = $LASTEXITCODE -eq 0
        if ($scheduled) {
            & schtasks.exe /Run /TN $plantTask | Out-Null
            $scheduled = $LASTEXITCODE -eq 0
        }
        $ErrorActionPreference = 'Stop'
        if ($scheduled) { $scheduled = Wait-PlantDir 8 }
        if (-not $scheduled) {
            $plantPasswordSecure = ConvertTo-SecureString $plantPassword -AsPlainText -Force
            $plantCred = New-Object System.Management.Automation.PSCredential(".\$plantUser", $plantPasswordSecure)
            $plantProc = Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $plantScript) -Credential $plantCred -Wait -PassThru -WindowStyle Hidden
            if ($null -eq $plantProc -or $plantProc.ExitCode -ne 0) { throw 'the standard user could not plant the probe directory' }
            if (-not (Wait-PlantDir 15)) { throw 'the standard user did not create the probe directory' }
        }
        $refused = $false
        try { Initialize-ProbeDir } catch { $refused = $true }
        if (-not $refused) { throw 'a user-owned probe directory was not refused' }
        Clear-ProbeDirForTest
        Initialize-ProbeDir
        $planted = Join-Path $probeDir 'restore.cmd'
        $utf8 = New-Object System.Text.UTF8Encoding $false
        Write-ProbeFile $planted $utf8.GetBytes("@echo off`r`n")
        $sid = (New-Object System.Security.Principal.NTAccount($plantUser)).Translate([System.Security.Principal.SecurityIdentifier])
        $acl = (Get-Item -LiteralPath $planted -Force).GetAccessControl()
        $acl.SetOwner($sid)
        Set-Acl -LiteralPath $planted -AclObject $acl
        $owner = [string]((Get-Item -LiteralPath $planted -Force).GetAccessControl().GetOwner([System.Security.Principal.SecurityIdentifier]).Value)
        if ($owner -eq 'S-1-5-18' -or $owner -eq 'S-1-5-32-544') { throw 'could not plant a user-owned file' }
        $refusedFile = $false
        try { Initialize-ProbeDir } catch { $refusedFile = $true }
        if (-not $refusedFile) { throw 'a user-owned file in the probe directory was not refused' }
    } finally {
        $ErrorActionPreference = 'Continue'
        try { Clear-ProbeDirForTest } catch { }
        if (Test-Path -LiteralPath $plantScript) { Remove-Item -LiteralPath $plantScript -Force }
        & schtasks.exe /Delete /TN $plantTask /F 2>&1 | Out-Null
        & cmd.exe /c "echo Y| net user $plantUser /delete" 2>&1 | Out-Null
        $ErrorActionPreference = $previous
    }
}

function Wait-ProbeDirGone([int]$seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($seconds)
    do {
        if (-not (Test-Path -LiteralPath $probeDir)) { return }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'probe directory was left behind after HKLM was restored'
}

function Invoke-SelfCheck {
    if (-not (Test-Elevated)) { throw 'self-check needs an elevated prompt' }
    if (-not $script:Hosted) { throw 'self-check runs only on a GitHub-hosted runner' }
    if (Test-Path -LiteralPath $pendingPath) { throw 'a restore is already pending; run restore.ps1 before --self-check' }
    Invoke-PlantChecks
    Initialize-ProbeDir
    $rows = @(
        @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableTaskMgr' },
        @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableLockWorkstation' },
        @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'DisableChangePassword' },
        @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; name = 'NoLogoff' },
        @{ hive = 'HKCU'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'; name = 'NoClose' },
        @{ hive = 'HKLM'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'HideFastUserSwitching' },
        @{ hive = 'HKLM'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'ConsentPromptBehaviorAdmin' },
        @{ hive = 'HKLM'; subkey = 'Software\Microsoft\Windows\CurrentVersion\Policies\System'; name = 'ConsentPromptBehaviorUser' }
    )
    $saved = @()
    foreach ($row in $rows) { $saved += (Read-Original $row.hive $row.subkey $row.name) }
    $hkcuOnly = @()
    foreach ($entry in $saved) { if ([string]$entry.hive -eq 'HKCU') { $hkcuOnly += $entry } }
    $evilKey = [Microsoft.Win32.Registry]::LocalMachine
    try {
        $decoyDir = Join-Path $env:LOCALAPPDATA 'os-lock-probe'
        New-Item -ItemType Directory -Force -Path $decoyDir | Out-Null
        $utf8 = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText((Join-Path $decoyDir 'pending.json'), '{"sid":"S-1-5-18","values":[{"hive":"HKCU","subkey":"Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\System","name":"DisableLockWorkstation","present":true,"kind":"DWord","data":"9","created_key":false}]}', $utf8)

        Save-PendingFile $hkcuOnly
        Register-RunOnceEntry
        Register-RestoreTaskEntry
        foreach ($entry in $hkcuOnly) { Set-ProbeAway $entry }
        Start-SystemRestore
        foreach ($entry in $hkcuOnly) { Wait-PolicyValue $entry 20 }
        Wait-TaskStays 3
        $locked = Read-Dword 'HKCU' 'Software\Microsoft\Windows\CurrentVersion\Policies\System' 'DisableLockWorkstation'
        if ($locked -eq 9) { throw 'SYSTEM restore read the user-profile journal' }

        Save-PendingFile $saved
        $text = [System.IO.File]::ReadAllText($pendingPath)
        $evil = ',{"hive":"HKLM","subkey":"Software\\os-lock-evil","name":"Evil","present":true,"kind":"DWord","data":"5","created_key":false}'
        if (-not $text.EndsWith(']}')) { throw 'journal shape was not written by this probe' }
        $text = $text.Substring(0, $text.Length - 2) + $evil + ']}'
        Write-ProbeFile $pendingPath $utf8.GetBytes($text)
        foreach ($entry in $saved) { Set-ProbeAway $entry }
        $evilRoot = $evilKey.CreateSubKey('Software\os-lock-evil')
        $evilRoot.SetValue('Evil', 1, [Microsoft.Win32.RegistryValueKind]::DWord)
        $evilRoot.Close()
        if (-not (Test-TaskPresent)) { Register-RestoreTaskEntry }
        Start-SystemRestore
        foreach ($entry in $saved) { Wait-PolicyValue $entry 20 }
        $evilLeft = Read-Dword 'HKLM' 'Software\os-lock-evil' 'Evil'
        if ($evilLeft -ne 1) { throw 'restore applied a value outside the allow-list' }
        Wait-TaskGone 20
        Wait-ProbeDirGone 20

        Initialize-ProbeDir
        Save-PendingFile $saved
        Register-RunOnceEntry
        Set-Dword 'HKCU' 'Software\Microsoft\Windows\CurrentVersion\Policies\System' 'DisableLockWorkstation' 7
        $run = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\RunOnce', $false)
        if ($null -eq $run) { throw 'RunOnce was not registered' }
        $command = [string]$run.GetValue($runOnceName, '')
        $run.Close()
        if ($command -notlike '*C:\ProgramData\os-lock-probe\restore.ps1*') { throw 'RunOnce does not call the protected script' }
        if ($command -notlike '*-Journal *C:\ProgramData\os-lock-probe\pending.json*') { throw 'RunOnce does not pass the journal path' }
        if (-not (Test-ProbeDirTrusted)) { throw 'refusing to run the restore command from an untrusted probe directory' }
        if (-not (Test-OwnerTrusted (Join-Path $probeDir 'restore.ps1'))) { throw 'refusing to run an untrusted restore script' }
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $probeDir 'restore.ps1') -Journal $pendingPath
            if ($LASTEXITCODE -ne 0) { throw 'RunOnce command failed' }
        } finally { $ErrorActionPreference = $previous }
        $lockEntry = $null
        foreach ($entry in $saved) { if ([string]$entry.name -eq 'DisableLockWorkstation') { $lockEntry = $entry } }
        Wait-PolicyValue $lockEntry 20
        Write-Host 'self-check ok'
    } finally {
        try { $evilKey.DeleteSubKeyTree('Software\os-lock-evil', $false) } catch { }
        if (-not (Test-Path -LiteralPath $probeDir)) {
            try { Initialize-ProbeDir } catch { }
        }
        try { Save-PendingFile $saved } catch { }
        try { Invoke-PendingRestore | Out-Null } catch { }
        try { Remove-ProbeFolder } catch { }
        try { $evilKey.DeleteSubKeyTree('Software\os-lock-evil', $false) } catch { }
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
    if ($exe -like 'C:\ProgramData\os-lock-probe\*') {
        $dirIssue = Get-ProbeDirIssue
        $fileIssue = Get-TrustIssue $exe
        if ($dirIssue -or $fileIssue) {
            throw "refusing to run a probe executable that is not owned by SYSTEM or Administrators ($dirIssue) ($fileIssue)"
        }
    }
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
    Write-Host "timing $id api=$($parsed.api_ok) notified=$($parsed.notified) kind=$kind elapsed_us=$elapsed win32=$($parsed.win32) session_locked=$($parsed.session_locked)"
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
    $path = Join-Path $probeDir 'reconnect.on'
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
    Initialize-ProbeDir
    $protected = Join-Path $probeDir 'os-lock-proof.exe'
    Write-ProbeFile $protected ([System.IO.File]::ReadAllBytes($exe))
    $exe = $protected
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null

    $pre = Invoke-Probe 'preflight'
    if ($pre.code -ne 0 -or $null -eq $pre.json) { throw "preflight failed: $($pre.text)" }
    if ($pre.json.identity_unreadable -eq $true) { throw 'workstation identity could not be read' }
    if (-not $pre.json.elevated) { Write-SkipReport 'needs-admin' 'not admin' }
    if (-not $pre.json.console_present) { Write-SkipReport 'not-applicable' 'no console session' }
    if ($pre.json.remote -eq $true) { Write-SkipReport 'not-applicable' 'remote session' }
    if (-not $pre.json.console_user_known) { throw 'could not read the console session user' }
    if (-not $pre.json.console_user_match) { throw 'elevated user is not the console session user' }
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
    $dirIssue = Get-ProbeDirIssue
    $fileIssue = Get-TrustIssue $exe
    if ($dirIssue -or $fileIssue) {
        throw "refusing to run a probe executable that is not owned by SYSTEM or Administrators ($dirIssue) ($fileIssue)"
    }
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

    if ($script:Hosted) {
        $service = Invoke-Probe 'install-service' '--watch-pid' "$PID" '--allow-local'
        $serviceUp = $service.code -eq 0
        if ($serviceUp) {
            [System.IO.File]::WriteAllText((Join-Path $probeDir 'reconnect.on'), "on`n")
            $watch.Restart()
            $fromService = Invoke-Probe 'service-disconnect' '--allow-local'
            Add-ProbeRow 'service-disconnect' 'Disconnect the console session from session 0' $fromService ([int]$watch.ElapsedMilliseconds)
            $back = Wait-Connected
            $connected = $back.json -and $back.json.connected -eq $true
            if ($connected) {
                $watch.Restart()
                $own = Invoke-Probe 'measure-self' '--allow-local' '--reconnect'
                Add-ProbeRow 'own-session' 'Disconnect this session' $own ([int]$watch.ElapsedMilliseconds)
                Wait-Connected | Out-Null
            } else {
                Add-Row 'own-session' 'Disconnect this session' 'fail' $null $true 0 'session stayed disconnected' '{}'
            }
        } else {
            $why = $service.text
            if (-not $why) { $why = 'the session 0 service did not start' }
            Add-Row 'service-disconnect' 'Disconnect the console session from session 0' 'fail' $null $true 0 $why '{}'
            Add-Row 'own-session' 'Disconnect this session' 'fail' $null $true 0 'the session 0 service was not running, so disconnect was not called' '{}'
        }
    } else {
        Add-Row 'service-disconnect' 'Disconnect the console session from session 0' 'skip' 'not-applicable' $true 0 'reconnect is ci only' '{}'
        Add-Row 'own-session' 'Disconnect this session' 'skip' 'not-applicable' $true 0 'reconnect is ci only' '{}'
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
        try { Remove-ProbeFolder } catch { }
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
        $state = 'never-unlocked'
        try { Write-ProofReport $script:Restored $state $primary $fallback } catch { Write-Host $_.Exception.Message }
        if ($script:MustEndLocked) {
            $watchEnd = [Diagnostics.Stopwatch]::StartNew()
            $end = Invoke-Probe 'fail-closed' '--allow-local'
            $endMs = [int]$watchEnd.ElapsedMilliseconds
            $confirmed = $end.json -and ($end.json.pass -eq $true) -and ([string]$end.json.error -ne 'fail-closed-failed')
            if ($confirmed) {
                Add-Row 'end-locked' 'End at the Windows sign-in screen' 'pass' $null $true $endMs '' (Json-Measure $end.json)
                $state = 'os-locked'
            } else {
                $detail = if ($end) { $end.text } else { 'fail-closed-failed' }
                Add-Row 'fail-closed-failed' 'Fail closed' 'fail' $null $true $endMs $detail '{}'
                $state = 'never-unlocked'
            }
            try { Write-ProofReport $script:Restored $state $primary $fallback } catch { Write-Host $_.Exception.Message }
        }
        if ($script:Restored) { try { Remove-ProbeFolder } catch { } }
        $anyFail = $false
        foreach ($row in (Get-Items $script:Rows)) { if ($row.status -eq 'fail') { $anyFail = $true } }
        if ($script:Failed -or -not $script:Restored -or $anyFail) { $script:ExitCode = 1 } else { $script:ExitCode = 0 }
    }
}

exit $script:ExitCode
