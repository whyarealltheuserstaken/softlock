# Registers the background service for local development.
# Names come from the binary so this script does not spell them.
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary
)
$ErrorActionPreference = "Stop"
$lines = & $Binary --print-names
$name = ($lines | Where-Object { $_ -like "name=*" }) -replace "^name=", ""
$display = ($lines | Where-Object { $_ -like "display=*" }) -replace "^display=", ""
if (-not $name) { throw "the binary did not report a service name" }
$quoted = "`"$Binary`" --scm"
& sc.exe stop $name | Out-Null
& sc.exe delete $name | Out-Null
Start-Sleep -Milliseconds 400
& sc.exe create $name binPath= $quoted start= auto obj= LocalSystem DisplayName= $display
if ($LASTEXITCODE -ne 0) { throw "create failed" }
& sc.exe start $name
if ($LASTEXITCODE -ne 0) { throw "start failed" }
