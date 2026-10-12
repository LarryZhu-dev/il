#!/usr/bin/env pwsh
# Compile and run il API server in one step

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

# Convert Windows path to Docker-compatible format for bind mounts
# D:\me\project\il -> /d/me/project/il
$dockerWorkspace = $workspace -replace '\\', '/'
$driveLetter = $dockerWorkspace.Substring(0, 1).ToLower()
$dockerWorkspace = '/' + $driveLetter + $dockerWorkspace.Substring(2)

Write-Host "Compiling il API server..." -ForegroundColor Cyan
Write-Host "  Source: examples/api_server.il" -ForegroundColor Gray
Write-Host ""

# Step 1: Import .il source to graph store
Write-Host "Step 1: Importing source to graph store..." -ForegroundColor Yellow

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
    cargo run --locked --bin il -- --store /workspace/.il/api_server transact

if ($LASTEXITCODE -ne 0) {
    Write-Host "Import failed" -ForegroundColor Red
    exit 1
}

Write-Host "Source imported successfully" -ForegroundColor Green
Write-Host ""

# Step 2: Build to native executable
Write-Host "Step 2: Building native Linux executable..." -ForegroundColor Yellow

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
    cargo run --locked --bin il -- --store /workspace/.il/api_server build

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed" -ForegroundColor Red
    exit 1
}

Write-Host "Build successful" -ForegroundColor Green
Write-Host ""

# Step 3: Run the server in Docker
Write-Host "Step 3: Starting API server..." -ForegroundColor Yellow
Write-Host ""
Write-Host "Server listening on http://localhost:8080" -ForegroundColor Cyan
Write-Host ""
Write-Host "Test the API in another terminal:" -ForegroundColor Cyan
Write-Host "  curl http://localhost:8080/health" -ForegroundColor Gray
Write-Host "  curl http://localhost:8080/hello" -ForegroundColor Gray
Write-Host "  curl http://localhost:8080/time" -ForegroundColor Gray
Write-Host "  curl http://localhost:8080/capabilities" -ForegroundColor Gray
Write-Host ""
Write-Host "Press Ctrl+C to stop the server" -ForegroundColor Yellow
Write-Host ""

# Run the server (map port to host)
docker run --rm -it --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --workdir /workspace `
    --publish 8080:8080 `
    il-toolchain:1.90.0-llvm14 `
    /workspace/.il/api_server/build/x86_64-unknown-linux-gnu/release/executable

$serverExit = $LASTEXITCODE
Write-Host ""
Write-Host "Server stopped (exit code: $serverExit)" -ForegroundColor Yellow
exit $serverExit
