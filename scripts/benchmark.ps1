# Measures MusicForge build-pipeline timings and stores them as a baseline.
# Usage:
#   ./scripts/benchmark.ps1 baseline   # record current timings
#   ./scripts/benchmark.ps1 compare    # compare against saved baseline
param([ValidateSet("baseline", "compare")] [string]$Mode = "baseline")

# Continue (not Stop): PowerShell 5.1 turns native stderr into error records;
# we check $LASTEXITCODE explicitly instead.
$ErrorActionPreference = "Continue"
$baseline = Join-Path $PSScriptRoot "..\.ecc\benchmarks\build.json"

function Get-CargoPath {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $fallback) { return $fallback }
    throw "cargo not found on PATH or at $fallback"
}
$cargo = Get-CargoPath

function Invoke-Measured([scriptblock]$Block) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Block 2>&1 | Out-Null
    $sw.Stop()
    if ($LASTEXITCODE -ne 0) { throw ("command failed with exit code $LASTEXITCODE") }
    return $sw.Elapsed.TotalMilliseconds
}

$npmBuild = Invoke-Measured { npm run build }
$cargoCheck = Invoke-Measured { & $cargo check --manifest-path src-tauri/Cargo.toml }

if ($Mode -eq "baseline") {
    $cargoTest = Invoke-Measured { & $cargo test --manifest-path src-tauri/Cargo.toml }
}

$current = @{
    project = "iwaks"
    updated = (Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ")
    metrics = @{
        npm_build_ms = [math]::Round($npmBuild)
        cargo_check_ms = [math]::Round($cargoCheck)
        cargo_test_ms = if ($Mode -eq "baseline") { [math]::Round($cargoTest) } else { $null }
    }
}

if ($Mode -eq "baseline") {
    $current | ConvertTo-Json -Depth 3 | Set-Content -Path $baseline -Encoding utf8
    Write-Host "Baseline saved to $baseline"
} else {
    if (-not (Test-Path $baseline)) { throw "No baseline yet. Run 'benchmark.ps1 baseline' first." }
    $saved = Get-Content $baseline -Raw | ConvertFrom-Json
    Write-Host "Metric          Before      Now"
    Write-Host "----------------------------"
    foreach ($k in $saved.metrics.PSObject.Properties.Name) {
        $before = $saved.metrics.$k
        $now = $current.metrics.$k
        $delta = if ($null -eq $before -or $null -eq $now) { "-" } elseif ($now -lt $before) { "+" + [math]::Round($before - $now) + "ms FASTER" } else { "-" + [math]::Round($now - $before) + "ms SLOWER" }
        Write-Host ("{0,-14} {1,8} {2,8}   {3}" -f $k, $before, $now, $delta)
    }
}