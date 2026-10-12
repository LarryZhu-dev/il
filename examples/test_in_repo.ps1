#!/usr/bin/env pwsh
# Test il compiler using the current repository context

$ErrorActionPreference = 'Stop'

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path.Replace('\', '/')

Write-Host "Testing il compiler in repository context..." -ForegroundColor Cyan
Write-Host "Workspace: $workspace" -ForegroundColor Gray
Write-Host ""

# Simple il source
$source = 'service greeter { fn greet(name: text) -> text { concat("Hello, ", name, "!") } }'

Write-Host "Step 1: Importing source to graph store..." -ForegroundColor Yellow

$importRequest = @{
    base_revision = 0
    operations = @(@{
        op = 'import_text'
        source = $source
    })
} | ConvertTo-Json -Depth 10 -Compress

$result = $importRequest | docker run --rm -i --platform linux/amd64 `
    --mount "type=bind,source=$workspace,target=/workspace" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    /workspace/target/release/il --store /workspace/.il/test transact

Write-Host ""
Write-Host "Result:" -ForegroundColor Cyan
$resultObj = $result | ConvertFrom-Json
$resultObj | ConvertTo-Json -Depth 10

if ($resultObj.ok) {
    Write-Host ""
    Write-Host "Success! Graph revision: $($resultObj.result_revision)" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Host "Error during import" -ForegroundColor Red
}
