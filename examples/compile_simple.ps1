#!/usr/bin/env pwsh
# Simple il compiler test without Git binding requirements

$ErrorActionPreference = 'Stop'

Write-Host "Testing il compiler with simple service..." -ForegroundColor Cyan

# Create a temporary directory for the store
$tempDir = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory -Path $_ }
$storePath = Join-Path $tempDir ".il-store"
New-Item -ItemType Directory -Path $storePath -Force | Out-Null

# Initialize a minimal Git repo in the temp directory to satisfy il's requirements
Push-Location $tempDir
git init | Out-Null
git config user.email "test@example.com" | Out-Null
git config user.name "Test User" | Out-Null

# Create a simple il source file
$source = @'
service greeter {
    fn greet(name: text) -> text {
        concat("Hello, ", name, "!")
    }
}
'@

$sourceFile = Join-Path $tempDir "test.il"
$source | Out-File -FilePath $sourceFile -Encoding utf8 -NoNewline
git add . | Out-Null
git commit -m "Initial commit" | Out-Null

Pop-Location

# Convert to Docker path
$dockerTempDir = $tempDir -replace '\\', '/'
$driveLetter = $dockerTempDir.Substring(0, 1).ToLower()
$dockerTempDir = '/' + $driveLetter + $dockerTempDir.Substring(2)

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$dockerWorkspace = $workspace -replace '\\', '/'
$driveLetter2 = $dockerWorkspace.Substring(0, 1).ToLower()
$dockerWorkspace = '/' + $driveLetter2 + $dockerWorkspace.Substring(2)

Write-Host "Step 1: Importing source..." -ForegroundColor Yellow

$importRequest = @{
    base_revision = 0
    operations = @(@{
        op = 'import_text'
        source = $source
    })
} | ConvertTo-Json -Depth 10 -Compress

$result = $importRequest | docker run --rm -i --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --mount "type=bind,source=$dockerTempDir,target=/temp" `
    --workdir /temp `
    il-toolchain:1.90.0-llvm14 `
    /workspace/target/release/il --store /temp/.il-store transact

Write-Host ""
Write-Host "Result:" -ForegroundColor Cyan
$result | ConvertFrom-Json | ConvertTo-Json -Depth 10

# Cleanup
Remove-Item -Recurse -Force $tempDir

Write-Host ""
Write-Host "Test complete" -ForegroundColor Green
