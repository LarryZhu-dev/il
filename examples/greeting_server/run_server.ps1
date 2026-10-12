# Run greeting server
# Run from the il project root directory

param(
    [string]$Host = "0.0.0.0",
    [int]$Port = 8080
)

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "il Greeting Server - Run" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Check if we're in the project root
if (-not (Test-Path "tools/dev.ps1")) {
    Write-Host "Error: Run this script from the il project root directory" -ForegroundColor Red
    exit 1
}

# Check if server executable exists
if (-not (Test-Path "examples/greeting_server/build/greeting_server")) {
    Write-Host "Error: Server executable not found" -ForegroundColor Red
    Write-Host "Please build the server first:" -ForegroundColor Yellow
    Write-Host "  powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/simple_start.ps1" -ForegroundColor White
    exit 1
}

$workspace = (Resolve-Path .).Path

Write-Host "Configuration:" -ForegroundColor Green
Write-Host "  Host: $Host"
Write-Host "  Port: $Port"
Write-Host ""
Write-Host "Starting server in container..." -ForegroundColor Green
Write-Host "Press Ctrl+C to stop" -ForegroundColor Yellow
Write-Host "----------------------------------------"

# Run server in container
docker run --rm -it `
    -v "${workspace}:/workspace" `
    -w /workspace/examples/greeting_server `
    -p "${Port}:${Port}" `
    il-toolchain:1.90.0-llvm14 `
    python3 build_and_run.py run --host $Host --port $Port
