#!/usr/bin/env pwsh
# Test il API server endpoints

$ErrorActionPreference = 'Stop'

$baseUrl = "http://localhost:8080"
$endpoints = @("/health", "/hello", "/time", "/capabilities")

Write-Host "Testing il API server" -ForegroundColor Cyan
Write-Host "  Server URL: $baseUrl" -ForegroundColor Gray
Write-Host ""

$allPassed = $true

foreach ($endpoint in $endpoints) {
    $url = "$baseUrl$endpoint"
    Write-Host "Testing: GET $endpoint" -ForegroundColor Yellow

    try {
        $response = Invoke-WebRequest -Uri $url -Method Get -TimeoutSec 5

        if ($response.StatusCode -eq 200) {
            Write-Host "  Status: $($response.StatusCode)" -ForegroundColor Green

            # Try to parse JSON
            try {
                $json = $response.Content | ConvertFrom-Json
                Write-Host "  JSON parsed successfully" -ForegroundColor Green
                Write-Host "  Response:" -ForegroundColor Cyan
                Write-Host ($response.Content | ConvertFrom-Json | ConvertTo-Json -Depth 10) -ForegroundColor Gray
            } catch {
                Write-Host "  JSON parse failed: $_" -ForegroundColor Yellow
                Write-Host "  Raw response: $($response.Content)" -ForegroundColor Gray
            }
        } else {
            Write-Host "  Status: $($response.StatusCode)" -ForegroundColor Red
            $allPassed = $false
        }
    } catch {
        Write-Host "  Request failed: $_" -ForegroundColor Red
        $allPassed = $false
    }

    Write-Host ""
}

if ($allPassed) {
    Write-Host "All tests passed!" -ForegroundColor Green
    exit 0
} else {
    Write-Host "Some tests failed" -ForegroundColor Red
    exit 1
}
