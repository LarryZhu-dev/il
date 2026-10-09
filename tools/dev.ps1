[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Position = 0)]
    [ValidateSet('image', 'check', 'test', 'build', 'il')]
    [string]$Action = 'check',
    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$ToolArguments,
    [Parameter(ValueFromPipeline = $true)]
    [AllowEmptyString()]
    [string]$RequestLine
)
begin {
    $requestLines = [System.Collections.Generic.List[string]]::new()
}
process {
    if ($null -ne $RequestLine) { $requestLines.Add($RequestLine) }
}
end {
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
    $requestLines | docker @containerArguments
} else {
    docker @containerArguments
}
$actionExit = $LASTEXITCODE
if ($actionExit -eq 0 -and $Action -eq 'build') {
    $buildProfile = if ($ToolArguments -contains '--release') { 'release' } else { 'debug' }
    docker run --rm --platform linux/amd64 --mount "type=bind,source=$workspace,target=/workspace" --workdir /workspace il-toolchain:1.90.0-llvm14 python3 runtime/minimal/build.py --output "target/$buildProfile/libil_minimal_runtime.a"
    $actionExit = $LASTEXITCODE
}
exit $actionExit
}
