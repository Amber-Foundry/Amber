# Copy ONNX Runtime shared libraries from the Cargo target directory into
# core/resources/onnxruntime/ for Tauri bundle.resources verification.
#
# Usage (from repo root):
#   powershell -ExecutionPolicy Bypass -File scripts/stage-onnxruntime.ps1
#
# Prerequisite: run `cargo build --manifest-path core/Cargo.toml` first so `ort`
# can download/copy runtime libraries into core/target/.

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$destDir = Join-Path $repoRoot "core\resources\onnxruntime"
$searchRoots = @(
    (Join-Path $repoRoot "core\target\debug\deps"),
    (Join-Path $repoRoot "core\target\debug"),
    (Join-Path $repoRoot "core\target\release\deps"),
    (Join-Path $repoRoot "core\target\release")
)

New-Item -ItemType Directory -Force -Path $destDir | Out-Null

$patterns = @("onnxruntime*.dll", "libonnxruntime*.dylib", "libonnxruntime*.so")
$found = @()

foreach ($root in $searchRoots) {
    if (-not (Test-Path $root)) {
        continue
    }
    foreach ($pattern in $patterns) {
        $found += Get-ChildItem -Path $root -Filter $pattern -File -ErrorAction SilentlyContinue
    }
}

$unique = $found | Sort-Object FullName -Unique

if ($unique.Count -eq 0) {
    Write-Host "No ONNX Runtime shared libraries found under core/target/."
    Write-Host "Run: cargo build --manifest-path core/Cargo.toml"
    Write-Host "If libraries still do not appear, copy platform runtime files into:"
    Write-Host "  $destDir"
    exit 1
}

foreach ($file in $unique) {
    $target = Join-Path $destDir $file.Name
    Copy-Item -Path $file.FullName -Destination $target -Force
    Write-Host "Staged $($file.Name)"
}

Write-Host "Staged $($unique.Count) file(s) to $destDir"
