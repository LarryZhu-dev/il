#!/usr/bin/env pwsh
# Compile il API server to native Linux executable

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

# Convert Windows path to Docker-compatible format for bind mounts
# D:\me\project\il -> /d/me/project/il
$dockerWorkspace = $workspace -replace '\\', '/' -replace '^([A-Z]):', { '/'+$_.Groups[1].Value.ToLower() }

Write-Host "Compiling il API server..." -ForegroundColor Cyan
Write-Host "  Source: examples/api_server.il" -ForegroundColor Gray
Write-Host ""

# Step 1: Import source code to graph store
Write-Host "Step 1/2: Importing source to graph store..." -ForegroundColor Yellow

$sourceCode = Get-Content "$workspace/examples/api_server.il" -Raw -Encoding utf8

$importRequest = @{
    task_id = 'api_server'
    base_revision = 0
    scope = @('program')
    operations = @(@{
        op = 'import_text'
        source = $sourceCode
    })
    required_checks = @()
} | ConvertTo-Json -Depth 10

$importResult = $importRequest | docker run --rm -i --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --mount "type=volume,source=il-cargo-registry,target=/usr/local/cargo/registry" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    cargo run --locked --bin il -- --store /workspace/.il/api_server transact 2>&1

if ($LASTEXITCODE -ne 0) {
    Write-Host "Import failed" -ForegroundColor Red
    Write-Host $importResult
    exit 1
}

Write-Host "Source imported successfully" -ForegroundColor Green
Write-Host ""

# Step 2: Build to native executable
Write-Host "Step 2/2: Building native executable..." -ForegroundColor Yellow

$buildRequest = @{
    revision = 1
    target = 'x86_64-unknown-linux-gnu'
    profile = 'release'
} | ConvertTo-Json

$buildResult = $buildRequest | docker run --rm -i --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --mount "type=volume,source=il-cargo-registry,target=/usr/local/cargo/registry" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    cargo run --locked --bin il -- --store /workspace/.il/api_server build 2>&1

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed" -ForegroundColor Red
    Write-Host $buildResult
    exit 1
}

Write-Host "Build successful!" -ForegroundColor Green
Write-Host ""
Write-Host "Executable created at:" -ForegroundColor Cyan
Write-Host "  .il/api_server/build/x86_64-unknown-linux-gnu/release/executable" -ForegroundColor Gray
Write-Host ""
Write-Host "To start the server:" -ForegroundColor Cyan
Write-Host "  ./examples/start_api_server.ps1" -ForegroundColor Gray
Write-Host ""
