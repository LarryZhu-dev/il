# Complete workflow: build, run, test
# Run from the il project root directory

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "il Greeting Server - Complete Workflow" -ForegroundColor Cyan
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

# Step 1: Build il CLI
Write-Host "Step 1/4: Building il CLI..." -ForegroundColor Green
Write-Host "----------------------------------------"
& ./tools/dev.ps1 build

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "il CLI build failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "il CLI build successful" -ForegroundColor Green

# Step 2: Build server
Write-Host ""
Write-Host "Step 2/4: Building greeting server..." -ForegroundColor Green
Write-Host "----------------------------------------"

docker run --rm `
    -v "${workspace}:/workspace" `
    -w /workspace/examples/greeting_server `
    il-toolchain:1.90.0-llvm14 `
    python3 build_and_run.py build

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Server build failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Server build successful" -ForegroundColor Green

# Step 3: Start server in background
Write-Host ""
Write-Host "Step 3/4: Starting server..." -ForegroundColor Green
Write-Host "----------------------------------------"

$serverJob = Start-Job -ScriptBlock {
    param($workspace)
    docker run --rm `
        -v "${workspace}:/workspace" `
        -w /workspace/examples/greeting_server `
        -p "8080:8080" `
        il-toolchain:1.90.0-llvm14 `
        python3 build_and_run.py run --host 0.0.0.0 --port 8080
} -ArgumentList $workspace

Write-Host "Server starting (Job ID: $($serverJob.Id))..."
Write-Host "Waiting 3 seconds for server to be ready..."
Start-Sleep -Seconds 3

if ($serverJob.State -eq "Running") {
    Write-Host ""
    Write-Host "Server is running" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Host "Server failed to start" -ForegroundColor Red
    Stop-Job -Job $serverJob
    Remove-Job -Job $serverJob
    exit 1
}

# Step 4: Run tests
Write-Host ""
Write-Host "Step 4/4: Running tests..." -ForegroundColor Green
Write-Host "----------------------------------------"

docker run --rm `
    -v "${workspace}:/workspace" `
    -w /workspace/examples/greeting_server `
    --network host `
    il-toolchain:1.90.0-llvm14 `
    python3 test_client.py --host localhost --port 8080

$testResult = $LASTEXITCODE

# Stop server
Write-Host ""
Write-Host "Stopping server..." -ForegroundColor Yellow
Stop-Job -Job $serverJob
Remove-Job -Job $serverJob

# Final result
Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan

if ($testResult -eq 0) {
    Write-Host "All tests passed!" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Server is ready to use!" -ForegroundColor Green
    Write-Host ""
    Write-Host "To start the server:" -ForegroundColor White
    Write-Host "  powershell -ExecutionPolicy Bypass -File ./examples/greeting_server/run_server.ps1" -ForegroundColor Gray
    Write-Host ""
    exit 0
} else {
    Write-Host "Tests failed" -ForegroundColor Red
    Write-Host "========================================" -ForegroundColor Cyan
    exit 1
}
