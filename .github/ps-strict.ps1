# Dot-source at the top of GitHub Actions PowerShell steps so native tools
# (cargo, npm, sc.exe, …) fail the step when they return a non-zero exit code.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true