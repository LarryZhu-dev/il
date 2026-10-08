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
$containerArguments = @('run', '--rm', '--platform', 'linux/amd64', '--mount', "type=bind,source=$workspace,target=/workspace", '--mount', 'type=volume,source=il-cargo-registry,target=/usr/local/cargo/registry', '--workdir', '/workspace', 'il-toolchain:1.90.0-llvm14')
switch ($Action) {
    'check' { $containerArguments += @('python3', 'tools/ci/run.py') }
    'test' { $containerArguments += @('cargo', 'test', '--workspace', '--locked'); $containerArguments += $ToolArguments }
    'build' { $containerArguments += @('cargo', 'build', '--workspace', '--locked'); $containerArguments += $ToolArguments }
    'il' {
        $containerArguments = @('run', '--rm', '-i') + $containerArguments[2..($containerArguments.Length - 1)]
        $containerArguments += @('cargo', 'run', '--locked', '--bin', 'il', '--')
        $containerArguments += $ToolArguments
    }
}
if ($Action -eq 'il') {
    $OutputEncoding = [System.Text.UTF8Encoding]::new($false)
    $input | docker @containerArguments
} else {
    docker @containerArguments
}
exit $LASTEXITCODE
