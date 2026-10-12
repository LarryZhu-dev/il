# il Examples

This directory contains working examples demonstrating il language features and runtime capabilities.

## Available Examples

### 1. API Server (`api_server.il`)

A production-ready HTTP API server showcasing:
- HTTP request handling with routing
- JSON response serialization
- System capabilities (time, hostname)
- Health check endpoint
- Capability introspection

**Quick start:**
```powershell
# Compile and run in one command
./examples/run_api_server.ps1

# Or compile first, then run separately
./examples/compile_api_server.ps1
./examples/start_api_server.ps1
```

**Endpoints:**
- `GET /health` - Health check (returns 200 OK)
- `GET /hello` - Greeting message
- `GET /time` - Current server time
- `GET /capabilities` - List available system capabilities

**Test the server:**
```bash
curl http://localhost:8080/health
curl http://localhost:8080/hello
curl http://localhost:8080/time
curl http://localhost:8080/capabilities
```

See [api_server.md](api_server.md) for detailed architecture and implementation guide.

### 2. Greeting Server (`greeting_server/`)

A minimal HTTP server demonstrating basic request handling.

**Quick start:**
```bash
cd examples/greeting_server
python build.py
./server
