#!/usr/bin/env pwsh
# Initialize il store and test compilation

$ErrorActionPreference = 'Stop'

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path.Replace('\', '/')

Write-Host "Initializing il store and testing compilation..." -ForegroundColor Cyan
Write-Host ""

# Ensure we're in a clean Git state for the test
Push-Location (Join-Path $PSScriptRoot '..')

# Create .il directory if it doesn't exist
$storePath = ".il/test"
if (!(Test-Path $storePath)) {
    New-Item -ItemType Directory -Path $storePath -Force | Out-Null
}

Pop-Location

Write-Host "Step 1: Initialize store with base revision 0..." -ForegroundColor Yellow

# Simple il source
$source = 'service greeter { fn greet(name: text) -> text { concat("Hello, ", name, "!") } }'

$importRequest = @{
    base_revision = 0
    operations = @(@{
        op = 'import_text'
        source = $source
    })
} | ConvertTo-Json -Depth 10 -Compress

Write-Host "Workspace: $workspace"
Write-Host "Store: /workspace/.il/test"
Write-Host ""

$result = $importRequest | docker run --rm -i --platform linux/amd64 `
    --mount "type=bind,source=$workspace,target=/workspace" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    /workspace/target/release/il --store /workspace/.il/test transact

Write-Host "Result:" -ForegroundColor Cyan
$resultObj = $result | ConvertFrom-Json
$resultObj | ConvertTo-Json -Depth 10

Write-Host ""
if ($resultObj.ok) {
    Write-Host "✓ Import successful! Revision: $($resultObj.result_revision)" -ForegroundColor Green

    # Now try to compile
    Write-Host ""
    Write-Host "Step 2: Compile the service..." -ForegroundColor Yellow

    $compileRequest = @{
        base_revision = $resultObj.result_revision
        operations = @(@{
            op = 'compile'
            target = 'greeter'
        })
    } | ConvertTo-Json -Depth 10 -Compress

    $compileResult = $compileRequest | docker run --rm -i --platform linux/amd64 `
        --mount "type=bind,source=$workspace,target=/workspace" `
        --workdir /workspace `
        il-toolchain:1.90.0-llvm14 `
        /workspace/target/release/il --store /workspace/.il/test transact

    Write-Host ""
    Write-Host "Compile Result:" -ForegroundColor Cyan
    $compileObj = $compileResult | ConvertFrom-Json
    $compileObj | ConvertTo-Json -Depth 10

    if ($compileObj.ok) {
        Write-Host ""
        Write-Host "✓ Compilation successful!" -ForegroundColor Green
    } else {
        Write-Host ""
        Write-Host "✗ Compilation failed" -ForegroundColor Red
    }
} else {
    Write-Host "✗ Import failed" -ForegroundColor Red
}
