# Test greeting server
# Run from the il project root directory

param(
    [string]$Host = "localhost",
    [int]$Port = 8080
)

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "il Greeting Server - Test Suite" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Check if we're in the project root
if (-not (Test-Path "tools/dev.ps1")) {
    Write-Host "Error: Run this script from the il project root directory" -ForegroundColor Red
    exit 1
}

$workspace = (Resolve-Path .).Path

Write-Host "Running test suite..." -ForegroundColor Green
Write-Host "----------------------------------------"
Write-Host ""

# Run tests in container
docker run --rm `
    -v "${workspace}:/workspace" `
    -w /workspace/examples/greeting_server `
    --network host `
    il-toolchain:1.90.0-llvm14 `
    python3 test_client.py --host $Host --port $Port

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "All tests passed!" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Host "Tests failed" -ForegroundColor Red
    exit 1
}
