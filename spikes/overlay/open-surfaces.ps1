# Opens each shell surface in turn while the pink overlay is up.
# The overlay does not block the mouse or the keyboard.
# Exit the pink layer with Ctrl+Alt+Shift+Q.
# It also closes itself after the timeout (default 5 minutes).
#
# Surfaces, in order:
#   Start                 injected Win key
#   TaskView              injected Win+Tab
#   NotificationCenter    injected Win+N on Windows 11, Win+A on Windows 10
#   Toast                 a toast banner
#   Uac                   consent prompt for "cmd.exe /c exit" (click No, never Yes)
#   Fullscreen            green exclusive window (Esc, or it exits within 30 seconds)
#   Monitor               injected Win+P. On the school PC, also plug a monitor in.

[CmdletBinding()]
param(
    [int]$PauseSecs = 20,
    [int]$TimeoutSecs = 300,
    [switch]$ReassertTimer,
    [switch]$ReassertPos,
    [switch]$CloseShell,
    [switch]$SuppressToasts,
    [string]$Only = ""
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$exe = Join-Path $root "target\debug\spike-overlay.exe"
if (-not (Test-Path $exe)) {
    Write-Error "Build it first: cargo build -p softlock-spike-overlay"
}

$tour = @(
    "tour",
    "--pause-secs", "$PauseSecs",
    "--timeout-secs", "$TimeoutSecs"
)
if ($ReassertTimer) { $tour += "--reassert-timer" }
if ($ReassertPos) { $tour += "--reassert-pos" }
if ($CloseShell) { $tour += "--close-shell" }
if ($SuppressToasts) { $tour += "--suppress-toasts" }
if ($Only -ne "") { $tour += @("--only", $Only) }

& $exe @tour
exit $LASTEXITCODE
