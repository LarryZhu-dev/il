# Simple start script for greeting server
# Run from the il project root directory

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "il Greeting Server - Simple Build" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Check if we're in the project root
if (-not (Test-Path "tools/dev.ps1")) {
    Write-Host "Error: Run this script from the il project root directory" -ForegroundColor Red
    exit 1
}

# Check if Docker is running
try {
    docker info > $null 2>&1
} catch {
    Write-Host "Error: Docker is not running" -ForegroundColor Red
    Write-Host "Please start Docker Desktop and try again" -ForegroundColor Yellow
    exit 1
}

$workspace = (Resolve-Path .).Path

Write-Host "Step 1: Building il CLI..." -ForegroundColor Green
Write-Host "----------------------------------------"
& ./tools/dev.ps1 build

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Step 2: Building greeting server executable..." -ForegroundColor Green
Write-Host "----------------------------------------"

# Run build script in container using docker directly
$buildCommand = "docker run --rm -v `"${workspace}:/workspace`" -w /workspace/examples/greeting_server il-toolchain:1.90.0-llvm14 python3 build_and_run.py build"

Write-Host "Running: docker run ... python3 build_and_run.py build" -ForegroundColor Gray

Invoke-Expression $buildCommand

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Build failed. Check the output above for errors." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Build completed successfully!" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "Server executable:" -ForegroundColor Yellow
Write-Host "  examples/greeting_server/build/greeting_server" -ForegroundColor White
Write-Host ""
Write-Host "To run the server:" -ForegroundColor Yellow
Write-Host "  cd examples/greeting_server" -ForegroundColor White
Write-Host "  python3 build_and_run.py run" -ForegroundColor White
Write-Host ""
Write-Host "Or with dev.ps1:" -ForegroundColor Yellow
Write-Host "  ./tools/dev.ps1 il -RequestLine 'python3 /workspace/examples/greeting_server/build_and_run.py run --host 0.0.0.0'" -ForegroundColor White
Write-Host ""
Write-Host "Test it:" -ForegroundColor Yellow
Write-Host "  ./tools/dev.ps1 il -RequestLine 'curl http://localhost:8080/greeting'" -ForegroundColor White
Write-Host ""
