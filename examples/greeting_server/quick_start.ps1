# Quick start script for greeting server in Docker
# Run from the il project root directory

$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "il Greeting Server - Docker Quick Start" -ForegroundColor Cyan
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

Write-Host "Step 1: Building il CLI in Docker..." -ForegroundColor Green
Write-Host "----------------------------------------"
& ./tools/dev.ps1 build

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Step 2: Building greeting server..." -ForegroundColor Green
Write-Host "----------------------------------------"

# Use il CLI through dev.ps1 to build in container
Write-Host "Creating transaction file..."

$buildScript = @'
cd /workspace/examples/greeting_server
python3 -c "
import json
from pathlib import Path

# Read sources
modules = ['core', 'alloc', 'io', 'time', 'net', 'json', 'http', 'tracing']
package_sources = []
for name in modules:
    pkg_file = Path(f'/workspace/packages/{name}/lib.il')
    package_sources.append(pkg_file.read_text(encoding='utf-8'))

main_source = Path('/workspace/examples/greeting_server/main.il').read_text(encoding='utf-8')
full_source = '\\n'.join(package_sources) + '\\n' + main_source

# Create transaction
transact = {
    'task_id': 'BUILD_GREETING_SERVER',
    'base_revision': 0,
    'scope': ['program'],
    'operations': [{'op': 'import_text', 'source': full_source}],
    'required_checks': []
}

Path('/workspace/examples/greeting_server/build').mkdir(exist_ok=True)
Path('/workspace/examples/greeting_server/build/transact.json').write_text(
    json.dumps(transact, ensure_ascii=False, indent=2) + '\\n', encoding='utf-8'
)
print('Transaction created')
"
'@

$result = & ./tools/dev.ps1 il -RequestLine "bash -c `"$buildScript`""

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Failed to create transaction. Manual steps:" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Run these commands to build manually:" -ForegroundColor Cyan
    Write-Host "  cd examples/greeting_server" -ForegroundColor White
    Write-Host "  docker run --rm -v `"`${PWD}:/workspace`" -w /workspace il-bootstrap python3 build_and_run.py build" -ForegroundColor White
    Write-Host ""
    exit 1
}

Write-Host ""
Write-Host "Step 3: Compiling with il CLI..." -ForegroundColor Green
Write-Host "----------------------------------------"

$compileScript = @'
cd /workspace/examples/greeting_server/build
/workspace/target/debug/il transact /workspace state.msgpack transact.json
/workspace/target/debug/il compile /workspace state.msgpack /workspace/examples/greeting_server/build/greeting_server
'@

$result = & ./tools/dev.ps1 il -RequestLine "bash -c `"$compileScript`""

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Compilation failed. Check the output above for errors." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Build completed successfully!" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "The server executable is at:" -ForegroundColor Yellow
Write-Host "  examples/greeting_server/build/greeting_server" -ForegroundColor White
Write-Host ""
Write-Host "To run the server:" -ForegroundColor Yellow
Write-Host "  cd examples/greeting_server" -ForegroundColor White
Write-Host "  docker run --rm -v `"`${PWD}:/workspace`" -w /workspace il-bootstrap python3 build_and_run.py run --host 0.0.0.0" -ForegroundColor White
Write-Host ""
Write-Host "Or use the build_and_run.py script:" -ForegroundColor Yellow
Write-Host "  python3 build_and_run.py run" -ForegroundColor White
Write-Host ""
