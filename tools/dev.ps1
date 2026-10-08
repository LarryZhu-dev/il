param(
    [ValidateSet('image', 'check', 'test', 'build', 'il')]
    [string]$Action = 'check',
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ToolArguments
)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($Action -eq 'image') {
    docker build --tag il-toolchain:1.90.0-llvm14 --file (Join-Path $PSScriptRoot 'Dockerfile') $workspace
    exit $LASTEXITCODE
}
$containerArguments = @('run', '--rm', '--platform', 'linux/amd64', '--mount', "type=bind,source=$workspace,target=/workspace", '--workdir', '/workspace', 'il-toolchain:1.90.0-llvm14')
switch ($Action) {
    'check' { $containerArguments += @('python3', 'tools/ci/run.py') }
    'test' { $containerArguments += @('cargo', 'test', '--workspace', '--locked'); $containerArguments += $ToolArguments }
    'build' { $containerArguments += @('cargo', 'build', '--workspace', '--locked'); $containerArguments += $ToolArguments }
    'il' { $containerArguments += @('cargo', 'run', '--locked', '--bin', 'il', '--'); $containerArguments += $ToolArguments }
}
docker @containerArguments
exit $LASTEXITCODE
