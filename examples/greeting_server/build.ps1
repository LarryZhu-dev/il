# Greeting Server 构建和运行脚本

param(
    [Parameter(Position = 0)]
    [ValidateSet('compile', 'run', 'test', 'all')]
    [string]$Action = 'all'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path

Write-Host "==> Greeting Server Build Script" -ForegroundColor Cyan
Write-Host "Workspace: $workspace" -ForegroundColor Gray

function Invoke-Compile {
    Write-Host "`n[1/3] 发布源代码到图存储..." -ForegroundColor Yellow

    $source = Get-Content (Join-Path $PSScriptRoot 'main.il') -Raw -Encoding utf8
    $transactRequest = @{
        task_id = 'greeting-server'
        base_revision = 0
        scope = @('program')
        operations = @(@{
            op = 'import_text'
            source = $source
        })
        required_checks = @()
    } | ConvertTo-Json -Depth 10 -Compress

    $transactResult = $transactRequest | & "$workspace/tools/dev.ps1" il --store /workspace/.il/greeting transact
    if ($LASTEXITCODE -ne 0) {
        throw "图事务失败"
    }
    Write-Host "✓ 源代码已发布" -ForegroundColor Green

    Write-Host "`n[2/3] 编译为 Linux 可执行文件..." -ForegroundColor Yellow

    $buildRequest = @{
        revision = 1
        target = 'x86_64-unknown-linux-gnu'
        profile = 'release'
    } | ConvertTo-Json -Compress

    $buildResult = $buildRequest | & "$workspace/tools/dev.ps1" il --store /workspace/.il/greeting build
    if ($LASTEXITCODE -ne 0) {
        throw "编译失败"
    }

    $buildOutput = $buildResult | ConvertFrom-Json
    $execPath = $buildOutput.result.native.artifacts.executable.path
    Write-Host "✓ 编译完成: $execPath" -ForegroundColor Green

    Write-Host "`n[3/3] 验证可执行文件..." -ForegroundColor Yellow
    docker run --rm --platform linux/amd64 `
        --mount "type=bind,source=$workspace,target=/workspace" `
        --workdir /workspace `
        il-toolchain:1.90.0-llvm14 `
        file $execPath

    Write-Host "✓ 构建完成！" -ForegroundColor Green
    return $execPath
}

function Invoke-Run {
    param([string]$ExecPath)

    Write-Host "`n==> 启动 Greeting Server" -ForegroundColor Cyan
    Write-Host "监听地址: http://127.0.0.1:8080" -ForegroundColor Gray
    Write-Host "路由: GET /greeting" -ForegroundColor Gray
    Write-Host "按 Ctrl+C 停止服务`n" -ForegroundColor Gray

    docker run --rm -it `
        --platform linux/amd64 `
        --mount "type=bind,source=$workspace,target=/workspace" `
        --workdir /workspace `
        --publish 8080:8080 `
        il-toolchain:1.90.0-llvm14 `
        $ExecPath `
        --host-policy /workspace/examples/greeting_server/greeting_policy.json
}

function Invoke-Test {
    Write-Host "`n==> 测试 Greeting Server" -ForegroundColor Cyan

    Write-Host "测试 GET /greeting ..." -ForegroundColor Yellow
    $response = curl.exe -s http://127.0.0.1:8080/greeting
    if ($response -eq "hi, this is a server based il") {
        Write-Host "✓ /greeting 测试通过" -ForegroundColor Green
    } else {
        Write-Host "✗ /greeting 测试失败: $response" -ForegroundColor Red
        exit 1
    }

    Write-Host "测试 404 错误 ..." -ForegroundColor Yellow
    $statusCode = curl.exe -s -o $null -w "%{http_code}" http://127.0.0.1:8080/notfound
    if ($statusCode -eq "404") {
        Write-Host "✓ 404 测试通过" -ForegroundColor Green
    } else {
        Write-Host "✗ 404 测试失败: HTTP $statusCode" -ForegroundColor Red
        exit 1
    }

    Write-Host "`n✓ 所有测试通过！" -ForegroundColor Green
}

# 主流程
try {
    switch ($Action) {
        'compile' {
            $execPath = Invoke-Compile
            Write-Host "`n要运行服务，执行: ./build.ps1 run" -ForegroundColor Cyan
        }
        'run' {
            $execPath = '/workspace/.il/greeting/target/x86_64-unknown-linux-gnu/release/program'
            Invoke-Run -ExecPath $execPath
        }
        'test' {
            Invoke-Test
        }
        'all' {
            $execPath = Invoke-Compile
            Write-Host "`n按任意键启动服务..." -ForegroundColor Cyan
            $null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
            Invoke-Run -ExecPath $execPath
        }
    }
} catch {
    Write-Host "`n✗ 错误: $_" -ForegroundColor Red
    exit 1
}
