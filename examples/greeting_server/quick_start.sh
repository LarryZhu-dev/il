#!/usr/bin/env bash
# Quick start script for greeting server in Docker

set -e

echo "========================================"
echo "il Greeting Server - Docker Quick Start"
echo "========================================"

# Check if we're in the project root
if [ ! -f "tools/dev.ps1" ]; then
    echo "Error: Run this script from the il project root directory"
    exit 1
fi

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "Error: Docker is not running"
    echo "Please start Docker Desktop and try again"
    exit 1
fi

echo ""
echo "Step 1: Building il CLI in Docker..."
echo "----------------------------------------"
powershell.exe -ExecutionPolicy Bypass -File ./tools/dev.ps1 build

echo ""
echo "Step 2: Starting interactive container..."
echo "----------------------------------------"
echo ""
echo "The container will start with the greeting server example."
echo "Inside the container, run:"
echo ""
echo "  cd examples/greeting_server"
echo "  python3 build_and_run.py build"
echo "  python3 build_and_run.py run --host 0.0.0.0"
echo ""
echo "Then from another terminal, test it:"
echo ""
echo "  docker exec -it il-dev curl http://localhost:8080/greeting"
echo ""
echo "Press Ctrl+D to exit the container."
echo ""

powershell.exe -ExecutionPolicy Bypass -File ./tools/dev.ps1 shell
