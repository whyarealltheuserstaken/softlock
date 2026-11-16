# Fail if the spike marker shows up in a product release binary.
$ErrorActionPreference = "Stop"
$marker = "SPIKE_P6_NOT_FOR_RELEASE"
$root = Join-Path (Get-Location) "target\release"
if (-not (Test-Path $root)) {
    throw "missing $root"
}

$files = @(Get-ChildItem -Path $root -Recurse -File | Where-Object {
    $_.Extension -in '.exe', '.dll', '.rlib'
})
if ($files.Count -eq 0) {
    throw "no release binaries to scan"
}

# Latin-1 keeps every byte, so the search works inside binaries.
$latin1 = [System.Text.Encoding]::GetEncoding(28591)
$spikeHits = @()
$productHits = @()
foreach ($file in $files) {
    $isSpike = $file.Name -match "hello-spike|hello_spike|softlock_hello_spike"
    $text = $latin1.GetString([System.IO.File]::ReadAllBytes($file.FullName))
    $hit = $text.Contains($marker)
    if ($isSpike -and $hit) {
        $spikeHits += $file.FullName
    }
    if (-not $isSpike -and $hit) {
        $productHits += $file.FullName
    }
}

if ($spikeHits.Count -eq 0) {
    throw "release marker was not found in the spike binary, so the scan cannot be trusted"
}
if ($productHits.Count -gt 0) {
    throw ("spike marker leaked into product binaries:`n" + ($productHits -join "`n"))
}
Write-Output "spike marker stayed inside the spike ($($spikeHits.Count) file(s))"
