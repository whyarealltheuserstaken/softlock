# Removes the development service registration.
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary
)
$ErrorActionPreference = "Stop"
$lines = & $Binary --print-names
$name = ($lines | Where-Object { $_ -like "name=*" }) -replace "^name=", ""
if (-not $name) { throw "the binary did not report a service name" }
& sc.exe stop $name | Out-Null
& sc.exe delete $name | Out-Null
