#!/usr/bin/env pwsh
# Start pre-compiled il API server

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

# Convert Windows path to Docker format
$dockerWorkspace = $workspace -replace '\\', '/'
$driveLetter = $dockerWorkspace.Substring(0, 1).ToLower()
$dockerWorkspace = '/' + $driveLetter + $dockerWorkspace.Substring(2)

$executablePath = "$workspace/.il/api_server/build/x86_64-unknown-linux-gnu/release/executable"

if (-not (Test-Path $executablePath)) {
    Write-Host "Executable not found: $executablePath" -ForegroundColor Red
    Write-Host ""
    Write-Host "Please compile first:" -ForegroundColor Yellow
    Write-Host "  ./examples/run_api_server.ps1" -ForegroundColor Gray
    Write-Host ""
    Write-Host "Or use the full compile workflow:" -ForegroundColor Yellow
    Write-Host "  ./examples/compile_api_server.ps1" -ForegroundColor Gray
    exit 1
}

Write-Host "Starting il API server..." -ForegroundColor Cyan
Write-Host ""
Write-Host "Server listening on http://localhost:8080" -ForegroundColor Cyan
Write-Host ""
Write-Host "Test the API:" -ForegroundColor Cyan
Write-Host "  curl http://localhost:8080/health" -ForegroundColor Gray
Write-Host "  curl http://localhost:8080/hello" -ForegroundColor Gray
Write-Host "  curl http://localhost:8080/time" -ForegroundColor Gray
Write-Host "  curl http://localhost:8080/capabilities" -ForegroundColor Gray
Write-Host ""
Write-Host "Press Ctrl+C to stop the server" -ForegroundColor Yellow
Write-Host ""

docker run --rm -it --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --workdir /workspace `
    --publish 8080:8080 `
    il-toolchain:1.90.0-llvm14 `
    /workspace/.il/api_server/build/x86_64-unknown-linux-gnu/release/executable

$exitCode = $LASTEXITCODE
Write-Host ""
if ($exitCode -eq 0) {
    Write-Host "Server stopped gracefully" -ForegroundColor Green
} else {
    Write-Host "Server exited with code: $exitCode" -ForegroundColor Yellow
}
exit $exitCode
