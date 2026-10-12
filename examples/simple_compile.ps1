#!/usr/bin/env pwsh
# Simple il compiler test

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

# Convert Windows path to Docker format
$dockerWorkspace = $workspace -replace '\\', '/'
$driveLetter = $dockerWorkspace.Substring(0, 1).ToLower()
$dockerWorkspace = '/' + $driveLetter + $dockerWorkspace.Substring(2)

Write-Host "Testing il compiler..." -ForegroundColor Cyan
Write-Host ""

# First, ensure il compiler is built in Docker
Write-Host "Building il compiler in Docker..." -ForegroundColor Yellow
docker run --rm --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --mount "type=volume,source=il-cargo-registry,target=/usr/local/cargo/registry" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    cargo build --locked --release --bin il

if ($LASTEXITCODE -ne 0) {
    Write-Host "Compiler build failed" -ForegroundColor Red
    exit 1
}

Write-Host "Compiler built successfully" -ForegroundColor Green
Write-Host ""

# Now test the compiler
Write-Host "Testing compiler version..." -ForegroundColor Yellow
docker run --rm --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    /workspace/target/release/il --version

Write-Host ""
Write-Host "Compiler is ready!" -ForegroundColor Green
