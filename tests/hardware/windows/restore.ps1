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

function Open-Root([string]$hive) {
    if ($hive -eq 'HKLM') { return [Microsoft.Win32.Registry]::LocalMachine }
    return [Microsoft.Win32.Registry]::CurrentUser
}

function Value-Matches($entry) {
    $root = Open-Root ([string]$entry.hive)
    $key = $root.OpenSubKey([string]$entry.subkey, $false)
    if ($null -eq $key) { return -not $entry.present }
    $exists = $false
    $kind = ''
    $data = ''
    foreach ($existing in @($key.GetValueNames())) {
        if ($existing -ne [string]$entry.name) { continue }
        $exists = $true
        $kind = $key.GetValueKind($existing).ToString()
        $raw = $key.GetValue($existing, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($kind -eq 'Binary') { $data = [Convert]::ToBase64String([byte[]]$raw) }
        elseif ($kind -eq 'MultiString') { $data = ([string[]]$raw) -join "`n" }
        else { $data = [string]$raw }
    }
    $key.Close()
    if (-not ($entry.present -eq $true)) { return -not $exists }
    if (-not $exists) { return $false }
    $kindName = 'DWord'
    if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
    return ($kind -eq $kindName) -and ($data -eq [string]$entry.data)
}

function Restore-Command($entry) {
    $path = "$($entry.hive)\$($entry.subkey)"
    $name = [string]$entry.name
    if (-not ($entry.present -eq $true)) { return "reg delete `"$path`" /v $name /f" }
    $kindName = 'DWord'
    if ($entry.PSObject.Properties.Name -contains 'kind' -and $entry.kind) { $kindName = [string]$entry.kind }
    $type = switch ($kindName) {
        'QWord' { 'REG_QWORD' }
        'String' { 'REG_SZ' }
        'ExpandString' { 'REG_EXPAND_SZ' }
        'MultiString' { 'REG_MULTI_SZ' }
        'Binary' { 'REG_BINARY' }
        default { 'REG_DWORD' }
    }
    return "reg add `"$path`" /v $name /t $type /d $($entry.data) /f"
}

function Normalize-Guid([string]$guid) {
    if (-not $guid) { return '' }
    $trimmed = $guid.Trim().Trim('{}')
    if ($trimmed -match '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$') {
        return $trimmed.ToLower()
    }
    return ''
}

function Get-ActiveScheme {
    try {
        $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes')
        if ($null -ne $key) {
            $guid = Normalize-Guid ([string]$key.GetValue('ActivePowerScheme'))
            $key.Close()
            if ($guid) { return $guid }
        }
    } catch { }
    $text = Read-Native { powercfg /GETACTIVESCHEME }
    $match = [regex]::Match($text, '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}')
    if ($match.Success) { return $match.Value.ToLower() }
    return ''
}

function Read-DwordAt([string]$path, [string]$name) {
    try { $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($path) } catch { return $null }
    if ($null -eq $key) { return $null }
    $value = $key.GetValue($name, $null)
    $key.Close()
    if ($null -eq $value) { return $null }
    return [int]$value
}

function Read-PowerIndex([string]$setting, [string]$scheme) {
    if (-not $scheme) { $scheme = Get-ActiveScheme }
    if (-not $scheme) { return $null }
    # Each index uses its own per-user override, then that index's scheme default.
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    $user = "SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes\$scheme\$sub\$setting"
    $fallback = "SYSTEM\CurrentControlSet\Control\Power\PowerSettings\$sub\$setting\DefaultPowerSchemeValues\$scheme"
    $ac = Read-DwordAt $user 'ACSettingIndex'
    if ($null -eq $ac) { $ac = Read-DwordAt $fallback 'ACSettingIndex' }
    $dc = Read-DwordAt $user 'DCSettingIndex'
    if ($null -eq $dc) { $dc = Read-DwordAt $fallback 'DCSettingIndex' }
    if ($null -eq $ac -or $null -eq $dc) { return $null }
    return [pscustomobject]@{ scheme = $scheme; ac = [int]$ac; dc = [int]$dc }
}

function Test-ShouldSetActive([string]$saved, [string]$current) {
    $a = Normalize-Guid $saved
    $b = Normalize-Guid $current
    return ($a -ne '' -and $a -eq $b)
}

function Query-PowerText([string]$scheme, [string]$setting) {
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    $hidden = Read-Native { powercfg /qh $scheme $sub }
    if ($LASTEXITCODE -eq 0 -and $hidden -match [regex]::Escape($setting)) { return $hidden }
    $named = Read-Native { powercfg /q $scheme $sub $setting }
    if ($named -match [regex]::Escape($setting)) { return $named }
    return Read-Native { powercfg /q $scheme $sub }
}

function Parse-QueryIndex([string]$text, [string]$setting) {
    if (-not $text -or -not $setting) { return $null }
    $needle = $setting.Trim().Trim('{}')
    $at = $text.ToLowerInvariant().IndexOf($needle.ToLowerInvariant())
    if ($at -lt 0) { return $null }
    $rest = $text.Substring($at + $needle.Length)
    $next = [regex]::Match($rest, '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}')
    if ($next.Success) { $rest = $rest.Substring(0, $next.Index) }
    $found = [regex]::Matches($rest, '0x([0-9a-fA-F]+)')
    if ($found.Count -lt 2) { return $null }
    $ac = [Convert]::ToInt32($found[$found.Count - 2].Groups[1].Value, 16)
    $dc = [Convert]::ToInt32($found[$found.Count - 1].Groups[1].Value, 16)
    return [pscustomobject]@{ ac = $ac; dc = $dc }
}

function Invoke-SetActive([string]$scheme) {
    $text = Read-Native { powercfg /SETACTIVE $scheme }
    if ($LASTEXITCODE -ne 0) { return "powercfg SETACTIVE failed: $text" }
    return ''
}

function Confirm-LiveIndex([string]$setting, [int]$ac, [int]$dc, [string]$scheme) {
    $parsed = Parse-QueryIndex (Query-PowerText $scheme $setting) $setting
    if ($null -eq $parsed) {
        return "powercfg query of $setting on $scheme did not include AC and DC"
    }
    if ($parsed.ac -ne $ac -or $parsed.dc -ne $dc) {
        return "in-effect ac=$($parsed.ac) dc=$($parsed.dc) wanted ac=$ac dc=$dc scheme=$scheme"
    }
    return ''
}

function Set-PowerIndex([string]$setting, [int]$ac, [int]$dc, [string]$scheme, [bool]$reactivate) {
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    if (-not $scheme) { $scheme = Get-ActiveScheme }
    if (-not $scheme) { return 'active power scheme was not found' }
    $acText = Read-Native { powercfg /SETACVALUEINDEX $scheme $sub $setting $ac }
    if ($LASTEXITCODE -ne 0) { return "powercfg AC failed: $acText" }
    $dcText = Read-Native { powercfg /SETDCVALUEINDEX $scheme $sub $setting $dc }
    if ($LASTEXITCODE -ne 0) { return "powercfg DC failed: $dcText" }
    if ($reactivate) {
        $err = Invoke-SetActive $scheme
        if ($err) { return $err }
    }
    $after = Read-PowerIndex $setting $scheme
    if ($null -eq $after -or $after.ac -ne $ac -or $after.dc -ne $dc) {
        $got = if ($null -eq $after) { 'missing' } else { "ac=$($after.ac) dc=$($after.dc)" }
        return "read-back $got wanted ac=$ac dc=$dc scheme=$scheme"
    }
    return ''
}

function Test-CreatedKey($entry) {
    return ($entry.PSObject.Properties.Name -contains 'created_key') -and ($entry.created_key -eq $true)
}

function Remove-CreatedKey($entry) {
    if (-not (Test-CreatedKey $entry)) { return }
    $root = Open-Root ([string]$entry.hive)
    $path = [string]$entry.subkey
    $key = $root.OpenSubKey($path, $true)
    if ($null -eq $key) { return }
    $empty = ($key.ValueCount -eq 0 -and $key.SubKeyCount -eq 0)
    $key.Close()
    if (-not $empty) { return }
    $slash = $path.LastIndexOf('\')
    if ($slash -lt 0) {
        $root.DeleteSubKey($path, $false)
        return
    }
    $parent = $root.OpenSubKey($path.Substring(0, $slash), $true)
    if ($null -eq $parent) { return }
    try { $parent.DeleteSubKey($path.Substring($slash + 1), $false) } finally { $parent.Close() }
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
    if (Value-Matches $entry) {
        try { Remove-CreatedKey $entry } catch {
            $failed = $true
            Write-Output "$($entry.hive)\$($entry.subkey) was created by the suite and was not deleted. $($_.Exception.Message)"
            $held += $entry
        }
        continue
    }
    $root = Open-Root ([string]$entry.hive)
    try {
        $key = $root.OpenSubKey([string]$entry.subkey, $true)
        if ($null -eq $key) {
            if (-not $entry.present) { continue }
            $key = $root.CreateSubKey([string]$entry.subkey)
        }
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
        if (-not (Value-Matches $entry)) {
            if ($created) { throw 'the suite created this value and it is still present' }
            throw 'the saved value was not put back'
        }
        Remove-CreatedKey $entry
    } catch {
        $failed = $true
        $verb = if ($entry.present -eq $true) { 'was changed and was not put back' } else { 'was created by the suite and was not deleted' }
        Write-Output "$($entry.hive)\$($entry.subkey) value $($entry.name) $verb. $($_.Exception.Message). Run: $(Restore-Command $entry)"
        $held += $entry
    }
}

$powerLeft = @()
foreach ($entry in $power) {
    $sub = '4f971e89-eebd-4455-a8de-9e59040e7347'
    $setting = [string]$entry.setting
    $savedScheme = ''
    if ($entry.PSObject.Properties.Name -contains 'scheme') { $savedScheme = Normalize-Guid ([string]$entry.scheme) }
    $current = Read-PowerIndex $setting $savedScheme
    $scheme = if ($savedScheme) { $savedScheme } else { Get-ActiveScheme }
    $reactivate = Test-ShouldSetActive $scheme (Get-ActiveScheme)
    $err = ''
    if ($null -ne $current -and $current.ac -eq [int]$entry.ac -and $current.dc -eq [int]$entry.dc) {
        if ($reactivate) {
            $err = Invoke-SetActive $scheme
            if (-not $err) { $err = Confirm-LiveIndex $setting ([int]$entry.ac) ([int]$entry.dc) $scheme }
        }
    } else {
        $err = Set-PowerIndex $setting ([int]$entry.ac) ([int]$entry.dc) $scheme $reactivate
        if (-not $err -and $reactivate) { $err = Confirm-LiveIndex $setting ([int]$entry.ac) ([int]$entry.dc) $scheme }
    }
    if ($err) {
        $failed = $true
        $cmd = "powercfg /SETACVALUEINDEX $scheme $sub $setting $($entry.ac) & powercfg /SETDCVALUEINDEX $scheme $sub $setting $($entry.dc)"
        if ($reactivate) { $cmd += " & powercfg /SETACTIVE $scheme" }
        Write-Output "power $setting was changed and was not put back. $err. Run: $cmd"
        $powerLeft += $entry
    }
}

if ($held.Count -gt 0 -or $powerLeft.Count -gt 0) {
    $items = @()
    foreach ($entry in $held) {
        $present = if ($entry.present) { 'true' } else { 'false' }
        $kind = if ($entry.PSObject.Properties.Name -contains 'kind') { [string]$entry.kind } else { 'DWord' }
        $data = if ($entry.PSObject.Properties.Name -contains 'data') { [string]$entry.data } else { '0' }
        $created = 'false'
        if ($entry.PSObject.Properties.Name -contains 'created_key' -and ($entry.created_key -eq $true)) { $created = 'true' }
        $items += '{"hive":"' + $entry.hive + '","subkey":"' + ([string]$entry.subkey).Replace('\', '\\').Replace('"', '\"') + '","name":"' + $entry.name + '","present":' + $present + ',"kind":"' + $kind + '","data":"' + ($data.Replace('\', '\\').Replace('"', '\"')) + '","created_key":' + $created + '}'
    }
    $powerJson = @()
    foreach ($entry in $powerLeft) {
        $scheme = ''
        if ($entry.PSObject.Properties.Name -contains 'scheme' -and $entry.scheme) { $scheme = [string]$entry.scheme }
        $active = 'true'
        if ($entry.PSObject.Properties.Name -contains 'active' -and -not $entry.active) { $active = 'false' }
        $powerJson += '{"setting":"' + $entry.setting + '","ac":' + [int]$entry.ac + ',"dc":' + [int]$entry.dc + ',"scheme":"' + $scheme.Replace('\', '\\').Replace('"', '\"') + '","active":' + $active + '}'
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
