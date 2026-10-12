$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

# Convert Windows path to Docker format
$dockerWorkspace = $workspace -replace '\\', '/'
$driveLetter = $dockerWorkspace.Substring(0, 1).ToLower()
$dockerWorkspace = '/' + $driveLetter + $dockerWorkspace.Substring(2)

Write-Host "Workspace: $workspace"
Write-Host "Docker path: $dockerWorkspace"

docker run --rm --platform linux/amd64 `
    --mount "type=bind,source=$dockerWorkspace,target=/workspace" `
    --workdir /workspace `
    il-toolchain:1.90.0-llvm14 `
    /bin/bash -c 'ls -la /workspace/examples/ && head -20 /workspace/examples/api_server.il'
