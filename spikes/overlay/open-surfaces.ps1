# Opens each shell surface in turn while the pink overlay is up.
# The overlay does not block the mouse or the keyboard.
# Exit the pink layer with Ctrl+Alt+Shift+Q.
# It also closes itself after the timeout (default 5 minutes).
#
# Put spike-overlay.exe in the same folder as this script. That is how it
# runs on the school PC, where Rust is not installed. If the exe is not
# beside the script, the script looks in target\release and then target\debug
# of this repo.
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
    [switch]$AllowAdmin,
    [string]$Only = ""
)

$ErrorActionPreference = "Stop"
$beside = Join-Path $PSScriptRoot "spike-overlay.exe"
if (Test-Path $beside) {
    $exe = $beside
} else {
    $repo = Join-Path $PSScriptRoot "..\.."
    $release = Join-Path $repo "target\release\spike-overlay.exe"
    $debug = Join-Path $repo "target\debug\spike-overlay.exe"
    if (Test-Path $release) {
        $exe = $release
    } elseif (Test-Path $debug) {
        $exe = $debug
    } else {
        Write-Error "Copy spike-overlay.exe into the same folder as this script. On the laptop, build it with: cargo build --release -p softlock-spike-overlay"
    }
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
if ($AllowAdmin) { $tour += "--allow-admin" }
if ($Only -ne "") { $tour += @("--only", $Only) }

& $exe @tour
exit $LASTEXITCODE
